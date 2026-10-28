# Checkpoint owner 跨节点登记与恢复组件

2026-10-04；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
本次接续[恢复证据保护](RAW-TTL-CHECKPOINT-HISTORY.md)，补上 data leader 与 metadata
leader 分居不同节点时的对象登记通道。[第二阶段](RAW-TTL-PHASE2.md)仍未完成。

## 已接入的路径

内部 V4 服务新增 `CheckpointRetention`，使用现有节点认证和已授权 replica 的 root
校验。请求明确区分规范 ledger command 与 owner 查询；响应绑定 root、操作类型和
完整请求内容的摘要。写入响应携带实际 applied term/index、ledger revision 和 changed；
重复确认使用新的 apply 回执，不能宣称是上次丢失的回执。查询区分 absent 与非法空记录，
客户端还用 metadata codec 核对返回 owner 的 root、ID、checksum、binding 和阶段。
旧服务缺少该方法时拒绝，不降级为无 owner 的对象上传。

Runtime 内部请求和原公开 retention API 共用同一套 planner、catalog 事务锁及提交代码：
先用实际 Noop 回执排空之前结果未知的 proposal，再取一个 snapshot 规划，最终追加受
相同 leader term 限制；owner 读取经过 ReadIndex。内部请求在追加前核对 root 和规范
请求，catalog 锁忙时立即拒绝。公开 API 保留原锁行为，两个提交阶段现在共用 20 秒
预算；内部调用共用它自己的更短预算。超时后的 apply 结果不会返回成功。

`RoutedLedger` 是 checkpoint 专用接口，只持有 backend 弱引用及现有 Tokio handle。
本机 metadata leader 走相同本地代码；否则根据已应用目录和初始节点地址寻找 metadata
leader。连接建立失败表示尚未发送 RPC，可以检查下一个地址；发送之后，只有明确的
`NotLeader` 拒绝允许继续路由。超时、失联、非法响应、未知 backend 错误都终止此次调用。
下一轮从持久 journal 和真实 owner 状态续做同一个操作，不能因错误创建新 checkpoint
身份，也不能在 owner 未确认时进行 PUT/GET。

现有 metadata checkpoint worker 已改用该路由接口。因此它在失去 metadata leadership
之后也能查询远端 owner 并续做原有 handoff。共享 `CheckpointOwners` 支持 metadata
region 与 data region（ID >= 100），仍将规范 manifest、实际 predecessor generation、
root 和完整 SST closure 绑定到确定性的 Pending/Version owner；解码本身不是发布权限。
后续已接入[data-group 自动调度器](RAW-TTL-DATA-CHECKPOINT.md)；下述日志仍是本轮
显式驱动 fixture 的历史证据，不扩大为自动调度验收。

## 资源和生命周期界限

- 每个内部服务最多两个正在执行的 ledger blocking job；RPC 取消后占位保留到实际
  job 退出。节点 checkpoint 客户端也共用两个占位，不为每个 group 创建运行时或线程。
- 生产路由单次调用总预算 2 秒，包含选路、连接、RPC 和提交等待；每个地址连接最多
  250 毫秒。接收端只接受 1 微秒至 30 秒预算，并在 backend 调用前后检查剩余时间。
- 应用层 command 上限 16 KiB、owner observation 上限 8 KiB；客户端还有对应的
  protobuf 消息上限。服务端与 Raft stream 共用现有传输层消息上限，本次没有宣称
  protobuf 解码前的分配只占 16 KiB。
- 本次 owner handoff 含多次 ledger 调用；后续 data checkpoint 引入控制流程总预算，
  但不抢占同步对象 I/O，metadata worker 也未统一整个 pass 预算。
  这些配额也未与 TTL、compaction、对象 I/O 统一，不能视为整机维护预算已经完成。
- 现有 checkpoint worker 仍由 runtime 显式停止并 join，早于 runtime/store guard 释放。
  新接口不会永久持有退役 data group 的 driver 或 engine。

## 验证

新增 5 项内部 RPC 测试和 4 项 server 集成测试：

1. 认证、root、请求大小/类型/预算、响应 root/操作关联、零位置、非法 owner 拒绝；
   明确区分可重定向的 typed NotLeader 与其他错误。
2. 取消两个真实阻塞请求后，新请求继续被配额拒绝，直到原 job 真正退出；backend
   在预算后完成也不能发布成功响应。
3. 三个 metadata voter 和三个 data voter 使用实际磁盘 engine/Raft。data leader 为
   节点 1，metadata leader 为节点 2；节点认证、内部 ledger、公开 TTL 激活/写入走
   HTTP/2 over Tokio duplex，Raft 消息使用 InProcHub。
4. 逐一在八个 owner 变更的真实提交之后丢弃成功回执。每处都终止本次流程且不向
   其他节点自动重发；保留并重读相同 journal、generation 和 manifest。确认 owner 后
   才上传，由真正的 ManifestSeam 获得正向发布结果，再完成 Pending → Version 交接。
5. 连续八代 checkpoint 实际发布/采用后，关闭所有 driver/engine，再打开 leader 的
   data checkpoint/WAL 和 metadata WAL。value、expiry、clock 与 Published Version /
   Released Pending 的精确资源集合均保留。
6. 真实 follower 返回 typed NotLeader 后，陈旧地址路由能继续到真正 leader；非法
   节点/token、跨 root、过期预算、catalog 忙拒绝。重复确认不增加 revision，但增加
   apply index；本机 leader 也经同一代码成功，不需要环回 RPC。

本次 Raft 本地回归 322 项通过、8 项忽略、37 项 socket 测试过滤；server 激活/目录/
路由/维护/新 owner 组合 13 项通过（包含上述 4 项）；严格 workspace/all-target Clippy
含 tracing、格式与 diff 检查通过。[原始命令、日志及源码摘要](raw-ttl-checkpoint-owners-validation-20261004/manifest.json)
保留独立记录，测试集合有重叠，不能重复计数。

测试显式驱动 checkpoint 步骤，不是自动 worker 或完整 NodeRuntime 启动验收；重建
调用者/journal 不是进程 kill 测试。对象后端为测试专用 MemoryObjectStore，没有验证
MinIO 持久性、实际 TCP、跨主机断网或 Chaos。此次最后的磁盘重开验证两个 leader 的
持久数据，并未声称三个副本分别重启或重新取得服务/投票资格。

## 下一项与完整阶段边界

后续[自动 checkpoint 组件](RAW-TTL-DATA-CHECKPOINT.md) 已将每组 journal、唯一
ManifestSeam、leader 发布、各副本采用和退出排空接入调度。候选 cut 在冻结时保护，
从实际持久 journal 在恢复启动时重建；本地保护不等于可转移恢复授权。

Reader 生命周期、负向结局的持久解除授权、旧 Version 的安全释放及对象 GC 尚未完成；
不能根据超时、进程死亡、对象缺失直接释放 owner。全量 checkpoint 48 MiB 限制、全量
resident index、manifest/configuration/owner 历史增长和稳态磁盘/RSS/FD/对象债务仍开放。
TTL resident 优化、split/merge/迁移/安装/新增副本/恢复转移、整机预算、部署时钟资格、
独立时间历史、完整 TLA/TLC/组合证明及真实故障门槛继续按完整第二阶段推进。
