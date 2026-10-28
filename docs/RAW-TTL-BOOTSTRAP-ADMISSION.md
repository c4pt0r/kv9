# BootstrapReader 的 catalog 准入约束

2026-10-05，接续 [已登记 Reader 连续代际闭合](RAW-TTL-READER-CLOSURE.md)。审查发现，
持久 bootstrap grant 的精确 Published Version 检查原先只在启动/选择调用方；catalog
仍允许通用 Acquire/Share 创建 BootstrapReader。该路径现在由 catalog 拒绝，新授权
必须提交 `BeginBootstrapRead`，再执行 Publish。

## 来源与身份

请求只携带精确 Pending token、节点和 store incarnation。catalog 从已保存的 canonical
checkpoint intent 重建完整 Version，要求它仍为 Published，再派生完整 metadata grant。
调用者不能提供另一份 owner、operation 或资源集合。Pending generation 必须为 1；当前
支持 root descriptor 中预置的 metadata voter 和其 provisioned store。joiner 的 root-bound
本地身份不等于 metadata membership，不能据此登记。动态 metadata 成员、store replacement
与可搬迁恢复资格仍需独立实现。

登记把 Held owner、不可变注册、所有资源 pin 和来源记录原子写入。新来源记录保存 root、
Pending、Version、节点、store、原 checkpoint operation、完整 binding 摘要及 admission
revision。Pending/Version ID 均由保存的 operation 重算验证；bootstrap ID/operation 由
Pending 和 store 重算验证。记录为固定 216 字节，查询使用有界点读。

Publish 重新读取来源 intent 和完整 Published Version，并检查精确拒绝事实；登记重试也
必须重新验证来源。来源已转移、缺失或失效时，既有 Held/Published grant 保留保护，但
不能凭一次旧成功观察重新取得在线授权。只有精确同身份的有效重试才能无变更确认。

## 持久化与兼容

ledger header 升为 `KV9LED12`；request tag 26 是 `BeginBootstrapRead`，key tag 28 保存
`KV9BRA01` 来源记录。新 owner/registration 使用 `KV9BRO01`，因此删除来源行、仅改回
普通 owner 编码、留下孤立来源记录或降低 header 都不能退回“旧记录”解释。216 字节
是新增来源记录上限，不是完整 owner/资源批次上限；批次仍受既有 8 MiB 限制。

11 格式的 Reader 索引、代际、已完成汇总及变更标记原样升级。旧 BootstrapReader 可被
查询，仍保留其原有 pin，也能经独立验证的 durable store read fence 退休。旧 grant 无法
通过重试被补造成新协议准入；其已有本地授权 receipt 仍支持离线启动。旧 Held 或没有
授权 receipt 的旧 Published grant 不能自动升级为可读授权，这一兼容边界明确保持关闭。

正常启动和 checkpoint 选择仍先持久保存 prospective slot，再登记、Publish、核对回执，
最后保存 authorized receipt。两次网络提交的丢响应边界不变。退休保留新来源记录与专用
owner 编码；已提交终态的重试无需重新取得旧 Version/intent，也不再产生多余修订。
未曾登记的 prospective grant 仍可由独立 store fence 建立不可复活的终态，不补造准入。

store/service floor 仍为 V8。新 header 会使旧 planner 拒绝，混合旧二进制部署和完整滚动
升级资格未被本轮认证。

## 验证

新增八项测试覆盖通用入口绕过、Pending/store/代际不匹配、来源转移前后重试、旧记录
兼容、来源/owner 损坏与降级、孤立记录拒绝、终态回执脱离来源历史、真实 WAL 尾部截断，
以及 11 格式完整 Reader 历史和已完成汇总的无损升级。

扩展现有三 metadata Raft + 鉴权 HTTP/2 duplex 启动组合测试：在线拒绝 Acquire、来自
Version/Pending 的通用 Share；目标副本停止后通过其余 quorum 登记，依次丢失已提交的
登记和 Publish 回执，中间切换 leader；确认授权前不发生 GET。来源记录跨 leader 和
真实 WAL 重开保持不变，所有服务停止后仍可重复离线恢复。本轮未增加对象删除路径。

最终八项检查通过：388 项测试通过，2 项真实 MinIO 测试忽略；完整 metadata、TTL
runtime、bootstrap、outbox、checkpoint 恢复、workspace 严格 Clippy、格式及差异检查
通过。开发中发现并修复了终态退休重试多写一次修订的问题；相应失败与编译错误日志
均保留。完整检查结果和源码摘要见
[验证包](raw-ttl-bootstrap-admission-validation-20261005/manifest.json)。这组运行不等于
TCP/MinIO、多进程断电、混合二进制或完整 NodeRuntime 的生产部署验收。

## 后续范围

新记录证明一次 catalog 准入的精确来源；它不是远端 prospective receipt 已落盘的独立
证明，也不是所有 store 的读者全集。下一步需要 BootstrapReader 的有界反向索引/旧记录
盘点，以及完整恢复依赖和 store 生命周期资格，才能把这些记录接入旧 Version 的释放
判定。现有进程 Reader 汇总仍不包含持久 bootstrap grant。

负向 Pending 自动释放、不同新镜像替代旧 Version、未知 PUT 的远端终止资格、终态历史
总量有界、物理 GC、全组缺凭据恢复、时钟与部署验收仍开放。
[第二阶段完整范围](RAW-TTL-PHASE2.md)保持不变，尚未达到生产验收。
