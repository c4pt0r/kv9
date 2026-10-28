# TTL split 的源核验、受保护提案和 runtime 发布

日期：2026-10-04。接续 [复制状态与恢复](RAW-TTL-SPLIT-STATE.md) 和
[V5 格式隔离](RAW-TTL-SPLIT-FORMAT.md)。现有 AdminApi 的 seal、populate、publish
路径已接入 TTL 分支；这不代表完整 NodeRuntime、容量或第二阶段验收完成。

## 已连接的路径

`committed_split_plan` 从一个实际已应用的 metadata snapshot 读取并重新交叉校验
split intent、父 binding、两侧已创建/激活的子组，生成不可由 decode 构造的
`CommittedSplitPlan`。计划包含精确 creation、范围、operation 和副本集合；发布后
可重建同一个封存前计划。现有协议限定父子组使用相同副本布局。

runtime 使用这个计划选择实际 group driver。父组 `Seal` 经过 V5 全副本确认后
写入不可变 seal，重试返回原始 cut；`changed` 取决于本次命令是否实际建立该 cut。
TTL 不再走丢弃 expiry 的旧 DataRange seal。

`capture_ttl_split_source` 只从实际 driver 的已应用、已封存 parent snapshot
生成 `SealedTtlSplitSource`，没有公开构造器、反序列化或可改写字段。它复核本地
creation、range、seal、clock、import 及 seal 不超出 snapshot 的 applied position。
父副本可以是 follower：源是已经提交并冻结的内容，不依赖读取时仍任 leader。
凭据保留这个不可变 snapshot，并计算两侧 count/摘要；原始 expiry 身份和期限参与
摘要，已过期行也保留，复制不重新计时。

源扫描检查每个 Raw key 的 keyspace/范围、expiry 编码，以及 Default 的实际遍历
数、Write expiry 总数和空 Lock CF。孤立 expiry、跨范围/跨 keyspace 数据、未知
用户 key、扫描区间外的高字节 key 和损坏 expiry 均拒绝。合法本地 checkpoint/
transfer 元数据由既有 apply/recovery 校验，不作为子组用户数据复制。每行必须能
容纳在完整 transfer frame 中。

子组 `Begin` 消费源凭据和实际空 snapshot 的位置；重试读取现有完全相同的 spec，
不会重置游标。后续 page/Finish 由源 snapshot 和实际 import 状态构造，调用者
不能向受保护入口传入任意 row 或摘要。页面同时受 256 行和完整 16 MiB 编码约束。
普通 `RawTtlSplit` 提案仍一律拒绝，harness 原始提案只用于低层故障测试。

每个 transfer 命令都有一次独立、不可克隆的格式确认。复用既有 activation 的
完整配置并集、exact store identity、peer incarnation、leader term、applied-conf
index、pending-conf 和原始绝对期限检查；incoming/outgoing voters、learners 和
learners_next 都必须确认。runtime 还核对完整集合等于已提交布局，最多并发 8 个
格式 RPC，提交前再检查 endpoint、metadata 计划及实际 driver 身份。异步收集任务
只读，没有后台追加权限；所有普通/受保护路径仍由有序 apply 的 cursor CAS 裁决竞争。

一个 populate 调用使用 30 秒控制期限，逐页等待完整 typed receipt。超时/丢回执
不会自动重发已经接受的命令；后续调用重新读取实际 import 状态续做。Finish 前
不绑定 child range；Finish 后直到 metadata 原子发布前，子组仍不进入 RawDirectory。
publication 在本机逐一复核两侧完成记录、范围、水位和实际 value/expiry/count/摘要，
再通过原有序列化、term-fenced metadata 提案原子发布。目录已发布的重试沿用原有
幂等路径，不回滚或重新复制子组新数据。

## 验证

[证据包](raw-ttl-split-runtime-validation-20261004/manifest.json)记录全部命令、源码
摘要和原始日志。本轮 **141 个不同 Rust 测试通过、1 个忽略**：state-machine 92、
activation/transfer gate 9、metadata split 5、server TTL activation 35。严格 workspace
Clippy（tracing/checkpoint-testing）、格式和 diff 检查通过。

新增 runtime 回归使用实际 metadata/parent/两侧 child 的三副本 raft-rs、磁盘
Raft/WAL、V5 认证 HTTP/2 duplex 和 InProcHub：

- 300 行 TTL 数据、永久值和已过期行通过公开 TTL API 写入；缺少第三个 store
  确认时不封存；确认后封存重试保持同一 cut。
- 通过生产源核验与格式门槛完成 Begin 和首个 256 行 page，随后停止并重开三个
  低子组副本，走 production data-group recovery。metadata 和源组仍运行。
- 低子组换到 node 2 续传，空高子组由 node 3 导入并继承水位；node 2 承担目录
  发布。任一子组未完成时拒绝发布，发布前不向公开 RawDirectory 暴露子组。
- 三副本原始 expiry 字节保持不变，永久值没有 expiry；发布后公开读取过滤已过期
  行，续期产生新身份，旧 populate 被拒绝，重复目录发布不改写数据。

其他新增测试覆盖实际源 snapshot 的孤立 expiry/高字节 key/Lock CF/坏编码/外来
keyspace 注入；未封存源、错误 plan 与已过期限拒绝；单 snapshot metadata 授权
在 commit 前不可取得，发布前后计划一致；五副本 joint 完整确认、换 driver、实际
退出 joint 的 pending/applied 两个阶段均阻止旧确认追加。

既有 [Lean transfer 模型](../proofs/lean/ttl-split/Transfer.lean)重新验证通过：9 条
组件定理、6 个语义错误控制、2 个 proof-policy 控制和 axiom audit。它仍只覆盖
复制计数、水位与完成状态；本轮没有新增源凭据/全副本门槛/runtime 的形式化证明，
没有 Rust 自动提取。TLA+ 通过 SANY；TLC 受已知环境限制未运行。

## 剩余边界

源摘要需要扫描冻结 snapshot，重试会重新计算；这是 deadline 检查下的流式读取，
尚未成为持久分页摘要。后续 [后台续做](RAW-TTL-SPLIT-MAINTENANCE.md) 已将自动
复制接入共享并发准入，但尚无完整扫描预算。持有 snapshot 会保留旧内存版本；
迭代器仍复制单行 value，单行超出 frame 明确拒绝，没有大 value 分段协议。
单次控制期限不等于 CPU、RSS、磁盘、WAL 或节点公平预算。

后续 [staging checkpoint](RAW-TTL-STAGING-CHECKPOINT.md) 已连接未完成 import 的
镜像与恢复，验证 engine WAL 段回收，并将当前格式 floor 提升为 V6。全量导入的
Raft/WAL 稳态、
超大历史值导致封存后无法继续、完整 abort/回收仍待完成。后续后台续做组件已修复
阈值 0 冻结已提交 intent，并移出同步节点控制循环；完整资源稳态保证继续开放。

测试运行的是实际 RuntimeBackend 和 driver，并手动执行与 range reconcile 对应
的目录安装；没有启动完整 NodeRuntime 或真实 TCP。完整进程崩溃、旧 binary、
跨主机、MinIO、Chaos、迁移拓扑、merge 及 [第二阶段](RAW-TTL-PHASE2.md)其他未完成
验收继续开放。V5 仍需协调版本切换，不能宣称支持保持服务的滚动升级。
