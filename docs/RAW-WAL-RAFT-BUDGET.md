# 节点共享 WAL / Raft 字节计费与准入

日期：2026-10-05。基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
接续[提案锁内条目背压](RAW-WRITE-ADMISSION.md)。metadata 和 data group 的实际
Raft 文件、engine WAL 以及未 apply 提案现在共享节点预算。此项是第二阶段的组件
进展；[完整生产范围](RAW-KV-PRODUCTION-SCOPE.md)继续开放，store/service floor
保持 V7，没有新增持久格式或 Raft tag。

## 配置与准入

`KV9_WAL_RAFT_BYTES` 默认 **4 GiB**，runtime 最小 **16 MiB**。零、负数、非十进制、
溢出值和过小值拒绝启动。它独立于每组 `KV9_MAX_RAFT_LOG_ENTRIES`、checkpoint
pending 和匿名 spool 配额。独立的 engine / Raft 库调用者须在启动 owner 前明确
绑定同一预算；RegionManager 默认有自己的预算，NodeRuntime 将其替换为和
metadata 相同的节点预算。

普通 Raw 写入、批量 fenced write、PopulateRaw、TTL Write/Observe/Cleanup 及
受保护的 TTL split Append 在 peer append 锁内原子申请容量，再追加日志。阈值为
`file_bytes + reserved_bytes + 本次预留 <= limit_bytes`；不满足时，在 append 前
返回原有类型化 `WriteBackpressure`。任何计费来源处于不确定状态也拒绝新数据。
每次准入不扫描目录或复制整个日志，多个组竞争同一把预算锁。

当前预留保守采用 `4 × encoded_command_bytes + layout_allowance`，其中固定部分
覆盖一份最大 4 MiB topology、最多 4,096 项 TTL 的 expiry/物理 key 元数据、一份
最大 split 状态记录及 framing 余量。它会为小请求预留约 5 MiB，降低可同时在途的
提案数；不是按平均写放大估算的性能调优结果。实际写盘字节与该预留在 apply 前
暂时重复计费，换取清晰的释放边界。控制命令的实际增长可能超出这个数据提案估算。

## 提案与文件的生命周期

预留接受后绑定 `(Raft 来源路径, index, term)`，由共享预算持有。请求取消、超时、
等待槽位归还或 driver drop 都不能释放它。实际 driver 报告 apply 后才移除已消费
位置。成功持久化替换后缀时，移除已被覆盖的未来 apply 债务；append-only Raft
文件中的旧记录继续占用 `file_bytes`，直到真实重写。

节点启动先扫描已有 WAL family 和 Raft 目录，包括隔离组、obsolete/orphan segment、
已知临时文件，以及只剩临时文件的目录。扫描只计算文件长度，不读取完整 payload，
不授予恢复或删除权限；无法测量的相关来源拒绝启动，不把 I/O 错误当作零字节。
被启动的实际 driver 再从恢复的 state-machine 水位之后，分页重建未 apply 的
日志预留。降低配置后，已有债务允许超过阈值，恢复与已接受 apply 继续进行。

实际文件 append 通过 `BudgetWriter` 在 I/O 前计费，覆盖 legacy WAL、segmented
WAL 的 header/record/topology、Raft 普通记录和 snapshot base 记录。短写仅扣回
明确未写的部分；含糊的写入失败保留本次完整计费。来源在 owner 退出后仍保留。

迁移、checkpoint、rotation 和 Raft rewrite 同时计入旧文件与新增临时文件。只有
原流程完成发布、所需目录同步及旧 writer 关闭，才重新测量并扣减空间。失败路径
不因临时文件看似消失就提前退款；测量失败标记不确定状态，阻止新数据准入。
RegionManager 在原有授权退役和 owner 排空后，完成删除与同步，再核对实际文件
并移除空来源。预算本身不改变 checkpoint、历史保留或退役的授权规则。

普通追加不做目录扫描；冷路径的成功布局变化才核对文件。现有唯一 store/group
writer 的约束仍是前提。此实现没有 pending journal 那样的 inode reader lease，
不承诺跟踪外部打开的旧 WAL FD，也不支持无关进程修改受管文件。

## 满额行为与观察

已接受 apply、follower 复制、选举、membership、checkpoint、压缩及 split 控制
命令继续写盘，并强制增加实际债务。逻辑压缩仅恢复条目槽位，不退还字节；实际
Raft rewrite 或 engine checkpoint 回收文件后才可能恢复字节准入。既有自动压缩/
回收配置仍需启用并验证。普通 Raw ReadIndex 读取和 TTL capability 查询仍可用；
需要追加 Observe 的 TTL 读取会返回 Backpressure。

节点 status 的 `wal_raft_budget` 包含阈值、文件/预留/峰值字节、拒绝次数、来源数、
未 apply 位置数和不确定来源数。它是诊断信息，不是文件删除或日志压缩凭据。

## 验证与边界

[本次证据包](raw-ttl-log-bytes-validation-20261005/manifest.json)保存命令、原始输出
和源码摘要。**707 个不同测试通过**；11 个既有测试按原配置忽略（9 个真实 MinIO、
2 个本地 encoder CPU 实验）。Raft 的 grpc/transport 两个测试模块因已知 TCP
sandbox 限制未运行，共过滤 63 项；单独的 runtime HTTP/2 duplex 回归通过。
workspace 严格 Clippy、格式和 diff 检查通过。新增测试覆盖：

- 两个实际磁盘 Raft group 并发提交 32 个请求，只接收预算允许的两个；取消等待、
  暂停 apply 并驱动 owner 后，等待槽位归零而日志预留仍在，真实 apply 才释放。
- 持久但未 apply 的后缀重启后重新计费；降低到 1 byte 的测试预算仍能恢复 apply。
  三副本中，两个低阈值 follower 仍持久化并应用 leader 已接受的写入。
- leader 替换未提交后缀只释放被覆盖的未来债务；逻辑压缩仍拒绝写入，真实 Raft
  rewrite 才恢复准入。fixture 的 compaction decision 不替代 metadata 授权验收。
- 最大 4,096 项 TTL batch 重复 48 次，跨过生产默认 16 MiB WAL rotation；逐次检查
  实际文件增长小于预留，文件计费与磁盘长度相同，expiry 记录真实存在。
- legacy/segmented WAL 迁移、checkpoint 回收、残留临时文件、失败 topology 发布、
  短写与含糊错误；启动只剩临时文件的来源也计费，授权删除后才移除。
- 公开 TTL RPC 在超预算重启后，对 Write/Observe/Delete 返回明确拒绝且没有追加
  日志；能力查询可用，恢复足够预算后原值仍在、拒绝的值不存在。

这里计算文件逻辑长度，**不是整个文件系统的绝对容量上限或磁盘空闲空间保证**。
follower/control 仍可超阈值，本地 leader 准入也没有远端磁盘压力反馈。安装/捕获
暂存、绑定预算前的建组或恢复写入、目录/block 成本和外部旧 FD 未纳入完整限制。
启动目录扫描、恢复债务表、历史/索引内存的规模资格也尚未完成。

测试使用实际磁盘 WAL/Raft、InProcHub 和现有认证 HTTP/2 duplex。没有重新运行
受 sandbox 限制的 TCP/MinIO 或 TLC，也没有把组件测试计作完整 NodeRuntime、
跨主机、进程 kill、混合版本或长期多组容量验收。下一步仍需磁盘安全余量与控制/
回收工作额度、远端压力协调及稳定负载；Reader/旧 Version/负向 Pending 释放、
unknown PUT 隔离、有界历史/GC、大镜像迁移、TTL merge 和整机 CPU/RSS/公平性
继续按完整生产范围推进。

后续进展：[本地磁盘余量准入](RAW-DISK-HEADROOM.md) 已接入 Linux 实际空间采样和
重叠写入扣减，默认余量 256 MiB。该策略没有预分配空间，也没有关闭上面的控制/
恢复峰值、远端反馈和长期资格要求。
