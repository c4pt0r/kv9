# Build owner 历史索引与有界回填

2026-10-05 后续：ledger **07** 的[已登记集合闭合核对](RAW-TTL-REGISTERED-CLOSURE.md)保留 06 birth 和全部旧来源语义，新增有界聚合与原子变更水位。当前 ledger 已由[迁移 Publisher](RAW-TTL-MIGRATION-PUBLISHER.md)升级到 **08**，这些 checkpoint 事实保持原语义。下文格式数字保留组件引入时的历史含义。

2026-10-05；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
接续[持久报告补交](RAW-TTL-DRAIN-OUTBOX.md)。第二阶段与生产验收仍开放。

原子 Begin 引入时，ledger header 为 **`KV9LED06`**，已接入[逐代原始准入凭据](RAW-TTL-EXECUTOR-ENROLLMENT.md)
和[尝试原子创建](RAW-TTL-ATOMIC-BEGIN.md)。下文记录本组件引入的 04 布局；新版本保留这些索引和进度，禁止旧代通过 Share /
Publish 重试补造新协议资格，回填也不产生这类凭据。

## 修复的遗漏

原 checkpoint executor index 只覆盖已登记到新协议的执行者。早于 intent/gate
的 Build owner，只有再次被操作时才可能进入它；因此该索引的 EOF 不能代表旧
Build 已枚举完。把 legacy Held 直接改成新协议执行者也不安全：旧 Held 没有
“已按新协议登记、尚未获准 I/O”的含义，不能借此套用 unpublished retirement。

新增独立 Build 反向索引，包含 metadata catalog 中所有 Build owner，并保留其
原 owner 格式、generation、phase 和资源绑定。旧记录不被提升为新协议执行者，
也不会产生排空报告或释放任何 pin。

## 布局、写入与恢复

| 记录 | tag / 格式 | 内容 |
| --- | --- | --- |
| Ledger header | 1 / `KV9LED04` | 新写入方必须维护 Build 索引 |
| Build 反向索引 | 14 / `KV9CBI01` | 由 operation 派生的 Pending ID + owner ID；root、两种 ID、immutable registration 摘要、checksum；136 字节 |
| 全局回填进度 | 15 / `KV9CBP01` | root、sequence、当前及上次 stage/cursor、checksum；最多 116 字节 |

新 ledger request tag 14 是 `AdvanceBuildIndex`，与 **Raft manifest tag 14**
属于不同编码空间。未增加 RPC 类型或修改 service/store V7 floor。
reader 接受 header 01–04，旧 header 的成功写事务原子升级到 04。旧 header
下若已有新索引/进度行，拒绝按降版布局继续执行。旧 retention planner 不认识
04，不能继续修改而遗漏新索引；这仍不构成混合二进制或滚动升级验收。

新 Build 的 acquisition/share、后续转换和 managed executor 写入，均在原
metadata Raft batch 内维护反向索引。摘要使用 generation 归一化的 immutable
registration，后续 generation 不创建重复索引项。回填途中新增 ID 即使排在
持久 cursor 之前，也由写事务登记。对已完成回填的 catalog，点读 Build 时若
发现索引丢失或绑定冲突会拒绝，不能把它解释成不存在。

回填对整个 catalog 只做两遍：先 owner state（tag 3），后 immutable
registration（tag 5）。每次最多处理 **4 行，读取 1 行 lookahead**。第二遍
能发现 registration 存在但 owner state 丢失的情况。每行核对双向 owner
绑定及完整资源 pin；索引写入与 cursor/sequence 推进在同一原子 batch 内。
实际页边界不匹配、跳页、过时请求或破损绑定均拒绝；上一条精确请求可幂等重试。

完成后，`builds(view, root, pending, after)` 在所给 snapshot 中按 Pending
前缀分页，每次最多读取 5 条索引并返回 4 个完整 owner。未完成时查询拒绝。
它不为每个 checkpoint 重扫全部历史；页内点读成本随 resource closure（最多
64 个资源）变化。旧 resource base 的解码成本仍受旧 codec 的 4096 项上限
约束，不能把固定行数表述为全路径固定字节或 CPU 成本。

## 后台接入

metadata leader 在现有 `CheckpointRetention` 共享维护槽内交替执行全局
Build 回填和 checkpoint 尝试维护；已有本地排空报告继续按原规则获得补交机会。
每页先做 ReadIndex，在同一 snapshot 核对 certified root 和进度，再提交
规范请求。取消、deadline 到期或已知失去 leader 身份时拒绝派发。

丢回执后从复制的进度续做；换主不需要继承进程游标。完成后只观察，不不断追加
进度命令。`build_index_pages/rows/upgrades/catalog_complete` 指标表示该组件
的已确认工作或观察，不表示全局 publisher 已关闭。

## 验证与边界

[独立证据包](raw-ttl-build-index-validation-20261005/manifest.json)保存本轮日志、
源码摘要和前序证据链接；历史证据不重写。

- 7 项 metadata 专项：legacy 与 cursor 前并发新增、精确分页/重试、双遍孤立
  registration 检测、缺失/跨 Pending 索引与 header 降版拒绝、后续 generation
  绑定稳定、13/113 条历史的固定读取量，以及实际 WAL 多处截断的原子恢复。
- 真实 Raft + 鉴权 HTTP/2 duplex：通过已提交 batch 构造 9 个 V3 legacy
  Build；丢失首个回填页回执后换主，由后台从复制进度完成。旧 leader 与取消
  请求拒绝，完成后不追加；受控停止并重开 metadata WAL，全部旧 owner 仍为
  原格式 Held，Pending 仍 Published。
- 仍使用 InProcHub/duplex；没有真实 TCP、MinIO、OS process-kill、断电或
  混合二进制资格结论。本轮未添加形式化证明。

metadata 全套 **168 通过、1 忽略**（需真实 MinIO）；TTL 集成全套串行与单独
并行运行均 **51 通过**。去重后共 219 项通过，专项和复验不重复计数；严格
workspace Clippy、格式和 diff 检查通过。

开发中的两个 fixture 问题已修正：WAL 初始化漏复制根元数据，以及 legacy
owner magic 断言拼写错误。另一次与 metadata 全套同时运行的并行集成中，
checkpoint worker 共存用例触及原有 20 秒 deadline；定向、完整串行及单独
完整并行复验均通过，期间没有修改代码或放宽时限。保留首次失败日志，这些
复验不能证明首次超时的根因，也不关闭高负载下的时限/公平性验收。

`Complete` 只代表在有效原子存储和新写入规则下，catalog 已执行上述回填。
它不是任意磁盘损坏下的集合完整性证书：例如完成后反向索引被删除，在未点读
对应 owner 的分页查询中不能检测这个遗漏。尚无跨全集合的摘要/计数认证，也
不覆盖从未登记的历史上传者。查询必须保留这一限制，不能据其 EOF 放行 GC。

逐代报告缺失、legacy Build 的真实 I/O 状态、崩溃 Published 执行者和远端未知
PUT 的终止或隔离，仍需继续解决。负向 Pending、Reader/旧 Version、可转移恢复
锚点和对象 GC 不由本组件释放。索引与 owner 历史仍随写入增长，物理稳态、时钟
部署资格、TTL split/merge 和完整第二阶段验收继续开放。
