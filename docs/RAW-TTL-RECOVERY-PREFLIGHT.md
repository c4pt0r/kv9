# 对象读取前的 checkpoint 发布校验

2026-10-05；接续 [安装 Reader](RAW-TTL-INSTALL-READERS.md)。本轮修复恢复入口的
校验顺序，并补齐运行态 Reader 准入所需的本地 publication predecessor 输入。
第二阶段和生产验收继续开放。

## 修复的问题

此前 metadata/data checkpoint 恢复先检查 manifest scope 与 cut 配置，再 GET SST，
最后从 WAL 的原子批次核对成功发布及对应的 committed Raft command。因此一个实际
已经提交、但 CAS 失败的 ManifestChange，只要被错误写成本地选中 checkpoint，仍会
触发对象读取，直到之后才拒绝恢复。恢复最终没有成功，但 GET 的准入顺序不正确。

现在先只读检查本地 selector 与选中 WAL 尾部。复用完整恢复的 winning batch 校验，
匹配 manifest、generation、change ID、原子 manifest pair、实际 committed position
以及该位置的精确 Raft command。还跟踪首次成功发布的 history row 和最终 manifest
pair，拒绝后续删除、覆盖或回退。相同镜像再次发布时，保留首次成功的 position 和
predecessor，不从较新的 generation 猜测原来源。

预检查失败不会发 SST GET，也不会创建、同步或截断引擎文件。允许存在不完整的 active
尾记录，报告其字节数，留给成功的完整恢复处理；完整记录 checksum 损坏仍拒绝。
closed segment 使用已有严格 summary/checksum 校验。旧 WAL 与分段 WAL 都复用实际
恢复的解析器和 covered-prefix 规则；分段 topology、旧 checkpoint 选择发生变化时拒绝。
旧 checkpoint 文件的读取也限制在已有的 1 MiB manifest 上限加一个探测字节以内。

## 接入与限制

`WalEngine::inspect_checkpoint_tail` 只在存在选中 checkpoint 时扫描尾部；没有选中
checkpoint 时不增加整条 WAL 扫描。`inspect_checkpoint_read` 返回不可修改的
`CheckpointReadPlan`，提供 manifest、publication position 和 predecessor，供后续
定位精确 Pending/Version owner。它本身不授予 Reader pin、GET 权限、恢复回执或 GC 权限。

metadata 的共同 checkpoint 恢复入口、普通 data group 恢复以及 adoption 后的 engine
恢复均已调用预检查。实际 GET 前再次匹配所选 manifest，完整恢复后再次核对 publication。
原有 SST、range、TTL、水位、WAL、pending journal 和 recovery dependencies 校验保持执行。
只有私有 adoption anchor 中的精确原始镜像可沿用“没有本地 publication”的例外，后续
checkpoint 仍需本地成功发布。原始镜像的 durable adoption/安装校验保持原有流程。

调用者仍须持有 store/group 锁直到完整恢复结束；这些 API 不提供跨进程并发写保护。
预检查流式保存一个 WAL record 和有界 manifest/发布状态，但增加一次本地尾部读取及
发布证据检查。完整恢复仍会构造内存引擎；本轮没有测量大规模重启吞吐或声称减少恢复延迟。

本轮没有新增 ledger 请求、磁盘格式、依赖包或 metadata 写操作，也没有引入启动时等待
Reader 提交的循环依赖。**runtime adoption、data 启动与 metadata 自身启动的 Reader
生命周期尚未接入**。下一步以该精确 predecessor 接入读者准入；普通 data group 必须在
metadata 就绪后重试，metadata 自身恢复则需要独立持久授权设计。旧 Version/负向 Pending、
未知 PUT、全体读写者清单、portable recovery 和物理 GC 仍开放。

## 验证

[证据包](raw-ttl-recovery-preflight-validation-20261005/manifest.json) 保留命令、原始日志
和源文件摘要。新增 9 个测试覆盖旧/分段 WAL、跨 closed/active segment、部分尾记录、
checksum 损坏、visitor 拒绝、selector 改变、无 checkpoint 的空路径、成功与失败 CAS、
伪造或未提交 publication、删除/回退 history/pair，以及相同镜像再次发布。

最终 **597 个不同测试通过，12 个既有测试忽略**：engine 240、metadata 214、
snapshot/install 24、checkpoint recovery 29、TTL runtime 72、RegionManager 18。
workspace 全目标严格 Clippy、格式和 diff 检查通过。adoption 的 TTL 回退/撤销测试
已补齐后继镜像的精确本地 publication，确认通过预检查后仍由原有 TTL 校验拒绝。

零 GET 断言通过计数的 MemoryObjectStore 验证；拒绝前后逐文件字节比对确认没有修改。
合法路径实际恢复 SST 和尾部数据。对象数据、Raft 和运行态回归仍属于本地组件验证，
不代表真实 TCP/MinIO、跨主机、进程 kill、断电或旧二进制升级验收。

后续：[运行态恢复 Reader](RAW-TTL-RUNTIME-READERS.md) 已消费这里的精确 predecessor，
接入 adoption/data 恢复准入和持久排空报告；metadata 自身的启动授权仍单独开放。
