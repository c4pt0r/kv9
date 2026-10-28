# Raw / TTL 提案锁内背压

日期：2026-10-05。基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
这是[完整生产范围](RAW-KV-PRODUCTION-SCOPE.md)中的组件修复，第二阶段继续进行。
没有新增持久格式、Raft tag 或服务版本；当前 store/service floor 保持 V7。

## 修复

原先 `RawGroup::permit` 在准备请求时读 `committed - first_index`。这个值少算首条
保留日志，不包含未提交的 proposal；检查和提交之间也没有容量预留。并发请求和批量
延迟窗口可以同时越过上限，直接 driver 提案及 split import 还会绕过该检查。

现在 `RaftPeer` 在同一把锁内检查领导权、规划 term、容量并调用 raft-rs append。
计数为 `last_index.saturating_sub(first_index - 1)`，包括未持久化和未提交的尾部，
不复制或扫描日志。容量满时在 append 前返回类型化 `WriteBackpressure`，检查通过
的请求再追加一条。前置检查只节省准备成本，不预留可跨并发使用的槽位。

受限制的命令为 Put、Write、Fenced Write、PopulateRaw、TTL Write/Observe/Cleanup，
以及受保护的 TTL split Append。后者仍必须满足原先的源证明和全副本格式确认。
批量请求的所有等待者收到相同类型的拒绝，不再转成普通 Raft 错误或未知写入。

限制属于实际 peer，RawGroup 包装对象重建不重置它。NodeRuntime 启动时验证配置，
RegionManager 在普通创建/恢复和已采用副本的 driver 启动前安装它。默认每组
`KV9_MAX_RAFT_LOG_ENTRIES=65536`；`0` 显式关闭。错误字符串、负数、非十进制或
溢出值拒绝启动。独立 Raft 库构造器仍默认关闭，由宿主明确设置。

## 回收与读取

已接受的日志继续持久化、复制和 apply；follower 不按本地写入阈值拒收 leader
日志。checkpoint 发布、压缩确认、membership、激活和 split Begin/Finish 等控制
命令仍可提交。普通 Raw ReadIndex 读取继续服务；TTL 读取若需要追加 Observe，
则返回公开协议的 Backpressure，不能退回未确认的本地时间过滤。

因此，阈值仅控制新数据提案。控制命令和选举可以使实际条目数超过它；它不是所有
命令的总数量上限，也不是节点级 WAL/Raft 字节配额。压缩恢复的是逻辑日志槽位，
物理重写及 engine checkpoint 才回收各自文件。现有自动压缩/重写配置没有随本次
修改启用；没有实际回收时，有限默认值最终会停止新数据提案。

状态增加每组 `log_last_index`、`retained_log_entries` 和 `max_write_log_entries`，
与原有 `log_first_index` 一起说明当前压力。这些观察值不预留容量或授权压缩。

## 验证

- 48 个并发调用先全部通过前置检查，再同时提交；仅剩余 7 个槽位被接受。没有驱动
  quorum 时，已提交水位不变，新增 7 条仍计入容量。更低阈值的 follower 仍能复制。
- 覆盖首条计数、空的已安装日志尾部、每组隔离、显式关闭和控制 Noop 推进。
- 实际 DiskRaftStorage / WalEngine 的已接受写入在背压下完成 apply；关闭重开后
  保留压力和数据，拒绝的 key 始终不存在。实际压缩和物理重写后恢复写入。
  测试的 floor decision 由 fixture 提供，不替代生产 metadata 授权验收。
- 两个批量请求准备后，另一条控制日志占满最后槽位；合并提案及两名等待者均收到
  类型化拒绝，没有增加日志；普通 Raw 读取仍可完成。
- 公开 TTL RPC 拒绝满容量下的读观察、写入和删除，日志位置不变；能力查询可用。
  直接 driver TTL Write/Observe/Cleanup 同样受限，拒绝时归还 async apply 槽位。
- 三副本 runtime split 经 HTTP/2 duplex 完成格式确认后，满容量的未绑定 child
  拒绝 Append，复制游标保持零；恢复阈值后按原源继续并完成已有恢复回归。

命令、原始输出和源文件摘要见[本次验证清单](raw-ttl-write-admission-validation-20261005/manifest.json)。
历史 artifact 与 manifest 保持原样；本次没有重新运行 TCP/MinIO 或 TLC，已有环境
限制没有变化。旧单线程抽象背压引理不构成本次并发实现的形式化证明。

## 完整范围继续开放

继续实现节点级 WAL/Raft 真实字节债务和准入，保留已承诺 apply/回收需要的空间，
覆盖重写临时文件、失败残留和多组长期压力。该条目阈值不能替代磁盘余量、总 RSS、
CPU/公平性或固定活跃数据规模下的持续资源验收。

后续组件进展见 [WAL/Raft 共享字节准入](RAW-WAL-RAFT-BUDGET.md)：已接入文件计费与
未 apply 预留，follower/control 仍可超阈值，磁盘安全余量与长期资格继续开放。

Reader / 旧 Version / 负向 Pending 的释放、unknown PUT 隔离、有界历史与 GC、
可转移恢复锚点、大镜像迁移、TTL merge、部署时钟资格和真实多节点升级/故障验证
继续按原完整生产范围执行。
