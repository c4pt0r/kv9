# 自动 data-group checkpoint 与候选历史保留

日期：2026-10-04；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
本轮接通 data group 的自动发布、各副本采用和持久 pending 续做，属于
[第二阶段](RAW-TTL-PHASE2.md)的组件进展，阶段和生产验收继续开放。

后续 2026-10-05：[staging checkpoint](RAW-TTL-STAGING-CHECKPOINT.md) 已连接未绑定
导入的 checkpoint 与恢复；当前 owner/service floor 为 V6。下文保留本组件当时
的实现和验证记录，完整升级/容量/部署资格仍未完成。

## 执行和恢复

配置远端对象后端后，NodeRuntime 启动共享 checkpoint 调度器。leader 在实际
ReadIndex 后冻结本组 image，从同一 snapshot 读取 range、predecessor generation
和数据 revision。每组仅有一个 ManifestSeam、一份 `data.pending` 和一个候选。
对象访问前经[跨节点 ledger](RAW-TTL-CHECKPOINT-OWNERS.md)确认 Pending owner；上传、
读回和完整 SST/range/TTL 检查通过后，才提交原有 ManifestChange。

正向 typed settlement 后完成 Version owner 交接，并采用实际已提交的获胜 manifest。
follower 也从自己的已应用 history 采用 image，必要时修复发布者中断的正向交接。
实际采用沿用 engine checkpoint reference 的持久发布及封闭 WAL segment 回收。
结果未知保留相同 journal、generation 和操作身份；下一轮先收敛旧操作。
负向 typed settlement 仅允许清理本地 journal 和候选保护，不释放全局 Pending owner。

候选保护与原有 selected checkpoint 保护共用 CheckpointHistory。持有 history 和
WAL 锁时只捕获不可变 snapshot、位置和 revision，建立限制后释放锁，再执行 scope
检查与序列化。两个实际 peer 压缩入口均不得越过 selected 或 pending 的较早 cut；
限制本身不授予压缩、发布或对象删除权限。

开始持久 journal 发布前，候选转换为 durable。executor 退出仅放弃执行占用，
保留候选 cut；typed settlement 后先删除并 fsync journal，再解除限制。首次 stage
失败只在确认尚无所选 journal、且缺失状态已 fsync 时取消候选。任何不确定状态
保持保护。磁盘重开在 driver 绑定、投票及压缩之前读取 pending，检查 scope 与真实
已提交 cut/configuration，重建限制；缺少对象配置、损坏、异组或已丢历史时拒绝启动。

runtime 自动压缩的建议 floor 也受此 ceiling 限制，仍须通过原有 metadata 决策、
all-matched 和 peer 重查。compaction confirmation 保留在 WAL/image 内，但不再增加
触发 checkpoint 的 data revision，避免没有数据变化时反复发布。

## 调度与退出

- 每节点最多两个实际 blocking job，每组最多一个；在途和空闲状态合计最多 255 个。
  每 50 ms 轮转，单次最多访问 32 个目录项；失败指数退避，最大 30 秒。
- 成功轮次使用 `KV9_FLUSH_INTERVAL_MS`，默认 5 秒、最小 100 ms。
  `KV9_DATA_CHECKPOINTS=0` 关闭新计划，仍继续 durable pending 和已提交 image 的采用；
  默认 `1`，其他值启动拒绝。无对象后端时不启动此任务。
- 每轮控制流程预算 30 秒，ledger 单次上限仍为 2 秒且不超过本轮剩余预算。
  同步对象 I/O 无法被本轮 deadline 抢占；返回后重查 deadline、取消和实际 owner。
  这不是整个对象操作的硬超时，也不是 TTL/checkpoint/compaction 共用的整机预算。
- 取消和退役不提前归还正在执行的 job 槽位。任务真正退出后才丢弃相应 seam/capture；
  runtime 先停止共识 owner，保留 group 锁，排空维护任务后再释放锁及 StoreGuard。
  已持久 Reclaimed fence 下的 payload 回收同时清理 pending 和临时文件，不解除对象 owner。
- 状态包含 publish_new、在途/峰值任务数、跟踪组数、计划、正向结局、采用、未知、拒绝、
  错误与已移除状态数。panic 或唯一 seam 初始化失败停止任务，由 runtime 检出；不重新铸造 seam。

后续[共享维护配额](RAW-TTL-SHARED-MAINTENANCE.md) 将这些 job、metadata checkpoint、
TTL、压缩及回收接入节点总上限和类型间等待；上述局部上限继续生效。该后续进展
没有把同步 I/O 变成可抢占任务，也尚未关闭完整整机预算验收。

## 本轮证据

[原始日志、命令与源码摘要](raw-ttl-data-checkpoint-validation-20261004/manifest.json)
单独保存，不改写前序验证记录。

5 项候选专项测试覆盖两个真实 peer 压缩入口、executor 丢弃、重复认领、scope 回调
不持锁、真实 stage/clear 文件失败、磁盘重开及四种启动拒绝。该 Raft fixture 使用
测试 Enable 注入；不把它当作公开激活证据。

3 项自动调度集成测试使用三个 metadata voter、三个 data voter、分离的 metadata
leader 2 与 data leader 1、持久 V4 StoreGuard 和实际公开 TTL 激活。Raft 经 InProcHub，
公开与控制调用经 Tokio duplex 上真实 HTTP/2，对象后端为 MemoryObjectStore：

1. Pending 登记已提交但回执丢失，原计划恢复；同时续期写入保留。三个副本自动采用后，
   每个副本的 engine segment 从 **18,903,328 字节降至 2,627,627 字节**。
2. 分别在对象 PUT 已写但报错，以及 manifest 已应用但 Version Share 回执丢失时，
   停止并关闭测试附加组的三个副本，重开实际磁盘。关闭新触发后仍完成原 generation 1，
   不创建 generation 2，TTL 读取正确。
3. 一个组上传阻塞时，另一个三副本组仍能发布/采用，实际峰值不超过两个任务。
   移除前者目录并停止 owner 后，取消排空等待真实上传返回，最后释放 engine capture，
   不在失去 owner 后采用 image。该故障点不等于完整 metadata 退役协议验收。

另有 engine 回归检查 compaction 确认不会造成空闲发布循环；RegionManager 回归检查
退出排空期间独占 group 锁保留，以及 Reclaimed 重启后完成 pending payload 删除。
Raft 本地回归、engine、激活组合、RegionManager、含 tracing 的全 workspace 严格
Clippy、格式和 diff 检查结果见 manifest。测试集合有重叠，不相加为独立用例数。

## 仍需完成

Reader 生命周期、负向 pending 的认证解除、旧 Version 安全释放及对象 GC 尚未完成。
现阶段保留这些 owner；一次本地 segment 回收不能证明远端对象债务有界。
manifest/configuration/owner history 仍增长，全量 checkpoint 仍有 48 MiB 序列化上限，
engine index 全量驻留。整机共享维护预算、公平性/债务和持续磁盘/RSS/FD/对象稳态验收仍开放。

完整 NodeRuntime 启动和自动压缩控制面、实际 TCP、MinIO 持久性、进程 kill、旧二进制
混部及跨主机故障未由本轮测试覆盖。TTL split/merge/迁移/安装/新增副本/恢复转移、
可转移恢复锚点、部署时钟资格、独立时间历史、完整 TLA/TLC 和 Chaos 门槛继续保留。

后续[规划与 journal 内存修正](RAW-TTL-CHECKPOINT-MEMORY.md) 将大小检查前移并消除
完整 journal 编码副本，data/metadata worker 在 durable stage 后、reload 前释放原 plan。

2026-10-05：[逐 SST 落盘与上传](RAW-TTL-SPOOLED-CHECKPOINT.md) 进一步让两个生产
worker 的计划只持有临时文件，journal 重开保留校验过的 inode，并逐 SST 申请额度。
原 candidate/owner/manifest/recovery 权限保持；48 MiB 格式和完整资源边界继续保留。
崩溃恢复格式与候选 pin/owner 语义不变；完整共享内存预算继续开放。
