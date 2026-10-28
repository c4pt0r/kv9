# 迁移上传的 Publisher 生命周期

2026-10-05；基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
接续 [payload 副本接管](RAW-TTL-CAPTURE-TRANSFER.md)。此前 capture 只核对长期
Snapshot/Migration pins；本轮把原 source 和接管节点的实际对象任务分别登记、
排空并保存 PUT 报告。完整第二阶段和生产验收仍开放。

## 原子准入

`capture_migration_image` 在任何对象 I/O 前，按固定 capture descriptor、node、
store incarnation 和随机 process incarnation 派生独立 Publisher ID。它拥有与
原镜像完全相同的 SST closure，并使用连续 generation。准备新代前，上一代必须
已 Released，而且该 owner 的持久报告槽必须已确认清理。

实际 runtime 必须先预留节点共享 drain outbox 的一个槽位；缺队列或槽位拒绝。
预留按 owner ID 排斥并发 RPC，不允许两个本地 scope 共用同一代。取得槽位后先
复核原 owner 观察，若其他 RPC 已完成并推进代际，则释放尚未使用的槽位并拒绝
本次调用，不保存冲突报告。随后绑定真实 object execution identity，并通过现有
metadata-leader 路由提交
`EnterMigrationPublisher`。metadata 在同一已建立 snapshot 内重新核对不可变
capture catalog、完整 Publisher binding 和两个精确 Published image owners，
拒绝已提交 abort。在一个 metadata Raft batch 中写 Published Publisher、所有
资源 pin、不可变注册和该代准入 revision；确认回执之后才允许 PUT/GET。

普通 Acquire/Share/Publish/Quiesce/Release 不能操作 Publisher，不能绕过原子
准入或报告结束。准入的精确重试不新建代际；已结束代不能重新获得 I/O 权限。
登记失败或回执不确定时不发对象请求，但仍排空绑定 scope 并结束同一代，阻止迟到
的 Enter 在返回之后重新启用它。

## 结束事实和原保护

最外层 execution 等待所有实际排队/运行的对象任务退出。取消、超时和内层 uploader
返回都不能提前生成结束报告。嵌套上传继续归属同一 scope；原 transfer 文件的稳定
锁另有显式 lease，保持至最外层排空，group owner 也由实际调用保留。

scope 产生的 `DrainedExecution` 绑定原 Publisher token，并携带实际 issued/resolved
PUT 计数。runtime 先 fsync 原始 `FinishMigrationPublisher` 到预留槽位，再提交
metadata；metadata 在同一 batch 中将本地 Publisher 与全部 pin 改为 Released，
并保存该代结束 revision 和原计数。需要至少一个精确 Snapshot/Migration owner
仍 Published，覆盖相同的全部资源。它不释放这两个长期 owners。

准入从未提交的首代或紧邻下一代，只允许零 PUT 的终态 fence；报告中的
`admitted_at=0` 保留这一来源。实际进入过的代际保留原非零准入 revision。不能
把缺来源、缺报告、超时、进程退出或一个新代的成功报告当作旧代的零 PUT。

完成事实按 generation 保存。旧代已结束的精确回执重试可以幂等确认，不能修改
当前新代的 pin；修改计数、跨 scope/身份、跳代、缺记录和破损链接均拒绝。
点查询只返回所指定代的事实，不产生“所有 Publisher 都已结束”的证明。

PUT 报告描述本地任务观察，**不证明未知远端写入已经终止**。特别是非零
`issued == resolved` 也不替代对象后端的部署资格。本轮的 MemoryObjectStore
没有远端终止资格，因此测试中已发出的 PUT 保留 `resolved=0`。

## 报告恢复

结束回执丢失时，报告文件保留；即使 metadata 中 Publisher 已 Released，未确认
槽位仍阻止同一进程开启下一代。现有后台 sweep 可以在没有 uploader 的 metadata
follower 上补交这些报告。新进程使用新 identity，队列重开仍重放保存的原
store/process/generation 和原计数，不改写成新进程报告。实际报告未持久化就退出
的 scope 保持 Active/未知，不自动清理其 pin。

源节点的旧 PUT 未结束时，接管节点有自己的 Publisher。接管成功可以结束后者，
不会结束前者；随后 GET-only 验证产生的零 PUT 代，也不会覆盖接管时的旧非零报告。
当前 runtime 把 capture 内的 PUT 和校验 GET 统一放在这个 Publisher scope 中；
安装、startup 以及其他独立 Reader 路径仍须分别接入。

后续：[离线安装 Reader](RAW-TTL-INSTALL-READERS.md) 已接入 CLI 的旧镜像、新镜像
与最终校验；runtime adoption 和启动恢复继续开放。

## 编码与升级边界

| 内容 | 编码 |
| --- | --- |
| Ledger header | `KV9LED08`，reader 保留 01–07 |
| Owner kind | `Publisher = 9`，沿用 owner/resource row 格式 |
| 原子请求 | ledger tag 20 Enter、tag 21 Finish；沿用 16 KiB 小请求上限 |
| 逐代事实 | retention namespace tag 22，owner ID + generation key |
| 事实内容 | `KV9MPB01`，176 B，root、operation、token、binding digest、准入/结束 revision、PUT 计数与 checksum |

V7 ledger 成功 mutation 原子升级为 V8，原 checkpoint gate、birth、closure、refusal、
selection 和 owner rows 的含义保持不变。V8 Publisher 事实出现在较旧 header 下时
拒绝，不使用降版或空值恢复。旧 ledger reader 不接受 08；旧 outbox reader 也不能
把完整未知新请求当作可删除残片。

store lifecycle/internal service 仍是 V8，capture/catalog/image 编码未改变。这个
局部 ledger 格式边界不提供混合二进制或滚动升级保证；继续遵守停机升级要求，
完整旧 binary/部署演练仍未验收。此前已运行但未登记的上传者也不会被本轮补造为
“没有旧 I/O”的干净起点。

## 验证与剩余工作

[本轮证据包](raw-ttl-migration-publisher-validation-20261005/manifest.json) 保存实际
命令、原始日志和 source hashes；旧证据包不改写。本轮去重后 **294 个测试通过、
8 个既有测试忽略**；metadata 162、catalog 26、snapshot/install 24、TTL runtime 67、
transfer 7、outbox 8。新增 metadata 测试 5 个、runtime 测试 2 个，另扩展既有接管
与文件锁测试；workspace 全目标严格 Clippy、格式及 diff 检查通过。

- metadata 专项验证完整 Published 覆盖、固定 catalog/binding、通用接口不能绕过、
  稠密代际、取消与实际准入的区别、旧未知 PUT、迟到回执、不变计数、编码/降版/
  缺失/损坏拒绝。实际 WAL 尾部截断覆盖 Entry 和 Finish 的全有或全无及全部 pin。
- 真实三副本 Raft、分离 leader、鉴权 HTTP/2 duplex：Enter 前拒绝与提交后丢回执
  均零 PUT；实际 PUT 写入后报错，Finish 未提交/回执丢失时拒绝新代；重建进程
  context/outbox、切换 metadata leader 后，无 uploader 的后台补交原报告，WAL
  重开保持 `(issued=1, resolved=0)` 与两个 Published image pins。
- 原 source PUT 阻塞期间停止其 data driver，新 leader 从另一 voter 接管；分别
  核对两端 Publisher，旧 scope 不提前结束。后续 GET-only 零 PUT 代和 WAL 重开
  保留原 source/接管 Publisher 的旧非零报告。大镜像与既有 TTL 路径继续回归。
- 文件测试验证 journal 本身退出后，外层 lease 仍持有同一稳定锁，直到实际 scope
  排空；现有 outbox 的容量、发布、重开与回执清理测试继续执行。
- 并发测试暂停首个 owner 查询的真实回执，让另一 capture 完成并释放报告槽，
  再交付旧观察：迟到调用不发 PUT、不留下错误报告，紧接的正常调用可开启下一代，
  原代未知 PUT 计数保持不变。

对象存储为 MemoryObjectStore，Raft 为 InProcHub，RPC 为 HTTP/2 duplex。新进程
context 与 WAL 重开是组件恢复，不是实际 kill/断电/跨主机/MinIO 资格。

下一步继续安装/startup Reader、完整 Publisher census 与 admission closure、
portable successor recovery authority，再组合负向 Pending、旧 Version 与对象 GC。
本轮只为实际迁移调用建立可核对的逐代来源和报告，没有放行未知 PUT 释放、全局
删除或历史回收；容量稳态、部署时钟和完整形式化组合继续保留。
