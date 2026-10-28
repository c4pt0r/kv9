# Checkpoint 执行者准入关闭与恢复

2026-10-05 后续：ledger **07** 的[已登记集合闭合核对](RAW-TTL-REGISTERED-CLOSURE.md)保留 06 birth 和全部旧来源语义，新增有界聚合与原子变更水位。当前 ledger 已由[迁移 Publisher](RAW-TTL-MIGRATION-PUBLISHER.md)升级到 **08**，这些 checkpoint 事实保持原语义。下文格式数字保留组件引入时的历史含义。

2026-10-04；基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
第二阶段和生产验收仍开放。本轮增加已归档 checkpoint 的持久准入门、按尝试索引的
执行者分页，以及未进入执行者的终态操作，解决关闭后重试和恢复路径无法收尾的问题。

## 状态与原子性

`RecordCheckpoint` 将规范 intent 与 Open gate 在同一个 metadata Raft batch 中提交。
`CloseCheckpointAdmission` 按精确 Pending token 单调关闭该 gate。重复 Record/Close
保留首次关闭的 revision；关闭后 Acquire、Share 和 Publish 都拒绝，包括原代已经
Held/Published 的重复进入请求。planner 仍由同一 catalog 锁、先前提案排空、同任期
append 和精确已应用回执约束；只做读取或收到不明确错误不能授权上传。

已经进入的执行者可以继续工作。实际本地 job 排空后，仍保留完整 Published Pending
或 Version 保护；当前 worker 通过[原子收尾](RAW-TTL-EXECUTOR-DRAIN.md)一起提交
Released 和逐代报告，旧分步 Quiesce/Release 仍兼容。Close 不会中止远端 PUT，也
不会把已有 Published owner 改为 Released。旧进程的遗留记录继续保留。

`CancelUnenteredCheckpoint` 只接受不存在的 generation 1，或已 Released 记录的
紧邻下一代；同代 Released 可幂等确认。它原子写入 Released owner、不可变 registration、
所有 resource pin 和执行者索引，使延迟到达的同代 Share 无法复活。Held、Published、
Quiesced 均不能取消，必须沿原排空流程收尾。读后与取消提交之间若发生实际进入，
取消会拒绝；调用者保留原 handle，下一轮重新观察并转交保护。

执行者绑定仍由 root、checkpoint operation、node/store/process 身份派生，完整
descriptor 和 resource closure 必须与 intent 一致，generation 不允许跳跃或回退。

| 持久记录 | 版本与 namespace tag | 核对关系 |
| --- | --- | --- |
| Ledger header | `KV9LED03`–`KV9LED08`，tag 1 | gate 接受第三至八版；当前写入原子升级为第八版 |
| Intent | `KV9CPI02`，tag 8 | 规范 descriptor，必须同时存在匹配 gate |
| Admission | `KV9CGA01`，tag 9 | root、Pending ID、intent 摘要、打开/关闭 revision、校验和；136 字节 |
| Executor index | `KV9CEX01`，tag 10 | Pending ID + executor ID 定位，核对 generation、phase、binding 摘要 |
| Executor owner/registration | `KV9OWN02`，tag 3/5 | 两行版本一致，反向要求 gate 和 index，逐 resource 校验正向 pin |

新增 ledger request tag 为 8（Close）和 9（Cancel）；原 16 KiB 小请求、8 KiB owner、
8 MiB batch 上限保持。旧 header 01/02 和 intent 01 可读；相同 intent 经 Record
原子升级为 02 + Open gate。已有 02 丢 gate、孤立 index、单边 owner 降版或 gate 下
header 降版均拒绝，不重建空集合。旧执行者只在精确绑定操作时增量升级，没有全量
legacy backfill 或旧 binary 混部资格声明。

2026-10-05 后续：[独立 Build 反向索引](RAW-TTL-BUILD-INDEX.md)已增加 header 04
下的全局双遍回填，覆盖 catalog 内 legacy Build。此处 executor index 仍只
表示 managed 执行者；回填不升级旧 Held，不补造缺失报告或授予 retirement。

同日后续：[逐代原始准入凭据](RAW-TTL-EXECUTOR-ENROLLMENT.md)阻止同代 Share /
Publish 重试补造 managed 资格。旧 V1 handoff 保留原格式；V2 本身也不能替代
新代进入时原子保存的 `KV9CEN01`。同代 Released 的 Cancel 不补造未进入凭据。

同日后续：[BeginCheckpoint](RAW-TTL-ATOMIC-BEGIN.md)原子创建新 Pending 与 intent /
gate；旧尝试继续保留旧来源。对带原子创建记录的新尝试，旧 Record 请求不再作为
上传前的最终授权，新 worker 的创建与 settlement 重试均使用 Begin。

## 分页观察与失败恢复

内部认证 `CheckpointRetention` RPC 新增 `Executors {pending, after, limit}`。
metadata leader 经 ReadIndex，在一个 snapshot 中检查 gate、owner/index 和全部 pin。
每页 1–16 个 owner，最多读取 17 个索引条目；结果按 ID 严格递增，cursor 是最后返回
的 ID。wire digest 绑定 root、query family、Pending、cursor 和 limit；独立规范页面
编码 `KV9CEP01` 上限为 131392 字节，拒绝错序、重复、跨查询、非法 cursor 和截断。
这是逐页的新鲜观察；并发变化下不是跨页固定 snapshot，也不是完整 publisher 集合证书。

data/metadata worker 在 `WindowRefused`（精确提交窗口由另一个 change 占用）后
自动 Close，并确认回执后才清除 journal。2026-10-05 同样接通正向 settlement：
`MyChangeApplied`、`AlreadyApplied`、`EffectSettled` 在 Pending→Version 交接前
关闭该精确尝试，即使 Pending 已 Released 的重试也先确认 Close。实际采用已提交
checkpoint 的 data follower 可以完成这一收尾；已有执行者仍可排空和转交保护。
NotSubmitted、ReceiptRefused、InvalidRefused 不触发永久关闭：这些结论不足以
排除同一身份在其他调用中已提交。新增验证见
[原子收尾证据](raw-ttl-finish-validation-20261005/manifest.json)。

恢复时先由 `ManifestSeam::inspect_checkpoint` 只读核对 descriptor 对应的精确窗口。
它可返回 AlreadyApplied 或 WindowRefused；缺历史、窗口尚未消耗或已有 in-flight
slot 返回未知。它不生成 PreparedFlush、不占用/清除 slot、不扫描 effect history，
不提案。已关闭的失败 journal 因而可以在上传前收尾；正向结果继续走原 owner 交接和
已验证 winner adoption。未知结果仍保留原 journal 和保护。

## 验证和证明边界

[独立证据包](raw-ttl-admission-validation-20261004/manifest.json)保存命令、日志、证明
输出与源码摘要，历史证据包保持原样。

- 新 metadata 测试覆盖关闭单调性、所有进入路径拒绝、取消和代际、35 个执行者分页、
  双向索引/owner 损坏拒绝、旧 intent 升级、规范 codec 与 query 相关性。
- 真实三副本 Raft、分离 metadata/data leader、认证 HTTP/2 duplex 验证 Close/Cancel
  丢回执、延迟进入、已有执行者排空、后续代取消；metadata WAL 重开保持 gate、
  全部 indexed owner 和 Published Pending。
- 自动 data worker 的故障前缀为“Close 已提交，未上传的 journal 尚在”。实际 data
  WAL 重开后禁用新触发，worker 以精确提交历史完成拒绝收尾，对象 I/O 为零，
  不创建 executor，Pending 仍 Published。受控停止/重开不等同 process-kill。
- common/meta **197 通过、1 忽略**；TTL 组合 **27 通过**；region **16 通过**；
  retention wire **7 通过**。严格 workspace Clippy、格式和 diff 检查通过。

[TLA+](../proofs/tla/checkpoint_admission/CheckpointAdmission.tla)描述共享 gate、多个
executor、代际、进入、Publish、Cancel、Drain 和 Release。重复请求、丢回执、重启
在 durable projection 中为 stutter。SANY 解析/语义检查通过，TLC 仍未执行；已有
sandbox socket/RMI 限制未重试。模型的有限配置不构成模型检查完成的证据。

[Lean](../proofs/lean/checkpoint-admission/Admission.lean)证明单执行者对共享 gate 的
投影：初始化/单步/可达不变式、关闭后不再进入或 Publish、代际不回退、已进入者
不能跳过排空、Released 同代不能重入、全局保护保留，共 **8 条组件定理**。归纳
覆盖任意代际与任意长度历史。五个错误变体（重开、关闭后进入/Publish、取消已进入
者、复用原代）均使证明失败；另有 proof hole 和自定义 axiom 两个拒绝控制。
原子 Raft apply、摘要身份、后继完整保护和实际本地排空为组合前提；没有从 Rust
自动提取模型，也未证明远端 PUT 终止、全体 publisher 关闭或完整 refinement。

仍待完成：旧 binary capability floor 与完整 backfill、崩溃遗留执行者处理、稳定的
全体执行者核对、独立 manifest 历史证明与持久关闭证书组合。空页和 Released owner
不授权释放负向 Pending 或删除对象。Reader/旧 Version、迁移和 TTL split/merge、
历史物理容量与 GC、共享 CPU/字节预算及真实 TCP/MinIO/Chaos 验收继续开放。
