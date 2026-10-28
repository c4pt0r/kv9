# Catalog 公平串行化与有界后台等待

日期：2026-10-06。接续[成员持久盘点](RAW-TTL-MEMBERSHIP-CENSUS.md)中观察到的
checkpoint 共存恢复超过 60 秒。本增量处理短暂 catalog 竞争被立即拒绝、继而放大为
整个 worker 长退避的问题。完整第二阶段、稳态资源与进度 SLO 仍未验收。

## 行为与安全边界

MetaRaft 的同一个 catalog 串行化点改用 `CatalogLock`，底层是固定版本
`parking_lot 0.12.5` 的 FairMutex。普通管理操作和后台等待共享这一把锁，测试中另保留非阻塞入口；
所有权仍覆盖 drain、元数据快照、规划、同 term 提案和精确 apply 回执，不能同时
基于同一旧序号构造两个 overlay。Cargo 只增加已在锁文件中存在的依赖边，没有新增
依赖包或网络下载。

底层原语公平交接给已经排队的等待者，避免当前持有者反复取得锁越过队列。
后台 retention Apply 和 capture plan 允许在已经取得 worker/RPC 准入后短暂等待，
期限为原请求 deadline 与当前时间加 100 ms 的较小值。期限到达仍未获得所有权时
保持原拒绝/未知结果语义；不因为返回 catalog busy 而自动重放已发送的数据请求。
这没有缩短或取消 checkpoint 原有指数退避，也没有延长原 RPC 总预算。

取得锁后再次检查期限；retention drain 和 capture 规划入口继续检查剩余预算。
等待超时会退出底层队列，不能在锁稍后释放时留下会自动执行的孤立任务。源查询仍在
锁外进行，最终 endpoint/ledger 决策仍在锁内复核。取消 RPC 不提前释放实际 worker
或它已有的配额，公平等待也不新增对象 I/O。

FairMutex 本身不提供 std Mutex 的 poisoning，因此包装层在持有者 panic 时先标记
poisoned、再释放底层 guard。后续阻塞、期限等待及测试非阻塞入口均拒绝正常取得该锁；不能把 planner panic 静默解释为安全继续。等待计数随成功、超时与 unwind
退出，测试据此等待真实 worker，而不依赖假设的网络延迟。

新后台等待沿用既有本地 outbound 和远端 inbound 的容量限制；普通管理操作沿用原
阻塞入口。这里没有为所有 catalog 调用者建立一个新全局队列上限，也没有证明系统
调度延迟、长时间 apply 或磁盘阻塞下的实时等待上界。100 ms 是 admission 等待预算，
不是事务完成时限。V11 服务/store floor 与 KV9LED23 均保持不变。

## 验证

新增五项测试覆盖：持锁时等待超期退出且无孤立 owner；八线程混合阻塞/期限等待的
256 次序号更新；panic 后三种入口均保留 poisoning；实际三副本 HTTP/2 的短锁竞争
在原 RPC 内完成且只提交一次；原 30 ms deadline 过期后，即使释放锁也不发生迟到
追加，后续显式新请求只产生自己的两条日志。

负向对照只把 retention 的有界等待替换回原来的立即 try-lock。新短竞争测试按预期
失败，随后恢复原源码并核对哈希。该记录保留在验证包，证明测试能识别原拒绝行为；
不是依靠客户端自动重放使结果变绿。

相关检查共 **126 个不同测试通过**：节点/锁 14、四线程 TTL runtime 106、server
文档测试 6，无选中测试忽略。严格 workspace 全 target Clippy（write-stage-tracing、
checkpoint-testing）、fmt 与 diff 通过。Clippy 修正把两个仅用于测试的 try-lock
方法限定为 cfg(test)；前后源码差异逐字节核对仅为这两个属性，runtime 测试构建中
的方法实现不变，因此复用该次 106 项结果，节点/锁与文档检查在最终源码重跑。

共存场景的本轮阶段日志：首次 data checkpoint 1.009 秒；第二次 data / 首次 metadata
checkpoint 6.020 秒；最后的 owner 交接检查立即成立。前一验证包记录的是整个共存
用例超过 60 秒后成功；这些是两次回归的实际观察，并非严格 A/B 压测、长期延迟或
吞吐资格。沿用前一组件的 90 秒第二阶段测试等待上限，没有修改生产退避策略。

首次开发构建因新测试缺少局部等待 helper、把 applied-position 当整数相加而失败；
修正后通过。第一次严格 Clippy 报告生产中不再使用的 try-lock 方法，已按测试范围
修正。全部原始失败、负向对照、命令、源码摘要与补丁保留在
[验证包](raw-ttl-catalog-admission-validation-20261006/manifest.json)。

实际组合使用 raft-rs、磁盘 WAL/Raft、鉴权 HTTP/2 duplex 和进程内复制，没有新增
真实 TCP/MinIO、完整 NodeRuntime、多进程 kill、跨主机、旧 binary、Chaos/TLC 或
完整协议证明；不能由这个局部等待改进关闭这些生产门槛。

## 仍需推进

需要在持续并发 checkpoint、census、迁移与管理操作下验证长尾进度、背压和全局公平
预算；单次测试变快不构成稳态 SLO。长临界区超过短等待预算时仍可能引发原有退避。
旧历史未确认交接的唯一原因并未由本轮反向确定。

完整历史 node/store 区间关联、snapshot 祖先、安装/备份/恢复来源和可转移锚点仍需
补齐；Reader/Publisher、未知 PUT、旧 Version/负向 Pending 的安全释放、物理 GC、
历史有界、merge、时钟与真实部署/故障/形式化资格继续按
[第二阶段](RAW-TTL-PHASE2.md)及[完整范围](RAW-KV-PRODUCTION-SCOPE.md)推进。
