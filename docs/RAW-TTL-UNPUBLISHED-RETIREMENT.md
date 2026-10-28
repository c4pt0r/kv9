# 未完成 Publish 的 checkpoint 执行者回收

2026-10-04；基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
第二阶段和生产验收继续开放。本轮处理一种有确切依据的崩溃遗留：执行者 Share 已
提交，但该代从未 Publish，随后 checkpoint 的准入已持久关闭。

## 允许回收的事实

上传必须先取得精确 Publish 的已应用回执。metadata 的同一代只会从 Held 前进到
Published/Quiesced/Released，不会从 Published 回到 Held。因而在一致视图中同时
看到原始 Entered 凭据、Closed gate 和精确 Held generation，才可证明该代尚未获得上传授权，且今后
也不能取得授权。它可能是已崩溃进程，也可能只是暂停在 Share 与 Publish 之间；
回收不依赖判断进程死活、超时、新 process identity 或 store 替换。

新增 `RetireUnpublishedCheckpoint {pending, executor}`（ledger request tag 10）
要求两个精确 token、原规范 intent、Closed gate、完整 Pending/Version Published
覆盖、`KV9OWN02` indexed executor 及全部 resource pin 一致。它在一个 metadata
Raft batch 中把该代 Held pin 改为 Released，同时更新 executor index 并保存证据。
通用 CancelUnentered 的语义不变；已经 Held 的代际不能用 Cancel 冒充从未登记。

2026-10-05 修正：[逐代准入凭据](RAW-TTL-EXECUTOR-ENROLLMENT.md)现为额外必需条件。
此前旧 Held 可经同代 Share 重试升级到 V2，说明 owner 格式本身不够；该路径已
封闭。缺原始凭据的旧 V1/V2 Held 不退役，旧回收记录也不能单独重新取得资格。

Published、Quiesced、普通 Released、未索引旧 owner、错 generation、错 Pending
以及破损交叉引用均拒绝。Released 只有原回收证据仍完整时才能幂等确认；丢失证据
不会从 Released 反推重建。元数据规划沿用 catalog 锁、已提交提案排空、同任期
提交与精确回执；分页时看到 Held 不直接产生回收权力，提交时必须重新核对。

后续任何迟到的同代 Share/Publish 均拒绝。此前其他 generation 可能已经上传，
这份证据只描述它绑定的一代，不证明其他代际、整个进程或整个 attempt 都已排空。

## 持久证据与恢复调度

`KV9CUR01` 记录位于原 retention namespace 的 tag 11，key 包含 Pending ID、
executor ID 和 generation。固定 160 字节保存 root、上述身份、不可变 binding
摘要、gate 关闭 revision、回收 revision 与 SHA-256 校验和。证据、owner、index、
resource pins 原子提交，回收 revision 必须严格晚于关闭 revision。

`unpublished_retirement` 从一个已建立的 metadata snapshot 校验记录和当前 owner/
gate/closure。之后仅取消了下一代时，旧代证据仍可按原 token 读取；旧代 mutation
不能作用于新代。没有新增公开查询 RPC；现有内部 Apply 承载新请求，既有认证的
Executors 分页已接入实际 worker。新增 evidence 不是 PreparedFlush、全局关闭
证书或对象 DELETE 凭据。

data/metadata worker 在精确 `WindowRefused` 后关闭准入，按每页四个 executor 处理
Held 遗留者，然后才清除原 journal。单轮最多检查四个 owner、提交四个回收请求；
Published/Quiesced/Released 跳过。只有整页操作全部确认后才前进游标；失败保留
游标和 journal。重启从第一页重新开始，已经提交的回收作为 Released 跳过。
关闭后不能新建 Held，因而游标之前出现的新 Cancel tombstone 不会遗漏待回收的 Held。
这不是跨页固定 snapshot，也没有产生“所有 publisher 均已退出”的结论。

分页结束只允许结束当前本地恢复调度；Published 遗留者和全局 Pending/Version
继续保护对象。已在旧版本清除 journal 的孤立 attempt 尚缺全局发现/调度，旧 owner
完整 backfill、已 Published 的崩溃遗留处理、远端未知 PUT 终止证明也仍待实现。
Released tombstone 和新证据都保留，本轮不删除对象或宣称物理存储已达稳态。

## 验证

[独立证据包](raw-ttl-unpublished-validation-20261004/manifest.json)记录命令、日志、
源码摘要及证明输出；此前证据包保持原样。

- 四项新 metadata 测试覆盖 Closed/Held 的原子回收和幂等、精确 token/规范请求、
  下一代取消后旧证据保留、Publish/Quiesce/普通 Release 拒绝、证据损坏/丢失拒绝
  重建，以及旧未索引 owner 和断裂交叉引用。
- 自动恢复组合测试扩展为十二个 executor：九个 Held、一个 Published、一个
  Quiesced、一个普通 Released。metadata/data 使用真实 Raft 与不同 leader，
  通过认证 HTTP/2 duplex 调用。首个回收已提交但回执丢失，核对仍有八个 Held、
  journal 保留；再次停止并重开 data WAL 后，原 journal 跨多页继续收敛。
  最终九个 Held pin 全部 Released，另三者不变；零对象 I/O、零新 checkpoint plan。
  再停止真实驱动并重开 metadata WAL，九份证据和 Published Pending 均保留。
- 新增 metadata worker 的实际恢复测试：七个 Held 和一个 Published 跨两页处理，
  原 journal 收尾，七份证据可核对；Published 及全局 Pending 保留，零对象 I/O。
- 初次故障注入漏重置累计 commits 计数，错误触及后续 adoption 失败；已修正计数，
  并增加“精确五次提交、首个 Held 已回收”的断言，保证测试实际覆盖目标故障点。
- common/meta **201 通过、1 忽略**；TTL 组合 **28 通过**。严格 workspace Clippy、
  格式与 diff 检查通过。网络仍为 duplex/InProcHub，对象为内存后端，受控重开
  不是完整 NodeRuntime、真实 TCP/MinIO 或 process-kill 验收。

## 协议模型与证明范围

[TLA+](../proofs/tla/checkpoint_unpublished/CheckpointUnpublished.tla)显式区分 Publish
提交、回执到达、对象使用、普通转交与未发布回收。迟到的 Publish 回执允许在关闭/
普通 Release 后到达，因而模型不会把 Released 等同于没有上传。SANY 解析与语义
检查通过；TLC 仍未运行，沿用现有 sandbox socket/RMI 限制，未重复已知失败启动。

[Lean](../proofs/lean/checkpoint-unpublished/Unpublished.lean)对一个任意、不可变的
root/owner/generation token 投影，归纳证明任意长度的转移历史：可达状态不变式、
回收代从未 Publish/获得回执/使用对象、回收必须有 Closed gate 和终态、关闭不重开、
证据保留、Publish 回执排除回收，以及全局保护保留，共 **9 条组件定理**。六个错误
变体与两个 proof-policy 控制均被拒绝。不同 generation 是不同 token，Rust 的
精确绑定/代际校验通过组件和组合测试核对，没有从 Rust 自动提取完整 refinement。

前一轮 admission 模型只包含原转移集合，其“Held 也必须排空”的定理不涵盖本轮
新增的 Closed+Held 规则；本轮模型明确补入该规则，并以 Publish 才能授权 I/O 的
事实证明它。Raft 原子性、身份摘要、所有上传者遵守 Publish 门槛、旧 binary 升级
和完整 backfill 都是组合前提，不能以单组件定理关闭全阶段验收。

下一步仍需已 Published 遗留执行者的真实终止凭据与全局发现/核对，再组合精确
manifest 历史与完整 publisher 集合，才能推进负向 Pending 释放。Reader/旧 Version、
迁移/TTL split/merge、整机预算、历史容量、部署时钟与完整 Chaos 继续保留。
