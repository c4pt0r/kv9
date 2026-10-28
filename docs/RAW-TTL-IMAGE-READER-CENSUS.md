# Metadata 镜像的跨来源 Reader 盘点

2026-10-05，接续[同镜像读关闭](RAW-TTL-METADATA-IMAGE-GATE.md)。关闭新读入口之后，
按 checkpoint operation 盘点仍会漏掉同一镜像上其他 Snapshot/Migration operation 的
既有 Reader。本轮增加按 region 和完整镜像摘要归集的目录，并将 metadata 镜像上所有
已登记 Reader 的连续代际纳入持久、有界的盘点。

## 目录与来源

所有非 anchor 的 per-process Reader 都以 `(region, image, owner)` 登记反向索引。
索引绑定 root、完整镜像 subject、owner 和规范 registration 摘要；不同 operation、
不同 Pending 别名不再分开。region 不同、完整 manifest 不同及 anchor 来源不会混入。
持久 BootstrapReader 继续使用原有独立目录。

新 Reader 的 owner、registration、代际事实、资源 pin、原 operation 索引和镜像索引
同批提交。旧 catalog 通过 owner 与 immutable registration 两轮扫描补齐，每页最多
四条并读取一个 lookahead；第二轮发现孤立 registration 时拒绝完成。新增 Reader
即使排序在游标之前也会原子登记。目录完成后，已知 owner 缺索引会阻止其生命周期
变更，不能通过重试重建为新的授权。

metadata 镜像的 mutation stamp 同时保存已登记 owner 数量。每个新镜像索引仅增加
一次计数；复用同一 owner 的新 generation 不增加 owner 数量。索引、数量和变更 revision
原子提交，旧格式回填的第二轮不会重复计数。数量丢失时，已有汇总或一个有界索引
lookahead 会阻止重建；扫描到结尾时必须核对累计 owner 数量。

数量检查来自本轮复现的缺口：若只扫描索引而不核对独立数量，丢失的索引行可能把
活跃 Reader 变成“空集合”。修改前失败与修改后通过的日志均保留。该检查与原子 WAL
记录结合使用，不声称能识别任意多处、相互一致的历史伪造或外部未知读者。

## 镜像级连续代际盘点

盘点必须具备已关闭的 metadata image gate，以及完成的镜像目录。一个镜像只有一个
进度，所有 Pending 别名读到相同结果。每次最多选择一个 Reader、检查四个连续
generation；保留当前 owner、generation 游标和累计计数，不在内存保存全镜像集合。
目录页至多核对四条及一个 lookahead，13 和 113 个 owner/代际的单页工作量相同。

每一代区分已实际排空、确认未进入、活跃、legacy 和缺失。不同 operation 的 Reader
从自身实际来源和连续代际事实核对，不能借另一个 checkpoint 的完成结果。legacy
Released 或历史缺口仍阻塞。原 Version 后续转移不重新生成读权限，也不改变已保存
的完整镜像身份和已结束事实。

Reader 状态、registration、generation 和目录回填都原子更新同一镜像 stamp。
晚到实际结束或 gate 之后确认未进入，会使旧汇总失效并从头盘点；已持有的稳定
ReadView 仍得到原有观察。镜像 gate 不替代 GET 实际排空。

后台沿用现有 checkpoint retention 维护槽位。全局目录在既有 Build/Reader/bootstrap
目录之后推进；镜像盘点在既有读取盘点任务中推进，一次最多提交一页。复用 leader、
read barrier、root、截止时间、取消与 discovery 轮换，无新增独立任务或 per-image
内存表。原按 operation 的盘点继续保留其用途。

## 格式与验证

ledger header 为 `KV9LED19`；request tag 31 推进镜像目录，tag 32 推进镜像盘点。
key tag 38 为 `KV9IRI01` 索引，固定 160 字节；39 为 `KV9IRP01` 全局目录进度，
最多 116 字节；40 为 `KV9ICM01` stamp/owner 数量，固定 120 字节；41 为
`KV9ICL01` 盘点进度，解码上限 272 字节。V18 及此前格式升级保留原有记录，新
namespace 出现在旧 header 下时拒绝。磁盘和内部服务 floor 仍为 V8；混合旧二进制
部署尚未验收。

新增八项 metadata 测试和一项真实三副本 runtime 测试，覆盖：

- 多个 Pending 别名、独立 Snapshot/Migration operation 的活跃代际汇入同一结果；
  同 SST、不同完整 manifest 的读者保持独立；晚到排空和未进入终态使汇总重新计算。
- legacy 与缺失代际保持阻塞，原 Version 转移不重开镜像或破坏历史盘点。
- V18 有界回填保留旧字节，registration 阶段发现丢失 owner；目录完整后，缺反向
  链接的 owner 不能通过结束请求自动修复；缺数量也不能被当作首次登记。
- 回填页面拒绝跳过 owner；排序在当前游标之前的晚到未进入终态仍被原子登记，
  第二轮扫描及精确重试均不会重复计数。
- 请求编解码、截断、合法 checksum 下的 identity/revision 损坏、缺 stamp 及旧
  header 拒绝；13/113 owner 与 13/113 连续代际的单页点读与扫描数量相同。
- 真实 WAL 尾部截断覆盖目录计数、盘点页和晚到结束失效，恢复只能看到完整的
  提交前或提交后状态，重试不重复累加。
- 实际保留读取适配器暂停一个独立 Snapshot 来源的 GET：旧 checkpoint operation
  盘点已闭合时，镜像盘点仍保留 active。盘点提交后丢回执、换主后重试，实际结束
  后才得到 drained；gate 拒绝的新请求产生未进入终态，没有对象 GET/PUT。三个
  store 从新 checkpoint、持久授权和 Raft/WAL 尾部离线重开，保留同一完整汇总，
  Snapshot 来源与旧 Version 继续 Published。

最终九项检查通过，去重后 445 个测试通过，2 项真实 MinIO 测试忽略。完整 metadata、
TTL runtime、bootstrap、outbox、恢复回归和补充镜像专项，以及 workspace 严格 Clippy、
格式与差异检查均通过。完整检查、开发期失败、源码摘要与增量保存在
[独立验证包](raw-ttl-image-reader-census-validation-20261005/manifest.json)。组件使用
真实 Raft、磁盘 WAL 和鉴权 HTTP/2 duplex；TCP/MinIO、多进程断电和完整部署没有
被这组证据替代。

## 继续开放的要求

本轮完成的是 catalog 内非 anchor 的 metadata image Reader 盘点。完整 store/恢复
依赖、可转移恢复锚点、data/joint 成员、外部/旧部署来源、全部 Publisher、安装/
迁移/备份/替换 store 仍须组合核验。持久 bootstrap 退役与此处 per-process 排空保持
独立。负向 Pending/旧 Version 自动释放、未知 PUT 终止、物理 GC、终态历史数量
有界、时钟和部署/形式化验收继续属于[完整第二阶段](RAW-TTL-PHASE2.md)。
