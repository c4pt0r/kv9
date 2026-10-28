# 节点共享维护配额

2026-10-04；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
本轮在[自动 data checkpoint](RAW-TTL-DATA-CHECKPOINT.md)之后，接通七类维护任务的
共享准入。它解决独立任务槽位叠加的问题；[第二阶段](RAW-TTL-PHASE2.md)的完整整机
资源预算、物理债务与生产验收仍未完成。

## 实际接入点

每个 RuntimeBackend 持有一份节点 Budget；同一 NodeRuntime 的 backend view 复用
该实例。以下维护者通过同一个配额获得不可克隆 permit：

| 类型 | 类型内上限 | 重 I/O 配额 | 执行路径 |
| --- | --- | --- | --- |
| TTL | 2 | 否 | 实际 blocking 页，包括 ReadIndex、Observe、扫描及 Cleanup |
| data checkpoint | 2 | 是 | 冻结、journal、owner 登记、对象上传/读回、提交和采用 |
| metadata checkpoint | 1 | 是 | 原有 RemoteStorage 线程中的完整 tick |
| compaction | 1 | 是 | 真实 leader/follower prefix 操作及确认 |
| compaction plan | 1 | 是 | 自动 floor 检查与 metadata 提交 |
| Raft reclaim | 1 | 是 | 实际持久日志重写 |
| retired reclaim | 1 | 是 | 原有持久 Reclaimed fence 下的本地 payload 删除 |

总计最多 **4 个已准入任务**，其中重 I/O 最多 **2 个**。TTL 仍可能执行磁盘 I/O，
分类只为防止两个长 checkpoint 阻塞所有清理槽位，不表示 TTL 是纯内存操作。
共享派发 token bucket 最多积累 4 个，每 10 ms 补充一个；长期最多每秒 100 次派发，
空闲时间不积累额外突发。字节数量和 CPU 成本没有由任务次数上限代替。

每类只有一个 dispatcher 和一个等待位置，总共固定七个位置，无按请求增长的队列。
最早等待且符合类型/重 I/O 上限的类型先获得空闲配额；已受重 I/O 上限阻塞的等待者
不挡住 TTL。每次取得配额后重新排队。无到期任务、禁用、取消和 dispatcher 退出时
撤回等待；同步控制循环不再 serving 时也撤回。公平性依赖 dispatcher 持续轮询和
已运行任务最终返回，不承诺系统调用永久阻塞时的全类型进展。

配额锁仅覆盖常数大小的记账，不覆盖 engine、RPC、共识或文件 I/O。TTL/data 的
permit 在派发前取得并移入实际 blocking closure，因此 blocking pool 排队也占用槽位。
取消等待 future、删除目录项或丢弃 dispatcher 不会提前释放正在执行任务的 permit。
共识 apply、owner ledger RPC 和前台操作不获取此 permit，避免 checkpoint 持有配额
再等待同一配额池处理其依赖操作。

## 控制循环与观测

四个同步控制维护入口也持有上述共享 permit。它们保存各自的 region 轮转位置，
每次最多检查 32 个候选，最多处理一个组的实际压缩、删除、重写或新 floor 提案。
已经完成的 compaction 不再重复 fsync；失败组之后从后续位置继续，保留原有 committed
决策、all-matched、checkpoint history ceiling 和确认校验。目录与历史 catalog 仍先
完整读取，分页化和这些读取的内存/CPU 上界尚未完成。

metadata 线程在周期到期后按 50 ms 轮询共享配额，获得后才执行 tick；成功/失败返回
后继续沿用配置的 flush interval。配额等待不会改写 journal、生成新操作身份或成为
释放 owner 的依据。原有未知结果、恢复和保留协议不变。

节点 status 新增 `maintenance_budget=<JSON>`：当前/峰值总占用、重 I/O 占用、等待类型数，
以及每类准入、释放、延后轮询、当前/最长等待和累计占用毫秒。释放统计包含失败、
取消和 panic 后退出，不是成功操作数；时间包含队列与同步等待。它们不是垃圾字节、
对象债务或公平吞吐量的直接测量。

## 验证与未完成范围

[命令、日志与源码摘要](raw-ttl-shared-maintenance-validation-20261004/manifest.json)
独立保存，前序证据保持原样。

- 三项配额测试检查重 I/O 饱和时 TTL 容量、等待类型不被后来请求越过、取消等待和
  丢弃 dispatcher 不释放实际 job、总派发速率以及空闲后无额外突发。
- 实际三 metadata voter、两个各三 data voter 的 HTTP/2 duplex/InProcHub 组合中，
  两个上传停在 MemoryObjectStore 包装器内部；TTL 清理两组所有副本的 value/expiry。
  同节点真实 metadata worker 等待共享配额。取消 data worker 后两个 permit 仍占用，
  释放阻塞并排空后归零。测试等待两个真实 PUT 入口，不能仅凭已派发计数判断 I/O 已阻塞。
- 另一个实际 metadata leader 测试用两个占位 permit 制造饱和：等待期间不上传也不提交
  manifest；释放一个后，原 RemoteStorage worker 实际完成 owner、上传、发布和本地采用。
  fixture 安装与生产相同的 CatalogFenceAdjudicator，不旁路 epoch 校验。
- 既有 TTL 续期竞争、未知 apply、取消重开、checkpoint 原计划恢复和退役回归继续通过；
  严格 Clippy 同时包含 write-stage-tracing 与 checkpoint-testing。各测试集合有重叠。

本轮并未把所有后台工作都纳入配额：自动 split、迁移/安装/启动恢复和其他控制扫描
仍需接入；共识与前台也需要独立资源契约。没有对象/WAL 字节速率、临时内存、CPU
成本配额或同步 I/O 抢占。一个 admitted job 可执行多个对象访问，两个永久阻塞任务
仍会阻止其他重 I/O 工作；任务数上限不证明全局尾延迟有界。

真实 NodeRuntime 控制循环、TCP、MinIO、持续负载的磁盘/RSS/FD/对象稳态、独立时钟
资格与完整故障验收未由本轮测试证明。下一步继续完成其余维护路径、字节/债务预算，
Reader/旧 Version/负向 Pending 释放及对象 GC、历史有界与 TTL 范围转移。全量 image
48 MiB 上限、全 resident index、完整可转移恢复锚点及原生产门槛继续保留。

后续[对象 payload 配额](RAW-TTL-OBJECT-IO.md) 已接入实际对象 worker 的 PUT/GET，
并限制 checkpoint GET 的 body 收集。它是上述字节预算的一个组件；WAL、CPU、临时
内存、其他维护路径及物理债务验收继续开放，本文原始日志不扩大为其完成证据。

同日后续：[checkpoint 本地对象排空](RAW-TTL-OBJECT-EXECUTION.md) 补上同步 MinIO
调用超时后对象 job 仍在执行的边界。维护 permit 现在保持至该 tick 的本地对象范围
排空；新增实际 dispatcher 与对象 worker 组合测试。上述旧 MemoryObjectStore 阻塞
测试只覆盖外层同步调用未返回的情形，不能单独证明这个新增边界。远端 PUT 取消、
全局执行者关闭和 owner 释放仍不是本地排空的结论。
