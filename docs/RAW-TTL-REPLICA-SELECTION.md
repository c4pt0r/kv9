# Data checkpoint 的逐 store 选择记录

2026-10-05；接续[本地选择校验](RAW-TTL-CHECKPOINT-SELECTION.md)。本次把实际副本的
checkpoint 选择观察接入 metadata 复制，第二阶段与生产资格仍未完成。

## 实际链路

此前 data worker 成功切换本地恢复引用后，只更新内存中的 `adopted`。metadata 保存了
checkpoint 的发布与对象保护，却没有记录哪个具体 store 已实际选中该镜像。仅有
`ManifestChange` 的提交历史不能证明副本已经选择 checkpoint。

现在 worker 在完整镜像验证、Reader 排空和 `checkpoint_applied` 成功后提交
`RecordCheckpointSelection`（ledger request tag 19）。metadata leader 先做 ReadIndex，
取得精确 intent 和来源目录，再独立读取实际副本。远端使用新增的
`InspectCheckpointSelection` RPC；本地调用同一 runtime 观察函数。

来源要求实际活跃的 engine/driver、持锁的健康选择观察、精确已提交 generation 历史
与 manifest、对应 cut 的保留配置，并在返回前重新核对 owner 和选择结果。相同 group
的提交历史、另一副本已采用、仅磁盘文件可见、选择已变化或来源缺失都不能代替它。

RPC 复用 checkpoint history 的有界身份 envelope、鉴权与容量限制，但使用独立方法名
和独立返回类型。旧服务不支持选择 RPC 时保持未知，客户端不会降级成历史查询。
请求与回复绑定 root、cluster、node、store incarnation、region、predecessor 和新
challenge。仅支持 data group；单次上限 512 字节、2 秒，与历史查询共用一个实际
reader 槽位，调用取消不提前归还仍在运行的 job。metadata preflight 使用至多 1 秒。

所有远端读取都在 catalog 锁外。随后在锁内排空 metadata 当前 term、重新核对完整
endpoint 与 intent，使用同 term 提案提交。未独立验证的普通 ledger/admin planner
明确拒绝创建记录；公开请求编码本身不提供证明。

## 持久语义与界限

namespace 21 的 key 绑定 `(node, store incarnation, region)`。`KV9CSR01` 记录固定
**168 字节**，包含 root、上述身份、Pending ID、完整 intent digest、来源 endpoint
generation、记录时的 ledger revision 和整体 SHA-256。完整 descriptor、predecessor
及 SST closure 仍来自原有精确 intent，不在每个副本记录中复制。

记录和 ledger revision 通过同一个 metadata batch 写入。更新必须保持相同 root/
cluster/group，predecessor 与 image cut 前进，term/conf_ver/version 不回退。相同窗口
的不同镜像与不可比较的观察被拒绝。相同或已被较新观察覆盖的旧请求只确认既有事实，
不能把指针退回，也不声称较旧镜像曾经被本机采用。

每个 store/group 只保留最新一行，不按 checkpoint 代数增加行数。不同 store incarnation
各自保留，不能借用旧 store 的记录；历史 store/group 总量及其退役回收仍未有界。
记录必须能追溯到规范 intent、原 Pending 身份，以及完整 Published Version 和所有
resource cross-links。缺失、损坏或不匹配的链路拒绝读取和更新。pre-V7 ledger header
下出现该 namespace 时，初始化/升级不能悄悄接受它。

worker 在回执未知时保留小的精确待确认请求，后续 pass 先重试确认，不重复 GET。
成功确认后才增加采用计数；若实际选择已推进，则通过新镜像自己的提交窗口报告。
进程退出不会生成成功结论；所选恢复引用已经持久化，重建 worker 后仍需按实际选择
与提交历史重新确认。这里没有新增可丢弃对象的 outbox 报告。

记录表示“这个 store 曾经独立确认选中该镜像”，是单调历史观察，不是当前在线性或
完整恢复依赖证书。原来源不可用时可确认已有记录；没有记录不表示仍用旧镜像，也
不表示没有恢复依赖。初始安装镜像、其他恢复/Reader/migration 路径、完整副本集合、
retired/replaced store 和可转移 protocol continuation 仍要单独核验。

本次没有释放任何 Version、负向 Pending 或未知 PUT，也没有开启对象 GC。磁盘和
内部服务 floor 仍为 V7、ledger header 仍为 07；新 RPC 是可拒绝的增量能力。这些
版本号不能被未来 GC 当作完整部署与副本覆盖资格。

## 验证

- metadata 专项检查规范 request、普通 planner 拒绝、连续 17 代保持一行、旧请求
  不回退、同窗口冲突、完整 Version 保护、跨 store 绑定、checksum/合法 checksum
  下的损坏、旧 ledger header 拒绝，以及五个真实 WAL 更新尾部截断位置。
- RPC 专项检查只有 history 的 backend 不能确认 selection，鉴权/来源撤销/data
  scope 拒绝及 Unknown；沿用相同 envelope 的关联与实际 job 排空回归。compile-fail
  用例确认历史查询结果不能构造选择查询的私有返回类型。
- 实际三副本、分离 metadata/data leader 和鉴权 HTTP/2 duplex：全部副本已有发布
  历史但尚未选择时拒绝记录；错误 store、Unsupported、Unknown、目录查询后变化和
  不确定 compaction 均不能生成记录。正常自动 worker 为三个实际 store 写入观察。
- 丢失记录的真实提交回执后，worker 连续重试不新增 GET。随后移除所有 data group
  查询入口并换 metadata leader，原记录仍能确认，其他 store 不能借用它；metadata
  WAL 重开保留原事实与 Published Version。

命令、数量、日志、开发过程失败及源码摘要见
[本轮独立验证包](raw-ttl-replica-selection-validation-20261005/manifest.json)。旧验证包
保持不变。metadata 全量 206 项通过、1 项忽略，RPC 专项 5 项、私有返回类型
compile-fail 1 项、TTL 三副本回归 61 项通过；本轮去重 **273 项通过、1 项忽略**。
新增专项包含在这些集合中，错误 store 场景扩展后又单独重跑两项 runtime 专项。
严格 workspace Clippy、格式和 diff 检查通过。

本机文件恢复、InProcHub 与 HTTP/2 duplex 不能替代完整 NodeRuntime
进程故障、跨主机网络、MinIO 持久性或旧二进制资格；本次没有新的完整形式化证明。

下一步需要把这些观察与每个实际副本/安装/恢复路径的依赖、可转移继任锚点及 store
退役 fence 组合，才能判断旧 Version 何时可释放。其余 Reader/publisher 资格、
非零/未知 PUT 终止或隔离、负向 Pending、物理 GC 和容量稳态继续按完整阶段推进。
