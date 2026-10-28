# Checkpoint 后台交接的换主覆盖

2026-10-05；接续[原子 Begin](RAW-TTL-ATOMIC-BEGIN.md)。本轮修正组合测试的节点
任务拓扑并补充确定性换主验证；没有改变生产协议、退避、超时或 GC 权限。

## 已定位的超时

原共存测试在 metadata 节点 2 上启动一份 checkpoint sweep，同时运行实际 metadata
和 data checkpoint worker。该测试没有禁止正常 Raft 选举，但隐含假设节点 2 在
发布、停止 publisher、收尾三个阶段始终保持 metadata leader。

新增阶段/状态诊断后，两次失败都出现在 **latest owner handoffs**：manifest 已经
提交；metadata Pending 已交接，data Pending 仍 Published，对应 Version 不存在。
sweep 自身仍在运行，但后续不再派发任务。第二份诊断显示三个 metadata 副本都已
apply 到 term 2 / index 66，节点 1 是 Leader，节点 2、3 是 Follower，所有节点
`ensure_serving` 均成功，driver 没有 fatal 或 step error。

这解释了停止派发：生产 sweep 在失去 leader 资格后正确撤回维护需求；测试的新
leader 没有 sweep 实例，未完成的 data owner 交接无任务接管。catalog busy 是此前
一次 pass 的保留错误，并不是这个停顿的根因。仅提高等待时间或修改退避不能补上
缺失的任务。诊断重现一次发生在连续第 27 次，增加角色诊断后在第 10 次重现。

## 修正与验证范围

生产 `NodeRuntime` 本来就为每个节点启动 `checkpoint_sweep::Maintenance`。
组合测试现在也在三个节点启动 sweep；各任务依据实际 leader/服务状态准入。
结束时先取消全部任务，再逐个等待真实任务退出，核对全部节点维护占用归零。
metadata/data publisher 的其他测试设置保持既有范围。

新增确定性用例在真实 data Raft 上提交 manifest，留下 Published Pending 和缺失
Version，随后一直持有原 metadata leader 的 catalog 锁。确认旧 sweep 已发现尝试
且交接被拒绝后，显式把 metadata leadership 从节点 2 转移至节点 3。节点 3 原已
启动的 sweep 必须完成正向交接；旧锁仍未释放，旧任务交接次数仍为零，原节点的
follower snapshot 已看到 Released Pending 和 Published Version。最后释放锁并
排空所有任务，检查共享维护 permit 归零。

本用例不借助 publisher 重启、手工调用 `Reconciler::pass` 或改写 owner 状态完成
接管。它验证实际后台任务的换主路径，不构成完整 NodeRuntime 多进程、TCP、MinIO、
网络分区或物理断电资格。负向 Pending、旧 Version/Reader 和未知远端 PUT 的释放
边界没有改变。

[本轮独立证据包](raw-ttl-failover-validation-20261005/manifest.json)保存失败诊断、
确定性回归、重复共存测试与完整 TTL integration 的日志及源码摘要。旧证据包和
其中“超时未定位”的历史结论保持原样，通过本证据的新结论接续。

本轮完整 TTL integration **53 项通过**；修正后的共存用例连续 30 次通过，
强制换主用例连续 10 次通过。另有 1 项并发 WAL 负载回归通过，合计 54 项不同
测试；严格 workspace/all-targets clippy、格式和差异空白检查通过。重复运行次数
不重复计入不同测试数量。

第二阶段仍须完成完整 publisher/generation 关闭、负向释放、恢复锚点、物理 GC、
长期容量、部署时钟、形式化组合与真实部署验收。
