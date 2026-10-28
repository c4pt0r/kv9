# TTL 导入期间的 Raft 日志回收

日期：2026-10-05。接续 [导入 checkpoint](RAW-TTL-STAGING-CHECKPOINT.md)。自动
compaction 现在包含尚未绑定 range 的 TTL 子组；三副本回归验证实际 Raft 文件
缩小、完整重开、换主以及 Finish/目录发布。第二阶段和生产验收仍未完成。

## 修复和权限边界

原执行器已经从实际 `group_handles` 获取副本，但自动规划只遍历已提交 range
绑定，导入子组因没有绑定而一直无法触发。现在规划逐项遍历真实本地 handle，
对已绑定组保持原触发条件；对其余组额外要求：

- 实际 engine 存在可解码 import，目标 region 匹配。
- import 的完整 split plan 与已提交 metadata plan 相等。
- 实际 driver 返回经校验的 import，且不可变 spec 与观察一致。
- 本地已有选中的 checkpoint；没有镜像时不自动为 import 提出截断 floor。

这些检查只决定调度资格。floor 不超过本地 applied position、已选镜像和现有
selected/pending checkpoint history 上限。实际截断仍复核 history；另一副本
选中更早镜像时可以拒绝本地执行，不会越过它的恢复保护。

仍使用原 kind-110 已提交 metadata 决策、tag 9 `CompactionConfirmed`、同步 engine、
leader 检查所有 voter/learner matched、group 内复制 confirmation、各副本独立
截断和 crash-safe `rewrite_log`。没有新日志语义或格式升级，当前 floor 仍为 V6。
Import target 不会因此成为服务绑定，不创建 `RANGE_KEY` 或公开 RawDirectory 条目。

恢复前仍须由已提交的 truncation/compaction 决策授权磁盘上的 compacted base。
`RecoveryDecisions::checked_base` 提取这一现有检查，真实 RegionManager 启动与
回归使用同一检查；没有 metadata 授权时拒绝构造 Raft owner。

## 调度与容量

规划和物理回收逐项读取有序 handle，游标越过不可用、尚未开始导入及失败候选，
每轮最多访问 32 组，最多规划/处理一组。没有全量 handle-vector 复制。
`KV9_AUTO_COMPACT_ENTRIES`、`KV9_AUTO_COMPACT_BYTES`、`KV9_AUTO_COMPACT_AGE_SECS`
维持原阈值语义，均为 0 时停止新规划；已有 committed floor 仍可执行。
`KV9_RECLAIM_RAFT_LOG_BYTES=0` 继续禁用物理重写。

NodeRuntime 保留 CompactionPlan、Compaction、RaftReclaim 三类共享维护准入。
规划、执行和重写的函数已抽到同一 runtime 模块，测试直接调用生产 pass。
完整 catalog range/history 发现仍非分页；confirmation 等待最多 10 秒，重写仍
持有 peer 锁，本轮没有将这几个控制 pass 改成后台异步任务。阈值不是磁盘硬上限，
也不保证慢副本或旧 checkpoint 长期阻塞时的稳态空间与延迟。

## 验证

[证据包](raw-ttl-import-compaction-validation-20261005/manifest.json)保留本轮源码
摘要、命令、原始输出及前一包链接。三副本场景先通过公开 TTL API 写入 300 个
60 KiB 值及永久/过期值，复制后停在 Finish 前，由生产 checkpoint worker 发布并
选中导入镜像，再由真实自动规划、执行和物理回收 pass 处理未绑定子组。

本轮 **99 个不同 Rust 测试通过、0 个忽略**：runtime 38、RegionManager 恢复 18、
data-group checkpoint recovery 18、Raft storage 11、metadata compaction 2、maintenance
12。严格 workspace Clippy（含 tracing/checkpoint-testing）、格式和 diff 检查通过。

每副本 Raft 文件从 **18,459,489 bytes 降到 1,796 bytes**。回归另外检查：

- 无选中镜像的 import 不具备自动规划资格；全零阈值和 metadata follower 不规划。
- 超过 selected checkpoint 的截断明确拒绝，磁盘 prefix 不变。
- 只有 metadata 决策、尚无 group confirmation 时，follower 不截断。
- 加入实际未追上的 learner 后，leader 拒绝截断且不发布 confirmation；经真实
  配置变更移除该 learner 后，同一已提交 floor 可以继续执行。
- 禁用重写时，逻辑截断不缩小文件；启用后实际磁盘文件缩小，仍无服务绑定。
- 三个 child 的 engine/Raft 都停止并重开；缺失 committed catalog 授权拒绝，
  正确授权恢复后换主，完成 Finish、目录发布和 TTL 原期限及续期验证。

本地场景使用 InProcHub、认证 HTTP/2 duplex、磁盘 Raft/WAL 和 MemoryObjectStore；
fixture 手动安装已发布 RawDirectory，没有启动完整 NodeRuntime 网络进程。
大页 child tick 为 100 ms，沿用前一轮存储测试设定。开发时补充 membership 案例
曾误等 command-only applied watermark；已改为检查真实 configuration apply
位置和成员列表，未改变生产确认逻辑。组合回归还发现重开后在重放保留的配置
条目前 campaign，会被 raft-rs 拒绝，随后测试依赖随机 timer 选主。现先通过真实
driver 重放，再检查三 voter、无 learner 的配置并显式换主。重开后的 child tick
也统一为 100 ms；未以重试掩盖已接受写，测试时钟不构成生产延迟/选举资格证明。

没有新增形式化模型、TLC、真实 TCP/MinIO/进程 kill/旧 binary 或跨主机验收。
48 MiB 全镜像限制、全量 hash/序列化成本、超大单值、旧 Reader/Version/Pending
释放、未知 PUT 隔离、长期对象/历史增长、merge、全节点资源预算和完整生产资格
继续开放。
