# Checkpoint 与 Raft 压缩的恢复证据保护

2026-10-04；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
本次接续[数据组恢复组件](RAW-TTL-CHECKPOINT.md)，修复实际压缩与 checkpoint 切换的竞态，
并修复历史成员配置查询/物理重写的问题。[第二阶段](RAW-TTL-PHASE2.md)仍未完成。

## 压缩与切换的共同限制

每个 WalEngine 持有一个 `CheckpointHistory`。选中 cut 来自实际恢复的 engine reference；
`NodeDriver` 构造时把同一个限制对象绑定到实际 peer，并读出实际 Raft compacted base。
对象只保存位置和失败状态，不持有 engine、driver、文件或对象 reader，不增加退役时的
engine/FD 引用。普通 apply 不获取它的锁。

两条 peer 压缩入口 `truncate_retained_log` 和 `compact_confirmed_prefix` 都应用这一限制。
因此管理请求直接截断、leader 周期截断、follower 按确认水位截断都受保护；持有 peer
clone 的调用者也不会绕过检查。原有已提交决策、all-matched、精确 term、apply 和 engine
sync 等要求继续适用。这个限制对象不授予压缩、发布或对象释放权限。

- 已选 checkpoint 的 cut 为 C 时，只允许压缩到 C 或更早。C 的精确 term/configuration
  保留在 durable Raft base；在 C 之后成功应用的 `ManifestChange` 必须仍留在日志中。
- `checkpoint_applied` 与实际协议压缩共用同一把锁，覆盖文件修改及内存结果发布。
  成功选中新 image 后，允许压缩的位置随新 cut 推进，不永久停在第一代 image。
- 若压缩先完成，新候选的 cut 已低于实际 compacted base，则在选择 reference 和删除
  engine segment 前拒绝它。旧完整 WAL 或原来选中的 checkpoint 仍然存在；之后可以
  准备更新的 image。本次原始验证未包含上传候选 pin；后续
  [自动 checkpoint](RAW-TTL-DATA-CHECKPOINT.md) 在冻结时加入候选保护并从 journal 重建。
- 文件操作失败可能已经发布新 reference，不能只按返回错误假设没有副作用。发生此类
  不确定失败后，新的切换和压缩都拒绝，直到重开实际文件、重新验证并恢复位置。
- 锁顺序为 peer → checkpoint history → engine WAL / Raft storage。切换不反向获取
  peer；普通 engine apply 不在持有 WAL 时获取 checkpoint history 或 peer。

本次不新增磁盘记录类型：重启从实际 engine reference 和 Raft base 重建限制。没有
把内存 pin 当作持久恢复锚点，也没有放宽原来的精确 publication 验证。已经丢失发布
证据的旧文件仍然拒绝恢复。

## 历史配置修正

新增成员变更用例暴露了旧实现的两个问题：压缩之后的查询把所有配置条目当作未应用，
而 raft-rs `MemStorage::snapshot()` 返回的配置是当前成员集合，不能充当较早 cut 的配置。
旧物理 rewrite 同样使用该接口，可能把最新配置写成历史 base 的配置。

DiskRaftStorage 现在单独保留 `REC_COMPACTION`、`REC_RETAINED_BASE` 或真实安装 snapshot
中的精确 `(term,index,ConfState)`。查询以这个历史 base 为起点，再选择保留后缀中
实际应用、带 index 的配置；检查后续未应用条目，不能用当前成员或未来配置补过去的 cut。
物理 rewrite 保存该精确 base，随后保留原来的配置历史和日志后缀。

如果旧文件中的 base 配置与仍保留的初始/索引配置历史冲突，查询和 rewrite 拒绝它，
不会自动选择其中一个来产生恢复授权。本次没有实现受影响旧文件的自动修复。真实
snapshot 安装会同步更新这个历史 base；这不表示 TTL snapshot 安装协议已经完成。

## 验证与实测

新增 9 项测试（恢复/并发 5、历史配置 4），另扩展已有三副本测试：

- 两个实际 peer 压缩入口都拒绝越过所选 image；压缩到 cut、实际重写 Raft 文件、
  选择第二代 image、继续压缩及重启均通过，重建 driver 后继续保护。
- 压缩先于首次选择时，旧候选被拒绝，新的候选仍能成功发布、选择并重开。
- 12 次真实线程竞态执行文件选择和协议压缩，每次恰有一个成功，物理重写后都能
  重开且保留 value/expiry；有超时 watchdog 检查死锁。
- 文件系统故障覆盖 reference 发布之前，以及已经发布后删除旧 segment 失败。
  不确定期间拒绝压缩和再选择；重启按真实选中结果恢复。
- 实际 Raft learner 添加/移除后，历史 image 仍恢复其原来的配置。此 fixture 验证
  底层配置机制，没有授予新增 TTL 副本的完整生产准入。
- 联合/稳定配置的所有成员集合、compacted base、后续已应用/未应用配置、矛盾历史、
  实际磁盘上的连续两次 rewrite 和重开均有正反例。旧测试中直接宣称配置已应用的
  fixture 已改为先持久化真实 indexed apply 记录，没有放宽拒绝条件。
- 三个真实 DiskRaft/WalEngine 副本：leader 在实际 all-matched 门槛下压缩到 image cut，
  提交并复制 `CompactionConfirmed`，随后各副本压缩并实际重写本地 Raft 文件；
  新 RegionManager 重开后，TTL、最新 value 和 image 之后的写入保留。

三副本测试实测文件字节如下：

| 副本 | engine segment 回收前 → 后 | Raft 文件回收前 → 后 |
| --- | ---: | ---: |
| 1 | 18,878,492 → 3,147,364 | 18,879,178 → 1,185 |
| 2 | 18,878,492 → 3,147,364 | 18,879,115 → 1,185 |
| 3 | 18,878,492 → 3,147,364 | 18,879,115 → 1,185 |

Raft 本地回归 317 项通过，8 项忽略、37 项 socket 测试过滤；engine 各 suite 合计
158 项通过、25 项忽略；RegionManager 15 项通过。checkpoint 恢复专项 11、配置专项
12 项包含在这些回归中，不能重复计数。workspace 全 target 严格 Clippy（含 tracing）、
格式和 diff 检查通过；[命令、日志及源码摘要](raw-ttl-checkpoint-history-validation-20261004/manifest.json)
独立记录，旧验证文件不修改。

三副本使用 InProcHub 和测试专用 MemoryObjectStore/Enable 注入。compaction decision
由 fixture 提供，确认命令实际复制；没有新增完整 metadata 控制面、NodeRuntime TCP、
MinIO 持久性或跨主机故障验收。单次回收也不证明持续负载下的磁盘/RSS/FD/对象稳态。

## 后续工作

后续的[跨节点 owner 登记组件](RAW-TTL-CHECKPOINT-OWNERS.md) 已支持 data manifest 和
远端 metadata leader，实际发布/正向交接有组合验证。后续
[自动 checkpoint](RAW-TTL-DATA-CHECKPOINT.md) 已接通每组发布/采用及候选历史保留，
自动压缩建议 floor 受本地 ceiling 限制；完整 pending/version/reader 生命周期仍开放。

完整 checkpoint 48 MiB 上限、全量 resident index、累积的 manifest/configuration history
和对象债务仍需解决。下一步补齐 owner 释放、统一整机维护预算并验证
持续回收。完整可转移恢复锚点、TTL split/merge/迁移/安装/新增副本、部署时钟资格、
独立时间历史、完整形式化证明和真实故障验收继续按原阶段范围推进。
