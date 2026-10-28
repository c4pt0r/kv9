# Raw / TTL 远端副本压力反馈

日期：2026-10-05。接续[本地磁盘余量](RAW-DISK-HEADROOM.md)和
[节点 WAL/Raft 预算](RAW-WAL-RAFT-BUDGET.md)。data group 的 leader 现在能依据
远端副本的容量报告提前拒绝新数据提案。本组件不改变复制成功、ReadIndex、apply
或回收授权条件；[第二阶段与生产范围](RAW-KV-PRODUCTION-SCOPE.md)继续开放。

## 报告内容与身份

内部 V7 服务新增只读 `InspectLogPressure` RPC，不新增磁盘格式或 Raft command tag。
请求经过现有节点鉴权，绑定 root、cluster、node、store incarnation 和随机 16 字节
challenge。服务端在实际探测前后核对 active store 的持久格式凭据与本地 Raft 授权。
客户端逐项检查回复身份和 challenge，不把 cluster token 单独当作 store 身份。

报告是以下二者之一，缺失值或 `uncertain=false` 都是非法回复：

- `available_bytes`：逻辑 WAL/Raft 剩余额度与实际磁盘剩余可准入量的较小者。
  磁盘量扣除最小余量、当前提案预留及采样重叠写入；零是明确的无容量观察。
- `uncertain=true`：本地计费不确定或空间探测失败，不能给出可用量。

采样沿用本地一秒有效期，在共享预算锁外执行 syscall，不产生提案预留或虚假的
写入拒绝计数。一个服务实例最多一个实际阻塞探测；RPC 超时或调用者取消后，
这个槽位一直由真实工作持有，直到探测结束。超时不能取消内核 syscall。

请求/回复的业务编码上限为 512 字节，客户端 codec 也限制为 512 字节；服务端先由
共享 Raft 服务的既有 decoder 接收，再执行该方法的长度检查，未新增独立的接收
内存预算。请求预算必须在 1–2,000,000 微秒。旧 V7 binary 的 Unimplemented、
鉴权失败、连接错误或超时均不构成健康报告；真实混合 binary 验收尚未执行。

## 后台轮询与缓存

每节点一个轮询器，从本地已 apply 的节点目录读取 active endpoint、generation
和 incarnation。250 毫秒调度一次，每次最多检查 16 个候选，最多两个并发 RPC，
一次连接及请求合计一秒期限。它加入节点共享维护预算的第十类轻任务，与已有
TTL/checkpoint/回收任务共享总占用和派发速率。这里的 250 毫秒是调度 tick，
不是每个远端节点的刷新 SLA；大集群、拥塞和维护竞争会延长完整轮询周期。

缓存最多 1024 个节点槽位，与当前节点 catalog 的数量约束一致。停用节点仍保留
槽位和报告，因此这是进程内历史节点数量上限，未提供无限新增节点身份的 GC。
超限、重复节点、同一磁盘的 generation 倒退或同代不同地址均拒绝整个缓存更新。

一个请求具有本地 route epoch、递增序号和发起时间。地址变化、停用/重新启用会
失效旧 epoch；晚到、乱序、身份不匹配以及发起后超过两秒的回复不能更新缓存。
同一磁盘换地址保留原容量压力；换成目录授权的新 incarnation 后回到未观察状态。

RPC 错误不能清除已知低容量或不确定状态。移除目录路由会停止轮询，但不会清除
旧报告，因为节点是否仍属于某个 group 要由那个 peer 的实际配置决定。负面观察
没有自动放行的 TTL；同一磁盘需要新的有效报告才能解除。缓存是内存状态，进程
重启后重新收集，未持久化远端报告。

## 实际写入门槛与控制工作

正常和 adopted data group 都接入同一节点缓存。数据提案在 peer append 锁内读取
真实 ConfState 的 voters、voters_outgoing、learners 和 learners_next；任一实际
成员报告不确定，或报告容量小于本次保守日志预留估算，即返回类型化
`WriteBackpressure`。无关节点的报告不阻挡该组。成员配置真正 apply 移除后，旧
缓存报告不再阻挡该组。检查不发 RPC、不读取 catalog、不执行文件 I/O。

普通/批量 Raw、PopulateRaw、TTL Write/Observe/Cleanup 和受保护 split Append
沿已有数据分类使用此门槛。拒绝发生在追加前，不留下新日志位置、临时字节预留或
异步 apply 等待槽位。TTL 读取需要提交 Observe 时同样可能被拒绝，不能改用本地
时间过滤绕过。已接受复制/apply、Noop、配置变更等现有控制命令不受此门槛限制。

**runtime 的 metadata peer 不绑定远端缓存**，避免旧的远端负面观察阻断修复节点
地址/成员关系所需的 catalog 写入。metadata 的实际日志仍计入本地共享额度并受
原有本地准入约束；本组件不为通过 metadata peer 提交的旧数据路径增加远端门槛，
也不证明所有管理操作在真实磁盘耗尽时都能完成。

状态输出新增 `replica_pressure` 和 `replica_pressure_worker`：逐节点报告身份、
路由、是否轮询、是否观察、不确定状态、可用字节、报告年龄和错误计数；worker
报告运行/在途/峰值/完成/错误数量。关闭时停止派发并排空本地网络任务，异常退出
会让 runtime 健康检查报错。远端尚未结束的 syscall 继续由远端实际探测槽位约束。

## 验证与剩余范围

[本次证据包](raw-ttl-replica-pressure-validation-20261005/manifest.json)记录原始检查
输出、命令和源码摘要。**744 个不同测试通过**，包括本次新增的 16 个；workspace
严格 Clippy、格式和 diff 检查通过，共 14 项检查。11 个既有忽略项保持原配置
（9 个真实 MinIO、2 个本地 encoder 实验）。广泛 Raft 回归排除 grpc/transport，
另执行其中 8 个不需 socket 的 RPC 测试，其余用例未计作通过。
覆盖身份/报文拒绝、探测期间撤销授权、超时/取消后真实槽位
保留、地址与 incarnation 变化、乱序/过期回复、目录原子更新与容量上限、全部配置
集合、实际三副本日志提交/移除成员，以及共享维护预算的占用与公平性。

三副本组合测试使用真实磁盘 WAL/Raft、raft-rs 与 InProcHub；公开 unary/point
stream 和内部节点鉴权 RPC 走 Tokio duplex 上的真实 HTTP/2。注入 follower 的
低空间与探测故障后，leader 拒绝 TTL 写入，控制日志及 metadata 写入继续 apply，
远端授权失败不能清除压力，新健康报告恢复写入。另测无 provider 时保留未知状态
与轮询关闭排空。空间故障来自可控探测器，没有填满开发机磁盘。

**这些报告是采样，不是远端预留或容量授予。** 多个 leader/group 仍可能消费同一
份报告的余量；采样之后的复制、控制、暂存和外部 I/O 也会消耗空间。尚未观察的
节点、重启后初始缓存和不支持该 RPC 的节点保留原有 quorum/本地准入可用性；
本组件不声称在这些情况下已取得全副本容量许可。已知负面观察则可能长期降低
可用性，需要恢复报告或完成实际成员调整。没有永不 ENOSPC 的保证。

远端跨组容量预留、完整控制/恢复工作空间、长期多组容量与 CPU/RSS/公平性资格
仍待完成。Reader/旧 Version/负向 Pending 释放、unknown PUT 隔离、历史有界与
GC、TTL merge、真实 NodeRuntime/TCP/MinIO、跨主机/混合 binary/故障验证和完整
形式化验收继续开放；本次没有重试已知受限的 TCP/MinIO 或 TLC 环境。
