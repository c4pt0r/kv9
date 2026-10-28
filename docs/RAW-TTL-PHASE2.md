# 第二阶段：Raw TTL

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

启动日期：2026-10-04。工作树基线：`4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
本轮暂按 [首版范围](RAW-KV-PRODUCTION-SCOPE.md) 的执行顺序第 2 项解释“第二阶段”。
该文档的完整目标是 TTL 的 API、时间语义、版本化存储、复制、读取、恢复和空间回收；
下述组件进展不代表第二阶段完成。生产审查中的存储稳态，以及第一阶段剩余的背压、
维护预算和 split 恢复项继续保留。

2026-10-06：[catalog 公平串行化与有界等待](RAW-TTL-CATALOG-ADMISSION.md) 已让
后台 retention/capture 在原准入和 deadline 内最多等待 100 ms，保留 drain/term fence、
过期退出及 panic poisoning。新增 5 项测试，相关回归 126 项通过，含四线程 runtime
106 项；原立即拒绝行为的负向对照按预期失败，严格检查通过。共存测试本轮首次 data
约 1 秒、第二次 data/首次 metadata 约 6 秒完成；单次改善不关闭持续负载进度或资源
资格。生产退避、V11/ledger23 和 GC 权限不变，完整第二阶段继续开放。

2026-10-06：[历史成员持久盘点](RAW-TTL-MEMBERSHIP-CENSUS.md) 已将独立来源页、
进度和节点角色并集写入 metadata Raft，并接入逐 store 的后台轮转。保留同节点不同
磁盘、未知节点与 snapshot 祖先缺口；来源失联不阻塞其他来源，精确最近回执可换主
续认。当前 V11 不变，ledger 升为 **23**。新增 8 项测试，相关回归 448 项通过，
含四线程 runtime 104 项；严格检查通过，一个既有 MinIO 测试忽略。共存测试实际
超过 60 秒后恢复，catalog 竞争/退避的延迟和稳态公平性仍需解决。完成的只是各来源
观察；全体历史 store 映射、恢复锚点、DATA 全 store 关闭、释放/GC 与完整阶段仍开放。

2026-10-06：[历史成员独立远端查询](RAW-TTL-MEMBERSHIP-RPC.md) 已接入实际 owned
data peer、精确来源 store 身份和 metadata leader 的 endpoint generation 复核。
每页四项、规范 cursor 与累计数量防止把变化/缺页当成完成；取消或超时保留实际
读取容量。冻结的旧 V11 dispatcher 明确拒绝新方法且不 fallback。新增 8 项测试，
相关回归 187 项通过，含四线程 runtime 102 项，严格检查通过；V11/ledger22 不变。
这只是来源明确的观察，完整历史 incarnation/祖先/安装/备份/恢复引用、持久全体
盘点、DATA 全 store 关闭、生命周期释放和 GC 仍未完成。

2026-10-06：[本地 Raft 历史节点盘点](RAW-TTL-MEMBERSHIP-HISTORY.md) 已保留被移除/
joint 节点和 snapshot 前的本地观察，明确标记源端前缀未知。每组最多 4096 个已知
节点、四项分页；重写若会抹掉较早 base 独有的观察，在文件变化前拒绝。新增 8 项
测试，296 个不同测试通过，含四线程 runtime 100 项；7 项既有安装测试忽略。
节点不是 store incarnation，本地盘点也不是全体成员证书；远端核对、祖先历史转移、
DATA 全 store gate 和完整第二阶段继续开放。V11/ledger22 不变。

此前 DATA placement 增量的磁盘/内部服务 floor 为 **V11**，ledger 为 **22**（当前已升至 23）。
[DATA 保留 placement 历史](RAW-TTL-DATA-STORE-HISTORY.md) 已接入任务修订、四行分页、
来源计数和后台续做；16 项新增测试、529 个不同测试通过。四线程 runtime 曾有一次
无法确认的交接结果；单独复测及串行全套通过，原始原因仍未确定，不能据此关闭并发
稳定性门槛。目录完成不等于全体实际历史成员或全部恢复来源闭合，不授权 GC。
同日[交接竞争回归](RAW-TTL-HANDOFF-CONTENTION.md) 已修正辅助函数要求单次交接
成功的假设；持有真实 metadata 锁时，三副本后台安全拒绝并在解锁后采用原镜像。
修正后的四线程 runtime 99/99 通过，严格检查通过；历史失败的唯一原因仍未确定，
持续负载资格与完整第二阶段仍开放。V11/ledger22 和生产协议保持不变。
此前 V10 的 [DATA 持久读下限](RAW-TTL-DATA-READ-FENCE.md) 和 ledger21 的
[DATA 独立确认](RAW-TTL-DATA-FENCE-OBSERVATION.md) 保留为历史组件记录。
V9 的 Reader/Bootstrap 生命周期与读准入隔离见[历史记录](RAW-TTL-READERS-UPGRADE.md)。此前 V8 capture catalog
边界见[历史记录](RAW-TTL-CAPTURE-UPGRADE.md)。大镜像仍使用 V2 manifest 与
tag 14，tag 7/13 保持 V1 语义；见 [大 checkpoint](RAW-TTL-LARGE-CHECKPOINT.md)。
以下按组件发生顺序记录进展，不代表完整阶段已验收。

retention ledger 的 **08** 增量：[迁移 Publisher 生命周期](RAW-TTL-MIGRATION-PUBLISHER.md)
已把原 source 与接管节点的对象任务纳入独立代际、原子准入、实际排空和持久 PUT
报告。它保留未知写入及长期 image pins，不构成全局 Publisher census 或 GC 权限。

同日后续：[离线安装 Reader](RAW-TTL-INSTALL-READERS.md) 已将 CLI 安装中的旧镜像、
新镜像恢复和发布前校验接入独立读者及持久结束报告；实际 GET 排空前保持 pin 和
文件锁。runtime adoption 与 data/metadata 启动恢复的读者仍未接入，继续开放。

同日后续：[读取前发布校验](RAW-TTL-RECOVERY-PREFLIGHT.md) 将实际 winning WAL batch
与 committed Raft command 的核对提前到 SST GET 之前，支持旧/分段 WAL 且不修补文件。
精确 publication predecessor 已可用于后续 Reader 准入；运行态读者与启动授权仍开放。

同日后续：[运行态恢复 Reader](RAW-TTL-RUNTIME-READERS.md) 已接入 adoption 和普通 data
恢复：先确认精确来源和 Reader，再 GET，实际排空后持久化结束报告。普通 Active 组延后到
metadata 就绪后轮转恢复，临时依赖失败保留重试；metadata 自身启动授权与完整生命周期仍开放。

同日后续：[metadata 冷启动授权](RAW-TTL-BOOTSTRAP-READERS.md) 已在选择前确认精确
store/镜像的持久保护，保存双槽凭据，再切换恢复引用；全组停止后无需在线 ledger 即可
进行受保护的 GET 和完整恢复校验。旧 metadata checkpoint 缺少凭据时明确拒绝启动。
旧 store 的升级 enrollment、旧 grant 准入关闭与退役、全局数量有界和完整 census 仍开放，
不构成旧 Version、负向 Pending、未知 PUT 或物理 GC 的释放资格。

同日后续：[bootstrap grant 退役](RAW-TTL-BOOTSTRAP-RETIREMENT.md) 已在 Share 前保存
prospective 身份，选择后持久关闭旧读准入，metadata leader 经独立 store fence RPC
和 endpoint 重检后原子释放旧 grant；未知回执重放先于新候选。新 worker 的本地状态
合计最多两槽/212 字节，V1 凭据升级需精确提交历史。旧 Version、全局终止历史、旧孤立
grant、缺凭据 store 升级和全体 Reader census 仍开放，不开启对象 GC。

同日后续：[缺凭据 metadata 在线登记](RAW-TTL-BOOTSTRAP-ENROLLMENT.md) 已接入生产
启动组合：先核对本地实际发布历史，再经在线 quorum 确认精确 Published Version，
持久保存 prospective 授权并确认 Published 后才 GET。未知回执支持换主后重试，
登记成功后保留全组离线恢复能力。没有可达 quorum、早于 ledger 的旧镜像及旧格式
Active 身份不足的升级路径仍开放，完整 census、Version 回收和 GC 未验收。

同日后续：[Reader 终态回执](RAW-TTL-TERMINAL-READ-ACK.md) 已把已 Released 的
bootstrap/checkpoint/image Reader 确认与旧 Version/intent 解耦；精确终态与自身
交叉引用仍须保留，首次结束的 Published 来源及独立 fence 门槛未放宽。实际回执
丢失、Version 同镜像覆盖转移、换主和完整重开已验证；没有授权新旧镜像替代或 GC。

同日后续：[进程 Reader 反向索引](RAW-TTL-READER-INDEX.md) 将 ledger 升为 **09**。
Reader 登记、变更和直接结束与索引原子提交；后台在原维护槽位中通过两轮有界扫描
补齐旧记录，支持丢失回执、换主与 WAL 重开。08 Publisher 记录和未知 PUT 原样保留。
盘点不覆盖持久 BootstrapReader，也不证明逐代实际排空或关闭未来准入；负向 Pending、
旧 Version、完整生命周期与 GC 仍未放行。

同日后续：[Reader 来源准入和逐代事实](RAW-TTL-READER-ADMISSION.md) 将 ledger 升为
**10**。Share/Publish 绑定精确 Published 来源并检查已提交拒绝；该代准入、发布和结束
与 owner/资源/索引原子保存。受管理 Reader 不再允许通用释放，旧代不补造准入事实。
负向拒绝阻止新读，原在途读仍须实际排空并保留 outbox；全体连续代际闭合、部署资格、
Pending/Version 生命周期及 GC 继续开放。

同日后续：[已登记 Reader 连续代际闭合](RAW-TTL-READER-CLOSURE.md) 将 ledger 升为
**11**。在精确拒绝及全局 Reader 索引完成后，每页核对至多四代，缺口、旧代和活跃代
都保留阻塞；晚到结束与持久失效标记原子提交，旧汇总会重新计算。后台复用原维护
槽位，已验证丢失回执、换主、实际 GET 排空和 WAL 重开。该观察不补齐外部/旧协议
部署来源、BootstrapReader、Publisher 或恢复引用，Pending/Version 和 GC 仍未放行。

同日后续：[bootstrap 精确准入](RAW-TTL-BOOTSTRAP-ADMISSION.md) 将 ledger 升为
**12**。catalog 拒绝通用 Acquire/Share 创建 BootstrapReader，新 typed entry 原子绑定
精确 Published Version、Pending 和 provisioned metadata store；Publish 重检来源。
丢失回执与终态确认保留来源，旧授权仍可离线恢复；旧未知历史不会被补造为新准入。

同日后续：[持久 bootstrap 目录](RAW-TTL-BOOTSTRAP-INDEX.md) 将 ledger 升为 **13**。
按镜像 subject 索引所有新旧 grant；三轮有界盘点 owner、registration 和 admission，
查询保留未知来源与原始 phase。后台复用既有槽位，支持页面丢回执、换主和 WAL 重开。
索引覆盖不等于 store 读准入已关闭；恢复依赖、全生命周期资格、Version/负向 Pending
释放及物理 GC 继续开放。

同日后续：[metadata 副本选择记录](RAW-TTL-METADATA-SELECTION.md) 将 ledger 升为
**14**。metadata leader 独立查询实际 store 的 selector、已提交历史和持久 bootstrap
授权，锁内重检 endpoint 后记录精确 grant。worker 丢回执时重发同一请求，不重复对象
I/O；缓存丢失可从 selector 与本地 receipt 重建报告。已退休 grant 可留作历史证据，
不授予新读权限。全体恢复依赖、未来读准入关闭、旧 Version/负向 Pending 及 GC 仍开放。

同日后续：[metadata 旧镜像读准入关闭](RAW-TTL-METADATA-READ-GATE.md) 将 ledger 升为
**15**。每页核对至多四个 provisioned root store 已选择严格更晚镜像；最后一页才拒绝
旧 checkpoint 的新 Reader/Bootstrap Share、Begin 和 Publish，并接入原有 Reader 逐代
盘点与晚到排空失效逻辑。已有授权、pins 和 Version 继续保留；data group、完整 store
读 fence/恢复引用、部署资格及安全释放/GC 仍须继续完成。

同日后续：[metadata 镜像 bootstrap grant 汇总](RAW-TTL-BOOTSTRAP-CLOSURE.md) 将 ledger
升为 **16**。每页检查至多四个 grant，涵盖不同 Pending 对同一镜像的全部登记；同镜像
准入、发布、退役和索引补齐与失效 stamp 原子提交，晚到变更使旧汇总重新计算。缺少
来源的 Released 仍视为未知。已验证丢回执、换主、独立 store 退役与三副本离线恢复；
未进入终态来源、所有同镜像未来准入、完整恢复依赖与安全释放/GC 继续开放。

同日后续：[bootstrap 未进入终态来源](RAW-TTL-BOOTSTRAP-UNENTERED.md) 将 ledger 升为
**17**。首次独立 store fence 在完全没有旧授权记录时，原子保存未进入事实及终态
owner/pin/index/stamp；已有准入和旧未知历史不补发证明。目录增加第四轮有界扫描，
并修复 Begin 在 owner 缺失而索引尚存时可能重建授权的问题。真实准入拒绝、提交后
丢回执、换主和三副本离线恢复均保留来源差异；同镜像未来准入、完整恢复依赖、
旧 Version/负向 Pending 释放与 GC 继续开放。

同日后续：[metadata 同镜像读关闭](RAW-TTL-METADATA-IMAGE-GATE.md) 将 ledger 升为
**18**。原全 store gate 的精确事实与配对标记绑定完整镜像，关闭已有及未来 Pending
别名和其他 typed image 来源的新 Reader/Bootstrap 准入，保留现有读者与离线授权。
后台修复别名 Build 未完成时读者盘点停滞的问题，交替推进盘点与发布结算；真实 GET
仍须实际排空。原 Version 转移、丢回执、换主和三副本离线重开保留关闭事实；完整
恢复依赖、全体生命周期、负向 Pending/旧 Version 释放及 GC 继续开放。

同日后续：[metadata 跨来源 Reader 盘点](RAW-TTL-IMAGE-READER-CENSUS.md) 将 ledger 升为
**19**。按 region/完整镜像归集不同 Pending、Snapshot/Migration operation 的读者，
两轮有界目录与逐代盘点复用原维护槽位。目录数量与索引原子更新，避免丢索引把活跃
读者误算为空；晚到排空使汇总失效。真实独立 Snapshot GET、丢回执、换主和三副本
离线重开已验证；完整恢复依赖、Publisher/部署资格、安全释放及 GC 继续开放。

同日后续：[Reader 生命周期升级隔离](RAW-TTL-READERS-UPGRADE.md) 将 store/service
floor 升为 **V9**，ledger 保持 **19**。冻结的 V8 codec 与生成协议验证旧版本无法
重开当前 store 或调用新服务；冷启动在已有离线授权快速返回前也检查持久 floor。
实际三副本停止、升级与离线恢复保留授权、image census 和旧 Version 保护。此为
停机升级边界，不补造旧来源或释放未知 GET/PUT；完整部署和安全回收仍未验收。

同日后续：[上传者终态回执](RAW-TTL-TERMINAL-PUBLISHER-ACK.md) 使已有 Build/迁移
Publisher 结束报告的精确确认不再依赖旧 Published 来源。历史报告可在较新一代
活跃时确认且不改当前 pins；首次结束、新准入与未知 PUT 条件保持。真实上传、
丢回执、同镜像来源转移、换主与磁盘 outbox/WAL 重开已验证；V9/ledger19 不变，
全体 Publisher、可转移恢复锚点、安全释放与物理 GC 继续开放。

同日后续：[metadata 镜像上传者盘点](RAW-TTL-IMAGE-PUBLISHER-CENSUS.md) 将 ledger 升为
**20**，store/service floor 保持 **V9**。独立写 gate 拒绝同镜像所有 Pending 别名的
新 Build 上传；跨 operation 目录、连续代际、数量/stamp 与有界 census 保留活跃、
缺来源和非零 PUT 阻塞项。实际上传、RPC 丢分页回执、换主及三份 metadata 离线
恢复覆盖门禁和未知计数保留。目录完成不授权资源释放；DATA 关闭、旧部署/外部
来源资格、完整恢复依赖、可转移锚点、安全释放、物理 GC 与历史有界仍开放。

2026-10-05 背压修复：[提案锁内准入](RAW-WRITE-ADMISSION.md) 已替换有并发窗口的
已提交条目预检查。普通/批量写入、TTL 观察/清理及 split Append 计入未提交尾部，
默认每组 65,536 条，配置错误拒绝启动；控制命令与已接受 apply 继续推进。
条目阈值不代表 WAL/Raft 字节配额，物理债务与长期稳态验收继续开放。

同日后续：[节点共享 WAL/Raft 字节准入](RAW-WAL-RAFT-BUDGET.md) 已接入实际 append、
布局重写、启动盘点和 driver 恢复；取消等待或 owner 退出不会释放未 apply 预留。
默认 4 GiB，满额拒绝新数据提案，follower/control 与已接受 apply 继续计费推进。
这不是整个磁盘的硬上限；控制/回收余量、远端压力协调和长期容量验收仍开放。

同日后续：[本地磁盘余量准入](RAW-DISK-HEADROOM.md) 已增加 Linux 实际空间探测，
默认保留 256 MiB，并扣除未 apply 预留和采样重叠写入。失败/过期读数拒绝新数据，
低余量 follower 继续 apply，当选 leader 后执行本地准入。尚无远端压力反馈或
预分配保障，完整控制/恢复峰值和稳定负载仍待验收。

2026-10-05 后续：[远端副本压力反馈](RAW-REPLICA-PRESSURE.md) 已接入有界鉴权 RPC、
节点维护轮询和实际 data peer 的完整成员配置检查。旧路由/过期回复与 RPC 错误
不能解除已知压力；公开 TTL 拒绝和健康恢复有三副本 HTTP/2 组合验证。metadata
保留本地准入以便修复目录；报告不提供远端预留，控制/恢复峰值及全副本容量仍开放。

2026-10-05 后续：[Raft 流式重写与空间预留](RAW-RAFT-REWRITE-WORKSPACE.md) 已去除
回收时的整段编码副本，在实际 writer 锁内核算并预留输出，默认共享额度 512 MiB。
低空间延期而不破坏原日志，数据额度满时仍可回收；新数据与远端报告扣除在途
重写额度。它不关闭跨节点预留、完整恢复空间、常驻内存与长期稳态验收。

## 本轮确定的语义

- API 使用有 presence 的 `ttl_secs`。未指定表示不设置期限，覆盖写会清除旧 TTL；
  `Some(0)` 表示此次写入在 apply 时即过期，正数以秒为单位。内部时间使用 UNIX 毫秒。
  单位换算或期限相加溢出时，在构造副作用前拒绝。
- 提案第一次准备时解析期限，之后提交、重提案和恢复保留相同的解析结果。
  apply 不读取本地时钟。排队时间属于期限经过的时间，已确认时可能已经过期。
- 时钟提供者给出包含真实当前时间的区间 `[earliest, latest]`。正 TTL 的期限是
  `latest + ttl`，复制的时间水位推进到 `max(previous, earliest)`。这在声明的时钟
  区间假设下避免提前过期；可能延后可见过期，不能承诺精确到毫秒的触发时间。
- 小幅时钟回退不会降低已提交水位。如果新采样的 `latest < previous`，采样不能
  授权推进，必须拒绝依赖它的 TTL 操作。编码器不能证明机器满足误差范围；Linux 时钟
  提供者、健康检查和显式误差配置已有组件实现，部署资格仍需验证。未检测到的时钟快跳不在已证明
  的真实时间保证内，不能把这个假设省略为“用了绝对时间戳所以正确”。
- 一次 TTL 读取先确保所需时间观察通过相应 data group 提交，再走现有一致性屏障。
  值、到期记录和时间水位来自同一不可变 snapshot。读取不能单独拿进程 `now` 过滤。
- 后台删除以 `(creation_digest, term, index, expires_at)` 作为观察身份，在有序 apply
  中重新匹配当前身份和当前水位。续期、普通覆盖和其他 group 的相同日志位置都不能
  被旧清理任务删除。删除后空间回收仍依赖每组 WAL/checkpoint/compaction。

## 存储和升级约束

默认 CF 中的用户 value 保持任意原始字节。在 Write CF 的同一个物理 Raw key 上存放
可选的到期记录，格式 `KV9TTL01 | creation_digest:32 | term:u64 | index:u64 | deadline:u64`，
整数采用 big-endian。没有到期记录表示永久值；不通过嗅探 value 的 magic 识别新旧格式。
Txn 与 Raw key 的 mode 前缀不同，该布局不会复用 Txn 的物理 key。

组时间水位在默认 CF 的保留系统 key `s\0\0\0kv9.raw-ttl-clock.v1` 下，值为
`KV9CLK01 | watermark:u64`。提案时间编码为
`KV9TPL01 | observed_floor:u64 | has_expiry:u8 | deadline:u64`，无期限时 deadline 必须为零。
解码拒绝未知标志、截断、额外尾部、无效版本身份和不合法的期限。上述编码已由独立的公开 `RawTtl` 协议使用；旧 Raw 端点继续拒绝 TTL。

值、到期记录、水位与 applied position 必须通过一个原子 engine batch 持久化。
普通覆盖/删除也必须更新或移除旧到期记录。不能仅给 Put 增加一个可选字段后继续让
旧 Write、batch、stream、重试或 split population 路径绕过这些规则。

上线需要显式的持久格式/能力 fence 和客户端能力确认。旧服务会忽略未知 protobuf
字段，并且旧 reader 不认识 Write CF 中的 Raw TTL；因此不能把新增 optional 字段
当作已经实现向后兼容。TTL 启用前应完成全节点升级，拒绝旧路径绕过和不安全降级。
V4 持久格式隔离和逐 store 的确认 RPC 已接入，见 [升级组件](RAW-TTL-UPGRADE.md)；
全组确认到实际 Enable 的[门槛组件](RAW-TTL-ACTIVATION.md) 已接入；独立的[公开 TTL 协议](RAW-TTL-PUBLIC.md) 已接入 unary/point stream；
[公开激活管理](RAW-TTL-ACTIVATION-API.md) 支持远端 metadata leader，具备三副本 HTTP/2
组合证据；真实跨节点升级验收仍未完成。

## 实现与验收清单

- [x] 时间解析、单调水位、固定期限、版本身份和规范编码组件：
  [common/raw_ttl.rs](../crates/common/src/raw_ttl.rs)。
- [x] 同一 snapshot 上的 Get/BatchGet 与有扫描行预算的分页组件：
  [txn/raw/ttl.rs](../crates/txn/src/raw/ttl.rs)。返回的 `next` 才决定是否继续，
  整页过期且 `rows` 为空不能被当作 EOF。保留任意 value 字节、重复请求 key 和空 value。
- [x] 组件测试覆盖回拨、期限溢出、零 TTL、延期 apply、编码损坏、旧清理身份、
  过期空页、prefix key、snapshot 隔离和实际 WAL 重开。
- [x] Raft command codec 与有序 apply：严格 scope/epoch fence、当前组写入身份、
  value/expiry/clock/applied-position 原子变化、旧命令防绕过、重放与错误注入。
  这是内部组件进展；`Enable` 不是全节点升级完成或允许公开服务的凭据。
- [x] V4 store lifecycle、内部 RPC 代际隔离和逐 store 格式确认；runtime 与 offline
  installer 都要求持久格式 fence。发布故障点与旧 codec 拒绝有组件覆盖。
- [x] 全副本确认绑定实际配置的激活组件：覆盖四个成员集合、精确 store identity、
  owner/term/配置 apply index、pending ConfChange、总 deadline 和单次消费；普通提案
  防绕过、Runtime 目录解析与确认收集已经接入。仅有本地组合证据。
- [ ] 真实 Runtime 跨节点收集到 Enable、已启用组新增副本/迁移/恢复及操作重试、
  旧二进制/网络混部升级验收。组件测试不是完整升级协议的完成证据。
  [TTL 镜像安装与 learner 尾部恢复](RAW-TTL-INSTALL.md) 已支持合法 expiry/clock/history、
  安装替换水位单调性及一 voter/一 learner 的 capture→install→尾部→WAL 重开组件链路；
  [采用后的恢复校验](RAW-TTL-ADOPTED-RECOVERY.md) 已接入 committed WAL/history、逐项
  range/TTL 转移与 pending 保护，后续压缩按实际 base 和 committed catalog 决策启动；
  [持久采用记录](RAW-TTL-ADOPTION-FACTS.md) 保存原始已验证 TTL 状态，使较新 checkpoint
  不再依赖原始 SST；含 V1 升级、发布故障窗口及删除旧对象后的实际重开验证。
  完整新增副本资格、迁移协调与真实分布式验收仍开放。
- [x] 单 range [公开 TTL 组件](RAW-TTL-PUBLIC.md)：独立 RPC/stream opcode，Put/BatchPut 的
  presence、Get/BatchGet/有行数及字节预算的 Scan，能力确认、客户端和 CLI；复制时间观察后
  再 ReadIndex，同快照过滤。未知写不自动重试；取消后 admission 保持到后台完成。
- [x] [公开激活管理](RAW-TTL-ACTIVATION-API.md)：独立 RPC、客户端和 CLI，共用实际全副本
  门槛；远端 metadata leader 的两次目录 snapshot、精确格式确认、同一 deadline 与回执。
  三副本 HTTP/2 组合覆盖分离的两个 leader、缺少成员、endpoint 变更、重定向、取消配额。
- [x] [自动 TTL 路由和跨 range 分页组件](RAW-TTL-ROUTING.md)：root/namespace 绑定、
  目录探测与 scope 刷新、逐次能力确认、数据调用不重放；空页/空范围仍可继续，游标
  保存查询范围并按当前位置重查 owner。CLI 同时支持显式与路由配置。
- [x] [完整 catalog 边界与分页 TTL DeleteRange](RAW-TTL-DELETE-RANGE.md)：公开协议 v2，
  逐页预算、两 CF 原子删除、精确回执及独立删除游标；显式删除无需新时钟采样，未知页不重放。
- [ ] TTL resident 优化与真实多节点公开链路验收。范围组件的本地组合证据不关闭网络、完整 NodeRuntime、
  实际多组范围转移或完整阶段验收。
- [ ] 时钟提供者及生产配置：声明并验证误差模型，检测回退/快跳异常，重启和 leader
  切换后的行为一致；CPU 测试和本机 wall clock 不能替代该分布式验收。
  [Linux 提供者、显式配置与复制时间观察](RAW-TTL-CLOCK.md) 已接入组件路径，具备
  持续拒绝、独立区间模型和 WAL 重开测试；部署资格及跨节点组合验收仍未完成。
- [x] 清理候选分页组件与条件清理 apply：只扫描 Write CF 元数据，检查行数、
  候选数、候选编码字节分别受限；字节边界保留尚未发出的 key，空候选页保留游标。
  apply 重查完整版本，永久覆盖清除旧元数据；旧候选、重复清理及续期竞争有回归。
- [x] [后台 TTL 调度组件](RAW-TTL-MAINTENANCE.md)：节点共享两个任务槽位，按组轮转、
  有界页、周期推进水位、条件清理、取消排空、失败退避及状态指标；未知位置消费前
  不追加新维护命令。实际续期竞争、WAL 重开和三副本组件测试通过。
- [ ] 将 TTL 调度与 checkpoint/compaction/对象 I/O 等其他维护纳入整机共享预算，
  验证完整控制面、负载下公平性和物理债务。
  [七类维护共享准入](RAW-TTL-SHARED-MAINTENANCE.md) 已接通节点任务/重 I/O 上限、派发
  速率、跨类型等待和真实 job 排空；仍缺其余维护路径、字节/内存/CPU 与债务预算。
  [对象 payload 配额与读回上限](RAW-TTL-OBJECT-IO.md) 已加入同一 runtime 的对象
  客户端，包含幂等比较读与配额等待取消；本地 WAL、临时内存、CPU 和稳态债务仍开放。
  [checkpoint 临时副本修正](RAW-TTL-CHECKPOINT-MEMORY.md) 已前置规划大小检查、流式
  编解码 pending journal，并在 reload 前释放 plan；整机内存准入与 RSS 验收仍开放。
  [共享 image 字节准入](RAW-TTL-IMAGE-BUDGET.md) 已覆盖 runtime 的计划/pending 对象
  集合并随实际上传持有；解析、网络副本、恢复 index 等其余临时内存成本仍未覆盖。
  2026-10-05 [逐 SST 落盘](RAW-TTL-SPOOLED-CHECKPOINT.md) 已替换生产 metadata/data
  worker 的完整 image buffers，并使 journal 重开/上传按 SST 准入；大于测试额度
  的实际三副本导入镜像可完成回收和重开。
  随后 [V2 大镜像](RAW-TTL-LARGE-CHECKPOINT.md) 用 tag 14/V7 贯通超过 48 MiB 的
  导入及 Bound 镜像发布/恢复，并加入活动匿名 spool 字节准入；metadata 实际启动
  journal 的流式加载遗漏也已修正。durable pending/WAL 债务、capture/迁移、常驻
  索引和完整资源/部署验收仍开放。
  同日后续：[生产 capture spool](RAW-TTL-CAPTURE-SPOOL.md)已把迁移规划/上传入口切换到
  独立匿名 spool 和节点共享额度，检查实际 engine/driver 归属并排空上传任务。约
  55.4 MB TTL 镜像经真实 RuntimeBackend 规划/绑定/捕获后在 learner store 安装并
  重开；当时持续写入下固定计划与崩溃续传、完整继任锚点和 owner 释放仍开放。
  同日后续：[持久固定 capture](RAW-TTL-DURABLE-CAPTURE.md) 已把同一 cut 的 record、
  配置和 SST 原子保存，接入两条实际迁移入口、共享 pending 预算和退休清理。原 source
  store 在继续写入、上传报错和存储句柄重开后复用原镜像；取消后的新 operation 按
  committed terminal 记录复用 slot，旧 pins 保留。
  同日后续：[复制 capture catalog](RAW-TTL-CAPTURE-CATALOG.md) 已加入独立 source
  核对与 metadata Raft 登记。新 data leader 在原 SST 完整存在时仅 GET 校验并返回
  原协议 record，覆盖 source 失效、metadata 切换和提交回执丢失。上传未完成时的
  payload 接手、完整继任锚点、owner 释放、GC 与混合版本生产验收仍开放。
  随后 [V8 持久/服务隔离](RAW-TTL-CAPTURE-UPGRADE.md) 排除可绕过这些约束的旧
  V7 writer/RPC，具备冻结 codec、双向 HTTP/2 和升级后原 catalog/payload 恢复证据。
  仍采用停机升级要求，完整旧 binary/真实部署演练尚未验收。
  随后 [未上传 payload 的副本接管](RAW-TTL-CAPTURE-TRANSFER.md) 已让成功 Plan 等待
  原镜像 voter 配置的完整文件多数副本，joint 配置分别满足双多数。新 leader 可从
  其他 voter 有界拉取原文件，在原 PUT 未结束时继续同一镜像；缺失/损坏拒绝且
  不生成新 cut。取消后实际任务、文件锁和计费保留到排空，启动盘点及退役覆盖副本。
  这不提供 GC/旧 PUT 释放权限；全部 Publisher/Reader 生命周期、完整继任锚点、
  配置变化后的副本维护和真实故障/部署资格仍开放。
  随后 [持久 pending 磁盘准入](RAW-TTL-PENDING-DISK.md) 已将 named journal、发布
  临时文件和仍打开的旧 inode 纳入节点共享配额，启动盘点并在授权退役后释放。
  满额时保留精确 durable plan 继续结算；metadata/三副本 data 的已有债务高于
  新额度场景通过。WAL/Raft 与完整文件系统债务及稳态验收仍开放。
  [对象 buffer 共享额度](RAW-TTL-OBJECT-BUFFERS.md) 随 PUT/GET 字节 owner 保持至最后
  一个引用释放，包含超时后队列持有；codec、协议层与恢复索引等成本仍继续补齐。
  [SST 借用校验与直接恢复](RAW-TTL-SST-VIEWS.md) 已去掉批量解析的 key/value 副本和
  全量 restore WriteBatch，所有 SST 验证通过后才返回索引；写入端、常驻索引与 RSS 仍开放。
  [manifest 有界流式编码](RAW-TTL-MANIFEST-ENCODING.md) 已在扩张输出前执行 1 MiB
  frame 上限，保持原字节及 change ID；输入/decode、历史数量和整机内存仍开放。
  [冻结视图直接生成 SST](RAW-TTL-BORROWED-WRITER.md) 已在 image 准入后直接编码最终
  buffer，去掉生产 planner 的 owned-entry 暂存集合；常驻索引、snapshot 保留及整机 RSS 仍开放。
  [checkpoint 结果收敛预算](RAW-TTL-SETTLEMENT-BUDGET.md) 已将精确历史查询前置，
  后备 effect 每次最多四行且可续做；同一 slot 排除并发驱动。历史总量、解码字节和
  整机 CPU/公平性仍未验收，扫描结束不能作为负向结果。
  [checkpoint 本地对象排空](RAW-TTL-OBJECT-EXECUTION.md) 已将维护 permit 和同组执行
  状态保留至实际对象队列/job 退出，覆盖调用者超时与取消；本地排空不证明远端 PUT
  已取消，也不授权释放全局 owner。
- [x] [每组 checkpoint 恢复组件](RAW-TTL-CHECKPOINT.md)：核对实际 SST 的 range/TTL/系统数据、
  精确已提交 publication 与 WAL 尾部；三副本在实际 segment 回收后通过 RegionManager
  重开，TTL 和尾部写入保留。后续发布、owner 和本地历史保护分别由下列组件推进。
- [x] [checkpoint 切换与 Raft 压缩的共享保护](RAW-TTL-CHECKPOINT-HISTORY.md)：两个 peer
  截断入口绑定实际 engine，原子保护所选 cut 和 publication；历史配置保留、真实并发/
  失败窗口及三副本双 WAL 回收后重开通过组件验证。
  2026-10-05 后续：[选择一致性与在线观察](RAW-TTL-CHECKPOINT-SELECTION.md)已统一
  legacy/segmented 的 scope、同 cut 身份、term/epoch 与 applied 检查，并在 legacy
  改写前验证保留 WAL term。在线采用和 import compaction 改用持锁的实际 owner
  观察，不确定失败要求恢复；它仍不是可转移锚点或旧 Version 释放凭据。
- [x] [跨节点 checkpoint owner 登记组件](RAW-TTL-CHECKPOINT-OWNERS.md)：有界内部 RPC、
  独立 metadata leader 路由、规范回执及结果未知时终止；三副本八个丢回执位置续做、
  实际发布及 data/metadata 磁盘重开有组合验证。
- [x] [每组自动 checkpoint 与候选保留](RAW-TTL-DATA-CHECKPOINT.md)：有界任务、leader 发布、
  各副本采用、持久 journal 恢复、候选历史保护及取消排空；三副本自动 segment 回收、
  两个失败位置的全组三副本重开，以及关闭新触发后续做原计划有组件证据。
- [ ] 完整可转移恢复锚点、Reader/旧 Version/负向 Pending 的安全释放及对象 GC；
  固定活跃数据下测量磁盘/RSS/FD/对象债务，不能把逻辑读过滤当作物理回收。
  三副本旧 pending 已验证从精确历史得到拒绝并清除本地 journal，同时保留全局 pin；
  负向 identity 结论仍缺全部上传执行者排空证明，不授权释放全局 Pending。
  [上传执行者 Build owner](RAW-TTL-EXECUTOR-OWNERS.md) 已接入真实 data/metadata 路径，
  精确登记后才 I/O，本地排空后在 Pending/Version 完整覆盖下交接；关闭未知阻止同一
  worker 新上传。完整登记/关闭门槛、崩溃遗留 owner、负向 Pending 与 GC 仍开放。
  [owner 分行存储](RAW-TTL-OWNER-ROWS.md) 已移除生产 ledger 每对象累计 4096 owner
  的单值上限；Released 代际记录保留，旧布局有界原子升级。4097 个释放记录的实际
  WAL 重开和远端升级丢回执有组件覆盖；历史物理回收、legacy base 压缩和稳态仍开放。
  [精确 checkpoint 意图](RAW-TTL-CHECKPOINT-INTENT.md) 已在上传前持久化完整规范
  descriptor 与前驱 generation，支持已知 Pending ID 的远端读回；本地 journal
  清除后、metadata WAL 重开后仍可核对。
  [持久执行者准入关闭](RAW-TTL-CHECKPOINT-ADMISSION.md) 已阻止关闭后新进入/Publish，
  并提供有界索引分页、未进入代际的终态操作；关闭后的失败 journal 可在实际 WAL
  重开后依精确提交历史收尾，零对象 I/O。完整 legacy backfill、旧 binary 门槛、
  崩溃遗留执行者、全局关闭证书和负向 Pending 释放仍开放。
  [未 Publish 执行者回收](RAW-TTL-UNPUBLISHED-RETIREMENT.md) 已能按 Closed gate +
  精确 Held generation 释放该代 Build pin，保留独立证据与全局保护；实际 worker
  每轮最多四条，丢回执、data/metadata WAL 重开后可续做。已 Published 遗留者、
  完整 backfill 和负向释放仍未完成。
  [全局发现与持久续做](RAW-TTL-CHECKPOINT-SWEEP.md) 已从 metadata gate 扫描没有本地
  journal 的尝试；每轮一尝试、四个 executor，持久 CAS 游标支持丢回执、leader
  切换和 WAL 重开。新增第八类共享维护任务，合法 key 下的坏记录不阻塞后续尝试。
  Done 只表示 indexed Held sweep 完成，已 Published 遗留者及无 gate 的旧尝试
  仍需完整关闭协议，不能由此释放全局对象保护。
  [逐代执行者排空记录](RAW-TTL-EXECUTOR-DRAIN.md) 已绑定实际对象 scope，区分未发出
  PUT、取得单次响应与未知写入，并在 Release 后追加不可变逐代记录；报告未确认
  阻止本 worker 新上传。三代记录跨丢回执和 WAL 重开保留，旧 unknown 不被新成功
  覆盖。无报告崩溃遗留者、远端未知 PUT 隔离、完整 legacy 集合与负向释放仍开放。
  2026-10-05 后续：同一[排空组件](RAW-TTL-EXECUTOR-DRAIN.md)已将生产执行者的
  Released 与逐代报告改为一次原子提交，并在正向 checkpoint settlement 时关闭
  上传准入；覆盖全部十一个交接丢回执前缀及实际 WAL 重开。它消除新路径的
  Release/报告提交窗口，提交前崩溃、历史缺失和未知远端 PUT 继续保留。
  同日后续：[持久报告补交](RAW-TTL-DRAIN-OUTBOX.md)为新执行者在 I/O 前预留有界
  本地槽，排空后 fsync 原始报告，再提交 metadata；原 process/generation 可在
  新进程中恢复。metadata follower 无 uploader 的后台补交、丢回执与 WAL 重开已
  有组合覆盖。持久报告产生前的崩溃和未知远端 PUT 仍不能认证，全局释放仍开放。
  同日后续：[Build 历史索引](RAW-TTL-BUILD-INDEX.md)已补全 metadata catalog 中
  legacy Build 的有界回填，新增写入原子维护反向索引；全局双遍游标可跨丢回执、
  换主和 WAL 重开续做。旧 Held 保留原协议含义，不自动转为可回收执行者；该
  catalog 观察不是完整 publisher/generation 关闭证书，尚不授权负向释放。
  同日后续：[逐代原始准入](RAW-TTL-EXECUTOR-ENROLLMENT.md)已修复旧 Held 经 Share
  重试升级后被误认为可回收的路径。新来源事实与新代 Held/未进入取消原子提交，
  缺事实的旧 V1/V2 不补证；有界稠密代际查询保留缺口和旧 unknown。ledger header
  为 05，service/store floor 仍为 V7；全局关闭证书与部署升级资格仍开放。
  同日后续：[尝试原子创建](RAW-TTL-ATOMIC-BEGIN.md)将新 Pending、资源保护、intent /
  gate 与来源记录合为一次提交，metadata/data worker 均已切换。旧尝试续做但不
  补证，已有旧 Build 继续阻止完整关闭；ledger header 为 06，未放行负向释放。
  同日后续：[后台换主覆盖](RAW-TTL-CHECKPOINT-FAILOVER.md)已定位并修正共存测试只
  启动初始 leader sweep 的遗漏；强制换主时新节点在原 catalog 锁持续忙的条件下
  完成真实正向交接。生产每节点启动路径未变，真实部署和完整 GC 资格继续开放。
  同日后续：[已登记集合闭合核对](RAW-TTL-REGISTERED-CLOSURE.md)将完整 Build 索引
  与逐代来源/终态组合为持久有界进度，事实变化与 stamp 原子更新并使扫描重启；
  所有缺口和非零/未知 PUT 保留。ledger header 为 07，后台接入、丢回执/换主/
  WAL 重开已验证；该观察仍不授予负向释放或对象 GC 权限。
  同日后续：[持久负向结算](RAW-TTL-CHECKPOINT-REFUSAL.md)由 metadata leader 独立
  复核精确 CAS winner，目录重查后与 Closed gate 原子提交。回执丢失、换主、
  来源退役及 WAL 重开后可确认原事实并继续 closure；伪造声明、Unknown 或目录
  变化拒绝。负向 Pending 仍 Published，尚不授予物理 GC 权限。
  同日后续：[data checkpoint Reader 生命周期](RAW-TTL-CHECKPOINT-READERS.md)已接入
  发布前校验与各副本采用时的远端读取。准入确认后才 GET，独立只读 scope 排空后
  原子结束 Reader，持久 outbox 可恢复原代报告；超时/取消和丢报告不能提前释放。
  同镜像 Pending→Version 交接期间可安全收尾；旧 Version、其余恢复/安装读者及
  全局物理 GC 的资格仍开放。
  同日后续：[逐 store 选择记录](RAW-TTL-REPLICA-SELECTION.md)已让 metadata leader
  独立查询实际 data 副本，再持久保存每个 node/store/group 的最新选择观察；旧请求
  不能回退，丢回执不重复 GET，来源不可用及换主/WAL 重开后可确认已有记录。它仍
  不是完整副本依赖或可转移锚点，旧 Version、负向 Pending 和对象 GC 继续受保护。
  同日后续：[运行态恢复依赖](RAW-TTL-RECOVERY-DEPENDENCIES.md)已让普通/安装启动保留
  同一次成功 open 的镜像、真实发布、range/import/TTL 和原始安装来源摘要；区分升级
  时读取旧镜像与 engine 的实际恢复镜像。按需查询识别选择前进与不可用 owner，固定
  大小观察不会冒充完整继任锚点；可转移恢复与 owner 释放资格仍开放。
  同日后续：[无 journal 的后台收尾](RAW-TTL-CHECKPOINT-SWEEP.md)已让扫描器按
  本节点实际 metadata/data 副本的精确提交历史关闭 Open 尝试，并完成正向
  Pending→Version 交接；Closed/Done 不会遮蔽中断的交接。负向 Pending 仍保留，
  完整 publisher 集合与旧 Version 释放仍开放。同日后续已接通鉴权远端精确历史
  查询：本地缺历史时按有界节点页轮询，一整轮尝试后再换节点；只接受精确 store、
  group、窗口和 challenge 的回复，并在交接前重新核对 endpoint generation。
  无 group handle 的 metadata leader 可借远端 follower 历史完成收尾，Unknown、
  查询失败和身份变化保留保护。完整 placement 索引、全部署收敛及资源验收仍开放。
- [ ] 迁移/安装与现有 split 的格式互通：复制 value 与到期记录，不重算期限和来源
  写入版本；继承时间水位，merge 必须取两侧最大水位。现有 split 只复制默认 CF，
  此项未完成前，TTL 组的旧 seal 在有序 apply 明确拒绝，避免丢失到期元数据。
  同组镜像安装已按上述组件支持 TTL；这不替代跨 group split/merge 的复制与发布协议。
  [普通 Raw split 复制屏障](RAW-SPLIT-POPULATION-FENCE.md) 已修复新复制路径在子范围
  发布后覆盖/复活数据的问题：专用有界指令在有序 apply 检查未绑定状态，三副本
  发布与 WAL 重开回归通过。旧日志语义保留；完整 TTL 转移及升级门槛仍未完成。
  [TTL 持久复制状态](RAW-TTL-SPLIT-STATE.md) 已增加 seal/import 游标、原始 expiry
  与水位继承、计数/摘要完成检查及整批 WAL 恢复验证；三副本中途重启换主续传通过。
  普通提案入口仍拒绝新指令，测试使用显式 harness；生产全副本格式门槛、源提交
  证明、runtime 双 CF 复制与目录发布尚未接通。未完成 import 仍依赖 WAL。
  随后 [V5 格式隔离](RAW-TTL-SPLIT-FORMAT.md) 已拒绝 V4 owner/服务路径及旧格式回执，
  既有全副本激活门槛现在要求 V5；Seal/Begin 受保护入口与源认证仍未接通。
  随后 [TTL split runtime](RAW-TTL-SPLIT-RUNTIME.md) 已用单 snapshot metadata 计划、
  实际已封存源凭据和逐命令全副本确认连接复制/发布；三副本子组 WAL 重开换主续传、
  空侧水位及发布后公开 TTL 读写回归通过。未完成 import checkpoint、超大值、
  WAL/内存/调度预算、暂停新触发后的续做与完整 NodeRuntime/网络验收仍开放。
  随后 [split 后台续做](RAW-TTL-SPLIT-MAINTENANCE.md) 已将阈值 0 限定为停止新触发，
  已提交 intent 自动继续；协调任务移出节点控制循环，使用第九类共享维护准入并
  排空实际任务。三副本零阈值复制/发布、准入等待及失去资格撤回需求通过。
  全目录发现仍非分页；staging checkpoint、导入 WAL 回收和完整部署验收继续开放。
  2026-10-05 [staging checkpoint](RAW-TTL-STAGING-CHECKPOINT.md) 已明确区分 Bound 与
  Import 镜像，验证 prefix hash/cursor/clock，接通实际 checkpoint worker、所有者
  结算和恢复。专用 tag 13 保持旧 tag 7 语义，该组件当时将磁盘/服务 floor 升到 V6。
  三副本 Finish 前选中镜像、engine WAL 18.5 MB→2.7 MB、重开换主后完成发布通过。
  导入 Raft 日志截断、48 MiB 镜像限制、完整形式化/容量/部署资格仍未完成。
  同日后续 [导入 Raft 回收](RAW-TTL-IMPORT-COMPACTION.md) 已接通未绑定子组的自动
  floor 规划，保留 checkpoint history 与全成员 matched 门槛；三副本物理回收、
  缺少 catalog 授权拒绝、重开换主后 Finish 通过。大镜像、稳态及完整验收仍开放。
  adopt 后重启不再绕过普通恢复；V2 marker 与合法后续 checkpoint 已消除仅旧镜像 SST
  的本地恢复依赖，完整继任锚点与所有 owner 的释放协议待完成。
  后续第三阶段仍负责完整 split 容量/故障验收。
- [ ] 完整状态机 TLA+/TLC、组合证明、独立 TTL 时间历史检查、真实多副本与 Chaos Mesh，
  包含 leader 切换、时钟异常、崩溃、过期清理/覆盖竞争和恢复；原有生产门槛不关闭。

## 当前证据和边界

新增内部 `RawTtl` 命令包括 `Enable`、`Observe`、`Write` 和 `Cleanup`。
codec 对入口和解码都限制 16 MiB、4096 个 mutation/candidate、64 KiB key，
拒绝截断、未知 tag、无效身份、跨范围 batch 和额外尾部。它们不能转换成普通 KV batch，
也不参与现有 Raw apply 合批；这些内部上限没有扩大公开客户端已有的更小限制。

启用状态来自持久 clock record；启动拒绝损坏记录。TTL 组的旧 Put/Write/CatalogTxn/
Fenced 写返回明确拒绝，回执保留实际 term/index，不能被计为成功或自动重试。
旧命令在启用前也不能直接写 clock 或 Raw expiry 元数据。公开旧 reader 从实际读取
snapshot 检查激活标记并拒绝，以免在读取协议尚未接通时直接返回已过期值。
独立 `RawTtl` 公开服务已接入已启用的精确 data range；旧 Raw API 仍明确拒绝 TTL。
持久 V4 fence 和全副本激活门槛继续必需，普通提案入口拒绝直接 Enable。
激活已经具备公开 RPC/CLI 和嵌入式接口，部署及完整生命周期没有验收；最新证据与边界见
[公开协议记录](RAW-TTL-PUBLIC.md) 和 [激活管理记录](RAW-TTL-ACTIVATION-API.md)。

本次验证包括 15 项 TTL codec/apply/driver 专项测试，以及 2 项新增候选扫描测试。
覆盖真实 raft-rs 三个投票副本的延迟 apply、leader 切换与续期/清理交错，实际 WAL 两次
重开，写前/写后错误导致 driver poison 且不发布成功回执，以及候选 batch 中途读失败
不留下已删除的前缀。三副本使用进程内传输，不能代替真实网络故障验收。
common 52、txn 23、region 10 项通过；Raft 本地回归和精确执行的 server 回执/读取隔离
测试也通过。含 write-stage-tracing 的相关 crate 全 target 严格 Clippy 通过。
网络相关测试与 MinIO 集成测试未验收，原始日志记录了明确的过滤/忽略项。

[Lean 组件证明](../proofs/lean/raw-ttl/Ttl.lean) 的 10 条定理和 axiom audit 通过，
4 个语义错误变体、2 个 proof-policy 变体被拒绝。证明包括单调水位、回拨不复活、
在时钟区间假设下不提前过期，以及清理必须匹配原写版本。Rust 的版本身份唯一性、
真实时间误差范围、Raft/snapshot 组合、GC 和整个 TTL 协议没有由这些单 key 引理证明。
TLA/TLC 与阶段验收仍未完成。

```sh
python3 proofs/lean/raw-ttl/check.py \
  --lean /path/to/lean-4.33.1 \
  --output /fresh/validation-directory
```

原始日志、源码摘要与明确的未完成标记保留在
[首次组件验证记录](raw-ttl-validation-20261004/manifest.json) 和
[本次 apply/清理组件验证记录](raw-ttl-apply-validation-20261004/manifest.json)。
时钟提供者与复制观察的组件进展见 [时钟记录](RAW-TTL-CLOCK.md)，单 range 公开链路见
[公开协议记录](RAW-TTL-PUBLIC.md)，公开激活与远端目录见[管理记录](RAW-TTL-ACTIVATION-API.md)。
自动路由和范围分页组件见[路由记录](RAW-TTL-ROUTING.md)，完整 catalog 边界与分页
删除见[范围操作记录](RAW-TTL-DELETE-RANGE.md)，后台分页调度见[维护记录](RAW-TTL-MAINTENANCE.md)，
实际 checkpoint/尾部重开见[数据组恢复记录](RAW-TTL-CHECKPOINT.md)，Raft 压缩与选择
竞态及历史配置修正见[日志保护记录](RAW-TTL-CHECKPOINT-HISTORY.md)。
跨节点 owner 登记进展见[对象登记记录](RAW-TTL-CHECKPOINT-OWNERS.md)，候选保护和
自动发布/采用见[自动 checkpoint 记录](RAW-TTL-DATA-CHECKPOINT.md)。
共享准入进展见[节点维护配额记录](RAW-TTL-SHARED-MAINTENANCE.md)。
下一项继续补完整 owner 生命周期、其余整机维护预算和历史有界，
并推进时钟资格和跨节点验证；split 复制和后台空间回收仍按上述完整阶段清单执行。

同日后续：[DATA store 持久读下限](RAW-TTL-DATA-READ-FENCE.md) 在实际选择 checkpoint 后
约束本 store 的后续 checkpoint、安装和恢复读取，旧镜像及回退的纯 WAL 恢复在 GET/
WAL 修改前拒绝。store/RPC 升到 V10，ledger 保持 20。它尚无独立远端 fence 确认、
完整 DATA 历史成员与恢复来源盘点，也不授权 Pending/Version 释放或物理 GC。
