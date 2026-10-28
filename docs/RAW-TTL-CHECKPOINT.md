# Data group checkpoint 与 TTL 恢复组件

2026-10-04；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
这是[第二阶段](RAW-TTL-PHASE2.md)的恢复组件；自动发布、对象保留及
稳态空间验收仍未完成。本次没有新增磁盘或公开 wire 格式。

## 恢复路径

`RegionManager` 的 Active group 现在使用与 metadata 相同的已配置 remote reader，
恢复本组实际选中的 checkpoint 及其 engine WAL 尾部。此前该路径拒绝所有 checkpoint。
未启动组继续拒绝 checkpoint、用户数据和 applied history；独占 store/group 锁及原有
配置、创建、激活和最终 applied position 检查继续适用。

恢复依据来自已提交的 creation 与本地 store identity，不能由 manifest 自行声明。
新的 `checkpoint_recovery::data_group::open` 检查：

1. 选中的规范 manifest 属于预期 cluster/group，cut 对应的 Raft 配置仍可从实际
   committed history 取得。SST 从配置的 reader 读取并校验编码、摘要、范围和记录数。
2. 在重放尾部之前检查实际恢复的 image。持久 DataRange 必须匹配 root/group/creation，
   manifest 的 epoch 必须与该 image 相同；用户记录必须是对应 keyspace/bounds 内的
   Raw key。Default CF 的 value 保持原始字节。
3. Write CF 只接受规范的到期记录，必须有对应 value 和 clock record。期限、来源
   creation、term/index 原样保留，已过期记录也不会在恢复时凭本机时间删除。
   来源 group 的日志位置不能与当前 group 的 cut 比较；保留该身份不代表已经实现范围转移。
4. 系统记录仅允许 range、clock、合法的 compaction confirmation，以及本组连续且与
   winning pair 一致的 manifest history。其他 CF、外组记录或损坏元数据拒绝。
5. 共同恢复层核对实际 WAL 中原子写入的 publication batch、winning pair，以及精确
   Raft `ManifestChange` 命令、term/index、generation 和 manifest 内容。
   上传成功、对象可读取或一个很大的 applied index 都不能替代成功发布。
6. 恢复后的 range 必须遵循 image 之后允许的状态转换。当前协议只允许不变或未启用
   TTL 的旧单向 seal；不能借尾部改写 tenant/keyspace、bounds 或 epoch。TTL 水位不能
   低于 image，激活标记不能消失。将来扩展 range 转换时必须同步修改 apply 与恢复规则。

缺少 reader、对象、配置历史或精确 publication 时，完整 open 失败。Runtime 将该组
隔离，不会用剩余的部分 WAL 开始服务，也不会因此取得投票或安装权限。返回的恢复
观察只属于此次本地 open，不是可转移的 snapshot/install 或 retention capability。
这些检查不等于对任意伪造 engine WAL 的所有业务副作用逐条重执行验证。

## 实际物理回收证据

新增六项 Raft 专项及一项三副本 RegionManager 组合测试。测试使用实际 DiskRaftStorage、
分段 WalEngine、SST 编码/上传/readback、`ManifestChange` 有序 apply 和重新打开的磁盘目录。
单副本覆盖连续两代 checkpoint、后续尾部、永久值、TTL 值、即时过期值及显式两 CF 删除。
三副本在各自观察到精确已发布 manifest 后，调用实际 `checkpoint_applied` 替换本地
engine manifest 并删除已覆盖的关闭 segment，随后写入尾部、关闭并由新 RegionManager 恢复。

固定活跃 key、18 次 1 MiB 覆盖产生的 engine segment 文件实测如下（字节）：

| 副本 | 回收前 | 回收后 |
| --- | ---: | ---: |
| 1 | 18,878,492 | 3,147,364 |
| 2 | 18,878,492 | 3,147,364 |
| 3 | 18,878,492 | 3,147,364 |

三个副本重开后都保留准确的 expiry record、clock、最新 value 和 checkpoint 之后的写入。
这只测量 engine segment 文件，不包含 Raft WAL、RSS、FD、远端对象或完整稳态债务。
末段仍保留，不声称本次回收让磁盘大小等于活跃数据大小。

拒绝测试覆盖缺少对象/reader、错误身份、上传后未经发布的提前本地选择、合法 SST 中
不合法的 TTL/系统数据，以及带有效校验和和真实 committed position 的错误尾部。
另有未启用 TTL 的正常 seal 尾部作为正例，防止恢复检查误拒绝现有合法转换。

三副本通过 InProcHub 运行；fixture 手动组合 RegionManager 和 driver，没有完整
NodeRuntime/TCP 启动。Enable 使用明确的测试注入，不替代已有实际激活门槛验证。
对象后端为 MemoryObjectStore，测试其实际 SST/replay 组合，不证明 MinIO 持久性。

测试新增 `RemoteUploader::for_harness`，仅在 `test` 或显式 `testing` feature 下存在。
普通生产构造函数继续只接受 MinIO。独立反向依赖编译探针确认普通构建调用 harness
构造函数以 E0599 拒绝，而 MinIO 构造函数可编译。首次探针因离线依赖解析失败而作废；
修正为复用仓库 Cargo.lock 后，在新输出目录保留真正的正反例，没有把依赖失败计为通过。

Raft 本地回归 308 项通过，8 项忽略、37 项 socket 测试过滤；engine 各 suite 合计
158 项通过、25 项忽略；RegionManager 15 项通过。这些集合包含新增专项，不能相加
当作互不重叠的测试数量。workspace 全 target 严格 Clippy（含 tracing）、格式和 diff
检查另行记录。命令、原始日志、构造函数探针及源码摘要见
[本次验证记录](raw-ttl-checkpoint-validation-20261004/manifest.json)。

## 必须继续完成的组合

首次组件验证中的实际负例确认：如果 Raft compaction 删除 image cut 的配置或精确
publication 证据，下一次恢复会拒绝。后续已接入[共享的压缩/切换保护](RAW-TTL-CHECKPOINT-HISTORY.md)，
同时修复历史配置和物理 rewrite，具备三副本双 WAL 回收后的重开证据。精确恢复检查
继续保留；完整可转移锚点仍需完成，后续本地自动回收进展如下。

后续已接入[跨节点对象登记组件](RAW-TTL-CHECKPOINT-OWNERS.md)：共享 `CheckpointOwners`
支持 data manifest，规范 ledger RPC 能路由到远端 metadata leader，八个丢回执位置和
真实 publication/owner 交接有组合验证。[自动 data-group checkpoint](RAW-TTL-DATA-CHECKPOINT.md)
进一步接通候选历史保留、发布/采用、journal 恢复及退出排空；Reader 生命周期和安全
释放仍未完成，没有自动开启远端对象删除。

完整 checkpoint 仍有 48 MiB 序列化上限，engine index 仍全量驻留内存，manifest history
仍累积；本次未扩展容量，也没有证明对象历史债务有界。下一步是补齐
owner 释放，纳入整机维护预算并做固定活跃数据的持续写入、回收、
重启与磁盘/RSS/FD/对象稳态验收。TTL resident 优化、split/merge/迁移/安装/新增副本
互通、部署时钟资格、独立时间历史和完整分布式故障验证继续保留在阶段清单中。
