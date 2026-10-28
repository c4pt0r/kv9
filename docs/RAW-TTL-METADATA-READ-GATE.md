# Metadata 旧 checkpoint 的新读准入关闭

2026-10-05，接续 [metadata 副本选择记录](RAW-TTL-METADATA-SELECTION.md)。此前只有
被独立确认拒绝的发布窗口能够关闭新 Reader。成功交接到 Version 的旧镜像仍可创建新
读者，因而无法稳定盘点其已登记 Reader。本轮加入 metadata 旧镜像的单调读准入门槛，
并复用现有逐代盘点；第二阶段完整生命周期和物理 GC 仍未完成。

## 关闭条件与分页

`AdvanceMetadataReadGate` 仅适用于 metadata checkpoint：精确 Pending 已 Released，
完整 Version 仍 Published，且没有负向拒绝事实。它不会把 Closed Build gate 当成读
关闭证明，也不会仅凭 Version 存在推断发布成功。

每页从不可变 root 的已核验前缀后检查最多四个 voter store。每个 store 必须有此前
独立核实并持久化的 metadata selection，且该记录覆盖严格更晚的发布窗口和镜像位置，
term、conf_ver、version 均不回退。只有发布历史、只选到目标镜像、另一 store 的事实、
缺失的选择或损坏的 grant/Version 交叉引用，都不能推进该页。

一个页的全部成员满足后，检查数量与序列原子提交。未完成时仍允许新读；最后一页与
`closed_at` 原子提交，才形成新读关闭事实。调用者只提供 Pending 和预期序列，无法
提交成员检查结果或跳页。紧邻的已提交页可无变更重试，下一页尚未满足也不会使这次
确认失败。未知回执、leader 变化及维护游标丢失均从持久进度继续。

已核验的前缀依赖现有 selection 的单调更新和 root/store 身份不可变约束，不在最后
一页重新扫描所有成员。目前没有删除 selection 的生产路径；后续历史回收必须保留或
转移这项证明。该前缀不是任意静默删行的全库审计。每页 selection 点读上限为四，root
证书自身仍按既有 1,024 成员解码上限校验，并不声称其解析成本与 root 长度无关。

## 新读、已有读取和恢复

最终关闭后，旧 checkpoint 的 Reader Share/Publish 和 Bootstrap Begin/Publish 都在
原准入入口检查该事实，包括尚未确认的重试。其他 checkpoint 不受此 gate 影响。
旧授权和 pin 原样保留：在途 GET 必须真实排空，Held/Published Reader 通过既有 finish
协议结束；已持久授权的 bootstrap 恢复仍遵循其本地 receipt 与独立 store 退休协议。
此 gate 不撤销既有持久授权，也不能取代 store read fence。

原 Reader closure 接受负向 refusal 或新的正向 read gate 作为读准入边界。原格式中的
`refusal_revision` 名称保留，但此时表示首次生效的读关闭 revision。负向事实仍保持
独立语义；若它晚于已有正向 read gate，不能改变已经开始的 Reader census 边界。

Reader 全局索引仍须先完成。每代保留 drained、unentered、legacy、missing、active
分类；关闭本身不会把活跃读者标记为 drained。晚到实际结束和未进入的终态与失效
stamp 原子提交，已完成的汇总会重新计算。旧 Version 始终提供完整保护。

后台复用原 checkpoint retention 维护槽位及 attempt 发现游标，每次至多提交一个
gate 页，并保持 leader、ReadIndex、取消与 deadline 检查。增加页数、等待和已关闭
观察统计，没有新增后台任务、对象 I/O 或逐 attempt 常驻 map。

## 格式和验证

ledger header 为 `KV9LED15`，request tag 28。key tag 32 的 `KV9MCG01` 为固定 152
字节，绑定 root、Pending、完整 intent digest、序列、已检查成员数、总成员数和关闭
revision。14 的 selection、bootstrap 准入和索引原样保留；旧 header 下出现新 gate
会拒绝而不修复。store/service floor 仍为 V8，没有新增混合旧二进制部署资格。

新增七项 metadata 测试和一项真实三 metadata Raft 组合测试，覆盖：

- 五成员分页，缺少最后一个 store 或仅选到旧镜像时保持可读；只有最后一页阻止新读。
- 新旧 Reader、Held/Published 重试与 bootstrap 准入，保留已有 pin 和 Version。
- 正向 gate 接入逐代盘点，实际晚到结束和 Unentered 终态使汇总失效；晚到负向事实
  不移动已生效的关闭边界。
- 请求截断、跳序列、合法 checksum 下的记录损坏、header 降级和 14 格式升级保留。
- 13 与 113 个 root store 的每页 selection 点读均为四，禁止 retention 全表扫描。
- 部分页和最终页的真实 WAL 尾部截断与精确重试，关闭状态和新读拒绝同生效。
- 实际三副本选择与后台发现、鉴权 HTTP/2 丢失已提交 gate 回执、换主重试；旧 GET
  已超时但实际任务仍暂停时保持 active，真实排空后才结束。关闭后的新读在对象 I/O
  前拒绝。全组三 store 停机后，均从新 checkpoint、各自授权及 WAL/Raft 尾部离线重开，
  保留旧 gate、Reader 终态和 Published Version。

真实网络/MinIO/多进程断电与完整生产 NodeRuntime 部署不在上述 fixture 证据范围。
开发时出现的测试类型适配错误、leader 本地调用绕过 RPC 故障注入，以及用普通 WAL
open 误读已压缩 checkpoint 的失败日志均保留；没有为通过测试放宽生产检查。

最终八项检查通过：412 个不同测试通过，2 项真实 MinIO 测试忽略。完整 metadata、TTL
runtime、bootstrap、drain outbox、checkpoint 恢复、workspace 严格 Clippy、格式和差异
检查通过。完整命令、结果和源码摘要见
[验证包](raw-ttl-read-admission-validation-20261005/manifest.json)。

## 剩余工作

该 gate 只覆盖现有 provisioned metadata root store 协议。data group 的实际配置、joint
成员、安装/迁移/备份/替换 store、既有 bootstrap 授权和未知来源不能据此判为空。
全体恢复依赖与可转移锚点、全部 Reader/Publisher 历史与部署来源、旧 Version 和负向
Pending 释放、未知 PUT 终止或隔离、历史总量有界、物理 GC、时钟与部署/形式化验收仍
按[第二阶段完整范围](RAW-TTL-PHASE2.md)推进。
