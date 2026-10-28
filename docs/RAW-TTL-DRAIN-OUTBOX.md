# Checkpoint 排空报告的本地持久补交

2026-10-05；基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
这是完整第二阶段的恢复补充，阶段与生产验收仍开放。

## 问题与行为

[原子执行者收尾](RAW-TTL-EXECUTOR-DRAIN.md)已把 Released 和逐代报告放在同一次
metadata 提交中，但真实 scope 返回的计数原先只在 worker 内存中。进程在 metadata
拒绝或丢失提交回执后退出，会失去补交依据，留下无法证明已排空的 Published owner。

现在生产 data/metadata worker 共用本节点 `checkpoint-drains/`：创建执行者时先
预留一个固定报告槽，排空后将已有 `FinishCheckpointExecution` 的规范请求字节
写入临时文件、fsync、rename，并 fsync 目录；成功后才提交 metadata。请求保留
原始 root、Pending、executor、generation、完整 owner binding、process identity
和实际 issued/resolved，不以重启进程的新 identity 重建旧报告。

只有取得原有鉴权 ledger 的确切成功回执，才 unlink 报告并 fsync 目录、释放槽。
丢回执保留文件并重复同一请求；原子 ledger 操作验证原始 binding 和计数，精确
重试不改写报告。文件提交后的 worker/process 丢失可由独立后台任务恢复，无需
原 data group 的查询 handle、本地 pending journal 或 uploader 配置。

未产生完整持久报告前的崩溃仍可能失去观察。缺文件不能推断零 PUT、进程退出不能
证明远端请求终止，报告 `(issued > resolved)` 仍是未知债务。这条链路不会给
无报告的执行者补造计数，也不会释放负向 Pending、旧 Version、Reader 或对象。

## 恢复与边界

生产启动在持有 StoreGuard 并取得 V7 fence 后打开 outbox，检查 root/node/store
incarnation 和规范请求。旧 process identity 是恢复对象，不是匹配当前 process
的条件。完整 `.tmp` 报告重新 fsync 后可提升为正式记录；截断/校验不完整的临时
写只清除本地暂存，不创建报告、不改变 metadata 或全局 pin。带完整 checksum
但身份/协议不符的临时文件拒绝恢复并保留；损坏的正式文件同样拒绝启动。

只接受固定槽位文件名和普通文件；未知条目、符号链接、超大文件、同一执行者的
重复报告都拒绝。运行中损坏的一个槽会报错，游标继续让后续槽得到处理。已取得
metadata 回执但本地目录同步失败时，内存保留已确认状态，续做本地删除；重启后
若文件仍在则重放原始请求。不能由部分文件或后一次上传成功替代丢失的旧报告。

报告槽与 generation 绑定，同一 executor 有保留项时不能再预留下一代。物理槽
复用有独立 reservation serial；旧 guard 或迟到回执不能释放另一个执行者的新
文件。文件锁不跨越 RPC/共识等待，并发 worker 与后台任务可幂等提交同一报告。

## 有界资源与调度

- 每节点固定 **1024 槽**，每条规范请求至多 **16 KiB**，每槽至多一份正式文件
  和一份临时文件。文件 payload 上界 **32 MiB**，另有文件系统元数据开销。
  这是独立额度，不计入原 pending journal 配额，也不是磁盘空间预分配保障。
- 槽位在新执行者 Share/Publish 和对象 I/O 前预留。满额拒绝新执行者；已有报告
  继续恢复。磁盘写入/fsync 失败仍保留内存执行者与全局保护，不能保证在掉电前
  成功保存报告。
- 内存只保留固定槽状态、身份和摘要，重放一次至多读取一条 16 KiB 请求。启动
  至多检查 2048 个文件名，不随 group、attempt 或 generation 历史增长缓存 payload。
- 复用 `checkpoint_retention` 共享维护槽和实际 job 排空。每个 pass 最多补交
  一条报告；metadata leader 在本地报告与全局 attempt 扫描间交替，其 follower
  也可向现任 leader 补交本地报告。取消、serving 和 deadline 在 mutation 前检查。
  RPC 继续使用现有两秒上限及五秒 pass deadline；文件系统阻塞没有硬时限。

状态新增 `checkpoint_drain_outbox` 的保留/待提交/恢复/已确认待清理数量；原维护
状态增加 `drain_reports_replayed`。计数包含取得回执后的本地清理，不代表所有历史
执行者已排空。metadata 中的 owner、逐代记录及未知债务仍可能持续增长。

沿用既有 tag 13 ledger 请求和 V7 RPC，没有新增共识请求或提升格式 floor。
旧 V7 binary 不负责重放新目录；混部部署与回退资格仍未验收。

## 验证

文件组件测试覆盖原字节重开与回执丢失、截断临时写、完整临时记录提升、正式记录
损坏与错误 store/root 拒绝、符号链接、大小/文件名边界、全部 1024 槽满额、坏槽
不阻塞后续槽、确认后的目录清理重试、同一 token 的 reservation 复用，以及 RPC
阻塞期间仍可写入其他报告、旧回执不删除新文件。

真实三副本 Raft 与鉴权 HTTP/2 duplex 组合覆盖 metadata 和 data 两类报告：一个
在提交前被拒绝，一个提交后丢回执且带未知 PUT，第三个故意不持久化报告。重建
本地 ActivationContext/process identity 和磁盘队列后，metadata follower 在没有
uploader 时自动补交前两个；第三个仍 Published/无报告。实际 metadata WAL 重开
再次核对原始计数、旧未知及所有 Pending 保护。

该组合保留现场 Raft driver，只重建本地恢复组件；不是 OS process-kill、真实 TCP、
MinIO、断电文件系统或跨主机部署验收。新增报告目录没有完成形式化 refinement；
原 scope/ledger 证明也不自动证明该文件协议。

本轮文件组件 **7 项通过**，完整 TTL 集成 **50 项通过**，共 **57 项通过、0 忽略**。
严格 workspace Clippy、格式和 diff 检查通过。开发期间状态输出初次引用了不存在
的 backend 字段，编译检查拒绝；已改为实际 `rpc_backend`，原诊断保留在证据包中。

证据见[持久报告验证包](raw-ttl-drain-outbox-validation-20261005/manifest.json)。完整
publisher 集合、无报告崩溃遗留者、未知 PUT 终止/隔离、负向 Pending、旧 Version
释放、可转移恢复锚点、物理容量稳态和真实部署验收继续推进。
