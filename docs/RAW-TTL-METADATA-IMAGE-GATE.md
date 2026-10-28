# Metadata 同镜像别名的读准入关闭

2026-10-05，接续 [metadata 读 gate](RAW-TTL-METADATA-READ-GATE.md) 和
[bootstrap 未进入来源](RAW-TTL-BOOTSTRAP-UNENTERED.md)。原 gate 绑定单次 Pending，
同一完整 metadata manifest 的其他 Pending 或 Snapshot 来源仍能申请新 Reader。
本轮将经过全体 provisioned root store 验证的读关闭事实绑定到完整镜像摘要，
覆盖已有和将来创建的同镜像别名，同时保留已经进入的读者及持久 bootstrap 授权。

## 事实与准入

`CloseMetadataImageReads` 必须引用已完成逐 store 核对的 metadata checkpoint gate。
首次写入要求原 Pending 已 Released、精确 Version 仍 Published；单个负向 checkpoint
拒绝不足以关闭同镜像其他来源。每个 store 的选择门槛继承原协议：独立查询得到严格
更晚的实际选择，所有固定 root voter 都确认后，原 gate 才能完成。

新事实绑定 root、完整 manifest 摘要、原 Pending、规范 intent 摘要、原 gate revision
及镜像关闭 revision。事实和摘要标记同批提交；一方丢失或不匹配均拒绝读取和重试，
不会自动重建为开放状态。检查保留原 intent 和已关闭 gate，精确重试不要求旧 Version
继续 Published，因此后续 Version 覆盖转移不能重新开放镜像。

typed Reader 的 Share/Publish 和 Bootstrap 的 Begin/Publish 均核对来源 binding 的
镜像 subject，覆盖 Version、Pending、Snapshot 和 Migration 的 metadata image 来源。
新建同镜像 Pending 也继承同一事实。完整 manifest 不同的镜像仍按自身状态判断，
即使它引用相同 SST；anchor 来源及 data region 不属于本轮 gate。

已有读者保留原准入和 pins，实际 GET 排空及独立 store 退役仍由各自协议确认。
暂停 worker、超时或读关闭事实都不能替代实际结束。已有离线 bootstrap 凭据不被
撤销；本轮只关闭新的 catalog 读准入。

## 盘点与调度修复

Reader/bootstrap 盘点能够继承同镜像 fence，无须扫描所有别名或批量改写其 stamp。
原先更早的单次拒绝或读关闭边界继续有效。对于继承镜像 fence 的 Reader，旧 mutation
stamp 可以早于镜像关闭；盘点使用二者较大值作为观察边界，保留后续 stamp 回退及
缺失记录检查。晚到实际结束仍原子更新 stamp 并使已有汇总失效。

真实三副本回归还发现后台将读者盘点放在 Build 清扫完成之后。同镜像别名可能只有
catalog owner，缺少独立 ManifestChange 提交历史，Build gate 因而保持 Open，读者
盘点始终得不到执行。现在每轮完整 discovery 交替安排读取盘点优先和发布结算优先；
已继承镜像 fence 的别名即使 Build 未完成，也能推进一页 Reader/bootstrap 盘点。
每次最多提交一个盘点页，继续复用现有维护槽位、leader/read barrier、截止时间和取消
检查，只增加一个轮换布尔值。发布结算仍获得轮次，镜像 fence 不替它生成成功或拒绝。

全局 Reader/bootstrap 目录仍须完成；Reader 汇总仍只覆盖对应 Pending 的登记代际。
镜像 gate 对其他 operation 的 Snapshot/Migration 来源关闭新入口，不表示这些来源的
所有既有读者已纳入同一汇总，更不授予 Pending、Version 或对象释放权限。

## 格式与验证

ledger header 为 `KV9LED18`，request tag 30。key tag 36 的 `KV9MIG01` 固定 168 字节，
key tag 37 的 `KV9MIM01` 固定 136 字节；事实不可变，提交开销不随别名数增长。
V17 升级保留既有 bootstrap 准入、未进入来源、目录和汇总编码；新记录出现在旧
header 下时拒绝。磁盘与内部服务 floor 仍为 V8，混合旧二进制部署尚未验收。

新增七项 metadata 测试和一项真实三副本 runtime 测试，覆盖：

- 部分 store 完成不足以关闭镜像；完整 gate 覆盖旧别名、新别名和 Snapshot 读入口，
  保留已有授权及同 SST、不同完整 manifest 的独立准入。
- 继承 fence 后的活跃 Reader、未进入 Reader 和 bootstrap 授权分别按实际来源盘点，
  原先更早的 fence 保持稳定；独立结束和退役后重新汇总。
- 原 Version 覆盖转移后历史事实和精确重试仍有效；单次负向拒绝不关闭其他别名。
- V17 升级、请求编码、截断、合法 checksum 下的身份/revision 损坏、缺任一配对记录、
  缺原 gate 和 header 降级均保持拒绝，不自动补造事实。
- 13 与 113 个别名使用相同点读数量，禁止别名扫描；实际 WAL 尾部截断检验事实与
  标记同批落盘，未提交时仍开放、已提交时所有别名拒绝，重放保持一致。
- 实际 Raft/RPC 提交后丢回执、metadata 换主、独立选择查询拒绝时仍能确认原事实。
  同镜像别名上的真实 GET 暂停期间计为 active，实际结束后才计为 drained；后台仍
  保留未知发布历史和 Open Build gate。新别名读入口在对象 GET/PUT 前拒绝，旧 Version
  保持 Published，三台 store 分别通过本地授权、checkpoint 与 Raft/WAL 尾部离线重开。

最终八项检查通过：436 个不同测试通过，2 项真实 MinIO 测试忽略；完整 metadata、
TTL runtime、bootstrap、outbox、恢复测试，以及 workspace 严格 Clippy、格式和差异
检查均通过。最终命令、运行结果、开发期失败、源码哈希和增量保存在
[验证包](raw-ttl-image-read-gate-validation-20261005/manifest.json)。测试使用真实 Raft
driver、持久文件和鉴权 HTTP/2 duplex；TCP/MinIO、多进程断电及完整部署不在本轮证据内。

## 剩余工作

固定 metadata root 的新 catalog 读关闭已经覆盖同镜像别名。完整恢复依赖、可转移
恢复锚点、data group 与 joint 成员、安装/迁移/备份/替换 store、旧协议及外部读者来源
仍须逐项闭合。负向 Pending 和旧 Version 自动释放、未知 PUT 终止判断、物理 GC、
终态历史数量有界、时钟与部署/形式化验收继续属于
[第二阶段完整目标](RAW-TTL-PHASE2.md)，本增量不代表阶段完成。
