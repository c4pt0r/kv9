# 已登记 Reader 的连续代际闭合

2026-10-05，接续 [Reader 准入与逐代记录](RAW-TTL-READER-ADMISSION.md)。当前 owner
已 Released 不等于它的所有旧代都具有准入和结束证明。本轮把全局 Reader 反向索引、
逐代来源记录和精确 checkpoint 拒绝事实接成可恢复的有界汇总；不授权 Pending、
旧 Version 或对象释放。

## 连续历史与阻塞

`reader_lifecycle::history` 从调用者给定的稳定 view 中按 generation 数字连续查询，
每页最多四代。缺失行也占一个位置并明确返回未知，不能用遍历“现存行”的方式跳过
缺口。请求的 operation、owner、代际上界和游标都须匹配当前完整 owner。返回值同时
携带实际当前 token，汇总器据此检查代际变化是否已被变更标记捕获。

汇总必须先有独立核验后持久保存的精确拒绝事实，以及完成的全局进程 Reader 索引。
只有 Build gate Closed 时仍返回 Open；正向 Version 的正常恢复读不会被这个检查关闭。
每个登记 owner 都从第 1 代检查到当前代，保留以下互斥计数：

- `drained`：有协议准入且结束报告已经提交。
- `unentered`：结束请求为从未成功登记的该代建立了终态。
- `legacy`：旧代有结束观察，但不能补造历史准入。
- `missing`：没有该代记录，属于未知。
- `active`：有准入记录，尚无结束报告。

Entered 记录的 Share 和非零 Publish revision 必须早于拒绝事实；不满足则拒绝汇总。
后来的成功代不会抵销早期的缺口、旧代或活跃代。只有遍历完全部已登记 owner，且每代
都归入 drained 或 unentered，才报告 `registered_generations_closed`。空目录也只说明
当前登记范围为空，不说明外部、旧协议或持久 bootstrap 读者不存在。

## 变更失效与原子恢复

ledger 升为 `KV9LED11`，新增 tag 26 的 `KV9RCL01` 游标、tag 27 的 `KV9RCM01`
变更标记，以及 request tag 25 `AdvanceReaderClosure`。游标绑定 root、Pending、
拒绝/关闭 revision、源变更 revision、当前 owner/代际位置和累积计数，编码最多
256 字节；变更标记为 96 字节。

每次请求推进一个 owner 的至多四代。选择下一个 owner 时只查询固定大小的反向索引
页，最多检查五条索引；资源点读仍受既有单 owner 资源上限约束。请求不能提交自己的
结论，只能提交期望游标序列；metadata 在当前 view 重新计算。紧邻的丢失回执重试
无变更确认，更旧的游标不能跳页。

批次边界统一检查 Reader owner、注册、generation 和反向索引的有界写集。相关变更
与变更标记原子提交，覆盖正常结束、未进入结束、旧 Reader 的通用转移和旧索引补齐。
已有 Reader 的拒绝事实提交也更新标记。汇总发现标记变化时重新开始；已完成的旧汇总
不会继续作为新 view 的闭合结果。标记缺失、回退或早于拒绝，游标损坏以及格式降级
均拒绝。其他 attempt 的 Reader 或纯 Build 更新不使本 attempt 的 Reader 汇总失效。

后台复用既有 metadata 维护槽位。在每个 attempt 的 Build 汇总本次不再需要推进时，
至多推进一个 Reader 汇总页，再继续原有轮转。没有新增独立后台任务或逐 attempt
常驻集合；同一 pass 不会同时提交 Build 和 Reader 两个汇总页。增加 Reader 页数、
重启次数、闭合和阻塞观察的统计。

## 升级和验证

最终八项检查全部通过：380 项测试通过，2 项真实 MinIO 测试忽略，新增 8 项测试。
完整 metadata、TTL runtime、bootstrap、outbox 与 checkpoint 恢复回归，以及 workspace
严格 Clippy、格式和差异检查通过。

10 的 Reader 代际记录及既有索引原样保留。升级不补造旧记录或修复缺失历史；已有新
游标/标记但使用旧 header 时拒绝。旧 planner 不能解释 11；store/service floor 仍为
V8，混合旧二进制部署资格继续开放。

测试覆盖五个 owner 的 65 代连续汇总、历史缺口/旧代/活跃代保留、关闭 Build gate
但没有拒绝、晚到结束和未进入终态导致重算、无关变更隔离、损坏/降级拒绝，以及
空登记集和 10 格式升级。13 与 113 个 owner 或 generation 的页成本相同；测试同时
检查迭代条数和点读次数，并拒绝使用全表 scan。

真实 WAL 尾部截断验证汇总页与游标原子提交，也验证晚到 Reader 结束与失效标记原子
恢复。三 metadata Raft 副本、鉴权 HTTP/2 duplex 组合测试使用生产 Reader scope：
先完成九个没有对象任务的读 scope，第十代运行真实被暂停的 GET。丢失已提交的第一页
汇总回执后切换 leader，原请求可确认，后台继续汇总并保持 active 阻塞；GET 真正退出
后原进程才提交结束，旧结果立即失效。后台重算后，另一个被拒绝的新 scope 提交实际
空排空报告，形成第十一代 Unentered，再次触发重算。所有服务/driver 停止后重开真实
WAL，保留完整汇总和 Published Pending。该测试不等同于进程断电、TCP/MinIO 或完整
NodeRuntime 部署验收。

完整命令、源码摘要和结果见
[验证包](raw-ttl-reader-closure-validation-20261005/manifest.json)。

## 第二阶段仍需完成

该汇总证明的是已登记进程 Reader 的连续历史观察。缺少全体 Reader/Publisher 的
部署来源资格、持久 BootstrapReader 和恢复引用盘点时，不能据此释放负向 Pending。
旧代和缺口继续阻塞；索引、终态及代际历史的总量尚未有界。持续变更可能持续使汇总
重启，本轮不宣称长期稳态或并发负载下的回收期限。

不同新镜像替代旧 Version、未知 PUT 隔离、原 store/进程不再使用对象的完整资格、
全组缺凭据恢复、历史压缩、物理 GC、时钟部署以及 [第二阶段](RAW-TTL-PHASE2.md)
的完整生产验收继续开放，原范围保持不变。
