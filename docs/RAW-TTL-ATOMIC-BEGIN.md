# Checkpoint 尝试的原子创建

2026-10-05 后续：ledger **07** 的[已登记集合闭合核对](RAW-TTL-REGISTERED-CLOSURE.md)保留 06 birth 和全部旧来源语义，新增有界聚合与原子变更水位。当前 ledger 已由[迁移 Publisher](RAW-TTL-MIGRATION-PUBLISHER.md)升级到 **08**，这些 checkpoint 事实保持原语义。下文格式数字保留组件引入时的历史含义。

2026-10-05；基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
接续[逐代原始准入](RAW-TTL-EXECUTOR-ENROLLMENT.md)。第二阶段与生产验收仍开放。

## 改变的行为

此前新 checkpoint 需要五次独立 mutation：Initialize、Register、Acquire Pending、
Publish Pending、Record intent/gate。执行者严格等全部回执后才开始 I/O，但中途
失败会留下不同创建前缀，最终的 intent/gate 也无法证明 Pending 与它们同时出生。

现在 data/metadata worker 使用 `BeginCheckpoint(intent)`，在一次 metadata Raft
batch 中创建 ledger（若为空）、所需资源登记和 Published pin、Pending owner /
registration、完整 intent、Open gate，以及不可变的原子创建记录。没有可见的
中间 Held Pending，也不需要对未提交 overlay 进行授权。未知回执仍停止当前工作；
下次按同一 descriptor、predecessor 和 Pending ID 幂等续做。

正向 settlement 也改用 Begin 核对原尝试，然后 Close、Share Version、Publish
Version、Quiesce Pending、Release Pending。首次创建加完整交接由十一处独立
mutation 变为 **七处**，新的故障用例逐一丢失这七处回执。

## 旧尝试与损坏处理

Begin 发现已有精确 Pending 时续做原状态，**不补造原子创建记录**：Held 在同一
batch 中进入 Published 并创建 intent/gate；Published 核对并记录 intent；
Quiesced/Released 必须仍有完整 Published Version 覆盖。Closed gate 不会重开。
Held 已有 intent/gate 属于不一致状态，不以“恢复”为由覆盖修复。

Pending 缺失而 registration、intent、gate、创建记录或对应 Version 已存在时，
拒绝猜测成新尝试。资源的 registration/header/pin 同样沿用全部交叉校验，已有
resource 的旧 base 保持有界升级；不会扫描或改写全部 resource owner 历史。

创建记录丢失时查询返回 None，代表来源缺失。因为 Pending 已存在，Begin 重试
也不能重新生成该记录。损坏或跨 root/intent/revision 的记录拒绝，不退回空状态。

## 格式和调用约束

新 ledger request **tag 15** 承载原 canonical `CheckpointIntent`，使用与 Record
相同的大 descriptor 上限和现有 8 MiB 原子 batch 上限。它不是新的 RPC 类型，也
不是 retention namespace 的 tag 15（后者仍是 Build census 进度）。

| 记录 | 格式与位置 | 校验 |
| --- | --- | --- |
| Ledger header | `KV9LED06`，tag 1 | reader 接受 01–06，旧格式成功写入时原子升级 |
| 原子创建 | `KV9CBO01`，tag 17 + Pending ID | 128 字节；root、Pending、intent 摘要、created revision、checksum |

created revision 必须等于该 gate 的 opened revision，不超过当前 ledger revision。
读回还要核对完整 Pending/Version 资源保护。原有 04 Build 索引/进度和 05 逐代
准入记录保留；低于 06 的 header 下出现新创建记录会拒绝。header、gate、逐代
准入和后台 Build census 的 reader/升级路径均已接通。

对具有原子创建记录的尝试，旧 `RecordCheckpoint` 请求拒绝作为最终上传授权；
新 worker 使用 Begin，不在未知结果后回退到旧创建流程。未带原子来源的旧尝试
仍保留原 Record 语义。这个拒绝点覆盖原先以 Record 结束 before_io 的调用序列。

内部 service/store floor 仍为 **V7**。header/request 拒绝没有替代完整旧二进制
与部署隔离：更早未按该序列登记的上传者、已发出的远端 PUT、暂停进程和旧读者
仍需单独处理。没有宣称本变更可直接进行不停服滚动升级。

## 这份记录能证明什么

它证明的是 Pending、完整保护、intent/gate 在一次 metadata 提交中创建。它
**不证明此前没有任何 Build 或外部写入**：通用 Build 可以早于 Pending 存在；
测试明确保留这种旧 owner，并让逐代查询报告 MissingOrigin。

`atomic_birth` 是 metadata 内部只读查询，没有新增 wire 查询或 GC 权限。完整
关闭还须组合 Build 全集合、每个 generation 的原始准入与结束事实、负向
settlement、远端终止/隔离及恢复依赖。不能把原子创建记录或任一 EOF 单独当作
负向 Pending 释放证书。Reader/旧 Version、对象 GC 和物理稳态仍未完成。

## 验证

[本轮独立证据包](raw-ttl-begin-validation-20261005/manifest.json)保存日志、源码摘要
和前序证据链接，历史证据不重写。

- 7 项 metadata 专项覆盖空 ledger 原子创建、规范请求与截断、四种旧 Pending
  phase 不补证、正向交接后原记录不变、缺失/损坏/降版拒绝、旧 Build 保留、
  既有 Version 不被误认为新尝试，以及实际 WAL 截断的全有或全无恢复。
- 真实 Raft/鉴权 HTTP/2 duplex 逐一覆盖七处丢回执。Begin 回执丢失时，同一
  metadata snapshot 已有创建记录和 Published Pending；没有开始对象上传。
  原身份续做发布/交接后，data 与 metadata WAL 重开保留 TTL、精确 intent、
  Closed gate、原子创建记录和 Published Version。
- 旧布局升级、后台 metadata/data worker、持久报告队列和 Held 回收继续组合
  验证。首个 Held 退役的故障坐标从第五次 Apply 改为第三次（Begin/Close/Retire），
  仍断言只完成第一条退役、其余八个 Held 保留。

本轮完整 metadata 测试 183 项通过、1 项既有 MinIO 用例忽略；完整 TTL integration
52 项通过，合计 235 项不同测试。workspace/all-targets 的严格 clippy、格式和
差异空白检查通过。

后台共存用例曾在一次完整串行运行中超过原有 20 秒时限（其余 51 项通过）。
已补充三个等待阶段、worker/维护状态与 owner phase 的诊断，**未放宽时限**。
随后单项运行、连续十次运行、与大量 WAL 写入测试并行的十次运行，以及完整
串行回归中的该用例均通过。这些结果尚未定位超时根因，不代表间歇性问题已修复；
失败日志和全部重跑日志一并保留。后续生产资格仍需解决该问题。

同日后续：[换主覆盖修正](RAW-TTL-CHECKPOINT-FAILOVER.md)已通过角色诊断定位上述
交接停顿：测试只在初始 metadata leader 上启动 sweep，换主后新 leader 缺任务。
测试现已对齐每节点启动路径，并增加旧 catalog 持续忙时的新 leader 后台接管回归。
上面的失败与重跑数字保留为原子 Begin 当轮的历史记录。

组合使用 InProcHub/duplex 和内存对象后端；没有新增真实 TCP、MinIO、进程 kill、
断电、混合二进制或形式化证明资格。完整第二阶段的剩余要求继续保留。
