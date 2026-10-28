# Metadata 镜像的已登记 bootstrap grant 汇总

2026-10-05，接续 [metadata 读准入关闭](RAW-TTL-METADATA-READ-GATE.md) 与
[持久 bootstrap 目录](RAW-TTL-BOOTSTRAP-INDEX.md)。新增每个镜像的有界汇总，区分
已独立退役、仍然活跃和缺少来源的 grant。同一镜像可能对应多个 Pending；汇总必须
包含它们的全部已登记授权，不能只查询正在关闭的 Pending。

## 资格与计数

开始前要求精确 metadata intent、既有保护关系、生效的 read fence，以及完整的
bootstrap 反向目录。read fence 沿用正向 metadata read gate 或既有负向 refusal 的
首次生效边界；data group 不使用此汇总。每页至多读取四个 grant，另有一个索引
lookahead 判断是否还有下一页。请求只携带 Pending 和预期序列。

有 typed admission 的 grant 必须匹配完整镜像、保留的规范 source intent、精确
Pending/Version token、checkpoint operation、store 身份和 provisioned root voter。
目标 Pending 的 admission 必须早于其 read fence。其他 Pending 的同镜像授权纳入
相同计数，并单独记录 `other_sources`；它不是可以忽略的额外读者。

`Held`/`Published` 计为 active。typed grant 的 `Released` 计为 retired，依赖既有
独立 store fence 退役协议和禁止通用释放的约束；汇总本身不执行退役。已经退休的
其他 Pending grant 使用保留的规范 intent 核对历史身份，其原 Version 后续转移不会
把历史授权变成无法读取的记录。目标 checkpoint 的完整 Version 保护仍然保留。

缺 admission 的记录一律计为 legacy/未知，即使 owner 已 Released；这也包括目前
未进入 typed Begin、但已通过独立退役产生的终态。此轮没有补造它们的来源或未进入
证明。`retired + active + legacy == grants`，且 `other_sources` 是 typed 数量的子集。

## 原子失效与后台恢复

同镜像的所有 Pending 共用一个 mutation stamp。bootstrap owner、registration、
admission 和反向索引的变更，与 stamp 在同一 ledger batch 提交；只检查本次有界
写集合。新授权、发布、退役、旧记录索引补齐都会使旧汇总失效，包括其他 Pending
在目标 gate 关闭以后新建的同镜像授权。精确无变更重试不增加 stamp。

汇总保存来源 revision、read fence revision、序列和最后处理的 owner。来源变化时
从该镜像开头重新计数，序列继续递增。紧邻的丢失页面回执可幂等确认，但不会消除
后续变更造成的 restart。已完成汇总必须与当前 stamp 一致才返回 Complete。

缺少 stamp 但仍有该镜像汇总时，用一次前缀 lookahead 拒绝，不能把丢失记录当成
初次运行并修补。损坏身份、回退 revision、非法计数或序列也拒绝。该检查不是对任意
静默删行的全库审计；后续历史回收必须保留或转移索引、准入和失效事实。

后台复用现有 maintenance 槽位和 Pending 发现游标；Build/Reader 页与 bootstrap 页
各自占用一次 pass，保留 leader、read barrier、root、deadline 和取消检查。新增
pages/restarts/retired/blocked 统计，没有新增后台任务、对象 I/O 或逐 attempt 常驻 map。

## 格式与验证

ledger header 升为 `KV9LED16`，request tag 29。key tag 33 的 `KV9BCM01` 是 112
字节镜像 stamp；tag 34 的 `KV9BCL01` 按 image/Pending 保存汇总，编码上限 224
字节（当前最多 210 字节）。15 的 gate、选择、准入及索引不重写；旧 header 携带
新记录时拒绝升级。旧格式测试只移除历史上不存在的新增记录，生产检查保持严格。
store/service floor 仍为 V8，不构成混合旧二进制部署资格。

新增六项 metadata 测试和一项真实三 metadata Raft 组合测试：

- 只有独立退役才能从 active 变为 retired；晚到退役使已完成汇总重新计算。
- 另一个 Pending 在目标 gate 关闭后准入同镜像 grant，仍使汇总失效；其历史来源
  Version 转移后，已退休 grant 的身份仍可核对。
- 旧 Released 与缺 origin 的未进入终态保持未知，不被统计为已知退役。
- 请求截断、序列重试/跳跃、generation 错误、损坏记录、降级拒绝与 V15 升级保留。
- 13 与 113 个不同来源的同镜像 grant，每页均只访问五个索引行且点读数量相同，
  禁止 retention 全表扫描；不声称 manifest 解码成本与镜像描述大小无关。
- 真实 WAL 页提交和晚到退役的尾部截断，保证 owner、stamp 与进度原子恢复。
- 实际 store selector 迁移、独立退役 RPC 拒绝、后台发现、鉴权 HTTP/2 提交后丢
  回执、换主重试和恢复退役；三台 store 全停后，分别从新 checkpoint、本地授权
  和 Raft/WAL 尾部离线重开，汇总一致且旧 Version 仍 Published。控制面重试和汇总
  不增加 GET/PUT；离线恢复仍读取其已授权的新镜像。

组合测试使用真实 Raft driver、持久本地文件和进程内 HTTP/2；真实 TCP/MinIO、
多进程断电和完整 NodeRuntime 部署不在该 fixture 的证明范围。
最终九项检查通过：419 个不同测试通过，2 项真实 MinIO 测试忽略。检查包含完整
metadata、TTL runtime、bootstrap、drain outbox、checkpoint 恢复、最终等待条件的
定向复验，以及 workspace 严格 Clippy、格式和差异检查。第一次 runtime 全套检查
发现旧 V12 测试夹具残留新增 stamp，被严格格式检查拒绝；已修正夹具并重跑全套，
原失败日志保留。最终命令、结果、源码与增量见
[验证包](raw-ttl-bootstrap-closure-validation-20261005/manifest.json)。

## 后续边界

`registered_grants_retired()` 只描述当前精确镜像目录内已登记 grant；同镜像其他
Pending 仍可能获得新准入，并使该结果失效。它不证明所有未来准入关闭、没有外部
读者、旧协议历史合格或完整恢复依赖已转移。没有增加 Version/负向 Pending 自动
释放，也没有启用物理 GC。

下一步需要为未进入的 bootstrap 终态保存独立且可核验的来源，继续闭合所有同镜像
来源的未来准入及完整 store/恢复依赖。data group 的实际配置、joint 成员、安装、迁移、
备份与替换 store、可转移恢复锚点、全部 Reader/Publisher 与部署历史、未知 PUT
隔离、历史总量有界、物理 GC、时钟和部署/形式化验收继续保留在
[第二阶段完整范围](RAW-TTL-PHASE2.md)中。
