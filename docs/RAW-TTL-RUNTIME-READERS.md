# 运行态 adoption 与 data 启动 Reader

2026-10-05；接续 [读取前发布校验](RAW-TTL-RECOVERY-PREFLIGHT.md)。本轮将 NodeRuntime
实际使用的 adoption 和普通 data group 恢复接入保留读者。第二阶段继续开放，metadata
自身启动的 Reader 与持久恢复授权仍需单独完成。

## 每次实际读取的来源

NodeRuntime 为 RegionManager 配置同一个 root、store identity、metadata-leader router
和当前进程 lifetime。恢复适配器必须取得该 store 的持久 drain outbox，才能准入读取。
这个配置在启动时不访问 metadata，也不请求 Reader 提交。

原始安装镜像使用由已提交 migration operation 和 destination incarnation 派生的精确
destination owner。首次 adoption、V1 receipt 升级，以及原始镜像仍被选中时的 engine
恢复，都通过已有 GET-only image Reader 读取。来源必须完整匹配 manifest 与所有 SST，
且处于 Published。缺少来源或准入回执时不发 GET，也不启动组 driver。

后继 checkpoint 使用本地 WAL/Raft 预检查得到的精确 predecessor。只做两个有界点查询：
优先采用同一 intent 的 Published Version，否则使用其 Published Pending。验证完整
owner binding，不用最新 generation 替代实际 publication，也不靠全表扫描猜来源。
合法 V2 adoption marker 加后继 checkpoint 不读取原始 SST，也不预先查询原始 owner。

`DataCheckpointReader` 将本地 publication 的 predecessor 传给读依赖；普通 WAL open、
installed engine open 和 recovery dependencies 继续共用原有完整恢复校验。uploader
另作本地 pending-journal 预算依赖，实际 GET 经上述 Reader。低层原有 uploader API
保留给其他调用者及组件 fixture；仅有该 API 不代表该调用者已完成读者接入。

## 结束报告与失败

每次 GET 前必须取得 Share 和 Publish 的精确回执。实际对象队列/worker 全部退出后，
才取得 typed `DrainedRead`，先 fsync 原身份的 Finish 报告，再提交 metadata 并清理
outbox。原始 image 使用 tag 22，checkpoint 使用 tag 18；没有新增 ledger 请求。
全程保留外层 store/group 锁。超时返回不能提前结束 pin，取消调用方也不能代替实际排空。

准入、结束回执和对象读取依赖失败返回可重试状态。完成报告未确认时，同一 Reader 的
outbox reservation 阻止后继 GET；已有 sweep 重放原报告。来源从 Pending 交接至 Version
仍由现有精确 successor 规则处理。没有报告的崩溃 Reader 不会被虚构为已排空。
这些操作只结束本次 Reader；来源的 Published pins 继续保留，不授予物理 GC。

## 普通启动的时序

之前 `RegionManager::recover` 在 metadata Raft owner 启动前打开所有 data engine。
在该处等待 Reader 提交会形成启动循环。现在生产路径先核对 durable Active record、
已提交 creation 和实际 store，只记录内存中的 `recovering` 状态，不打开该组 Raft/engine，
不查询 Reader、不读取 SST。磁盘上的 Active 格式保持不变。

metadata、endpoint 和 Raft 接收权限就绪后，100 ms 维护轮次最多尝试一个待恢复组。
每次尝试后轮转，失败的组不会一直占据首位。准入/对象依赖错误继续显示 `recovering`
及原因；本地 WAL/Raft 或恢复后语义校验错误保持 `failed` 隔离。成功后才发布完整恢复
事实，再经已提交 compaction/truncation 约束启动 driver。

维护先处理已提交 removal 和已发布 split 的退役。未打开组可在 group 锁下持久化 Retired，
无需 SST GET；失败先隔离其重试。目录检查失败时不继续启动组。adoption 候选也过滤本地
已提交的 removal、abort 和 sealed parent，排除已启动/退役组，并在剩余 region 间轮转。
因此历史迁移的成功空操作或尚未安装的首个目的组不会永久阻挡后面的组。

重试轮次是有界轮转，尚未增加指数退避。恢复仍同步执行于控制调用，实际 I/O 排空可长于
请求超时；本轮没有验证数百个组的重启耗时，也没有把该机制视为维护延迟验收。

## 验证与剩余范围

[本轮证据包](raw-ttl-runtime-reader-validation-20261005/manifest.json) 保存精确命令、
原始日志、源文件摘要及本轮增量 patch。新增 7 个测试：

- 运行态 adoption、实际 driver 启动及重开；本次读取完成后只释放 Reader。
- 准入失败零 GET、结束回执丢失、持久报告阻止后继读取及重放后恢复。
- 真实 MinIO worker 队列中的暂停 GET：超时后仍保持 Reader 和 group/store 锁直到 worker 退出。
- V2 选中后继 checkpoint，分别使用 Published Pending/Version；故意丢失原始 SST 后仍可启动。
- 三个真实 data Raft 副本回收 WAL/协议前缀后，延迟启动、轮转重试、退役未打开组、重放报告并保留 TTL/尾部。
- 多个 migration 候选的轮转与 terminal group 排除。
- 未打开组退役发布失败后，不再恢复或暴露 group handle。

最终 **158 个不同测试通过，8 个既有测试忽略**：snapshot/install 24、checkpoint
recovery 29、TTL runtime 76、RegionManager 21、drain outbox 8。workspace 全目标严格
Clippy、格式与 diff 检查通过；同一测试的开发期定向运行不重复计数。

adoption 场景使用真实三副本 metadata、鉴权 HTTP/2 duplex 和 MemoryObjectStore。后继
checkpoint 控制在目的端直接写入精确匹配的 committed protocol entry 与 apply batch，
不是通过真实四节点复制产生该尾部。普通启动测试使用三个实际 data Raft 副本和本地原子
metadata ledger fixture，不等同于完整 NodeRuntime 多机重启。暂停对象场景使用真实队列
与内存后端；本轮未执行真实 TCP/MinIO、进程 kill、断电、跨主机或旧二进制升级验收。

metadata 自身恢复发生在它的 Raft owner 启动之前，仍使用未接入的低层读路径，需要独立
持久恢复授权，不能照搬 data 的“等待 metadata 就绪”。原始 image source 的后继交接、
无报告 Reader、全体 Publisher/Reader 清单、旧 Version、负向 Pending、未知 PUT、portable
recovery、物理 GC，以及时钟/容量/部署/完整阶段验收仍开放。V8、KV9LED08 和全部持久格式不变。

## 后续进展

Metadata 自身冷启动的持久授权已接入，见
[bootstrap Readers](RAW-TTL-BOOTSTRAP-READERS.md)。旧 checkpoint 的升级 enrollment、
持久准入关闭、grant 退役与完整 Reader census 仍未完成。
