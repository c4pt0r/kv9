# Checkpoint 上传执行者的持久 owner

2026-10-04；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
接续 [本地对象任务排空](RAW-TTL-OBJECT-EXECUTION.md)。第二阶段与生产验收仍未完成。

## 身份和进入顺序

自动 data/metadata checkpoint 现在为实际上传执行登记独立 Build owner。继续使用
既有 ledger 的 `Share`、`Publish`、`QuiesceAfterTransfer` 和 `Release`，没有新增
RPC、ledger、WAL、journal 或 store 格式，也没有打开对象 GC。

执行 owner ID 从 domain label、root digest、原 checkpoint change ID、node、store
incarnation 和 process incarnation 计算。scope、operation、subject 与完整 SST closure
从原 Pending binding 继承。`LocalLifetime` 保存实际 node/store 和 16 字节随机 process
身份；Runtime 的后端视图复用同一 context，新的 runtime owner 从 OS entropy 重新生成
`ProcessIncarnation`，不使用 PID、时钟或超时租约推断旧执行者已经结束。

进入顺序为：已有 durable plan → 确认原全局 Pending/Version 保护 → 保存精确执行
handle → ledger `Share` → `Publish` 的精确已应用回执 → 外部 PUT/读回。生成 handle
本身没有上传权力；任何未确认的写都不能进入对象 I/O。

同一 checkpoint attempt、同一 runtime 身份的重试复用 owner ID。只有上一代 Released
时才能建立下一代，generation 严格加一且溢出拒绝；重试不增加该 owner 的 tombstone
数量。活跃或 Quiesced 的旧代不能由新 handle 当作可复用状态，原 handle 必须先收敛。
不同 runtime 身份得到不同 owner，不能借新进程启动把旧 Published 记录改成 Released。

## 本地排空后的交接

外层 `with_local_execution` 返回后才关闭该代执行记录。关闭前要求同一 subject 和
完整 SST closure 仍由原全局 Pending 或对应 Version 的 Published owner 保护。
若 Pending 已完成正向交接，使用 Version。再通过既有 ledger 原子转移规则将 Build
变为 Quiesced、Released。此处释放的是执行 Build 的 pin，**全局 Pending/Version
仍保留对象保护**，不代表远端 PUT 已取消或所有执行者已停止。

关闭错误保留原 handle；data worker 的下一轮首先重试关闭，metadata worker 也在
下次 tick 开头重试。期间不启动新上传。即使本地 journal 已在正向发布后清除，内存
执行 handle 仍需完成原代关闭。取消后的本地排空结束也会尝试交接；metadata 不可用
或进程退出时，不把未确认关闭改写成成功，已登记 pin 继续存在。

未知进入也必须有精确终态：如果 Share 未被观察到，关闭路径先提交同一代 Share，
再交接释放。这样迟到的同代 Share 无法在 Released 后复活。如果下一代进入在追加
前被拒绝，读到的可能仍是上一代 Released；该状态只允许补完精确的下一代终态，
不能错误地关闭已开始的更新一代。旧 handle 对后来 generation 的关闭明确拒绝。

这些仍是既有 retention owner 语义。`Released` 表示其保护已交给完整后继，不是
独立、可移交的远端取消证明。管理接口原本也可按完整后继覆盖转移 Build；不能仅凭
一条 Released 观察断言发布者集合已经永久关闭。

## 验证

[命令、日志与源码摘要](raw-ttl-executor-owners-validation-20261004/manifest.json)另存。

- 新增真实三副本 metadata/data 组合测试，对执行 Share、Publish、Quiesce、Release
  四处分别注入已提交但丢失回执，随后用原 handle 完成同一代并核对完整 binding。
  同时验证下一代追加前拒绝、重复关闭、旧代不能关闭新代、活跃代不能被新 handle
  接管，以及不同 process 身份保持旧执行者 Published。测试停止实际 Raft 驱动并
  重开 metadata WAL，核对全部终态、generation 与未关闭记录。
- 新增自动 data worker 测试：manifest 已发布后，执行 Quiesce 回执丢失，后续 Release
  持续被拒绝。期间加入新的 TTL 写入，多轮真实重试都不增加 PUT 数或 manifest
  generation；恢复 ledger 后，补完原代关闭才发布下一 checkpoint。
- 扩展上一轮实际 MinIO job/deadline 循环的 data/metadata 测试：对象调用者已超时、
  scope 已关闭而本地 job 尚未退出时，Build 仍 Published；排空后才 Released，原
  Pending 继续 Published，原 durable plan 仍可恢复。
- 既有磁盘重开测试仍在原正向交接步骤丢回执。新增执行登记增加两次 mutation，
  因此故障注入从第 5 次改为第 7 次，保留原来的行为断言。

TTL 激活组合 **24 通过**；common/meta 的单元、集成和文档测试合计 **183 通过、1 忽略**。
专项集合重叠，不相加。严格 workspace Clippy、格式及 diff 检查通过。以上网络是带认证
的 HTTP/2 duplex 与 InProcHub；对象后端为 MemoryObjectStore/InMemory。WAL 重开
不等于完整 NodeRuntime 重启或进程 kill，未运行真实 TCP/MinIO/旧二进制验收。

## 仍须完成的协议和容量工作

此实现增加了实际新上传路径的持久记录，没有证明所有历史执行者已登记。旧 binary/
旧调用路径可没有 Build 记录；遗失的进程身份、远端未知 PUT、其他节点和 SDK 内部
执行者不能靠空列表、超时或新 process 身份判为已结束。全局禁止新增发布的门槛、
完整 backfill、崩溃遗留执行者的可靠关闭、负向 Pending release/GC 仍需完整协议。
历史遗留 Build 的保留是待处理债务，不是已达到空间稳态。

既有每 resource **4096 owner（含 tombstone）** 和 ledger batch 上限保持不变。每个
不同 runtime/attempt 新增一个 Build owner；同 owner 重试仅推进 generation。历史
回收、容量与长期物理债务没有因此解决，也不能靠驱逐旧 owner 达到容量目标。
后续的 [owner 分行存储](RAW-TTL-OWNER-ROWS.md) 已解除这条 production ledger 单值
累计上限，保留旧 base 和 Released 代际记录；物理历史回收和长期债务仍未解决。
Reader、旧 Version、迁移/安装和所有副本的继任恢复引用、TTL split/merge、常驻索引、
整机预算、48 MiB image 替代、部署时钟与独立时间历史、真实网络/旧二进制、完整
TLA/TLC/Chaos 继续保留在完整第二阶段范围中。
