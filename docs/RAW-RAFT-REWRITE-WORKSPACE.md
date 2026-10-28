# Raft 日志流式重写与临时空间预留

日期：2026-10-05。接续[节点日志预算](RAW-WAL-RAFT-BUDGET.md)、
[实际磁盘余量](RAW-DISK-HEADROOM.md)和[远端压力反馈](RAW-REPLICA-PRESSURE.md)。
本组件修复回收路径的整段临时复制和并发临时文件空间竞争；不新增磁盘格式、
Raft command tag 或 RPC，store/service floor 保持 V7。完整第二阶段继续开放。

## 流式重写

原先 `reclamation_records` 先复制配置历史、取得完整保留尾部，再把每条日志编码
收集到一个 `Vec`，最后才写临时文件。现在在同一个实际 writer 锁内执行两遍
流式遍历：第一遍核算精确输出大小，第二遍直接写入 `raft.log.tmp`。

每遍按原顺序访问历史 base、初始及逐次配置、保留条目和 HardState；每次只编码
一条记录。配置历史通过借用迭代器访问。日志读取把索引区间限制到单条，因为当前
raft-rs MemStorage 会先 clone 整个请求区间，再执行 `max_size` 截断；仅给整个
尾部传一个字节页大小不能限制这个中间分配。retained-byte 维护扫描也改成逐条
求和，仍保留原有只计 entry payload 的统计语义。

写入帧沿用原来的长度、kind、payload 和 FNV 校验。编码器直接构造最终 frame，
去掉另一个完整 body 副本；重写时不保留前一条编码结果。单帧仍受既有 64 MiB
格式上限约束。输出预留计算为每条 payload 加 9 字节 framing，第二遍核对实际
输出不能超过或少于第一遍结果。

原子的发布顺序保持：完整临时文件 sync → rename → directory sync → 重开新
writer、关闭旧 FD。rename 前失败保持原 writer 可用；之后失败要求重新恢复。
已提交前缀的截断仍由原有 catalog/全副本匹配/checkpoint history 门槛授权。
重写本身不能授权新的截断或删除。

lease/protocol snapshot 的排除检查移入实际 writer 锁内，避免等待 writer 时
发生的新发布被先前检查漏过；这些布局继续延期，不由普通重写清除其记录。

## 节点共享临时空间

新增 `KV9_RAFT_REWRITE_BYTES`，默认 **512 MiB**、最小 **16 MiB**。零、过小值、
负数、非十进制及溢出配置拒绝启动。它与 metadata/所有 data group 使用的同一
LogBudget 绑定，约束正在执行的 Raft 重写所预留的完整输出字节总和。库用户没有
绑定 LogBudget 时仍可流式重写，但不会自动获得 runtime 的节点预算。

在创建、截断或删除任何本次临时文件之前，完整输出必须满足共享重写额度；启用
磁盘探测时还必须满足：

```
采样可用空间 >= 最小磁盘余量 + 当前数据提案预留
               + 其他重写的完整预留 + 本次完整输出 + 采样重叠写入
```

空间采样在共享预算锁外，沿用一秒有效期和在途/并发写入补偿。核算不确定、额度
不足、探测失败或过期时返回 `Ok(false)`，让后台延期并继续轮转其他组。它不把
空间不足升级为 Raft 持久化故障，也不改写原日志或已有 orphan 临时文件。

**数据日志额度已满时仍允许重写**：重写有独立的输出额度及真实磁盘检查，不要求
`file_bytes` 先降到数据准入阈值以内，避免用需要回收才能腾出的数据额度阻止回收。
同时，新数据提案和远端容量报告都会扣除在途重写预留；它们不能与并发重写重复
消费同一份观测空间。实际临时文件写入仍经 BudgetWriter 计费。计费同时保留完整
重写预留直到工作结束，故有保守重复扣减，未把它优化成可提前归还的容量。

预留由同步重写工作持有。结束时只归还未来输出额度；已经写入的文件字节按实际
布局成功发布、旧 FD 关闭和确认盘点后才释放。失败 cleanup 不构成释放已有文件
债务的证据。启动仍将旧 `raft.log.tmp` 计入文件债务；活动预留本身不跨进程持久化。
这份额度不是所有历史临时文件的硬上限。

状态 `wal_raft_budget.rewrite_workspace` 给出 limit/reserved/peak 字节、active、
admitted 和 deferred 数量。总 `peak_accounted_bytes` 纳入未来重写预留；原有
`reserved_bytes` 仍专指数据提案/位置债务。观察或延期重写不会制造数据拒绝计数。

## 验证及边界

[本次证据包](raw-ttl-rewrite-validation-20261005/manifest.json)记录命令、原始输出和
源码摘要。**756 个不同测试通过**，含本次新增 12 个；workspace 严格 Clippy、
格式与 diff 检查通过，共 14 项检查。11 个既有忽略项（9 个真实 MinIO、2 个
本地 encoder 实验）保持原配置；广泛 Raft 回归排除 grpc/transport，另外执行
其中 8 个不需 socket 的 RPC 测试，未将其余用例计为通过。
新增测试覆盖共享额度竞争、数据额度耗尽时仍能回收、探测期间完成的
写入、低空间/探测故障、临时文件创建失败、已有 orphan 保留、精确输出边界、
配置验证、lease fence 保留及真实双 group 回收/重启后继续写入。

2048 条、每条 16 KiB 的实际磁盘日志回归包含未提交尾部。输出为 **33,591,200
字节**；当前测试的逐线程临时堆分配峰值为 **33,181 字节**，retained-byte 扫描
为 **104 字节**。重开后逐条对比日志，并核对 vote/term/commit/configuration。
这个测量在既有 resident mirror 建立后开始；它证明该同步路径没有整段临时复制，
**不代表完整进程 RSS 或常驻索引已受限**。测试 allocator 不进入生产 binary。

双 group 回归中，保留尾部超过 4096 字节测试额度的大组延期，小组可继续回收；
大组获得更晚的精确截断 floor 后也能回收。两组通过携带已知 compaction cut 的
恢复入口重开并继续提交写入。fixture 的 decision 不能替代 metadata 授权的生产
验收；完整 runtime 组合回归另保留既有 TTL/import checkpoint 恢复测试。

这不是磁盘预分配或永不 ENOSPC 的保证。块分配、inode/目录元数据、外部进程和
其他未接入工作区的写入仍可能消耗空间。单个输出超过额度会持续延期，直到提高
配置或出现允许更小尾部的已授权 floor。已有 orphan、长期阻塞 syscall 和大尾部
的 CPU/暂停时间仍需部署容量验证；两遍扫描没有降低持有当前 peer/writer 锁的
总时间上界，也没有解决整个 resident Raft mirror 的内存规模。

跨节点预留仍未实现：一个可超时的 RPC 临时 claim 不能替代已提交日志位置的持久
归属，leader 切换/重启/未知结果也不能自动退款。本次先补其所依赖的本地回收空间
约束，继续保留远端共享容量协议、完整安装/恢复/控制空间、长期多组稳态、对象
owner/GC、TTL merge、真实网络/MinIO/故障和完整形式化验收。未重试既有受限环境。
