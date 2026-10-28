# Bootstrap 未进入终态的独立来源

2026-10-05，接续[镜像授权汇总](RAW-TTL-BOOTSTRAP-CLOSURE.md)。此前，prospective
本地凭据可能已持久化，但 Begin 尚未提交。独立 store fence 可以终结这个授权，
catalog 却只保存普通 Released owner，无法区分它与缺少来源的旧历史。本轮在首次
退役时保存可核验的未进入来源，并接入有界盘点；不补发旧记录的历史证明。

## 首次退役与准入

仍使用既有 `RetireBootstrapRead` 协议：metadata leader 经独立 store 查询核对精确
Pending、镜像和持久读 fence，确认 store 已 Serving、实际 bootstrap GET 已排空且
StoreGuard 仍持有，再在 catalog 锁内重检 endpoint。普通 ledger 请求不能自行声明
这些事实，客户端取消或超时也不能替代它们。

只有同一 catalog view 中完全不存在该 grant 的 owner/registration、typed admission、
未进入事实、反向索引及资源 pin，且它属于 provisioned metadata root voter，才创建
新的未进入记录。来源绑定精确 Pending/Version token、checkpoint operation、node、
store、完整 binding digest 和独立退役 revision。首次退役继续要求精确 Published
Version 提供完整镜像保护。

事实与 Released owner、不可变 registration、每项资源的终态 pin、反向索引及镜像
mutation stamp 原子提交。owner 使用独立编码，缺失事实时不能被解释成旧记录。
未进入证明没有 `entered_at`，不构成 admission，也不授权 GET、Begin 或 Publish。

已有 Held/Published grant 按原准入来源退休；即使 Begin 的回执丢失，也不能被改算为
未进入。旧 Held/Published/Released、缺来源和未知历史保持原分类，不会在重试时获得
新证明。精确终态确认继续不依赖旧 intent 或 Version 仍然 Published，不重复独立
查询、不改写首次 revision。延迟到达的 Begin/Publish 不能使 generation 1 复活。

本轮还修复了新的 Begin 在 owner 丢失、反向索引仍在时可能重建 owner 的缺口。
新准入和首次退役现在均拒绝残留 registration、admission、未进入事实、索引或 pin。
这是拒绝不完整历史，不能自动删除残留或修补成“从未进入”。针对该缺口的修改前
失败和修改后成功日志均保留。

## 目录、汇总与格式

ledger header 为 `KV9LED17`。key tag 35 的 `KV9BUT01` 为固定 216 字节来源事实，
`KV9BUO01` 区分其 owner/registration。既有 entered admission 与本地双槽 receipt
格式不变，服务和磁盘 floor 仍为 V8；混合旧二进制部署继续未验收。

bootstrap 全局目录增加第四轮扫描：owner、registration、admission、未进入事实。
每页最多四条，加一个 lookahead。进度使用 `KV9BRP02`；原 `KV9BRP01` 的已完成
三轮进度在升级后继续第四轮，不直接视为新目录已完整。原有 owner 缺索引检查在
第四轮完成前仍生效；新事实即使插入在游标之前，也与索引同批落盘。新事实或 V2
进度出现在旧 header 下时拒绝，升级不重写原准入、gate 或镜像汇总。

镜像汇总同时读取 entered admission 与独立未进入事实，二者互斥。两类来源均核对
保留的规范 source intent、完整镜像、Version、operation 和 provisioned store。
未进入终态可以晚于读 gate：它不是一次新读准入；它会原子使此前的空汇总失效，
随后作为已知 retired 计数。缺少两类来源的 Released 继续算 legacy/未知。

原 `KV9BCL01` 汇总和 image stamp 编码保持不变。每页工作继续有界，复用原后台
槽位、持久游标、取消和 leader/read barrier 约束，没有新增对象 I/O 或后台任务。

## 验证

新增八项 metadata 测试与一项真实三 metadata Raft 组合测试，覆盖：

- 独立首次退役的完整身份、无新读权限、精确重试、不影响 Published Version。
- 旧 Held/Published/Released 不补造来源；registration/admission/index/pin/fact
  任一残片均阻止首次退役及重新准入。
- 请求入口权限、记录截断、合法 checksum 下的身份/revision 损坏、缺失事实、
  owner 编码不一致及 header 降级均拒绝，且不修改原有字节。
- V16 已完成目录升级后补做第四轮；孤立的未进入事实阻止目录完成。
- 第四轮在 13 和 113 条记录时均只读取五个索引行，点读数量相同；禁止全表 scan。
- gate 之后的首次未进入退役使空汇总失效；真实 WAL 尾部截断覆盖事实、owner、
  pin、index 和 stamp 的原子性。原目录 WAL 用例也扩展到包含非空第四轮。
- 实际 RPC 准入拒绝与提交后回执丢失产生不同来源；本地 prospective 凭据已保存
  时仍须独立 fence，查询拒绝不能生成事实。退役提交后丢回执、换主重放保持原事实。
  所有 store 选择新镜像并完成目录/汇总后，三台 store 分别从新 checkpoint、本地
  授权和 Raft/WAL 尾部离线重开，保留两类来源及旧 Published Version。

真实 Raft driver、持久文件与鉴权 HTTP/2 duplex 提供组件证据；真实 TCP/MinIO、多
进程断电及完整 NodeRuntime 部署仍未验收。最终八项检查通过：428 个不同测试通过，
2 项真实 MinIO 测试忽略；完整 metadata/TTL runtime、bootstrap、outbox、恢复测试、
workspace 严格 Clippy、格式和差异检查均通过。完整命令、开发失败记录、源码哈希与
增量保存在[验证包](raw-ttl-bootstrap-unentered-validation-20261005/manifest.json)。

## 下一步

已知终态来源补齐不等于完整回收资格。同镜像其他 Pending 的未来准入仍需关闭，
所有 store 的恢复引用和可转移恢复锚点仍需闭合。旧历史、外部读者、data group/joint
成员、安装/迁移/备份/替换 store 以及全部 Reader/Publisher 的部署来源继续保留。
没有启用旧 Version/负向 Pending 自动释放、未知 PUT 终止判断或物理 GC；历史总量
有界、时钟和部署/形式化验收仍属于[第二阶段完整目标](RAW-TTL-PHASE2.md)。
