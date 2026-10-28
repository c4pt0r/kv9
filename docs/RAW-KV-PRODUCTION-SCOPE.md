# Raw KV 首个生产版本范围

2026-10-07：[实际配置到历史磁盘的区间映射](RAW-TTL-CONFIGURATION-INTERVALS.md)
已持久化并接入后台续做。原建组与变更 task 独立核对，推导结果必须匹配实际配置；
追加历史增量推进，补齐原生映射后分代重建，旧/joint/snapshot 缺口保持明确。
新增 13 项测试，相关回归 555 项通过，原有 1 项真实 MinIO 测试忽略；严格 Clippy、
格式及 diff 检查通过。V11 不变，ledger 升为 27。全体历史磁盘、snapshot 祖先与
恢复来源、DATA 关闭、安全释放、GC、merge 和完整生产验收继续保留在原范围内。

2026-10-06：[实际配置时间线持久盘点](RAW-TTL-TIMELINE-CENSUS.md) 已将逐次配置与
分页进度原子保存到 metadata，后台轮转成员、来源与时间线；丢回执、换主、源历史
变化和 WAL 重开可续做。新增 11 项测试，相关回归 542 项通过，原有一项真实 MinIO
测试忽略；严格 Clippy、格式及 diff 检查通过。V11 不变，ledger 升到 26。
完整配置/store 区间、snapshot
祖先和恢复来源、DATA 关闭、安全释放、GC、merge 与生产验收仍在原范围内。

2026-10-06：[逐次实际配置历史与远端查询](RAW-TTL-CONFIGURATION-TIMELINE.md) 已保留
initial、每次 apply 的完整成员集合和 snapshot 边界，包含 joint 角色与已知变更
来源；每页四条，metadata leader 独立核对原 task 和 endpoint。V11/ledger25 不变。
新增 13 项测试，相关回归 601 项通过、原有一项真实 MinIO 测试忽略；严格 Clippy、
格式与 diff 检查通过，开发编译及 Clippy 失败日志保留。
仍需持久后台盘点、完整区间/store 对齐及 snapshot 祖先；DATA 关闭、安全释放、
GC、merge 和完整生产资格继续开放。

2026-10-06：[初始配置与原建组来源](RAW-TTL-INITIAL-ORIGIN.md) 已把磁盘初始 voters
与原始建组 node/store 独立核对，并随来源进度原子持久化；snapshot 当前配置不能
补造缺失历史，替换或导入磁盘不能冒充原生来源。内部查询版本 2，V11 不变，
ledger 升为 25。新增 11 项测试，相关回归 588 项通过，原有 1 项真实 MinIO 测试
忽略；严格 Clippy/格式检查通过。完整旧/joint 区间、历史磁盘与恢复来源、DATA 关闭、安全释放、
GC、merge 和生产资格继续开放。

2026-10-06：[精确来源持久盘点](RAW-TTL-ORIGIN-CENSUS.md) 已把已 apply 配置来源及
分页进度保存在 metadata Raft，后台交替推进成员/来源盘点；丢回执、换主和 WAL
重开后可续做，旧 context/snapshot 缺口不被清除。V11 不变，ledger 升为 24。
新增 9 项测试，相关回归 510 项通过，原有 1 项真实 MinIO 测试忽略。
初始/旧配置区间、全体历史磁盘与恢复来源、DATA 关闭、安全释放、GC、merge 和
完整生产资格继续开放。

2026-10-06：[精确配置来源分页与远端核对](RAW-TTL-ORIGIN-QUERY.md)已接入内部 RPC
和 metadata leader 独立校验；每页四项、精确 store/endpoint fence、旧 context 与
snapshot 缺口保留，超时/取消不提前归还实际 worker 容量。新增 10 项测试，相关回归
352 项通过，所选测试无忽略。仍需持久后台盘点、全体
历史区间与恢复来源；DATA 关闭、安全释放、GC、merge 和生产资格继续开放。

2026-10-06：[实际配置变更的 store 来源](RAW-TTL-CONFIGURATION-ORIGIN.md)已接入迁移的
四条成员变更路径。原 task、精确 store 和配置位置原子持久化，日志重写保留来源，
snapshot 的未知历史不被补造。新增 kind 9 为单 store 前向日志格式；V11/ledger23
不变，旧在线副本仍可能缺少来源。完整历史区间、跨节点盘点、恢复锚点、安全释放、
GC、merge 与生产资格继续开放。新增 8 项测试，相关回归 648 项通过；原有 1 项
真实 MinIO 测试跳过。

更新：2026-09-26。首个生产版本的必选功能为 **Raw KV、TTL、range split、range merge**。这是一份交付与验收范围，不代表四项已经实现或通过生产验收。生产基础问题见 [生产审查](PRODUCTION-READINESS-REVIEW-2026-09-26.md)。

2026-10-06：[catalog 公平锁与后台短等待](RAW-TTL-CATALOG-ADMISSION.md) 已减少
短暂事务锁竞争导致的立即拒绝及 worker 长退避，保留原请求期限和未知结果语义。
新增 5 项测试，相关回归 126 项通过；四线程共存场景的第二次 data/首次 metadata
checkpoint 约 6 秒完成。此为局部进度改善，长期竞争、公平性与稳态容量仍待验收；
完整历史来源、恢复锚点、安全释放、GC、merge 与第二阶段生产资格继续开放。

2026-10-06：[成员持久盘点与后台轮转](RAW-TTL-MEMBERSHIP-CENSUS.md) 已保存逐来源
分页进度、未知节点和不可清除的源历史缺口，并独立核对保留 placement。V11 不变，
ledger 升为 23；新增 8 项测试、相关回归 448 项通过，含四线程 runtime 104 项。
三个旧 census 测试改为检查 owner/索引/placement 原字节保持；checkpoint 共存测试
的第二阶段等待覆盖已有退避策略，最终用例超过 60 秒后恢复。竞争下进度与稳态资源
仍未验收，全体实际历史磁盘/恢复来源、DATA 关闭、安全释放与 GC 继续开放。

2026-10-06：[历史成员跨节点查询](RAW-TTL-MEMBERSHIP-RPC.md) 已从 metadata leader
独立核对来源 store、endpoint generation 和实际 data peer 分页，保留缺口与 Unknown；
旧 V11 方法缺失不降级，取消/超时不提前释放后台读取容量。新增 8 项测试、相关
回归 187 项通过，含四线程 runtime 102 项。尚无全体历史磁盘/恢复来源完成证明，
不授予旧镜像关闭、Version/Pending 释放或 GC；完整第二阶段继续开放。

2026-10-06：[本地成员历史](RAW-TTL-MEMBERSHIP-HISTORY.md) 已从实际 Raft 配置记录
保留被移除/joint 节点，snapshot 的未知祖先前缀保持未知；四项分页和 4096 节点
上限不干扰共识 apply。日志重写增加观察保留检查。新增 8 项测试、相关回归 296 项
通过；完整历史 store/恢复来源覆盖、全局关闭和 GC 仍未完成。

2026-10-06：[DATA checkpoint 交接竞争回归](RAW-TTL-HANDOFF-CONTENTION.md) 已用真实
目录锁验证三副本后台受阻与原意图续做，修正测试的单次交接成功假设。四线程 TTL
runtime 99/99 通过；历史未确认错误的唯一原因、持续负载与生产资格仍未确定。
本轮没有修改生产超时、未知结果语义、格式或 GC 权限。

2026-10-05 背压进展：[Raft append 锁内准入](RAW-WRITE-ADMISSION.md) 修复并发越限、
未提交尾部漏计和批量拒绝类型丢失，覆盖 TTL split Append。默认每组 65,536 条，
非法配置拒绝启动；控制命令与已接受 apply 继续推进。WAL/Raft 实际字节配额、
完整物理回收和长期多组稳态容量仍未验收。

同日后续：[WAL/Raft 字节计费与提案预留](RAW-WAL-RAFT-BUDGET.md) 已覆盖 metadata/
data group 共享准入、重写临时文件和重启后未 apply 债务。实际文件回收才释放容量，
请求超时或 owner 退出不释放。默认阈值 4 GiB；follower/control 可继续超阈值写入，
磁盘安全余量、远端压力、完整物理债务和长期多组资格仍未验收。

同日后续：[本地实际空间余量](RAW-DISK-HEADROOM.md) 已接入数据提案，默认保留
256 MiB，扣除共享预留及采样期间写入；错误/过期读数不放行，跨 device 日志布局
拒绝绑定。余量属于准入策略，尚不是控制/恢复空间保障或全副本容量验收。

这四项以现有 multi-Raft group runtime 为基础：每个 data group 有独立的 Raft peer、持久化状态和 state machine，由节点内共享 worker pool 调度；metadata 保持单独的执行者。动态建组、按范围路由和 split child 激活已有运行路径，见 [`region_manager.rs`](../crates/server/src/region_manager.rs) 和 [多组基准](MULTI-GROUP-THROUGHPUT.md)。现有单机多进程证据不等于跨主机扩展验收；255 槽位的历史消耗、退役资源释放及跨拓扑维护仍须按生产审查补齐。

2026-10-05 后续：[远端副本压力反馈](RAW-REPLICA-PRESSURE.md) 已接入实际 data group
append：按完整当前配置拒绝已知低容量/不确定成员，错误与旧回复不能清除压力。
有界后台轮询和公开 TTL 停写/恢复通过三副本组合测试。metadata 仍按本地准入，
保留目录修复路径；报告不是跨副本预留，全副本空间保障和长期生产资格仍未验收。

2026-09-27 进展：本地 worker/inbox 槽位和退役 payload/FD 回收已修复并通过组件测试，见 [退役验证记录](GROUP-RETIREMENT.md)。当时组件长循环显式清理测试 catalog，以隔离任务表独立的 255 行历史预算。

2026-10-05 后续：[Raft 流式重写与共享临时额度](RAW-RAFT-REWRITE-WORKSPACE.md) 已
限制回收时的临时复制，并在创建临时文件前预留精确输出。额度满的组可以延期，
其他组继续回收；新数据与远端容量报告都扣除在途重写预留。完整控制/恢复空间、
跨节点预留及持续负载下的容量和暂停时间仍需验收。

2026-10-03 进展：[任务历史分页](TASK-HISTORY-PAGINATION.md) 已移除该历史总量上限，保留全部创建、激活及维护凭据。组件长循环现在保留 261 个组的 522 行 catalog，不再清理测试历史，恢复和旧请求重试通过。索引与维护预算、历史安全回收及实际 Chaos Mesh 验证仍未完成，不能视为生产全链路容量验收。

2026-10-04 进展：[第二阶段 TTL](RAW-TTL-PHASE2.md) 已有时间/编码、snapshot 读取、Raft codec/有序 apply 和有预算的清理候选组件；复制回归包含三个投票副本的延迟 apply 与 leader 切换。[V4 升级组件](RAW-TTL-UPGRADE.md) 已接入本地持久格式隔离和逐 store 确认。[全副本激活门槛](RAW-TTL-ACTIVATION.md) 已接入 Runtime 和受保护的提案入口，并有五副本 joint 配置的进程内验证；TTL 组拒绝旧 split seal。局部 Lean 引理仅覆盖时间和单 key 清理。跨节点激活/拓扑验收、公开 API、时钟提供者、后台维护及空间回收仍未完成，第一阶段未完成的基础项继续保留。

同日后续：[Linux 时钟提供者与复制时间观察](RAW-TTL-CLOCK.md) 已有显式配置、异常持续
拒绝、独立区间模型与实际 WAL 重开组件测试。部署时钟资格、真实故障与跨节点组合
验收仍未完成，不能把这个进展视为上面时钟门槛已关闭。

同日公开链路进展：[精确 scope 的 TTL RPC/stream 与客户端/CLI](RAW-TTL-PUBLIC.md) 已实现，
具备 Raft/WAL 和内存 I/O 上实际 HTTP/2 的组件验证。公开管理、跨 range、后台空间回收及
split/迁移互通仍未完成；历史段落描述当时进展，不替代这个完整验收边界。

同日管理链路进展：[公开激活 RPC/CLI 与远端 metadata 目录](RAW-TTL-ACTIVATION-API.md)
已接入全副本门槛，三个真实副本和分离的 metadata/data leader 经 HTTP/2 duplex 验证。
目录变化或任意 store 未确认时不提交 Enable；未知结果不自动重试。部署、自动范围路由、
后台空间回收与 split/迁移互通仍未完成。

同日路由进展：[TTL 自动发现与跨范围分页组件](RAW-TTL-ROUTING.md) 已接入客户端和 CLI，
能力确认前可刷新 scope，数据请求至多发送一次；空页和空范围返回明确续页位置。三副本
公开链路具备本地组合证据，较长 catalog 边界、DeleteRange、完整多组及真实网络验收仍开放。

同日范围操作进展：[完整 catalog 边界与分页 TTL DeleteRange](RAW-TTL-DELETE-RANGE.md)
已接入公开协议 v2、客户端和 CLI；每页有预算、原子删除两 CF 并返回精确回执，未知页
不自动重放。实际 WAL 重开续删和三副本删除通过组件验证，后台物理空间回收、完整
NodeRuntime/网络、范围转移及第二阶段验收仍开放。

同日后台进展：[TTL 时间推进与条件清理调度](RAW-TTL-MAINTENANCE.md) 已接入启动/关闭及
状态输出，具备多组共享预算、轮转、退避、未知追加位置约束和取消排空；续期竞争、
WAL 重开及三副本清理有组件证据。整机其他维护预算、每组物理回收、稳态债务与完整
分布式验收仍开放，默认启用该任务不代表启用 TTL 或通过生产验收。

同日恢复进展：[数据组 checkpoint 与 TTL 恢复](RAW-TTL-CHECKPOINT.md) 已接入 Active group
重开，核对实际 image、精确 publication 和尾部；三副本测试各自回收约 18.9 MB 至 3.1 MB
的 engine segment 后保留 TTL 和后续写入。自动发布/采用、对象 owner、Raft 压缩时的
恢复证据保护及完整稳态资源验收仍未完成，第二阶段继续开放。

同日压缩进展：[checkpoint/Raft 共享保护与历史配置修正](RAW-TTL-CHECKPOINT-HISTORY.md)
已接入实际 driver/peer；并发选择、失败窗口、新旧成员配置及重启有组件验证。三副本
在回收 engine segment 的同时，各自把约 18.9 MB 的 Raft 文件重写至 1,185 字节后成功
恢复。每组自动发布、对象 owner、整机预算和持续负载下的稳态资源验收仍开放。

同日对象登记进展：[跨节点 checkpoint owner](RAW-TTL-CHECKPOINT-OWNERS.md) 已具备有界
内部 RPC 与 metadata leader 路由；data leader 和 metadata leader 分离时，八处提交后
丢回执均沿原计划续做，实际 checkpoint/TTL 和 owner 状态重开保留。自动数据组发布/
采用、候选历史保留、Reader/Version 释放和对象 GC 仍未接通，不构成空间稳态验收。

同日自动回收进展：[data-group checkpoint 调度与候选保护](RAW-TTL-DATA-CHECKPOINT.md)
已接通有界发布/采用、持久 pending 重启恢复和退出排空。三个副本自动把各自 engine
segment 从 18,903,328 降至 2,627,627 字节；上传/交接故障后关闭新触发仍恢复原计划。
整机维护预算、Reader/旧 Version/负向 Pending 释放、对象 GC 和历史有界仍开放，
上述单次回收不构成稳态资源或完整阶段验收。

同日配额进展：[七类维护共享准入](RAW-TTL-SHARED-MAINTENANCE.md) 已约束 TTL、两类
checkpoint、压缩及回收的节点总占用和派发速率。实际双上传阻塞时 TTL 继续，取消
等待真正 I/O 返回才释放配额；完整维护路径、字节/内存/CPU 与物理债务预算仍待完成。

同日对象 I/O 进展：[payload 字节配额与 checkpoint GET 上限](RAW-TTL-OBJECT-IO.md)
已接到配置的对象 worker，包含幂等 PUT 的比较读；限流超时后原 pending 可恢复。
这不包含网卡硬限速、整机内存/CPU、本地 WAL 字节或稳态债务验收。

同日内存路径进展：[checkpoint 规划与 pending journal](RAW-TTL-CHECKPOINT-MEMORY.md)
已在复制记录前核算完整 SST 大小，流式读写兼容 journal，并释放 reload 前的重复 plan。
这消除了具体的完整 image 副本；共享内存准入和持续 RSS/磁盘/对象稳态仍未验收。

同日 image 预算进展：[共享 plan/pending 字节准入](RAW-TTL-IMAGE-BUDGET.md) 已接入
新规划与磁盘恢复，默认 96 MiB，实际 worker 结束前保持额度；完整临时内存与 RSS 仍未验收。

同日对象缓冲进展：[PUT/GET buffer 共享额度](RAW-TTL-OBJECT-BUFFERS.md) 已绑定真实
共享字节 owner，超时后队列或 SDK 持有的引用继续计费；checkpoint 读回/恢复改用共享
结果接口。默认 96 MiB，完整 RSS/codec/恢复索引与稳态验收仍开放。

同日恢复内存进展：[SST 借用校验与直接恢复索引](RAW-TTL-SST-VIEWS.md) 已去掉生产
批量解析副本和全量 restore WriteBatch；后续 SST 失败丢弃所有未发布状态。SST 写入端、
最终常驻索引及整机 RSS 仍未完成预算与验收，48 MiB 全量 image 限制仍在。

同日编码边界进展：[manifest 有界流式编码](RAW-TTL-MANIFEST-ENCODING.md) 已在输出
扩容前检查 1 MiB frame 上限，消除完整 JSON 临时副本；这不覆盖输入/decode 分配、
manifest 历史数量或整机共享内存预算。

同日规划内存进展：[冻结视图直接编码 SST](RAW-TTL-BORROWED-WRITER.md) 已移除生产
planner 的 owned-entry 副本，预检并取得精确 image 额度后直接生成最终 buffer，保持
原格式、分片与哈希。常驻索引、snapshot 保留、元数据和整机 RSS 仍未完整预算与验收。

同日转移格式进展：[TTL 镜像安装与 learner 恢复](RAW-TTL-INSTALL.md) 已修正安装器
拒绝合法 clock/expiry/history 的问题，替换镜像禁止移除 activation 或回退水位；
真实本地 voter/learner 的 capture、安装、尾部续期/清理和 WAL 重开通过组件验证。
完整迁移协调、恢复授权、split/merge 格式转移及真实网络/MinIO 验收仍开放，TTL 旧 seal
保护继续生效。

同日采用后恢复进展：[adopted generation 恢复校验](RAW-TTL-ADOPTED-RECOVERY.md) 已补上
committed WAL/history、pending 保护和逐项 range/TTL 检查，拒绝中间回退后再修复的记录。
后续 checkpoint/压缩后的启动使用实际 base，并要求 committed catalog 决策。原始镜像
对象仍须可读，完整继任恢复锚点、owner 释放与迁移/生产验收继续开放。

同日持久记录进展：[adoption V2 的 TTL 事实](RAW-TTL-ADOPTION-FACTS.md) 保存经原始镜像
验证的 activation/水位，旧 V1 在原始对象可读时升级。较新 checkpoint 的完整恢复验证
通过后，已经不再读取仅旧镜像引用的 SST；测试删除旧 SST 后重启成功。此进展消除了
上一段的一条读取依赖，没有授权释放对象 owner，完整继任锚点与生产 GC 仍待完成。

同日收敛预算进展：[checkpoint 历史查询](RAW-TTL-SETTLEMENT-BUDGET.md) 改为精确窗口
优先，后备 effect 每次四行并保留游标，同一 pending 排除并发驱动。三副本验证旧
pending 得到精确拒绝后仅清除本地 journal，全局 pin 保持 Published。历史总量、
执行者排空与负向 Pending 释放、GC 及整机 CPU/内存预算仍待完成。

同日本地排空进展：[checkpoint 对象任务范围](RAW-TTL-OBJECT-EXECUTION.md) 已修复
同步调用超时后提前归还维护配额的问题；data/metadata tick 保留容量至实际队列与
job 退出。真实 checkpoint dispatcher 和对象 worker 的组合测试通过，原 pending
保持可恢复。远端未知 PUT、其他执行者和全局负向释放/GC 仍需持久协议证明。

同日执行登记进展：[持久 Build owner](RAW-TTL-EXECUTOR-OWNERS.md) 已在真实上传前
记录 node/store/process 身份与完整 closure；本地排空后按原代交接，关闭未确认会阻止
同一 worker 开始新上传。丢回执、代际重试、自动 worker 及 metadata WAL 重开通过验证。
全局完整枚举/关闭、崩溃遗留 pin、owner 历史容量、负向 Pending 释放和 GC 仍未验收。

同日容量修正：[owner 分行存储](RAW-TTL-OWNER-ROWS.md) 已取消生产 ledger 每 resource
累计 4096 owner 的单值限制；旧 Released 记录保留，并以有界 base 原子升级。4097 个
释放记录仍能以固定点读/写入量继续操作，实际 WAL 重开和远端升级丢回执有组合证据。
旧 base 仍有最多 4096 项的解码成本，所有记录的物理总量、历史回收与生产稳态继续开放。

同日恢复信息进展：[checkpoint 精确意图](RAW-TTL-CHECKPOINT-INTENT.md) 已将完整
manifest 与前驱 generation 在上传前归档至 metadata Raft。按 Pending ID 跨节点
读回、本地 journal 清除及 WAL 重开有验证；记录本身不证明所有执行者关闭，不授权
负向 Pending 释放。全局关闭协议、历史压缩和真实环境验收仍未完成。

同日关闭进展：[checkpoint 准入关闭与恢复](RAW-TTL-CHECKPOINT-ADMISSION.md) 已加入
单调关闭、执行者有界索引查询及未进入代际取消；真实 Raft 丢回执、metadata WAL
重开和 data worker 零 I/O 失败恢复已验证。仍不把关闭/空页视作全体 publisher
排空；legacy backfill、旧 binary 门槛、崩溃遗留 pin 和负向释放证书待完成。

同日遗留处理：[未 Publish 执行者回收](RAW-TTL-UNPUBLISHED-RETIREMENT.md) 已支持
Closed gate 下精确 Held 代际的有界恢复和独立持久证据；九个遗留者跨丢回执及真实
WAL 重开完成 pin 释放，Published/Quiesced 与全局保护保留。完整已发布执行者终止、
负向 Pending 释放和物理空间回收仍开放。

同日后台续做：[checkpoint 全局发现与持久分页](RAW-TTL-CHECKPOINT-SWEEP.md) 已能
处理没有本地 journal 的 gated 尝试，接入 metadata leader 与第八类共享维护准入。
两次丢回执、跨尝试轮转、leader 切换、WAL 重开和破损值隔离已有实际 Raft 覆盖。
Done 仍允许 Published 遗留者存在；无 gate 的 legacy backfill、全局终止证明和
负向释放继续开放，历史物理增长未消除。

2026-10-05 后续：同一维护链路已按精确提交窗口完成正向 Pending→Version 交接
和负向准入关闭；本地缺历史时使用鉴权远端查询。有界节点轮询保留 Unknown，
回复后复核 endpoint generation，超时不提前释放实际读取槽。无本地目标组 handle
的 metadata leader 借远端 follower 收尾已有 Raft/duplex 验证；完整 placement、
真实进程部署及负向 Pending/旧 Version/Reader 的释放仍未验收。

同日执行者事实进展：[逐代排空记录](RAW-TTL-EXECUTOR-DRAIN.md) 已从真实对象 scope
捕获 PUT 发出/响应计数，按精确 generation 保留不可改写记录；新代成功不覆盖旧代
未知，Release 后报告丢回执也阻止同 worker 新上传。实际队列、三代记录、认证 Raft
和 WAL 重开有覆盖。缺失报告的崩溃遗留者、远端未知写入隔离、后端终止资格和
全局 publisher 完整性仍未完成，本轮没有新增全局 Pending 释放或对象删除权限。

同日 split 安全修正：[普通 Raw 复制屏障](RAW-SPLIT-POPULATION-FENCE.md) 已将 runtime
复制改为有界专用指令，在子范围绑定后拒绝延迟批次，避免覆盖新值或复活删除；三副本
复制/发布/WAL 重开已验证。旧日志解释保留，旧 proposer 混部门槛、TTL 元数据/水位
转移、完整 staging/恢复协议和容量验收仍开放。

同日 TTL 转移进展：[持久复制状态](RAW-TTL-SPLIT-STATE.md) 已保留原始 expiry、
继承空/非空子组水位，按计数与摘要完成复制，严格校验 transfer WAL 原子布局。
三副本中途重启换主续传、完成后续期/清理和再次恢复通过。生产新指令仍禁止普通
提案；全副本新格式确认、源提交证明、runtime 复制/发布及容量验收继续开放。

同日升级边界进展：[V5 磁盘/服务隔离](RAW-TTL-SPLIT-FORMAT.md) 已拒绝旧 V4 owner
与格式回执；全配置 TTL 激活门槛改为要求 V5。源认证和 Seal/Begin 生产门槛尚未
连接，TTL split 继续关闭；版本服务隔离不构成保持服务的滚动升级验收。

同日 runtime 接入：[TTL 源核验与受保护复制](RAW-TTL-SPLIT-RUNTIME.md) 已连接既有
seal/populate/publish AdminApi，逐页验证 V5 全副本配置与原始 value/expiry；三副本
子组恢复换主续传、空侧水位、目录发布与发布后公开 TTL 读写通过。未完成 import
checkpoint、超大行、稳态资源和完整自动续做、NodeRuntime/真实网络验收仍未完成。

同日调度修正：[split 后台续做](RAW-TTL-SPLIT-MAINTENANCE.md) 已分离新触发与已提交
intent 的恢复驱动。阈值 0 时实际三副本后台完成 seal、两侧复制和发布；节点控制
循环不再同步等待复制，第九类共享准入限制单节点一个 split，并排空实际任务。
全目录扫描、staging checkpoint、WAL/内存稳态与完整 NodeRuntime 验收仍开放。

2026-10-05：[未完成导入 checkpoint](RAW-TTL-STAGING-CHECKPOINT.md) 已在实际 worker
中贯通持久 Pending/owner、导入镜像检验、publication 和恢复，三副本 engine WAL
段回收、重开换主后 Finish/目录发布通过。tag 13 区分新语义，V6 排除旧 owner 和
服务路径；tag 7 不变。导入 Raft 截断、大镜像、稳态和完整部署验收继续开放。

同日后续：[导入 Raft 回收](RAW-TTL-IMPORT-COMPACTION.md) 已补齐未绑定子组的自动
规划，复用已提交 floor、全成员 matched 和 checkpoint history 保护。三副本实际
文件回收与授权重开、换主 Finish/发布通过；落后 learner 阻止截断。48 MiB 镜像
限制、长时间阻塞下资源预算、merge 和完整生产验收仍未完成。

同日后续：[checkpoint 逐 SST 落盘](RAW-TTL-SPOOLED-CHECKPOINT.md) 已接入生产
metadata/data worker，journal 重开和上传不再持有完整镜像 buffers。实际三副本
导入镜像大于显式测试额度仍可完成恢复链路；48 MiB 格式上限、resident index、
磁盘/CPU/RSS 稳态和其余生产验收仍保留。

同日后续：[V2 大 checkpoint](RAW-TTL-LARGE-CHECKPOINT.md) 用新 tag 14/V7 floor
保留旧 tag 7/13 语义，实际三副本超过 48 MiB 的导入/Bound 镜像可完成发布、回收
和恢复。增加共享活动匿名 spool 字节额度，并修正 metadata 启动加载遗漏。
大镜像 capture/迁移仍未接通；durable pending/WAL 磁盘债务、常驻索引、长期 GC、
merge 和完整第二阶段生产验收继续开放。

同日后续：[持久 pending 磁盘准入](RAW-TTL-PENDING-DISK.md) 已覆盖 metadata/data
journal、发布临时文件和仍打开的旧 inode，启动时盘点，授权删除后归还。
满额恢复保留精确 durable plan，实际 worker 可在已有债务高于新额度时完成结算；
WAL/Raft、整个文件系统、CPU/RSS、公平性与完整生产验收继续开放。

| 必选功能 | 当前状态 | 交付要求 |
| --- | --- | --- |
| Raw KV | 已有 Put/Get/Delete、batch、scan、delete-range 与范围路由 | 保持明确的确认、线性一致读、单 range 原子性、分页和未知写语义；修复背压与客户端拒绝分类 |
| TTL | 公开激活、RawTtl RPC/stream、自动路由、分页 Scan/DeleteRange 客户端/CLI 已接通；旧 Raw 接口仍拒绝 TTL，完整生命周期未验收 | API、复制命令、WAL/SST、读过滤、恢复与后台空间回收完整贯通 |
| range split | 已有 intent、seal、child population、publication、retirement 和自动触发 | 完成故障恢复、暂停时间和资源预算验收；关闭新触发后仍能完成已提交操作 |
| range merge | `crates/region/src/split_merge.rs:86` 仅有候选判断，尚无跨组执行协议 | 相邻范围安全合并、原子路由发布、可恢复状态机和旧副本资源释放；沿用 D05 的安全依赖 |

事务、lease reads、物理双 WAL 合并与 Redis 性能对齐不列入本次功能必选范围。现有持久性、备份恢复、访问隔离、升级与资源有界要求继续作为上线门槛。范围调整不自动关闭任何已有 issue 或验收项。

2026-10-06：[DATA 保留 placement 盘点](RAW-TTL-DATA-STORE-HISTORY.md) 已加入原子任务
修订、来源计数、有界扫描和自动续做，保留已移除副本及中止迁移目标。磁盘/内部服务
floor 为 V11，retention ledger 为 22。它不覆盖全体实际历史配置或全部恢复来源，
不授予旧 Version/Pending 释放或对象 GC；四线程回归中的一次未确认交接仍待确定原因，
串行回归通过不等于并发或生产资格，完整第二阶段继续开放。

**Raw KV 契约**

- 成功写的确认位置必须对应已提交、按序应用且可恢复的结果；明确拒绝与结果未知分开处理。
- Get、BatchGet 和单 range Scan 保持已声明的一致性；TTL 判定必须与这次读取使用的数据视图和时间语义一致。
- 单 range BatchPut 保持原子性；跨 range batch 在没有对应协议前明确拒绝。跨 range Scan 维持显式 cursor、每段一致性，不能被误解为全局统一快照。
- DeleteRange 的逐段提交和部分完成契约必须可见；自动重试只能走已证明安全的路径。

**TTL 是数据语义与格式的一部分。**

实现前需要在 API/格式契约中确定 TTL 单位、起算点、未指定/零值/溢出、覆盖写是否替换旧 TTL，以及 batch 与分页的观察语义。这些选项尚未由本范围文件决定。

设计应让一次被复制的写携带已经确定的到期元数据，例如 `expire_at`，并与 value、写入版本一起原子持久化。不能让各副本 apply 或恢复时重新用本机 `now + ttl` 计算期限；同一次提案的重试也不能重新起算。新增格式必须区分旧值和带到期元数据的值，并说明旧 reader/writer 的兼容或拒绝策略。

绝对时间戳本身不足以解决 leader 切换与时钟回拨。必须先明确支持的时钟偏差/回拨模型、时间权威或水位及异常处置；在声明的模型内验证到期判定不会倒退。禁止仅凭单机时钟测试就宣称分布式 TTL 正确。若选择复制的时间水位，split/merge/recovery 也必须传递并验证该水位。

读侧按契约隐藏过期版本，空间回收在后台受预算约束地推进。后台过期删除若与新写并发，必须绑定观察到的写入版本或等价条件，避免旧清理任务误删新值。单纯读过滤不代表磁盘空间得到回收。

Get、BatchGet、Scan、分页和 range 操作都要覆盖 TTL；过滤掉一页中的过期键后必须继续正确推进 cursor，不能把一个空的过滤结果误认为范围已经结束。

**Split 与 merge 共用所有权和数据格式约束。**

Split 必须保留原值的到期元数据和写入版本，不能从复制时刻重新起算 TTL。若复制或 digest 校验要过滤过期数据，各参与方必须使用同一个经过定义的 cut/时间依据；各自读取本机时钟可能产生不同的复制集合与摘要。

Merge 限制为同一 tenant/keyspace 内相邻、格式兼容的范围；副本布局若暂不支持，必须在产生副作用前拒绝或先完成明确的对齐流程。已有 split/migration/merge intent 之间要有互斥或经证明的协调规则。

Merge 需要自己的持久化协议：操作身份、两侧准备与写入 fence、确定的数据 cut、数据或文件引用归并、完整性校验、原子目录发布，以及旧范围退役。超时、leader 变化、重复请求、abort 与发布后的恢复都要有合法状态转换；不能把它实现为两个目录行的直接替换，也不能简单反转 split。

合并后的范围应恰好等于两侧范围的并集，不出现空洞、重叠或双写所有者。旧 epoch 请求必须被明确拒绝或按受约束的路由规则处理。TTL 期限、删除状态和 retention owners 必须保留；释放旧范围资源要晚于 durable publication 和读者/执行者退出。

协议支持与自动调度策略分开交付。先验证可恢复的手动 split/merge，再接入受预算限制的触发器；使用有意义的数据量/负载统计、滞回与冷却，避免 TTL 大量过期后 merge、随后又立即 split 的振荡。暂停新调度不得冻结已提交操作。

**联合验收**

| 场景 | 必须验证的结果 |
| --- | --- |
| TTL 写 → split → merge → 全集群重启 | value、到期元数据、已提交删除与范围边界保持正确；TTL 不重新起算 |
| 旧版本过期清理与覆盖/续期并发 | 旧清理任务不会删除新版本；期限遵循已定义的覆盖语义 |
| leader 切换、时钟异常、节点重启 | 在冻结的时间模型内判定不倒退；异常行为符合明确契约 |
| Scan 包含整页过期键，期间边界变化 | cursor 前进，无静默截断；符合既定的每段一致性和重试语义 |
| split/merge 各持久化阶段崩溃 | 无双写所有者、无范围空洞、无旧值复活；新协调者能继续合法操作 |
| 关闭自动触发器时已有操作在途 | 已提交操作仍能完成或进入协议允许的终态 |
| 累计超过 255 次创建/退役，活跃组数保持较小 | worker 槽位、engine、文件句柄和 payload 可回收；tombstone 不耗尽活跃容量 |
| 固定规模活跃数据，持续 TTL 写与 split/merge | RSS、FD、本地磁盘、对象存储债务和维护延迟符合预声明预算 |
| 大量同时到期、慢副本、对象存储中断 | 前台拒绝语义正确，控制面能推进，后台任务不引起无界排队 |

核心协议继续遵循项目的形式化证明、独立历史检查和实际 Chaos Mesh 要求。TTL 历史检查器必须包含声明的时间模型；只比较最终存活键不够。性能验收使用 release、持久磁盘和完整后台维护周期，并报告成功、明确拒绝、未知结果与 split/merge 暂停时间。

**执行顺序**

1. 修复共同基础：并发背压、typed refusal、group 注销/资源归还、公平回收和既有 split 的恢复驱动。
2. 定义并实现 TTL 的 API、时间模型、版本化编码与 Raw 读写路径，同时贯通每个 data group 的持久化和空间回收。
3. 让现有 split 在 TTL 下保留期限、版本和恢复语义，完成其容量与故障验收。
4. 按 D05 实现相邻 range merge 协议，复用已完成的生命周期和数据格式基础，再接自动策略。
5. 执行 TTL × split × merge × 故障恢复联合验收，连同生产基础门槛一起决定是否发布。

以上是依赖顺序，不要求所有设计工作串行；TTL 格式与 merge 协议可提前设计。没有新增交付日期、性能承诺或未经验证的完成状态。
