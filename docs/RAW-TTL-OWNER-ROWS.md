# Retention owner 分行存储与旧记录升级

2026-10-05 后续：ledger **07** 的[已登记集合闭合核对](RAW-TTL-REGISTERED-CLOSURE.md)保留 06 birth 和全部旧来源语义，新增有界聚合与原子变更水位。当前 ledger 已由[迁移 Publisher](RAW-TTL-MIGRATION-PUBLISHER.md)升级到 **08**，这些 checkpoint 事实保持原语义。下文格式数字保留组件引入时的历史含义。

2026-10-04；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
接续 [checkpoint 执行者登记](RAW-TTL-EXECUTOR-OWNERS.md)。第二阶段和生产验收仍开放。

## 修正的容量限制

原来的 production ledger 把同一 resource 的所有 owner（含 Released tombstone）放在
一个 `KV9PIN01` 值中。第 4097 个不同 owner 会被拒绝；即使历史执行者都已经交接，
持续重启或复用同一个 SST 也可能耗尽容量。每次变化还要重写全部历史。

现在新资源使用小的 resource header，每个 `(resource kind, instance, owner ID)`
单独保存 generation、phase 和该行的 resource revision。**生产 ledger 不再受每
resource 累计 4096 owner 的上限约束**。原有 owner descriptor、完整资源 closure、
immutable registration 和所有 forward link 仍在同一个元数据 Raft batch 中更新。
Released 行不删除、不驱逐；同一 owner 只有在上一代 Released 后才能递增 generation。

没有 legacy base 的资源只做点读。单次读写量取决于这次 owner 的 closure（最多
64 个 resource），不随已释放 owner 数量增长。每个 pin 值 154 字节，resource header
为 131 字节；带 legacy base 时 header 为 163 字节。revision/generation 溢出继续拒绝。
resource 的完整 root/kind/instance/content、owner ID 和校验和均保留在记录中。

## 格式与兼容升级

保留原命名空间 `\0kv9\0retention_v1\0` 和 registration/owner keys；ledger header
改为 `KV9LED02`，resource header 为 `KV9RES02`，pin row 为 `KV9RPO02`。新增 tag 6
存 per-resource owner rows，tag 7 存 immutable legacy base。RPC/request、owner
observation、Raft command、WAL envelope 和 checkpoint journal 的格式不变。

当前 reader 同时读取 `KV9LED01` 和 `KV9LED02`。写事务原子发布 V2 ledger header；
V1 header 下出现 V2 resource 被拒绝。旧 ledger reader 不认识 V2 header，会拒绝
继续操作。这是格式拒绝行为的组件验证，**没有完成旧二进制滚动升级/降级资格验证**，
也没有新增全成员升级确认协议。

2026-10-05 后续：当时 ledger header 推进到 `KV9LED07`，reader 接受 01–07；
resource 的分行布局保持上述语义。新增[Build 反向索引与双遍回填](RAW-TTL-BUILD-INDEX.md)
使旧 Build 可被按 checkpoint 分页枚举，同时保留原 owner 格式和 phase。
05 另加[逐代原始准入](RAW-TTL-EXECUTOR-ENROLLMENT.md)，不改 resource 分行编码。
06 接入[尝试原子创建](RAW-TTL-ATOMIC-BEGIN.md)，既有分行 pin 和历史保持原义。

`Register` 或首次改变一个 legacy resource 时，将原 `KV9PIN01` 原始字节原子移至
tag 7，再写入带原字节 SHA-256 的 V2 resource header。原始 owner、generation、
phase、revision 和退休状态均保留。新变化只覆盖对应 owner 的独立行；未变的 owner
从已验证 base 中读取。丢失覆盖行不能把已开始的新代误认成旧代，owner state 与
全部 forward link 仍需精确匹配。

这一方式避免将 64 × 4096 个旧 pin 一次扩成几十 MiB 的独立行。最多 64 个满载旧
resource 的升级批次仍小于现有 **8 MiB** 上限。原 `RetentionRecord` 的 4096 容量
约束保留为 **旧 codec 的单值边界**，不再限制生产 ledger 的后续新 owner。
旧 base 的读取仍须校验并解码最多 4096 项；本轮没有宣称它与新资源一样只需小值点读。

base 缺失或 checksum/身份不符、point row 缺失或跨对象/owner、generation/phase
不符、revision 超出 resource header，以及 legacy/headerless resource 下存在
遗留新格式行，都拒绝；不会自动重建为空记录。Register 是格式升级，不是完整历史
backfill 或全局发布者关闭证明，也不产生 retirement/物理删除权力。

## 验证与限制

[命令、日志与源码摘要](raw-ttl-owner-rows-validation-20261004/manifest.json)单独保存。

- 实际 WalEngine 对同一资源逐次提交 4097 个不同 owner 的 Share、Quiesce、Release，
  外加一个保持 Published 的后继。每次 Share 的 ledger 点读次数、返回字节、batch
  行数和字节数保持不变，测试禁止在这些操作中扫描 retention 历史。重开 WAL 后核对
  全部 Released binding，再验证新代可进入、旧代不能释放新代。
- 4096 个 Released owner 的满载真实 V1 codec 仍可增加第 4097 个 owner；迁移后的
  base 字节保持不变，旧代不能复活。另有 64 个满载 resource 的升级字节边界测试；
  该边界测试的 forward pin 是合成数据，不宣称已完成 owner backfill。
- 检查八类行/base/格式损坏和 orphan row；失败无部分 batch。实际 WAL 多处截断
  恢复只得到完整旧布局或完整新布局与 acquisition，不暴露半次升级。
- 真实三副本 metadata/data 组合以固定 V1 格式 fixture 启动 retention 状态，远端
  metadata leader 提交 Register 升级后丢失回执。原请求续做，旧 owner 进入并关闭
  generation 2；停下驱动、重开 metadata WAL 后，原 base 与精确终态仍保留。

common/meta 单元、集成及文档测试合计 **188 通过、1 忽略**；TTL 组合 **25 通过**，
其中执行者专项 **2 通过**（集合重叠，不相加）。严格 workspace Clippy、格式与 diff
检查通过。本轮新增 5 项 metadata 测试和 1 项远端组合测试。

这些记录的物理总量仍随不同 owner 增长；本轮解决单值累计容量和操作随历史增长的
成本，没有证明磁盘/RSS/FD 或 checkpoint 大小已达稳态。legacy base 后续压缩、
owner 历史的安全回收和物理债务预算继续待做。新的持久布局也不解除 48 MiB image
上限。全局发布者关闭、遗留 Build、Reader/旧 Version/负向 Pending 的安全释放和
对象 GC、TTL split/merge、时钟与真实网络/MinIO/旧二进制验收仍属于完整第二阶段。

网络组合使用带认证 HTTP/2 duplex 与 InProcHub；WAL 重开不是完整 NodeRuntime
重启或 process-kill 验收。未运行真实 TCP/MinIO，也不据此宣布生产就绪。
