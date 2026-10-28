# Checkpoint 对象任务的本地排空

2026-10-04；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
接续 [checkpoint 收敛预算](RAW-TTL-SETTLEMENT-BUDGET.md)。完整第二阶段仍开放。

## 实际修复

MinIO 的同步调用使用 `recv_timeout` 等待专用对象 worker。调用者返回超时，不代表
队列中的 job 或正在执行的 future 已退出。原 data/metadata checkpoint 持有维护
permit 到外层同步 tick 返回为止，因此在这条路径上会早于实际对象 job 归还配额。
对象 buffer 已按真实 Bytes owner 计费，但 buffer 额度不能替代任务占用的计账。

现在每次 checkpoint tick 使用 `RemoteUploader::with_local_execution`。作用范围内
的 PUT、上传读回、prepared 恢复和 restore GET 都把不可克隆 lease 交给实际 MinIO
job；caller 和 reply channel 不持有结束该 job 的权力。lease 跟随 job 留在有界队列
或 worker 中，直到拒绝过期任务、执行 future 返回/取消、队列丢弃或 worker unwind。
提交失败会直接销毁未入队 job，不留下虚假的占用。

tick 返回或 unwind 前先关闭该范围的准入，再等待它的全部本地 job 退出。等待使用
Condvar，不轮询；登记、关闭和减计数在同一小锁下，锁不跨对象 I/O。嵌套 scope 共用
同一执行范围。无对象 job 的另一范围可以直接完成，不等待整个客户端队列为空。
该范围是内存内的执行生命周期，不是持久 owner token 或可跨节点移交的证明。

实际接入两个生产入口：

- data checkpoint 的 blocking closure 在排空结束后才返回 State/Work/Result，继续
  持有共享 permit 和该组的 worker。取消、retirement 和 dispatcher drain 不会提前
  归还这些资源；同组不会在旧本地 job 未退出时开始下一轮 tick。
- metadata checkpoint 在线程的同一次 tick 内完成排空，已有共享 permit 和 shutdown
  join 覆盖这个等待。原 journal 与 manifest settlement 次序不变。

对象 API 的原 deadline 和未知结果不变。新执行范围的排空可能超过该 deadline：本地
worker 若永久阻塞，checkpoint 继续占用容量并等待，不能伪装成已经排空。正常队列
处理仍检查原 deadline 并拒绝过期请求，future 仍受既有 wire budget 限制。

范围等待的是本地 job/future，不等待所有 payload Bytes 引用消失。例如测试 SDK 的
InMemory 后端可以在 PUT 完成后继续保留 payload；此时 job 已结束，buffer 额度继续
由原 owner 保留。这两项统计各自对应不同资源。

## 验证

[命令、原始日志和源码摘要](raw-ttl-object-execution-validation-20261004/manifest.json)另存。

新增四项 engine 测试：

- PUT、restore GET、prepared GET 分别走真实对象队列。将 worker 停在 deadline
  检查前，观察到 caller 已关闭 execution 后，外层仍不能返回；释放 worker 后拒绝
  过期 job，外层返回原错误，且没有对象副作用。独立空 scope 此间可以完成。
- 调用者已经收到对象超时后注入 panic；unwind 仍等到实际 job 退出。
- 一个被 hold 的 job 加满实际 32 项队列，再提交被拒绝的一项：33 个 buffer 在
  caller 超时后仍存在；释放后全部 job 排空，无 admission lease 泄漏。
- 成功上传、读回、restore、prepared 重建和嵌套 scope 保持行为；后端仍持有 payload
  时范围也能正确结束，buffer 只在最后持有者释放后归零。

新增两个实际 runtime 组合测试，使用三 metadata voter、三 data voter、分离的 leader、
带认证的 HTTP/2 duplex，以及生产 MinIO job/deadline 循环。测试专用 factory 只把 SDK
transport 换成 InMemory，不需要网络或凭据；gate API 仅在 testing feature 中存在。

- 自动 data checkpoint 的对象调用超时且执行范围已关闭后，共享重 I/O 占用仍为 1，
  journal 仍是原计划，尚未采用 checkpoint。同节点真实 TTL worker 清理过期数据。
  取消并 drain checkpoint 后，worker gate 未释放时仍占用容量；释放 gate 才完成。
  全局 Pending 仍 Published，原 journal 可恢复出同一个 prepared identity。
- 实际 metadata checkpoint 线程经过同样的对象超时；shutdown join 与 permit 保持到
  worker 退出，journal 保留，manifest generation 未变。

Engine 全集合 **205 通过、25 忽略**；Raft 本地回归 **342 通过、8 忽略、37 过滤**；
TTL 激活组合 **22 通过**。专项集合重叠，不相加。默认 workspace 编译、严格 Clippy、
格式及 diff 检查通过。gate 测试验证了真实队列与 scope 退出的因果次序，不是远端
MinIO 服务、非协作网络 future、进程 kill 或完整 NodeRuntime 的验收。

## 仍须完成

本地 job 退出不能证明远端 PUT 已取消，不能排除 SDK 内部独立执行者或其他节点的
同 identity 上传者。超时/丢回执仍是未知，不能凭本轮内存内排空 quiesce/release 全局
Pending，更不能删除对象。协议、WAL/journal/ledger 格式和对象释放入口均未改变。

Reader/旧 Version、持久执行者登记/关闭、全部副本恢复引用切换、负向 settlement 与
全局 release/GC 仍待完成。安装/迁移/启动恢复等其他路径没有因此自动取得维护配额；
完整控制扫描、历史总量、CPU/本地磁盘/常驻索引/RSS、公平性及物理债务也未验收。
TTL split/merge、完整可转移恢复锚点、48 MiB 全量 image 替代、部署时钟与独立时间
历史、真实 TCP/MinIO/旧二进制、完整 TLA/TLC/Chaos 继续保留在完整阶段范围中。
