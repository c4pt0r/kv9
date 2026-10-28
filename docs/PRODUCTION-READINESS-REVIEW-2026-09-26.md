# 生产目标下的限制与优化审查

审查日期：2026-09-26。代码基线：`4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
本次审查不修改运行时代码，也不覆盖现有两份 checkpoint 测试的未提交改动。

后续实现（2026-09-27）：[本地组退役修复](GROUP-RETIREMENT.md) 处理了 R5 的 worker/inbox 槽位、payload 句柄与 tombstone FD 回收；下文保留原始基线发现。当时新验证还发现 metadata 任务表独立的 255 行历史上限。

后续实现（2026-10-03）：[任务历史分页](TASK-HISTORY-PAGINATION.md) 已移除该历史总量上限并保留旧请求和恢复凭据，长循环不再清理测试 catalog。全量 metadata、局部退役/恢复及 WAL 重开测试通过；索引与维护预算、历史回收及完整容量与故障验收仍未完成。

结论：目前具备多组 Raw KV、迁移、分裂和部分持久化机制，但尚未满足长期运行的生产数据库要求。近期验证建立了若干组件的正确性；资源回收、故障后持续服务、拓扑变化和客户端语义之间仍有缺口。下一阶段应先建立“在声明的容量和故障预算内持续可用、资源可回收”的闭环，再追求更高 QPS。

首个生产版本的必选范围为 **Raw KV、TTL、range split、range merge**；具体契约、当前实现缺口和联合验收见 [首版范围](RAW-KV-PRODUCTION-SCOPE.md)。TTL 和 merge 都是上线前必需功能。事务、lease read、Redis 性能对齐可分期。持久性、恢复、容量、访问隔离和升级规则必须与对外承诺一致，不能只写成无限期的 known limits。若限定可信单租户、维护窗口或数据规模，必须把这些限制变成可验证的准入条件。

**证据范围**

- 源码与当前文档交叉审查；旧增量文档只代表当时的结果，不自动代表 HEAD。
- 继承 9 月 25 日的三副本背压复现：上限 120、原有 119，16 个并发写全部成功，按实现的计量方式到达 135；后续选举条目使最终观察值为 136。本次未重跑该集群。
- 本次运行三个独立 Rust 组件探针，复用 `target/debug` 既有库，全部完成。结果、源码和 SHA-256 见 [验证记录](production-review-20260926/manifest.json) 与 [探针输出](production-review-20260926/probe.log)。这不是 fresh workspace 构建或生产负载测试。
- 本次没有重跑全量测试、Lean、Chaos Mesh、断电或跨主机基准。下文分别注明实测结果、源码确认和需要验收的设计判断。

| 分类 | 生产判断 |
| --- | --- |
| 已复现的契约缺口 | 并发写超过日志上限；停止的 driver 仍被 pool 持有；相同日志可重复重写；deferred sync 空闲时没有 100ms 主动同步 |
| 源码确认的组合缺口 | 背压未进入客户端明确拒绝分支；回收调度可能饥饿；自动压缩依赖 metadata leader 本地副本；退役记录消耗容量；关闭 split 触发器同时冻结已有操作 |
| 必须补齐的生产能力 | data-group 存储稳态、故障副本恢复/替换、真实资源预算、备份恢复、升级与访问隔离 |
| 必选产品能力 | Raw KV、TTL、range split、range merge；四项须通过恢复与资源回收的组合验收 |
| 可以保留的产品限制 | 首版不含事务；跨 range scan 不提供统一快照；跨 range batch 拒绝；受限维护窗口——前提是 API 与部署契约明确 |
| 可推迟的优化 | 索引布局微调、WAL 编码小收益、lease reads、物理双 WAL 合并；不能以此推迟恢复与容量问题 |

**R1 — 上线阻断：背压尚未构成可执行的资源上限。**

`crates/server/src/runtime/range_api.rs:323` 的检查只观察 committed 与 first index 的差，既没有预留容量，也没有与提案追加原子执行。在途提案可同时通过检查；上一轮实际复现已经覆盖这个交错。batching 延迟窗口、重提案、内部控制条目、leader 变化也必须进入容量模型。

`crates/server/src/grpc.rs:962` 只返回普通 `RESOURCE_EXHAUSTED`。`client.rs:748` 把无专用标记的状态识别为普通 RPC 错误，`client/routed.rs:547` 随后清理路由、标记节点可疑，并以 `UnknownWrite` 终止。这与“明确未写入，在同一路由退避重试”的文档契约不符。不能通过把所有 `RESOURCE_EXHAUSTED` 都视为可重试来修补：其他来源仍可能没有无副作用保证。

生产要求：在追加/预留的同一同步边界计入 outstanding 工作，按字节、条目、节点总量与磁盘余量实施预算；预留归还要覆盖失败、取消、领导权变化和已经追加的未知结果。用户预算与恢复/控制预算分开。发送专用且严格校验的拒绝标记，实现有总 deadline 和 attempt 上限的退避；未知写继续禁止盲重试。

当前 e2e 在 `scripts/write-backpressure-e2e.py:190` 串行发送请求，并在第 219 行允许 `after <= before + 8`；Lean 的 `admitWrite` 把检查和增长合成一步。它们不能证明 Rust 并发实现的绝对上限。修复验收必须包含 cap-1 并发、暂停副本、batching on/off、取消和 leader 切换，并精确区分用户数据与保留的控制开销。

**R2 — 上线阻断：压缩 Raft 日志没有解决整个数据库的存储增长。**

`crates/engine/src/persist.rs:30` 的 `WalEngine` 仍用 `MemEngine` 保存完整可见数据；checkpoint 编码有 `MAX_CHECKPOINT_BYTES = 48 MiB`（`checkpoint.rs:23`）。不能把“Raft 日志受控”推导为“内存、engine WAL、对象存储都受控”。48 MiB 是该全量 checkpoint 的限制，不是所有数据组天然受保护的写入上限。

自动 `CheckpointWorker` 在 `crates/server/src/remote_storage.rs:96` 使用 metadata engine 和 `META_REGION_0`；当前没有同等的常态 data-group 自动 flush/adopt/reclaim 循环。迁移时的 source capture/安装提供了局部能力，不能替代每个服务中数据组的持续维护。因此固定大小的键集合反复覆盖，也不能据现有 Raft 回收验收声称 `data.segments` 已进入有界稳态。

生产要求：先接通每组 checkpoint/恢复锚点与 engine WAL 回收，再实现分块读取、受预算约束的缓存和增量持久化；或者把首版严格限制在可容纳的数据规模，并在超过恢复/迁移能力前拒绝新增数据。不能仅提高 48 MiB 常量。长期目标下还需完整 pin 归属、reader/backup/transfer 生命周期和对象 GC；在 GC 完成前必须有明确容量预算与告警。

对象存储中的 checkpoint 也不等于已覆盖所有成功写：未上传的尾部仍依赖 Raft 副本的本地持久化介质。应明确故障预算及备份 RPO，不能据此承诺任意清空所有本地盘或无条件缩到零。

**R3 — 上线阻断：回收的活性依赖最慢副本。**

`crates/raft/src/rawnode.rs:487` 的截断检查要求所有 tracked peers（包括 learners）都匹配 floor；`:760` 仍拒绝网络 `MsgSnapshot`。这是当前缺少在线 engine/protocol 成对安装时必要的安全防线。但一个持续离线或极慢的副本会阻止日志继续压缩；达到背压阈值后，即使多数派健康也会停止接收写入，除非恢复或执行安全的成员变更。

生产目标应明确“允许落后多少字节/多久、何时触发恢复或替换、恢复期间多数派能持续写多久”。需要可恢复的在线 snapshot 安装或经过验证的替换流程、对应的传输和磁盘预算，以及追赶耗时 SLO。不能简单移除 all-matched 检查。Raft 本身通过向落后于保留前缀的副本发送 snapshot 来补齐日志缺口；实现还必须把 engine 状态与协议状态一起安装。[Raft 论文第 7 节](https://raft.github.io/raft.pdf)

**R4 — 扩容上线阻断：自动压缩的观察范围受 metadata leader 所在节点限制。**

`runtime.rs:5584` 只允许 metadata leader 运行自动 floor 选择；`:5615` 遍历目录后仅通过本机 `group_handles` 获取进度，找不到就跳过。将某个组的全部副本迁到其他节点后，metadata leader 可能不再持有该组，自动压缩便无法为它提出新 floor。三进程、全部组共置的测试覆盖不了这一点。

生产要求：使用有作用域和新鲜度约束的组进度报告，或由数据组 leader 请求经 metadata 提交的维护决策；保留现有 authority 校验。验收必须让 metadata leader 不持有目标数据组，再进行持续写入、leader 切换和迁移，确认所有组都获得维护机会。

**R5 — 上线阻断：退役释放了路由，却没有完整释放运行时资源。**

`region_manager.rs:623` 调用 `driver.stop()` 后移除路由和 handle。`crates/raft/src/driver/pool.rs:74` 的 registry 只追加，finished slot 仍保存强引用，直到整个 pool 销毁。组件探针确认：丢弃外部引用并 stop 后 `Weak::upgrade()` 仍成功；drop pool 后才释放。

driver 持有 peer/storage 与 state machine/engine，因此退休组的内存和文件句柄也随之保留。若随后 unlink payload，文件名消失并不能证明所有磁盘块已释放；Linux 会等待最后一个打开引用关闭。[Linux unlink 手册](https://man7.org/linux/man-pages/man2/unlink.2.html) 本次证明了 driver 引用保留，未做完整 split 集群的 RSS/文件系统空闲块测量。

容量还存在第二层问题：`region_manager.rs:911` 按整个 `groups.len()` 限制 255，Retired/Reclaimed 记录也在 map 中；恢复扫描同样有目录数量上限。持续分裂和迁移会消耗历史容量，不能把这描述为“最多 255 个活跃组”，也不能只靠重启恢复容量。

生产要求：stop → 等待执行者与读者退出 → 安全注销 worker/transport → 释放 engine 和文件句柄 → 依据持久化 fence 回收 payload。轻量 tombstone 与活跃执行槽位分别计量。验收应保持活跃组数很小，累计创建/退役超过 255 次，并观察 RSS、FD、已 unlink 但仍打开的文件和真实磁盘空间。

**R6 — 上线阻断：物理日志回收缺少收益条件和公平调度。**

`runtime.rs:5701` 仅检查曾压缩过和文件大小超过阈值；`:5727` 在第一个候选上无条件 break。存在两种源码明确的路径：

- 重写后 live tail 自身仍大于阈值，下一轮仍选择它；没有新压缩或新垃圾也继续重写。
- snapshot/lease 组返回 `Ok(false)`，但仍结束这一轮；排序靠前且一直满足条件的不可回收组会阻挡后续组。

组件探针再次调用 `rewrite_log()`，得到字节完全相同的 74,075-byte 文件，同时发生 21 次额外 record write，确认底层不会自行消除这类重复工作。scheduler 的持续选择与跨组饥饿由源码确认，未做多组持续负载复现。

`rawnode.rs:630` 还在整个重写过程中持有 peer lock；`storage.rs:162` 构建全量 live records，再写临时文件、sync、rename 和目录 sync。峰值内存、临时磁盘和锁暂停随 live tail 增长，“每轮一个”并不构成耗时上限。回收还会跳过 protocol-snapshot 组，因此迁移目的副本不自动获得与普通组相同的物理回收覆盖。

生产要求：用真实可回收字节/比例、距离上次完成的 floor 与 cooldown 决定是否执行；round-robin 或显式队列避免失败/跳过项饥饿；限制后台 I/O、内存和并发，预留临时空间。长期应优先考虑分段 Raft 日志或经证明的后台重写切换，缩短阻塞 peer 的临界区。选择前要在大日志和慢盘上测量 p99、最大暂停、额外空间与故障恢复。

**R7 — 自动分裂上线阻断：关闭新触发应允许已有操作继续完成。**

`runtime.rs:5732` 在 `KV9_AUTO_SPLIT_BYTES=0` 时直接返回，seal/populate/publish 的既有操作也被一起停掉；这个行为已在 `CASCADE-SPLIT.md` 记录。生产中为了止住 split 风暴关闭开关，可能留下 sealed parent，持续拒绝这一范围的读写。

当前还先封闭 parent，再经 child 的 Raft 日志复制整段数据（`runtime.rs:1963`、`:2059`）；暂停时长取决于数据量和故障进展，不能因为短测试结束便称为有界在线 split。触发用的是累计 engine WAL 字节，而中位数只取排序前 1024 个键（`:5930`）；反复覆盖与偏斜数据可能引起不均衡分裂。复制按 256 条记录切批，并未在该循环中按编码字节分批。

生产要求：把“允许新 intent”与“恢复并完成已提交 intent”拆开；加入后台字节预算、最大并行 split、有效收益判断、可信容量观察与全范围采样。小规模维护窗口内可以先保留 fence-copy 方案；面向大 range 的持续服务应采用预复制/短切换或共享不可变文件等经过证明的方案，并设置明确的暂停与恢复目标。

**R8 — 控制面保活与生产配置需要成为完整契约。**

`admission.rs:241` 预留 metadata 请求槽位，但第 248 行仍共享同一个 encoded-byte 预算。少量大 raw 请求可以耗尽字节预算，metadata 即使还有槽位也无法进入。内部 Raft 不经过 public admission 是正确的，但不能由此推导磁盘、worker、apply 与管理请求都有足够进度。

`runtime.rs:5223` 起多个保护参数以及 `range_api.rs:246` 的上限解析使用 `parse().ok().unwrap_or(0)`；错误配置会静默变成关闭保护。相关维护默认关闭适合实验阶段，不应直接视为生产配置。

需要统一且严格校验的配置、启动时显示 effective values、明确软/硬水位和控制面应急余量；metadata 请求应预留字节与执行机会，后台 I/O 也要受预算约束。启用保护之前必须先验证其恢复路径，不能只把所有默认 0 改成非零。监控需给出阻塞原因、债务、预算余量和最后成功进展，而不只是一个会被后续错误覆盖的字符串。

**R9 — 跨主机、多租户和可恢复交付的前置条件不能全部留在最后。**

`grpc.rs:50` 明确使用明文 bearer token，当前 server builder 未配置 TLS；代码自身也注明 TLS 是跨主机部署门槛。应在真实网络生产部署前提供受验证的加密与身份认证方案，连同证书/凭证轮换和节点撤销。

range 验证校验 tenant/keyspace/epoch 一致性，解决路由与状态归属；它没有把请求 principal 与允许访问的 tenant/role 绑定。`range_api.rs:267` 不使用 `ctx.origin`，多个管理实现接受 `_caller`。这些检查不能替代多租户 ACL。首版可限制为可信单租户管理域；对外承诺多租户前要有数据与管理权限边界、资源配额及 noisy-neighbor 验收。

checkpoint 恢复不能替代完整备份产品。最低交付要求是明确覆盖数据、metadata、root/configuration 与必要历史的备份集，在全新环境演练恢复并验证 RPO/RTO。PITR 可以按产品范围分期。当前 V3 routing 文档要求 offline all-node upgrade，新的 retained-base 记录也有旧 reader 兼容限制；若承诺无停机维护，必须补能力协商、mixed-version 和滚动升级验收，否则明确维护窗口和不可回退边界。

**优化重新排序**

| 机制 | 本次判断 | 下一次推进依据 |
| --- | --- | --- |
| Safe ReadIndex、epoch fence、精确 apply receipt | 保留，属于正确性基线 | 故障与所有权变化下的独立历史检查；不为性能放宽 |
| CRC、rpds、ThinLTO | 保持已选基线，避免并行引入无关变量 | 新版本仍需自身回归，历史 QPS 不直接迁移到新功能组合 |
| deferred engine sync | 值得优先资格验证，但继续默认关闭 | 已有 2.13× 是 debug、单主机、单组结果；补 release/真实磁盘、断电/重排模型、compaction 交错和 RTO |
| proposal batching | 保留为候选，旧性能结论不成立 | `scripts/batching-bench.py:143` 使用 legacy create-keyspace；`DEFERRED-SYNC.md` 已承认未经过 aggregator。先证明确实命中新路径及合批计数，再测吞吐、fsync/op、p99 与取消 |
| 自动压缩/回收 | 必须补齐的存活能力 | 先解决 R2–R6，测试稳态与故障进展，再发布 production profile |
| WAL 预分配、索引/回调布局微调 | 暂停主线投入，保留历史证据 | end-to-end 小收益、次序反转或读回退不足以承担复杂度；容量和 p99 问题优先 |
| 物理双 WAL 合并 | 不列为首版上线必需项 | 当前两种逻辑 authority 有合理边界；等恢复、回收、迁移成熟，再用真实 I/O 与故障隔离收益决定 |
| TTL、range split/merge | 首版必选功能，优先于微优化 | TTL 格式与期限贯通复制、恢复、split/merge 和 GC；merge 必须实际释放旧组资源 |
| lease reads、完整事务 | 按产品需求分期 | 不阻塞上述首版范围；也不能由“尚未实现”直接推断其他已实现 API 不可靠 |

deferred sync 另有一个必须修正文档或实现的细节：`wal_segment.rs:327` 的 100ms 判断只在 append 中执行。探针 append 一条后空闲 300ms，sync 计数仍为 1；显式 `sync_now()` 才变为 2。当前没有独立的 idle sync 保证。它不直接证明已确认数据丢失，因为恢复依赖 synced Raft log；但它不能支持“所有未同步 engine 数据最多保留 100ms”的运维承诺。若需要这个保证，应添加有预算的 timer/flush 驱动并测其调度上界。

**建议交付顺序与验收**

1. 修复明确的契约缺口：并发 admission/typed refusal、driver 注销与容量归还、回收公平性/收益判断、split 的触发与恢复分离、严格配置。验收要在修复前失败，覆盖真实交错，不能只增加与实现同构的模型。
2. 建立存储和恢复稳态：每个 data group 的 checkpoint/engine WAL 回收、落后副本安装/替换、跨拓扑维护、内存/磁盘/后台 I/O 预算；为完整对象 GC 保留严格 ownership 前置条件。固定数据集长期覆盖写时，磁盘、RSS、FD 和对象债务必须符合预声明的界限。
3. 完成 TTL 与 range split/merge：先确定 TTL 时间语义、版本化存储与读过滤，再贯通 split 和恢复；为相邻 range merge 实现独立的持久化协议、原子发布及资源退役。四项通过联合验收后才满足功能范围。
4. 建立最小生产交付包：加密与授权范围、备份新环境恢复、版本/维护策略、容量规划与告警。多租户隔离在对外提供多租户之前完成；不必等事务实现才开始。
5. 资格验证优化：release 构建、持久磁盘、正确数据路径、相同复制与确认语义；测吞吐之外还测 p99/p99.9、拒绝与未知结果、恢复耗时、放大与后台干扰。steady-state 负载必须经历多轮 flush/compaction/GC，不能用十秒 tmpfs overwrite 代替。
6. 完成声明范围内的故障与扩容验收：保留项目既定的实际 Chaos Mesh 和独立历史检查，补慢盘、ENOSPC/EIO、丢失最近 unsynced 写的断电模型、对象存储中断、长时间落后副本、metadata/data leader 分离、TTL 过期与反复 split/merge/move。跨主机 3/6/9 节点证明可扩展性；长期 soak 与备份恢复证明可运维性，二者不能互相代替。

具体容量、恢复时间、允许故障时长和延迟 SLO 应在验收前按目标部署冻结。本审查没有替用户假定吞吐目标，也不把组件通过数量或同一主机的进程数量当作生产可用性证明。
