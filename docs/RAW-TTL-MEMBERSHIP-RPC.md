# DATA 历史成员的独立远端查询

日期：2026-10-06。接续[本地持久成员历史](RAW-TTL-MEMBERSHIP-HISTORY.md)。
本增量把本地配置观察接入独立鉴权 RPC，并由 metadata leader 核对来源 store 与
endpoint。它为后续完整 DATA 来源盘点提供查询路径；不授予旧镜像关闭、保留对象
释放或 GC 权限，第二阶段和生产验收保持开放。

## 查询契约

新增内部 `InspectDataMembership` 方法，使用当前 V11 服务路径。它只读，不变更
磁盘 floor 或 KV9LED22。冻结的先前 V11 协议仍支持格式确认，但没有此方法；
新客户端收到不支持时保留未确认结果，不回退到配置、selection 或普通历史接口。
这不是新旧完整二进制混部或滚动升级资格。

请求绑定 root、cluster、node、store incarnation、region、creation、随机 challenge、
剩余预算、协议版本、cursor 与已见条目数。服务端读前、读后都确认实际 StoreGuard
格式身份和接收资格。响应回显完整请求，独立编码观察页或明确 Unknown。
请求/响应最大 1024 字节，单 RPC 预算最多 2 秒；消息解码另受既有服务上限限制。

规范 cursor 固定 48 字节，页最多 112 字节、四个节点。解码核对角色、排序、
snapshot 缺口、总数和规范表示；续页绑定原摘要及累计条目数，终页必须凑齐已报告
数量。历史变化使旧 cursor 失效；新的续页使用新的 challenge。Unknown 不能变成
普通分页结束，终页也不代表没有 snapshot 祖先缺口或全局来源完整。

## 实际来源与资源持有

RuntimeBackend 只读取实际拥有的 data driver/engine。它从真实 RANGE 或 staging
IMPORT 核对组的创建身份，读取 peer 的本地持久成员历史，并在返回前复核 owner
和 scope。没有 handle 返回 Unknown，不重新打开可能仍被使用的 Raft 文件。
故障 writer/peer、错误创建身份或过期预算拒绝观察。

调用者显式提供预期 StoreIdentity。metadata leader 先经 ReadIndex 读取当前 endpoint，
核对精确 store，再发起独立鉴权查询；回复后再次 ReadIndex，比较完整 endpoint，
包括 generation。当前 endpoint 的新 incarnation 不能代替调用者指定的历史 store。
这里只确认回答者的来源，不能把页面中的历史 node ID 解释成它今天的磁盘身份。

嵌入式 `NodeRuntime::inspect_data_membership` 提供一次分页查询，外层总预算最多
30 秒。派发前获取既有 retention 容量；permit 移交实际 blocking worker，外层取消
或超时不提前释放。服务端复用 checkpoint 历史/读下限的读取容量，名额同样保留
到真正的后台查询退出。不新增无限队列，也不在这一步写 metadata ledger。

## 验证

新测试覆盖：规范编码及损坏/跳页拒绝；认证与全部请求字段相关性；显式 Unknown、
坏页和终页计数；超时、取消后的实际配额保留和结束前权限撤销；真实 HTTP/2 上
旧 V11 dispatcher 拒绝新方法且无 fallback，以及当前方法的正常分页。

真实三副本 raft-rs、磁盘 WAL/Raft 与认证 HTTP/2 duplex 组合测试由 metadata
leader 2 查询 data follower 3。实际加入再移除 learner 后，跨页保留其历史角色；
新配置使续页失效，endpoint generation 变化拒绝本次结果，错误 incarnation/creation
被拒绝，owner 消失保持 Unknown。三份磁盘日志重开保留相同观察，查询不产生
retention 提交。测试中的附加 learner ID 没有对应第四台实际 store。

本轮新增 **8 项测试**，相关回归 **187 项通过**，其中四线程完整 TTL runtime
**102/102**。含 write-stage-tracing/checkpoint-testing 的 workspace 全 target 严格
Clippy、格式和 diff 检查通过；本次选择的测试中没有 ignored。

结果、命令与源码摘要见[独立证据包](raw-ttl-membership-rpc-validation-20261006/manifest.json)。
本轮没有重新运行实际 TCP/MinIO、多进程 kill、旧 binary、跨主机时钟、Chaos 或 TLC。
历史失败和验证包保留原样；这次回归不能确定早前未知交接错误的唯一原因，也不代表
持续并发负载已通过生产资格。

## 后续完整范围

还需把本地实际配置观察与持久 placement 来源交叉核对，覆盖所有历史磁盘实例、
joint/replacement/removed 成员、snapshot 来源祖先以及安装、迁移、备份和恢复引用。
需建立可恢复、有界、不会因来源变化误判完成的持久盘点与全 store 关闭协议；失联
旧 store、缺失历史或没有可转移恢复锚点时必须继续保留保护。

完整 Publisher/Reader 生命周期、unknown PUT 隔离、负向 Pending/旧 Version 安全
释放、物理 GC、历史总量/资源稳态、merge、部署时钟和完整故障/形式化验收继续按
[生产范围](RAW-KV-PRODUCTION-SCOPE.md)和[第二阶段](RAW-TTL-PHASE2.md)推进。
