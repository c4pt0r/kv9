# Raw TTL 后台时间推进与条件清理

2026-10-04；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
这是[第二阶段](RAW-TTL-PHASE2.md)的后台调度组件，阶段和生产验收继续开放。

## 运行方式

`NodeRuntime` 启动独立 TTL 维护任务，不在同步 group reconcile 中等待 Raft 回执。
默认启用；`KV9_RAW_TTL_MAINTENANCE=0` 在下次进程启动时禁用该任务，`1` 显式启用，
其他值在启动创建 owner 前拒绝。它只处理已经完成 TTL 格式激活的本地 data group，
不会自动激活 TTL，也不会改变公开协议 v2、持久格式 V4 或时钟资格要求。

每个节点维护一个轮转位置，从本地目录按 region 取下一组；不会一次复制整个目录。
每轮至多检查 32 个候选，两个任务槽位由所有本地 group 共享。同一 group 同时至多
一个任务，follower 和未激活组跳过。目录锁不覆盖 storage I/O 或共识等待。

| 预算 | 当前值 |
| --- | --- |
| 节点同时运行/排队的 TTL blocking 任务 | 2 |
| 派发周期 | 50 ms，错过周期不补发积压 tick |
| 单页从派发开始的总 deadline | 2 秒，包括 blocking pool 排队 |
| 每页最多检查的 Write CF 记录 | 1024 |
| 每页最多候选 | 256 |
| 每页候选编码体 | 256 KiB，另加固定 Raft envelope |
| 一轮扫描完成后的最短间隔 | 1 秒，轮转和负载可能增加等待 |
| 失败退避 | 250 ms 起指数增长，上限 30 秒 |
| 空闲组状态上限 | 256，另有最多 2 个运行状态 |

当前 RegionManager 和共享 Raft pool 最多容纳 255 个活跃 group，状态预算覆盖全部。
退役组会移除状态，不消耗永久槽位；超出缓存容量的历史状态可安全淘汰并重新扫描。
未来扩大活跃组上限时，也必须重新评估此状态预算和公平性。每个游标只保留一个
最大 65537 字节的位置，不保留 engine、旧 RawGroup wrapper 或整个扫描结果。
活跃任务才持有实际 engine/driver；其完成前，退役资源仍按已有引用门槛等待。

这些是 **TTL 调度器共享预算**。现有 checkpoint、compaction、对象 I/O 等维护者尚未
全部纳入同一个整机预算。engine snapshot/iterator 的内部内存、磁盘 I/O 和不可中断
系统调用也没有被上述行数/字节数证明为有界时延；两个槽位限制并发和排队，不能保证
两个永久阻塞的 I/O 任务之外仍有清理进展。完整整机资源验收仍未完成。

后续已接入[七类维护共享准入](RAW-TTL-SHARED-MAINTENANCE.md)，本表的 TTL 局部上限
同时受节点总任务数、重 I/O 并发和派发速率限制。两项真实上传阻塞时 TTL 仍推进有
组合证据；其余维护路径、字节/成本与物理债务预算仍未完成。

## 时间与删除权限

一个维护页只使用现有 `Observe` 和 `Cleanup` 命令，不新增 apply 时钟或旁路写：

1. 检查节点 serving 状态、目录中的实际 owner、精确 binding、剩余总 deadline。
   本地状态只能用于拒绝未激活组，不能代替后续 leader ReadIndex。
2. ReadIndex 后，从同一 snapshot 校验持久 range 和时间水位；继续遵守该组的
   `KV9_MAX_RAFT_LOG_ENTRIES` 写背压。需要推进时，读取已配置的合格时钟区间，
   提交单调 Observe，等待精确 apply 回执，再建立新的 ReadIndex。
3. 时钟未配置、拒绝或落后于水位时，不自行制造新时间；仍可按**已经提交的水位**
   清理已过期版本。这样即时 TTL 或过去确认过期的数据不依赖重新取得时钟资格。
4. 从一个新 snapshot 的 Write CF 收集有预算的候选。没有候选但有 `next` 的页
   继续前进；不通过读取所有用户 value 寻找期限。提交前再次检查 owner、deadline
   和写背压，条件 Cleanup 在有序 apply 中重查完整版本身份及当前水位。
5. 仅精确 apply 成功后推进该页游标。期间发生的续期或永久覆盖不会被旧候选删除；
   `candidates_applied` 统计确认应用的候选检查数量，不冒充实际删除数量。

同一 owner 的 RawGroup wrapper 被定期重新发布时，游标仍保留；判断依赖完整 binding
digest 和实际 driver 身份。真正更换 owner/binding 后，从新范围起点重新扫描。
扫描状态不持久化，也不是删除授权；取消或 WAL 重开后重新扫描持久到期记录即可，
重复候选在 apply 中重新匹配。这里没有跨页 snapshot、全局期限排序或即时回收承诺。

## 未知提交、取消与恢复

超时不撤回已 append 的命令，也不能释放该组的追加预算。调度器保存接受的
`(term,index)`；在本地 driver 确认日志已消费到这个 index 前，不再为该组追加任何
Observe/Cleanup。它会退避等待，其他 group 仍可使用任务槽位，不因每次超时堆积
新的维护命令。每组最多保留一个这样的未确认位置。

日志消费到位置只允许新 ReadIndex 和重新扫描，不会把原先的未知结果追认成精确
成功，也不会提升旧页游标或成功计数。确定的 apply 拒绝/替换可结束该位置的等待，
后续仍按新页重新准备。重启后的新 leader barrier 和恢复状态用于重新扫描；没有
依赖内存游标跳过未知数据的恢复步骤。

关闭时先取消派发，再停止 data driver 唤醒共识等待，最后等待真实 blocking 任务
退出，之后才释放 store guard。丢弃异步 future 不等于底层工作结束；尚未返回的
任务仍占配额。worker panic 会停止新派发并排空其他任务，NodeRuntime 将异常结束的
维护 owner 作为运行错误处理，不会默默启动另一个任务重放丢失的位置。

## 可观测性

节点 status 增加 `raw_ttl_maintenance=<JSON>`；禁用时为 `null`。包含运行中/峰值任务数、
跟踪组数、带游标的组数、退避组数、等待未知位置的组数、最久到期等待、完成页/轮次、
检查行/候选字节、确认的候选检查与时间观察、时钟拒绝、失败、背压、跳过、owner 变化
及状态淘汰计数。错误字符串截断至 160 字符，不输出原始用户游标。

这些指标反映已观察到的分页进度和等待状态；尚未扫描的数据不在统计里。它们不是
精确过期字节总量、SST/WAL 垃圾字节或物理空间回收债务，不能据此宣布稳态存储达标。

## 验证

新增 8 项专项测试通过：

- 配置严格解析、指数退避及上限。
- 1024 条仍存活到期记录后的空候选页能继续；实际复制时间推进后无需前台读取即可
  清理后续过期值；重新发布同一 owner 不重置游标，永久值和未到期值保留。
- 三个不同 group 中一个 blocking 任务停住，另外两个继续清理；取消后它仍占槽，
  drain 等待实际任务返回，不产生未授权的新页。
- 在候选选出后执行真实 Raft 续期，旧 Cleanup 不误删新值及新到期记录。
- 写背压阻止维护追加，解除后恢复；没有把被拒绝的页计为成功。
- 620 个即时过期 key 分页清理，取消后实际 WAL 重开，重新扫描并清除剩余两 CF。
- 暂停真实 driver apply，使 Cleanup 已提交但回执超时；不推进游标、不累计新追加。
  恢复后处理剩余 key，晚到的原结果不计入确认成功数量。
- 三副本实际格式门槛和公开激活之后，通过 unary/stream 写入，生产 backend 的后台
  维护删除所有副本的 Default/Write CF 记录；follower 跳过。Raft 为 InProcHub，RPC
  为 Tokio duplex 上实际 HTTP/2，时钟为受控 fixture。

相关公开 TTL 14、激活 9、group 生命周期 14 项回归通过；这些集合包含部分上述专项，
不能相加作为独立测试总数。workspace 全 target 严格 Clippy（含 tracing）、格式和
diff 检查通过。[原始命令、日志和源码摘要](raw-ttl-maintenance-validation-20261004/manifest.json)
另行保存，历史验证记录不修改。

没有新增完整 NodeRuntime TCP 启动、真实网络/跨主机故障、旧二进制混部、部署时钟资格、
独立时间历史检查、完整 TLA/TLC 或 Chaos Mesh 证据。后续继续统一整机维护预算、
实现每组 checkpoint/engine WAL 回收并测量稳态磁盘/RSS/FD/对象债务，再完成 TTL
resident 优化及 split/merge/迁移/安装/新增副本/恢复互通。逻辑删除不是物理回收。
