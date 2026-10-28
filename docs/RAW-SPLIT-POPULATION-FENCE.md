# Raw split 复制重试的有序屏障

日期：2026-10-04。第二阶段的前置修复；TTL split/merge 尚未开放。

旧 population 用普通 `Write` 复制 Default CF。发布后的 split intent 仍可读回，
因此再次调用复制，或让已在途的旧批次延后应用，可能覆盖子范围的新值或复活已删除
的 key。仅在 RPC 开始时读取目录无法防止发布和 apply 之间的竞争。

现有 runtime 已改用 `PopulateRaw`（Raft entry tag 11）。每帧绑定 operation、
封存父范围、子范围的 root/creation/region/tenant/keyspace 和精确半区间。编码与
构造器检查有序唯一 key、严格子区间、最多 256 行及完整 frame 最多 16 MiB。
runtime 按行数与编码字节拆分；单行放不下时明确拒绝。

有序 apply 核对本地不可变 creation，并要求 `RANGE_KEY` 尚不存在且 TTL 未启用。
子组一旦应用所有权描述，后续新复制帧全部返回 `FenceRejected`，不会修改 value；
拒绝与 applied position 原子持久化。该指令不能走通用 batch lowering，也不参加
普通 Raw 合并 apply。所有权 row 本身就是本次修复所需的持久屏障，不依赖进程内锁。

runtime 在目录已发布或子组已有范围 row 时提前返回 `StaleEpoch`；真正的竞争由
上述 apply 检查裁决。population 和 seal 现在检查完整 typed receipt，拒绝、被替换
和未确认不再被当作成功。复制与发布校验共同使用父范围实际 start/end，支持再次
拆分已有子范围，不再扩大扫描到整个 keyspace。

## 兼容性和范围

旧 `Write`、`Put`、`CatalogTxn` 的 apply 语义保留，已有日志重放不会被重新解释。
因此本保证针对新指令及已更新的生产复制路径；旧版本 proposer 的普通复制命令
仍属于 legacy 行为。新 tag 会被不认识它的 decoder 拒绝，但这不构成完整的在线
升级协议。上线需要全副本/协调者支持新指令并停止旧复制路径；当前没有新增逐副本
能力握手、持久 binary floor 或旧二进制混部验收，不能宣称支持滚动升级。

source 描述不是跨组提交证明；metadata 已提交 intent 和可信内部 runtime 仍是
调用前提。每次重试仍从头复制尚未绑定的子组，没有新增持久复制游标、完成证书、
全量源摘要提交或 abort 协议。16 MiB 限制约束日志帧，现有 snapshot scan 返回
最多 256 行的内存成本尚未纳入节点总预算。

该命令只复制普通 Raw Default CF。TTL 组的旧 seal 继续拒绝；value/expiry 原始
身份和绝对期限、水位继承、全副本格式门槛、持久 staging/完成状态及恢复校验仍需
完整 TTL 转移协议。没有放宽 snapshot/WAL 对 TTL 与所有权状态的约束。

## 验证

[证据包](raw-ttl-population-validation-20261004/manifest.json)保留命令、源码摘要和日志。

- 三副本真实 Raft：300 行低半区跨页复制、发布前重试、高半区复制、不同 metadata/
  data leader 下发布、新值覆盖和删除，再提交旧复制帧；三副本均拒绝且数据不变。
- 停止实际 driver 后逐个重开三个子副本的磁盘 Raft/WAL，走 production data-group
  recovery；重新构造状态机后再次复制仍被拒绝，覆盖值与删除均保留。
- apply/codec 覆盖绑定前幂等、绑定/封存后拒绝、错误 creation、损坏 range row、
  截断、尾随字节、过多行、重复 key、系统 key、超限帧及 legacy Write 重放兼容。
- state-machine 测试 **84 通过、1 忽略**；TTL integration **32 通过**；command
  名称过滤测试 **18 通过**（与前一集合有重叠，不累加为独立测试数量）。严格 workspace
  Clippy（现有 tracing/checkpoint-testing 配置）、格式和 diff 检查通过。额外尝试
  all-features Clippy 因未解包的 bincode 依赖及只读 Cargo 缓存未运行完成。

[Lean](../proofs/lean/split-population/Population.lean)包含七条组件定理：所有权单调、
绑定后复制不变、TTL 拒绝普通复制、有效复制需要未绑定非 TTL 状态、任意长度的延迟
复制历史不变、删除不被复制复活及封存值不变。四个错误变体与两个 proof-policy
控制被拒绝，axiom audit 通过。模型仅覆盖新 tag 的单 key 屏障，不是 Rust 自动
提取，也不证明源认证、完整转移、资源预算或升级协议。

[TLA+](../proofs/tla/split_population/SplitPopulation.tla)通过 SANY 解析/语义检查。
TLC 仍受已知环境限制未运行。此测试使用 InProcHub 和本地持久 WAL；未验收真实
TCP、完整 NodeRuntime、MinIO、旧 binary、进程强杀或跨主机 Chaos。
