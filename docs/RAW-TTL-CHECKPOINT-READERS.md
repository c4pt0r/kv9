# Data checkpoint 读取的 Reader 生命周期

2026-10-05；接续[持久负向结算](RAW-TTL-CHECKPOINT-REFUSAL.md)。完整 TTL 第二阶段、
旧 Version 退役和物理 GC 仍未验收。

后续：[离线安装 Reader](RAW-TTL-INSTALL-READERS.md) 已覆盖实际安装 CLI 的镜像读取，
并补上取得报告槽后的 owner 复核。当前 floor 为 V8、ledger 为 08；下面保留本组件
当时的验证记录。runtime adoption 与 data/metadata 启动恢复仍须接入。

## 实际接入的读取

data checkpoint worker 在发布前校验镜像，以及各副本采用已提交镜像时，会调用
远端 restore。此前这些 GET 依赖全局 Pending/Version 保护，没有独立的本地读者。
现在每次 restore 前注册并确认 Reader 的 Share、Publish，之后才提交 GET。
发布前使用该候选的 Pending；采用已提交镜像时使用精确 Version。原有恢复、
TTL 镜像检查和采用条件继续生效。

Reader identity 绑定 root、源 owner、镜像完整 SST closure，以及 node/store/process
生命周期。每个源和进程使用连续 generation，前一代必须 Released 且本地报告
槽已确认清理，才能开启下一代。重复或迟到的旧代消息不能关闭、复活新代。

## GET 超时与真实任务寿命

`RemoteUploader::with_local_read_report` 提供独立的 GET-only 借用接口。该接口没有
PUT 方法；绑定 identity 前 restore 拒绝。它可嵌套在上传执行 scope 内，但拥有
自己的 actual-job leases，并在所有已入队/运行中的 GET 退出后才能返回。
调用超时和取消不能缩短这个等待，worker 的维护额度也继续保留到实际任务排空。

返回的 `DrainedRead` 与上传者的 `DrainedExecution` 是不同类型，无法用子 Reader
的结束报告伪装整个上传者已排空。父 scope 在子读结束后仍可继续自己的工作；
子 scope 的 GET 不会改写父 scope 的 identity 或 PUT 计数。

runtime 检查报告与保存的 Reader/root/Pending/generation 一致，才提交
`FinishCheckpointRead`。未确认准入时不发 GET，但该零 I/O scope 仍须结束原代，
防止较早的不确定 Share 在稍后重新建立可用 reader。排空过程 panic 或报告丢失
不会自动释放 pin。

## 原子结束与报告恢复

新请求是 ledger tag 18，携带原 successor token 和完整 Reader binding，沿用
16 KiB 小请求上限。planner 在一个已建立 snapshot 上校验身份、代际和每个资源
的保护，把 Reader owner 与所有资源 pin 原子改为 Released。它同时能为未提交
的首次/紧邻下一代 Share 建立终态 fence。相同代的精确重试不修改状态。

结束 Reader 始终要求完整 Published 覆盖。若原 Pending 在读期间发生正向交接，
planner 从同一 snapshot 校验原 intent、Pending binding 与对应 Published Version，
不能使用另一镜像或一个较新的 checkpoint 替代。Version 本身已经退役时，该操作
继续拒绝；本轮没有实现脱离继任保护的读者释放权限。

Reader 在 GET 前预留已有 drain outbox 的槽；真实排空后 fsync 原始请求，再交给
metadata。该队列共享 1,024 槽和每条 16 KiB 上限，完整报告以原 store/process/
generation 重放。确认回执后才 unlink 并 fsync 目录。失败阻止 worker 发起新读，
后台可在没有 uploader 的 metadata follower 上补交；新进程不能补造旧进程报告。
不完整临时文件或丢失报告继续保留 metadata pin。

本轮保持 `KV9LED07` 与 V7 store/service floor：Reader 已使用现有 owner/resource
行编码，新操作合并的是完整 Published 继任保护下已有的结束语义，不增加全局
删除权限。旧程序不认识 tag 18 时拒绝，旧 outbox reader 也不能把完整未知报告
当作可删除的残片。未宣称混合二进制或滚动升级已通过。

## 验证与开放项

[独立证据包](raw-ttl-reader-validation-20261005/manifest.json)包括：

- metadata：真实进入/未确认进入、连续代际与旧消息、身份/来源不符、读期间正向
  交接、幂等确认、publisher closure stamp 不变，以及五个真实 WAL 尾部截断位置。
- 引擎：GET-only 接口和不可冒充上传者报告的编译失败用例、独立嵌套读取、实际
  MinIO 工作队列的 GET 超时后继续等待排空；传输后端是测试 InMemory。
- 实际三副本 Raft/鉴权 HTTP/2 duplex：未确认 Reader 准入不发 GET；采用读取
  超时/取消后 pin 和维护 permit 仍保留；排空后原子结束。丢回执阻止新读，持久
  报告在重建 process context、换 metadata leader 后由后台补交，WAL 重开保留
  Released Reader 与 Published Version；另一份丢失报告的 Reader 仍 Published。

本轮 metadata 全套 **201 项通过、1 项既有 MinIO 用例忽略**，完整 TTL integration
**59 项通过**，另有引擎执行 scope 8 项、类型约束 2 项和 outbox 8 项通过，合计
278 项不同测试。专项重复运行不重复计数；严格 workspace/all-targets clippy、
格式和差异空白检查通过。

重建 process context 和 WAL 重开是组件恢复验证，不是完整 NodeRuntime 多进程
故障或远端 MinIO 资格。metadata 启动恢复、安装/迁移和其他引用仍须与其原保护
及恢复锚点组合；本轮不表示所有对象 reader 路径都已纳入。已报告的 Reader
生命周期是旧 Version/资源回收的一个输入，不是完整 publisher 或全局释放证书。
无报告崩溃遗留者、历史容量上限、继任恢复锚点、旧 Version/负向 Pending 释放与
对象 GC 继续开放。
