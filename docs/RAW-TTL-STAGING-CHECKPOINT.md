# TTL 导入期间的 checkpoint、恢复与 engine WAL 回收

日期：2026-10-05。接续 [split 后台续做](RAW-TTL-SPLIT-MAINTENANCE.md)。未完成的
TTL import 现在可由实际 checkpoint worker 发布和选中镜像、回收已覆盖的 engine
WAL 段，并从镜像与后续日志恢复；不会提前绑定 child range。第二阶段与生产范围
仍未完成。

## 镜像与日志语义

`DataCheckpointState` 明确区分 `Bound(DataRange)` 和 `Import(ImportState)`。
Import 的 target 只是预期范围，不是服务所有权。`RecoveredDataCheckpoint::range`
在原始镜像为 Import 时返回 None，即使后续 WAL 已经完成 Finish；实际恢复后的
engine 状态由镜像和全部合法 tail 决定。

`checkpoint_state` 从同一个真实 snapshot 读取 descriptor、clock、seal、import
及 manifest generation。镜像经 uploader 验证完整 SST closure 后，
`inspect_checkpoint` 重新检查 child root/region/creation、scope、未完成标志、
原始 source seal、clock、原值和 expiry，并按实际物理键顺序重建 prefix hash。
实际行数、最后一键和摘要必须精确等于 import 游标。采用前还与真实 driver 的
不可变 import spec 和进度比较，拒绝错误 operation、超前游标及同进度不同摘要。缺行、多行、孤立 expiry、
被改写的值/expiry/游标/计数、水位回退和提前绑定均拒绝。

子组的 source seal / expiry 身份属于父组；父组 index 可以大于子组镜像 cut，
不会被错误地当成子组日志位置比较。恢复仍要求实际 committed configuration、
精确成功 publication 指令及其原子 pair/history WAL 记录，镜像 descriptor 自身
不能授权选择。Tail 从 Import 状态续接，继续验证每个 page 和 Finish 的完整原子
布局。已有 snapshot install / migration 入口仍只接受真正的 Bound 镜像。

新增 **tag 13 `StagingManifestChange`**，使用现有 PreparedFlush 构造限制、generation
CAS、不可变 history、exact apply receipt 和唯一 ManifestSeam。真实 driver 在
提交 seam 请求时，根据本组实际 import 状态选择新指令；有序 apply 再验证精确
import target epoch。Finish 与提交竞争时，匹配同一 import 的完成状态仍可接受。
普通 fenced write 和公开 RawDirectory 不因此获得权限。

**旧 tag 7 的含义不变。** 未绑定 import 上的旧 ManifestChange 仍被拒绝，升级后
重放也不能把历史拒绝变成接受。两种指令都纳入 committed checkpoint 检查和精确
publication recovery；不能把新指令当成缺失的 checkpoint 历史。

## Runtime 与保留权限

checkpoint dispatcher 逐项合并 RawDirectory 和真实 group_handles 的有序目录，
一次只取一个候选，不复制完整目录；空组或坏记录也推进访问游标。原有每轮至多
32 次访问、两个 checkpoint 任务、255 个跟踪状态和共享维护预算继续生效。

未绑定子组必须持有真实 driver/engine，且其 import spec 与实际已提交 split plan
一致。扫描观察不是权限：工作过程中复核 owner 身份和计划，freeze 与镜像检查
读取真正的状态。Finish / metadata publication 后，仍是同一 driver 的 worker
可以继续使用原有 seam，不再另造一个协调器；只有进入原有 range reconcile 才
会获得公开服务入口。

导入镜像复用已有 durable candidate、Pending、全局 intent/admission、逐代 executor、
远端写入排空、成功结算和 Published/Current 所有者流程。只有这些条件以及镜像
验证成立后才调用实际 `checkpoint_applied` 选中镜像并回收被覆盖的 engine WAL 段。
失败或未知结果保留原候选/保护，不增加全局 Pending 或对象删除权限。

## V6 升级边界

当前 owner 使用 `KV9LIFE6`，内部服务为 `kv9.raft.v6`。V1–V5 lifecycle 在原独占锁
下原子升级并保持 node、store incarnation、phase 和 root；任何升级持久化失败
都要求重开后再获得 writer authority。冻结的 V5 codec 拒绝 V6；服务不提供 V1–V5
路径，格式确认只接受精确 V6。既有 activation / split transfer 的全配置确认
门槛随之要求 V6。

这仍是协调版本切换，未提供保持服务的滚动升级保证。旧 tag 7 保持历史语义与
磁盘/服务隔离是两个必要条件，不能互相替代。版本探测 example 和现有 endpoint/
replacement 脚本已更新；脚本仅做语法检查，没有执行真实集群操作。

## 验证与容量边界

[证据包](raw-ttl-staging-validation-20261005/manifest.json)记录源码摘要与原始日志。
**199 个不同 Rust 测试通过、8 个按原配置忽略**：lifecycle 9、state-machine 94、
command 15、格式 RPC 4、全配置 activation gate 9、snapshot install 19、server
runtime 37、maintenance 12。严格 workspace Clippy、格式、diff 和脚本语法检查通过。

新增实际三副本 runtime 回归经公开 API 写入 300 个 60 KiB TTL 值、永久值和已过期
值。完成两页复制后故意停在 Finish 前，启动生产 checkpoint worker，在 metadata
leader 与 child leader 分离的条件下发布并在三个副本选中导入镜像。每副本实际
engine WAL 文件总量从 **18,465,519 bytes 降到 2,712,201 bytes**；此时没有
`RANGE_KEY`，也没有公开子组目录。随后停止并重开三个 child Raft/engine、换主，
从 checkpoint 与 tail 恢复并完成 Finish/publication；原 expiry 与后续读写正确。

低层三副本测试另外覆盖 Begin 空导入镜像、部分复制镜像、恢复后追加 page 与
Finish tail；父位置显式高于子位置。旧 tag 7 拒绝、新 tag 13 编解码/有序 apply、
非服务写入拒绝均有断言。另有 10 类逻辑篡改先生成合法校验和的 SST，再检查恢复
内容验证明确拒绝。

大页测试的 child owner tick 为 100 ms；此前小值 fixture 的 5 ms tick 在 debug
大页处理期间出现 proposal replacement。最终测试没有以重试已接受写掩盖该结果，
也没有将测试时钟调整作为生产延迟修复。因此这里的字节下降只证明有限样本的实际
engine WAL 回收，不是慢副本下的可用性、暂停时间或稳态容量证明。

既有 Lean transfer 的 9 条组件定理、6 个语义错误控制、2 个 proof-policy 控制及
axiom audit 通过，TLA+ 通过 SANY。本轮没有新增 staging checkpoint 的形式化模型
或 Rust refinement；TLC 受既有环境限制未执行。这些证据不覆盖完整 checkpoint/
retention/upgrade 协议。

导入期间的 **Raft 日志**仍受现有 compaction/history 约束，本轮没有接通未绑定组的
自动 Raft 截断。48 MiB 全镜像限制、全量 hash/序列化的 CPU/RSS、超大历史值、长期
历史与对象债务、旧 Reader/Version/Pending 释放、unknown PUT 隔离、merge 和完整
NodeRuntime/TCP/MinIO/旧 binary/Chaos/跨主机验收继续开放。测试使用磁盘 Raft/WAL、
InProcHub、认证 HTTP/2 duplex 与 MemoryObjectStore；没有完整进程 kill 或真实网络
资格结论。

同日后续：[导入 Raft 回收](RAW-TTL-IMPORT-COMPACTION.md) 已补上自动规划对未绑定
子组的发现，三副本实际截断、物理重写、授权恢复与换主 Finish 通过。其前提和
剩余容量/部署限制见该文；本页原证据包保持不变。
