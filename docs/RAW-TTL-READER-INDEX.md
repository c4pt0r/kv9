# 进程 Reader 的持久反向索引与后台盘点

2026-10-05，接续 [Reader 终态回执](RAW-TTL-TERMINAL-READ-ACK.md)。负向 Pending
释放前，必须发现独立的读取依赖。现有 Build 零 PUT 闭合只覆盖写入执行者，不能证明
同镜像的 Reader 已消失。本轮补齐进程 Reader 的目录索引和有界盘点，仍不授权释放
Pending、旧 Version 或物理对象。

## 原子索引

retention ledger 升为 `KV9LED09`，保留 01–08 的读取和受检查升级路径。新增：

- tag 23：`KV9CRI01` 反向索引，绑定 root、operation 派生的分组 ID、Reader ID 和
  完整不可变 owner 注册的摘要。
- tag 24：`KV9CRP01` 全局盘点游标，保存序列、阶段、当前位置及上一页起点。
- ledger request tag 24：`AdvanceReaderIndex`，沿用小请求边界和严格规范编码。

`Acquire`、`Share`、状态变更以及 checkpoint/image 的直接结束路径均在同一批次
维护索引。首次 Share 未提交而先收到结束报告时，也原子创建 Released owner、注册、
resource links 和索引。后续 generation 使用同一不可变 binding 索引；盘点不会把
旧 Released 状态追认为实际排空证明。

索引只包含 `OwnerKind::Reader`。Pending 和 Version 派生的 Reader 保留同一 operation，
因此可以一起查询。image Reader 按自己的 operation 分组，分组 ID 不证明一个同名
Pending 存在，也不证明来源或镜像覆盖正确。调用者仍须核对完整 binding。持久
`BootstrapReader` grant 有独立的 store 关闭协议，明确不属于这个进程 Reader 索引。

## 有界补齐和恢复

盘点先扫描 owner 行，再扫描不可变注册行；每次最多处理四行，并读取一行判断是否
存在下一页。第二轮会发现只有注册、没有 owner 的损坏，不能把它当作读取依赖缺席。
每行仍核验注册及全部 resource links；这些点读受既有单 owner 资源上限约束。

请求必须匹配实际页边界、阶段和序列，游标与索引在一个 metadata 批次中提交。紧接着
重试同一页无变更确认，更旧请求冲突。扫描期间插入到游标之前的新 Reader 由登记路径
直接索引，不会漏掉。完成后新登记继续原子维护；已登记 owner 缺少索引会被拒绝，
普通重试不会悄悄重建并掩盖缺失。

查询按 operation 分页，只在全局补齐完成后开放；多页查询必须使用同一个稳定 view。
每页至多返回四个 owner，最多检查五条索引。它是该 view 的目录观察，不是后续 I/O
准入关闭证书。

生产后台沿用原有单个维护槽位：先完成 Build 盘点，再每次推进一页 Reader 盘点，
继续与每个 checkpoint 的维护轮转。没有新增独立任务或逐 attempt 全表扫描。
leader、ReadIndex、root、取消和期限检查继续生效；增加 Reader 页数、行数和目录完成
状态的统计。完成后的空转无需提交 Raft 条目。

## 格式兼容边界

08 的 Publisher generation 行继续可读，升级不会把合法 tag 22 当作旧格式污染，也
不会清除未知 PUT。已有新 Reader 索引或游标却使用旧 header 时拒绝升级；通用 planner
和经独立验证的专用 planner 在写入新 header 前使用同一检查。

旧版 ledger planner 会拒绝 09。store/service floor 仍为 V8；本轮不宣称旧二进制可以
与新请求混部，也不提供滚动升级部署资格。历史封存验证包保持原字节。

## 验证

最终八项检查全部通过：364 项测试通过，2 项真实 MinIO 测试忽略；新增 12 项测试。
完整 metadata、TTL runtime、bootstrap、outbox 与 checkpoint 恢复回归通过，
workspace 严格 Clippy、格式和差异检查通过。

新增测试覆盖有界扫描、游标后的并发登记、精确页重试、双向注册损坏、完整索引缺失、
错误 operation、代际和未进入 Reader 的结束、Pending/Version 聚合与 image 分组、
BootstrapReader 排除、旧 Publisher 未知 PUT 保留，以及 WAL 尾部截断后的原子恢复。

三 metadata Raft 副本组合测试使用鉴权 HTTP/2 duplex：明确构造 08 旧目录，丢失实际
提交后的第一页回执，切换 leader，后台从持久游标完成两轮，停止服务和 driver 后重开
真实 WAL。Held、Published、Released 的旧 Reader 状态完整保留，Pending 仍为 Published。
它不替代 NodeRuntime/TCP/MinIO、外部 Reader census 或进程断电验收。

完整命令、结果、源码摘要和开发日志见
[验证包](raw-ttl-reader-index-validation-20261005/manifest.json)。

## 仍需完成

该索引只解决进程 Reader 的目录发现。下一步仍需建立 Reader 的逐代准入与实际排空
来源证明、拒绝事实后的新读准入关闭，以及完整 BootstrapReader/Publisher census。
旧 generation、未登记进程及未知 PUT 不能由本轮记录补造证明。

负向 Pending 和旧 Version 的释放还依赖完整恢复引用、替代镜像、执行者终止和部署
边界；相关终态历史仍需安全保留。索引总量随注册历史增长，本轮只限制单次维护成本，
不宣称历史空间稳态。物理 GC、全组缺凭据冷启动、全局空间与历史有界，以及
[第二阶段](RAW-TTL-PHASE2.md) 的完整生产验收继续开放。
