# 未上传 capture payload 的副本接管

2026-10-05；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
接续 [固定 capture catalog](RAW-TTL-CAPTURE-CATALOG.md) 和
[V8 升级隔离](RAW-TTL-CAPTURE-UPGRADE.md)。原 source 尚未完成上传就失效时，
新 data leader 现在可以取得其他原 voter 保存的完整文件，继续上传原镜像。
第二阶段和生产验收仍未完成。

后续：[迁移 Publisher 生命周期](RAW-TTL-MIGRATION-PUBLISHER.md) 已将两端实际
对象任务纳入独立代际、原子准入与持久排空报告，ledger 升到 08；本文件其余格式
和剩余工作描述保留本包当时的边界。未知 PUT、长期 image pins 和 GC 权限不变。

## 成功计划的条件

`plan_migration_image` 仍先原子保存 `capture.plan`，通过独立 source 核对把
不可变 descriptor 提交到 metadata Raft。收到登记回执后，调用节点等待本地
metadata apply 到该位置，再开始 payload 复制。

计划成功返回前，完整 payload 必须已持久保存在原镜像配置的 voter 多数派。
本地有效文件计一份，其他节点只在精确关联的复制成功回执后计入；learner 不计入。
joint 配置要求 incoming、outgoing 各自满足多数。每次重试重新确认副本，
不会把旧进程的成功计数当作新回执，也不重新生成 cut、manifest 或协议 record。
规划和复制不进行对象存储 I/O。

这项保证适用于 **Plan 成功返回时**。catalog 提交与 payload 多数复制不是一个
跨组原子事务：中间失败可能留下原文件、登记记录及部分副本，Plan 仍返回错误。
如果原 source 在足够副本建立前永久丢失，该 operation 可能无法接管；不能把
“catalog 中有 descriptor”解释成 payload 已达到多数。也没有新增持久 copy-set
证书、配置变更后的副本迁移机制或后台自动补副本调度。

## 文件与传输

既有 `KV9XFR01` 文件完整包含 `KV9CAP01` descriptor、canonical manifest 和
每个 SST。其他 voter 使用独立的 `capture-replica.plan`，稳定锁文件为
`capture-replica.lock`，发布临时文件为 `capture-replica.pending.tmp`。
复制同一份完整原文件，不根据本地较新的 engine snapshot 重建镜像。

- 导出先验证整个文件、所有 SST 和绑定，读取同一个已打开 inode，并保持实际
  group owner、稳定文件锁与 pending 磁盘计费，直到真实 reader 退出。
- 接收先预留完整文件的 pending 磁盘额度，再流式写临时文件。metadata、manifest、
  每个 SST、完整 checksum、精确 EOF 全部通过后，才 fsync、rename、同步目录。
  截断、尾部字节、错误绑定和损坏均不发布为可用计划；失败临时文件仍算磁盘债务。
- 每个 SST 的验证共用现有 image 额度；不会一次载入整幅镜像。启动盘点覆盖正常
  group 目录和所有真实 install generation 的副本、临时文件。授权退役清理也覆盖
  两种目录，并保留稳定锁文件。
- 复用其他 operation 占用的副本槽位，需要精确匹配 root、region、creation、目标、
  manifest subject 的 committed abort，或同时匹配 image digest、cut 的安装证据。
  这仅允许清理本地文件，不释放任何远端 pin。

内部 V8 服务新增 `FetchMigrationCapture` server stream 和 `ReplicateMigrationCapture`
unary RPC。复制接收端主动拉取指定 source；每帧 payload 最多 256 KiB，应用发送
队列容量 1，单次 RPC 预算不超过 120 秒。每轮候选重试共用 120 秒总预算，每次
为后续候选留下时间，避免第一个无响应连接耗尽全部预算。请求绑定当前精确 store
identity、region、operation、descriptor digest 和随机 challenge；接收端检查每帧绑定、offset、总长度、
terminal flag 和流结束，不接受重复、跳跃、超界或 terminal 后的额外数据。

RPC 使用现有 cluster 鉴权、Active V8 fence 和实际 runtime。双方必须是原镜像
配置中的 voter，实际 driver 必须持有该 engine，range 相同且已 apply 到原 cut。
必须存在精确 committed catalog，已 abort 的 operation 拒绝。peer endpoint 每次按
当前元数据的 incarnation 解析，原 source 节点不能用替换后的磁盘冒充原身份。
复制与 history/capture 查询共享一个实际阻塞任务许可；客户端取消或 RPC 超时
不会释放仍读文件/接收的后台任务，实际工作结束才归还许可。

磁盘、catalog、manifest、Raft command 编码不变，floor 仍为 V8，retention header
仍为 `KV9LED07`。更早的 V8 binary 不提供新 RPC，会使复制失败而不能产生成功回执。
这不是新的混合二进制或滚动升级验收；继续遵守现有停机升级要求。

## 接管与拒绝

`capture_migration_image` 仍要求当前健康 data leader、原 range/目标和两个精确
Published owners。successor 优先 GET 校验完整原 SST，齐全时直接返回原 record；
如果对象不全，则验证本地副本，或从可达的原配置 voter 拉取精确文件，然后上传。
每个 PUT 后仍核对对象，结果保持原 cut/configuration/record。

原 source 的 PUT 可以尚未返回，新 leader 同时上传相同 immutable 字节。接管成功
不代表旧 PUT 已终止，也不允许因此释放旧执行者、Snapshot/Migration pins 或对象。
损坏的已选择本地文件拒绝使用；所有原 payload 都不可达时也拒绝，不能用新 cut
填补旧 descriptor。正常 slot 复用、重试、文件删除都不提供远端 GC 权限。

## 验证

命令、源码哈希、测试日志和开发失败日志保存在
[本轮证据包](raw-ttl-capture-transfer-validation-20261005/manifest.json)。

- 实际三副本 metadata/data Raft 和鉴权 HTTP/2 duplex：登记回执丢失、source 查询
  拒绝之后，缺 payload quorum 的 Plan 拒绝；启用副本接收后重试得到原计划。
- 阻塞 source 首个 PUT，转移 data leadership，停止 source driver 并移除其目录。
  新 leader 从另一 voter 取得 payload，在旧 PUT 尚未返回时完成原镜像；随后旧任务
  排空，metadata 切换不改变 record。删除远端 SST 可以从原副本修复；损坏本地副本
  或让所有完整 payload 不可达时拒绝且无新增 PUT。两个 pins 仍为 Published。
- 900 个 TTL value、约 55.4 MB 镜像经过分块复制。接收 voter 的 SST 验证峰值受
  9 MiB 测试预算限制，完整文件计入其 64 MiB pending 预算；包含未知 PUT、后续
  写入、降低原 source 配额、abort 后复用槽位、实际 learner 安装和值/期限重开验证。
- 真实生成 HTTP/2 客户端取消 Fetch/Copy、Copy 服务端超时，后台许可保持到实际
  reader/worker 结束；重复调用不能越过单任务上限。有效多帧传输完成，错误鉴权、
  store/范围/长度/预算在对象 I/O 前拒绝。独立帧检查覆盖错误序列和不完整 EOF。
- 文件测试覆盖任意截断、完整 checksum 重算后的 SST 损坏、错误 metadata、尾部
  字节、发布故障窗口、重开、低磁盘额度、原 inode/锁的存续；实际 engine 归属与
  健康 voter 检查、较新源状态仍恢复旧 record、joint 双多数、盘点和退役清理回归。

对象存储是 MemoryObjectStore，Raft 为 InProcHub，RPC 为 HTTP/2 duplex。停止
fixture driver 及注入发布错误不等于真实 kill、断电、跨主机 TCP 或 MinIO 演练。
本轮未新增长期性能结论或形式化组合证明。

相关回归共 339 项不同测试通过，10 项既有测试忽略；严格 workspace Clippy
（含 write-stage-tracing/checkpoint-testing）、格式及 diff 检查通过。
候选预算调整后单独复测迁移接管及大镜像用例，重复测试不重复计数。

## 后续

继续补齐 source/install/startup 的 Publisher/Reader 生命周期和 portable successor
recovery authority，再处理可证明的负向 Pending、旧 Version 释放及对象 GC。
当前副本传输并未完成这些回收前提。真实旧 binary/部署恢复、配置变化后的副本维护、
持续容量和时钟资格、完整多组 split/merge 生命周期及第二阶段验收继续开放。
