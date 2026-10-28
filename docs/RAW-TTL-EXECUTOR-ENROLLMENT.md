# Checkpoint 执行者逐代准入凭据

2026-10-05 后续：ledger **07** 的[已登记集合闭合核对](RAW-TTL-REGISTERED-CLOSURE.md)保留 06 birth 和全部旧来源语义，新增有界聚合与原子变更水位。当前 ledger 已由[迁移 Publisher](RAW-TTL-MIGRATION-PUBLISHER.md)升级到 **08**，这些 checkpoint 事实保持原语义。下文格式数字保留组件引入时的历史含义。

2026-10-05；基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
接续 [Build 历史索引](RAW-TTL-BUILD-INDEX.md)。完整第二阶段与生产验收仍开放。

原子 Begin 引入时，ledger header 为 **06**，[尝试原子创建](RAW-TTL-ATOMIC-BEGIN.md)
保留本组件的 05 准入记录和语义；下文描述本组件引入时的格式。

## 修复的安全缺口

此前新协议以 `KV9OWN02` owner/index 表示 managed executor，但这些字节也能由
旧 owner 的重试生成：generic V1 Held → 创建 intent/gate → 重试同代 Share →
升级为 V2 Held → 关闭 gate → unpublished retirement。由于旧 Held 不提供新
协议的“从未获准 I/O”保证，仅核对当前 V2 格式并不足以授权回收。

同代 Released 的 Cancel 重试也不能表明它从未进入过。它可能来自实际上传后的
通用 handoff；把这种重试当作取消事实，会在后续逐代核对时掩盖未知写入。

现在，只有与**真正的新代际**一起提交的不可变准入凭据才能证明其来源。同代
重试、owner 格式升级、普通 handoff、Released 状态、扫描 EOF 和新代成功都不能
补造旧代的凭据。旧 V2 缺凭据的记录也按不确定历史处理。

## 原子写入与格式边界

`KV9CEN01` 位于 retention namespace **tag 16**，key 为 Pending ID、executor ID
和 generation。固定 **161 字节**包含 root、两个 ID、generation、immutable
binding 摘要、gate opened revision、recorded revision、origin 和 checksum。

origin 有两种：

- `Entered`：新一代 Share/Acquire 创建 Held，完整资源 pin、owner、executor /
  Build 索引与凭据在同一个 metadata Raft batch 中提交。必须是第一代，或在前代
  Released 后紧邻的新代；gate 必须仍 Open。
- `Cancelled`：第一代未进入，或前代 Released 后尚未进入的紧邻新代。Released
  tombstone、所有 pin/index 和凭据原子提交；允许在 gate Closed 后取消。已经
  存在的同代 Released 重试只确认原状态，不产生新的 Cancelled 凭据或升级格式。

ledger header 升到 **`KV9LED05`**。reader 接受 01–05，旧 header 的成功 mutation
原子升级；04 下既有 Build 索引和回填进度保留。低于 05 的 header 下出现 tag 16
会拒绝，避免降版忽略凭据。gate reader 接受 03–05，Build 回填调度器也负责旧
header 升级。04 及以前的 retention planner 不认识新 header。

没有增加 ledger request/RPC 类型，也没有修改内部 service/store **V7** floor。
header 拒绝是局部格式防护，**混合二进制、滚动升级和完整部署 fencing 未验收**。
不能把这一变更描述为可直接滚动发布的兼容扩展。

## 操作的资格

同代 managed Share/Acquire 重试、Publish，以及 RetireUnpublished 现在要求
该代存在精确 `Entered` 凭据。凭据绑定原 gate、generation、不可变 descriptor /
resource closure，记录 revision 必须处于 gate 开启后、关闭前。缺失或不一致
拒绝；不从现有 Held/Published 重建。

generic legacy owner 仍可在完整 successor 覆盖下 Quiesce/Release，但保留原
V1 格式。它进入下一代后可以按新协议登记；历史查询仍保留上一代的缺口。
此前已存在的 V2 Published/Released 和排空记录仍可保留、读回，原排空接口不
凭空补全其来源。缺来源的旧 unpublished retirement 记录也不再是可验证凭据。

Cancelled 或有资格的 RetiredUnpublished 代际只能产生零 PUT 报告，非零 issued
与已知事实矛盾时，旧 RecordDrain 和原子 Finish 都拒绝，且不部分写入。
可信内部执行者的实际 scope 报告仍是原有信任前提；这不证明未知远端 PUT 已终止。

## 有界逐代核对

`enrollment::history(view, root, pending, through, after)` 在一个已建立的 metadata
snapshot 上按 **1…through.generation 的稠密代际序列**查询。每页最多 4 代，只做
点读；缺记录也占据一项，不能靠“只扫描存在的报告”跳过空洞。请求绑定 executor
token 和上界；读取更新代的 owner 时仍能查询旧上界，拒绝超过已知 generation 的
上界，以及达到或超过查询上界的 cursor。调用者可以重新读取较早的页面。

返回状态区分：缺原始准入凭据、仍 Active、缺结束记录、未进入取消、未 Publish
回收，以及实际 Drained 的原 issued/resolved。较新 generation 的成功或取消不
改变旧代状态。`issued == resolved > 0` 仍只表示报告中的响应计数，不能直接变成
部署后端已合格的终止事实。API 是 metadata 内部函数，本轮未新增 wire 查询。

每页复核当前 owner、gate 和全部资源 pin。成本取决于固定页数、资源 closure
和旧 resource base 解码上限，不随该 executor 的历史代数增长。跨页调用者必须
保留所见阻塞项；返回 EOF 不产生全局释放凭据。

## 验证与尚未完成的工作

[本轮独立证据](raw-ttl-enrollment-validation-20261005/manifest.json)记录命令、日志、
源码摘要与前序证据链接；历史证据不重写。

- 8 项专项覆盖 V1 Held 通过重试升级的反例、缺来源的旧 V2、同代 Cancel 不补证、
  新取消/进入跨代保留、十代历史中的缺口与未知 PUT、矛盾报告拒绝、破损记录和
  header 降版、13/113 代的固定点读量，以及实际 WAL 截断下 Share 与来源的原子性。
- Raft/鉴权 HTTP/2 duplex 组合通过提交 batch 构造 V4 下的 V1/V2 两类旧 Held，
  均不能重新授权或退役。新 Share 已提交后丢回执，精确重试继续原代；第一代
  `(issued=2, resolved=1)` 后取消第二代，metadata WAL 重开仍保留两个不同结论、
  原准入字节、两个旧 Held 和 Published Pending。

metadata 全套 **176 通过、1 项真实 MinIO 测试忽略**；完整 TTL 集成串行运行
**52 通过**。专项和完整套件去重后共 228 项通过，严格 workspace Clippy、格式
与 diff 检查通过。已有 Build 索引用例原先允许旧 Held Publish，本轮改为断言
拒绝，随后继续验证合法 handoff/新代取消不改变反向索引绑定；初次回归失败日志
保留。组合仍使用 InProcHub 和鉴权 duplex，未运行真实部署或新增形式化证明。

旧 Build 的实际 I/O、缺报告的崩溃遗留者以及未知 PUT 的终止/隔离仍未解决。
全局 catalog 索引和逐代来源两者也不证明历史上不存在未登记的上传者：还需要
完整 attempt 起源、全集合核对、合格的远端终止及精确负向 settlement 的组合。
负向 Pending、Reader/旧 Version、可转移恢复锚点和对象 GC 没有由本轮放行。
固定活跃数据下的物理稳态、真实时钟/网络/MinIO/进程故障和完整形式化验收继续开放。
