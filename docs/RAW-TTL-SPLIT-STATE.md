# TTL split 的持久复制状态与恢复

日期：2026-10-04。这是第二阶段的状态迁移组件，生产 TTL split 入口尚未开放。

后续同日进展见 [V5 格式隔离](RAW-TTL-SPLIT-FORMAT.md)。下文描述本组件验收时
的边界；V5 磁盘/服务隔离随后已实现，生产 Seal/Begin 门槛与源认证仍待接通。
其后的 [runtime 接入](RAW-TTL-SPLIT-RUNTIME.md) 已连接本地已提交源 snapshot、
全副本门槛、双 CF 复制与目录发布；导入 checkpoint 和完整生产验收仍开放。

新增 `RawTtlSplit`（Raft entry tag 12）及四个有序步骤。`Seal` 将父范围的精确
拆分计划、实际 Raft cut 与已复制时间水位原子封存；`Begin` 在空子组建立持久
import 状态并继承水位；`Append` 原子写入 value、原始 expiry 和下一页游标；
`Finish` 仅在累计行数与摘要匹配时完成复制并写入子范围所有权。

计划绑定 operation、root、父/子 creation、region、tenant/keyspace 和精确半区间。
源封存与 import 记录有规范编码、长度上限和 SHA-256 校验。每个复制帧最多 256 行、
完整编码最多 16 MiB；key 严格递增，页提交比较前一持久状态摘要。摘要覆盖 key、
opaque value 及 expiry 是否存在和全部原始字节。复制不重算绝对期限、不改写来源
creation/term/index；永久值同时清除对应的 Write CF expiry。

复制期间没有范围 row，客户端写入和提前绑定范围均被拒绝。水位在 `Begin` 写入，
即使子范围没有任何行也继承父水位。重复 Begin 不会清空游标，重复 Finish 不会
重置完成后新写入的数据或水位；过时页面和完成后页面返回 typed `FenceRejected`。
父组封存后拒绝 Raw 写和时间推进，固定复制 cut。完成后的子组可继续续期，并用
原始身份清理未续期的到期值；旧清理身份不能删除新值。

## 空子组观察与恢复

`inspect_empty_ttl_import` 从实际 driver 的 positioned resident snapshot 取得
同一时点的各 CF 精确行数；只允许空引擎或合法 compaction floor。它不遍历或复制
大 value。`ReadView::resident_len` 默认不提供能力，MemSnapshot 使用持久树的
精确 size；不把 `scan(b"", b"")` 的空区间误当作全范围扫描。

观察结果绑定子组和 durable applied position，不能反序列化或克隆。首次导入在
apply 再比较该位置，期间发生写入就拒绝。新组需显式命令 barrier；Raft 自身的
空 leader no-op 不保证应用引擎已有位置。这只是一个已观察前缀，尚不是格式确认、
源认证或提交授权。

生产 data-group recovery 校验本地 range、clock、seal、import 的组合，以及每个
WAL transfer batch 的完整布局：封存的实际 cut、Begin 的水位继承、每行 value/
expiry 与游标一起写入、Finish 的计数/摘要及范围绑定。缺失、重排、单独游标、
改写 expiry 身份、提前绑定与删除 transfer row 均拒绝恢复。外来 expiry 的来源
index 可以高于子组本地 index，不错误地按子组日志位置裁剪。

完成的子组可生成并校验带 import 记录的 checkpoint 镜像；未完成 import 仍依赖
WAL，镜像检查明确拒绝没有所有权 row 的 staging 状态。未实现 staging checkpoint
或 WAL 空间预算，不能据此宣称大规模复制期间磁盘使用有界。
后续 2026-10-05 [staging checkpoint](RAW-TTL-STAGING-CHECKPOINT.md) 已补未完成 import
的镜像与恢复，按真实 publication 回收 engine WAL；完整空间稳态仍未验收。

## 生产接入边界

所有 `RawTtlSplit` 普通提案均拒绝。现有 V4 激活确认不证明 tag 12 的支持；尚未
增加新持久 binary floor、逐副本新格式门槛及生产受保护提案入口。测试通过显式
harness 提交编码命令，不能作为完整 NodeRuntime TTL split 的证据。

`SplitPlan`、`SplitSeal`、`ImportSpec` 是校验过的数据，不能仅靠其内容证明远端
Raft 已提交。接下来需将 metadata 已提交 intent、实际源 seal/cut、全副本格式
确认、精确空子组观察与受保护入口连接，再接通 runtime 复制和目录原子发布。
runtime 还需从稳定源 snapshot 的 Default/Write CF 构造两侧完整摘要，拒绝孤立
expiry，并在重试/恢复中验证相同的 source commitment。当前没有生产源摘要构造器。

既有普通 Raw split 路径继续使用 [tag 11 复制屏障](RAW-SPLIT-POPULATION-FENCE.md)，
TTL 旧 seal 仍拒绝。没有开放 merge，也没有完成复制页内存、节点公平预算、超大行、
中途 abort、旧组退役、恢复锚点及对象 GC 的完整协议。

## 验证

[证据包](raw-ttl-split-state-validation-20261004/manifest.json)记录源码摘要与原始日志。

- 三副本真实 raft-rs、磁盘 Raft/WAL、InProcHub：封存、复制首批、全部停止、走
  production data-group recovery 重开、换 leader 续传、完成、续期/清理、再次
  全部重开；父封存和子状态均保留。完成镜像校验通过，未完成镜像明确拒绝。
- WAL 写入前后注入错误，检查 value、expiry、cursor、clock、range 与 applied
  position 全旧或全新；空观察后写入、Lock CF 残留、错误 creation 均被拒绝。
- state-machine **91 通过、1 忽略**，engine mem **23 通过**，server TTL activation
  **34 通过**，共 148 个不同测试；严格 workspace Clippy（tracing/checkpoint-testing）、
  格式和 diff 检查通过。

[Lean](../proofs/lean/ttl-split/Transfer.lean)的 9 条定理覆盖计数上限、水位继承与
单调性、完成前计数/摘要检查、完成不可逆和完成后游标不变。6 个语义错误变体与
2 个 proof-policy 控制被拒绝，axiom audit 通过。模型把摘要视为抽象值，不证明
哈希抗碰撞、行编码、CF 原子性、源授权或 Rust refinement。
[TLA+](../proofs/tla/ttl_split/Transfer.tla)通过 SANY 解析/语义检查；TLC 因已知环境
限制未运行。真实 TCP、MinIO、旧 binary 混部、进程强杀和跨主机 Chaos 尚未验收。
