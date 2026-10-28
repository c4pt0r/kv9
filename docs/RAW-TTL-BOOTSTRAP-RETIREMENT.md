# Metadata bootstrap grant 的持久关闭与退役

2026-10-05；接续 [冷启动授权](RAW-TTL-BOOTSTRAP-READERS.md)。本轮接通正常在线 store
的旧 bootstrap grant 退役；第二阶段和生产资格仍未完成。

## 本地状态与崩溃顺序

上一版在确认 Published grant 后才保存凭据。若提交成功但回执未知、随后进程退出，
可能留下本地找不到的保护。本轮改为：先核对精确 Published Version，将 prospective
grant 的身份持久化，再提交 Share/Publish。只有独立确认完整 Published 回执后，才
把该槽位标为可读取，并允许选择 checkpoint。未确认槽位不允许 bootstrap GET。

文件仍为 `catalog.bootstrap-reads`，新格式 **`KV9MBR02`**：root/node/store 身份，
活动与待退役数量，以及槽位的 predecessor、完整 manifest digest、Pending ID 和读
准入标志，最后是 checksum。两种状态合计最多 **2 个槽位、212 字节**。活动槽位包含
未确认的候选；待退役槽位必须已经关闭读取。临时文件不构成任何授权或关闭回执。

切换之前保留实际旧选择和候选。确认本地 selector 已选中候选后，先持久删除旧槽位
的读权限，将其移入待退役状态，然后请求 metadata 退役。替换从未选中的候选也采用
相同顺序。缺少 Share、只到 Held、已 Published 的三种状态均能终止在 generation 1。
如果文件同步或 metadata 回执失败，关闭记录保持可恢复，后续 pass 先重放它；不能
把新候选不断堆在未知结果之后。确认释放后才删除本地待退役槽位。

每个 metadata worker pass 都先尝试持久退役重放，独立于是否存在更新的可选镜像；
旧镜像再次发布、generation 不变或已失去采用进度都不能让关闭记录永久滞留。
旧 publisher 的实际结束报告与 bootstrap 退役报告各得到一次尝试，两者均确认后才
允许新增候选。

所有更新先 fsync 文件、rename，再 fsync 目录。重试在发送退役前重新同步完整状态；
独立 inspector 也同步命名文件与目录，不能把 rename 后未知的目录同步结果当作证据。
启动 Reader 只接受可读的活动槽位，并验证 Pending ID 与该 manifest/window 一致。
已经关闭的槽位不能再读，也不会凭超时重新变为活动。已退役 owner 不能重入 generation 1，
kind 10 同时禁止 generation 2，因而清除本地关闭记录不会重新打开旧授权。

`KV9MBR01` 仍可用于原有冷启动。在线 worker 升级 V1 槽位时，必须从自身已提交
manifest history 取得精确 descriptor 并重新派生 Pending ID；缺失或不匹配则原文件
不变。**没有任何 bootstrap 凭据的旧 metadata checkpoint 仍拒绝启动**，本轮没有
完成该类旧 store 的升级登记。

## 独立核验与复制

新增 ledger tag **23 `RetireBootstrapRead`**，绑定 Pending generation 1、node 和 store。
普通 ledger/admin planner 拒绝直接按请求内容退役。metadata leader 先 ReadIndex，
读取规范 intent，再在 catalog 锁外独立查询精确来源 store。

新 RPC **`InspectBootstrapReadFence`** 使用独立方法和私有返回类型，复用有界的
checkpoint history 身份 envelope：root、cluster、node、store、metadata region、
predecessor、随机 challenge。仅支持 metadata，单次最多 512 字节和 2 秒；与历史/
selection 共用实际 reader 槽位，取消不提前归还仍在运行的任务。客户端无旧方法降级。

来源必须处于 serving 状态、具有完成恢复的 metadata engine 及原 StoreGuard 所绑定
的本地上下文。metadata 启动的 GET-only scope 在发布这些运行态对象之前已实际排空；
StoreGuard 排除了另一个启动进程。来源核对持久待退役记录、实际当前 selector，以及
自身已提交 history 中的完整规范 descriptor/Pending；所选镜像和待退役镜像相同、
未关闭、旧协议不支持、损坏或 checksum 合法但 Pending 不匹配，均不能返回肯定证明。

metadata leader 随后在 catalog 锁内排空当前 term，重新核对完整 endpoint 与 intent，
并以同 term 提交。规划器要求旧镜像的完整 **Published Version** 仍存在，将精确
bootstrap owner 和全部 resource cross-links 原子推进到 Released。通用 transfer/
release、现有 Reader/publisher finish 仍不能代替该流程。

丢失真实释放回执时，原请求可在新 leader 上确认已有 Released 事实，不必重新取得
原节点的在线证明；这条重试不能创建新的释放。Version 持续保护完整旧镜像，不会因为
bootstrap owner 已 Released 就释放旧 Version、Pending、其他读者或未知 PUT。
同日后续的[终态确认](RAW-TTL-TERMINAL-READ-ACK.md)已将精确 Released 请求的重试
与规范 intent/Published Version 解耦。首次退役仍依赖这些来源，已释放的 owner、
注册和资源交叉引用仍须保留；这并不授权 Version 或终态历史回收。

worker 也修正了非前进选择的重试：比实际 selector 更旧的再次发布不创建临时授权；
未成功切换的候选继续保留授权，不立即终止同一窗口。丢失采用进度后仍能继续处理
下一代，避免重复撞上已终止的 owner。

## 验证与边界

最终 10 项检查全部通过：362 项不重复测试通过、2 项真实 MinIO 测试忽略，
包含 8 项新增测试；workspace 严格 Clippy、格式和差异检查通过。最后补上每次
worker pass 的退役重放后，重新执行了全部受影响的 server 回归及静态检查。

- metadata 组件验证请求 codec、普通 planner 拒绝，以及未入场/Held/Published 三种
  终止情况、完整 Version 保护、终止后的延迟 Share/Publish 和 generation 重用拒绝。
- 本地状态验证不确定 Share/Publish、待退役回执丢失、恢复重放、拒绝继续堆积候选、
  关闭后零 GET、V1 精确历史升级、持续切换的两个槽位上限和 checksum 合法的身份损坏。
- RPC 验证 history/selection 不能替代新能力，鉴权、精确 store、metadata-only scope、
  Unknown、challenge 关联；沿用实际任务排空回归，compile-fail 检查返回类型不能伪造。
- 三个真实 metadata Raft 副本与鉴权 HTTP/2 duplex：仍选中的 grant 拒绝退役，
  Unknown/Unsupported、错误 store、观察后 endpoint generation 变化、checksum 合法
  的 Pending 损坏都保留保护；真实释放回执丢失后换主、源查询拒绝、重放确认及完整 WAL/
  checkpoint 重开通过。原 Version 仍 Published，旧 bootstrap owner 保持 Released。
- 真实 worker tick 在采用进度丢失时跳过旧镜像再次发布，再正常采用新镜像。旧镜像
  再次发布由显式 harness 构造命令，先校验原 SST，再通过实际 Raft 复制；测试保留
  原节点唯一 ManifestSeam，仅清除调度游标，不冒充三次完整进程重启。
- 原有全组 metadata 停止后重复恢复、零新增 PUT、真实 GET 超时不提前释放 store 锁，
  以及其他 TTL/runtime/region manager 回归继续执行。

源码摘要、命令、数量、开发失败与最终日志见
[独立验证包](raw-ttl-bootstrap-retirement-validation-20261005/manifest.json)。此前证据、
两个用户 chaos 脚本与 FrozenV7 fixtures 未改。

Store/service floor 仍为 V8，ledger header 仍为 `KV9LED08`。旧 bootstrap 文件读取器
不能读 V2，旧服务不支持新 RPC 时不会释放；这些边界不构成混合二进制/滚动升级资格。

本轮限制的是**由新 worker 持久跟踪的 bootstrap 活动保护**，不是全部历史 owner 的
数量。Released owner、resource history 和原 Version 仍占用目录；V1 在凭据保存前
崩溃留下的旧孤立 grant、未接入路径、缺凭据升级、store 退役/替换和全体 Reader census
仍开放。全局目录和物理对象稳态没有验收，没有开启 GC，也没有消除未知 PUT。
进程内 Raft/HTTP2 与本机文件恢复不能替代真实 TCP/MinIO、进程断电、跨主机持久性、
完整形式化或生产部署验收。

同日后续：[缺凭据启动的在线登记](RAW-TTL-BOOTSTRAP-ENROLLMENT.md) 已补齐具有精确
Published Version 和另一个 serving quorum 的旧 metadata store 启动路径；先持久
登记再 GET，已有关闭记录保持关闭。其余全组缺凭据、旧格式、孤立 grant 与全局回收
边界继续保留。
