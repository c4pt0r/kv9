# Raw / TTL 本地磁盘余量准入

日期：2026-10-05。接续[节点 WAL/Raft 字节预算](RAW-WAL-RAFT-BUDGET.md)。本次给新
数据提案增加实际文件系统余量检查，保留原有成功写、已接受 apply、恢复和回收语义。
没有新增磁盘格式、Raft tag 或 wire 字段，store/service floor 仍为 V7。
[完整第二阶段和生产范围](RAW-KV-PRODUCTION-SCOPE.md)继续开放。

同日后续的[远端副本压力反馈](RAW-REPLICA-PRESSURE.md)已增加 data group 采样门槛。
下文“尚无远端反馈”描述本组件交付时的边界；它的历史证据包保持原样。

## 配置、来源与计算

`KV9_MIN_FREE_DISK_BYTES` 默认 **256 MiB**，最小 **16 MiB**。零、负数、非十进制、
溢出值和过小值均拒绝启动；不提供隐式关闭或探测失败时放行。它独立于默认 4 GiB
的逻辑文件/提案预算。runtime 在 store 独占锁建立后打开 data directory，保留该
FD，并把带此探测器的同一 LogBudget 交给 metadata 和所有 data group。

Linux 实现使用 `fstatvfs` 的 `f_bavail × f_frsize`，不把特权用户保留块算作普通
写入空间；只读文件系统、无效分配单位、乘法溢出和 syscall 错误都拒绝新数据。
字段语义见 [Linux statvfs(3)](https://man7.org/linux/man-pages/man3/statvfs.3.html)。
目前非 Linux runtime 明确拒绝配置该实现，不静默退化为无余量检查。库用户仍需
显式绑定磁盘探测器，纯逻辑额度的构造器保留给没有本地磁盘 owner 的使用者。

启动扫描和后续 engine/Raft 绑定检查所有相关路径的 device，包括临时文件、
WAL family、嵌套孤儿 segment 和 Raft 目录。不存在的文件以最近存在的祖先判断。
跨 device 布局拒绝启动/绑定，不能用一个磁盘的空间准入另一个磁盘上的日志。
这不是对多卷独立预算的支持；运行期间无关进程挂载或替换目录不在 owner 契约内。

每个数据提案仍在实际 peer append 锁内预留。逻辑预算已经满或来源不确定时直接
拒绝，不额外探测。否则采样文件系统，然后在共享预算锁下同时满足：

```
逻辑文件债务 + 未 apply/临时提案预留 + 本次预留 <= WAL/Raft 额度
最近采样可用字节 >= 最小余量 + 当前全部提案预留 + 本次预留 + 与采样重叠的写入
```

可用空间已经反映已有文件，因此第二式不再扣一次全部文件债务。现有保守提案估算
仍覆盖未来 Raft/engine 增长；小提案约 5 MiB 的预留成本没有被本次性能调优。

## 探测与并发写入

系统调用在共享预算锁外执行。采样前记录累计开始写入字节和仍在 I/O 中的字节；
采样后用同一预算锁读取增量，再扣掉采样开始时的在途写入。实际 BudgetWriter
在调用底层 write 前登记，成功、短写或错误返回时才结束这次在途计费。
即使某次写入在探测返回前完成，也不会因为在途计数归零就从增量中消失。
短写/失败仍按尝试字节保守扣减本次重叠量；原有文件债务的释放规则保持不变。

多个组的空间读数可以相同，但提案预留在同一个预算锁下增加，不能重复消费同一
余量。等待取消或 owner 退出仍不释放已接受位置。apply、真实后缀覆盖和授权文件
回收各按原边界结算；文件回收后的下一次探测才看到新增空闲空间。

探测失败或从开始到准入检查已超过 **1 秒**，返回类型化 WriteBackpressure，不
沿用旧读数。下一次提案重新采样，健康读数可恢复准入。这个期限是结果有效期，
**不能取消阻塞中的系统调用**。调用期间当前 peer 锁仍被持有，可能延迟该组控制
工作；共享预算锁不被持有，其他来源可以继续计费和释放。每次数据提案增加一次
同步 syscall，端到端性能和调度公平性尚未验收。

TTL split 使用的受保护格式确认，在空间探测后、append 前再次检查原 deadline。
探测不能延长确认的有效期；过期时归还尚未提交的临时预留，日志位置不变。

## 低余量下的行为与状态

普通/批量 Raw、PopulateRaw、TTL Write/Observe/Cleanup 和受保护 split Append
都使用同一准入。TTL 读取需要 Observe 时返回明确 Backpressure，不退回本地时间
过滤。普通 Raw ReadIndex 和 capability 查询保持原语义。

follower 继续持久化和 apply 已接受日志，控制/恢复/回收工作不按这条阈值拒绝。
低余量 follower 当选 leader 后，按自己的空间拒绝新数据；余量恢复后可继续写入。
当前 leader 尚未收集远端副本空间，不具备跨副本磁盘压力协调。

`wal_raft_budget.disk_headroom` 增加最小余量、上次可用字节、上次采样重叠量、
在途写入字节、探测/探测失败/余量拒绝计数和最后错误。`last_*` 是历史诊断，
不能用来跳过下一次采样，也不授予压缩或删除权限。

## 验证与未完成项

[本次证据包](raw-ttl-disk-headroom-validation-20261005/manifest.json)保存原始输出、
命令和源码摘要。**718 个不同测试通过**，workspace 严格 Clippy、格式及 diff
检查通过。11 个既有测试按原配置忽略（9 个真实 MinIO、2 个本地 encoder 实验）；
Raft grpc/transport 模块的 63 项因已知 TCP sandbox 限制过滤，单独 runtime
HTTP/2 duplex 回归通过。新增测试覆盖共享余量竞争、在途及采样期间写入、阻塞探测下其他
来源的计费/apply、低余量/故障/恢复、过期与溢出读数、真实 Linux 探测和跨 device
检查、嵌套 orphan 路径验证、公开 TTL 拒绝、受保护确认过期，以及三副本选举后的
本地准入。磁盘空间变化使用可控探测器注入，没有填满开发机磁盘；实际 FD/fstatvfs
和磁盘 WAL/Raft 另有直接测试。选举采用真实 raft-rs tick/消息，由 InProcHub 传输。

这是一条**测量余量的准入策略，不是预分配的磁盘保障**。follower/control、其他
进程和未接入该在途计数的 pending/spool/安装写入仍可能消耗空间；文件系统配额、
块分配/元数据、inode 耗尽或设备故障也不由逻辑字节估算保证。256 MiB 是可配置的
策略默认值，没有证明能覆盖所有控制/恢复峰值，不能据此承诺永不 ENOSPC。

远端磁盘反馈、完整暂存与控制工作空间、长期多组稳定负载、CPU/RSS/公平性，仍须
继续完成。Reader/旧 Version/负向 Pending 释放、unknown PUT 隔离、有界历史/GC、
大镜像迁移和 TTL merge 也继续保留。没有重新运行受限的 TCP/MinIO 或 TLC，没有
把这些组件测试计作完整 NodeRuntime、跨主机、混合 binary 或真实故障生产验收。
