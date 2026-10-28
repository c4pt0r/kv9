# Raw TTL 全副本格式激活门槛

2026-10-04，工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
这是 [TTL 第二阶段](RAW-TTL-PHASE2.md) 的内部控制组件。后续已接入
[公开激活 API](RAW-TTL-ACTIVATION-API.md) 和 [TTL 读写](RAW-TTL-PUBLIC.md)，完整阶段仍未完成。

## 当前路径

`NodeRuntime::activate_raw_ttl_format` 是嵌入式阻塞控制接口。公开的 `ActivateRawTtl`
RPC/CLI 与它共用收集路径，支持远端 metadata leader，不要求迁移 data leader。
具体控制协议、认证、预算和新增三副本 HTTP/2 证据见[公开管理记录](RAW-TTL-ACTIVATION-API.md)。

1. 从已经发布的 RawDirectory 找到实际 group，driver 读取本组 applied DataRange。
   在 peer 锁内捕获 term、配置 apply index 和完整配置：voters、voters_outgoing、
   learners、learners_next、auto_leave。成员取四个集合的并集，最多 1024 个。
   不能使用只返回 incoming voters/普通 learners 的 `membership()`。
2. metadata ReadIndex 后，在同一 catalog snapshot 中逐个解析 active node 的 endpoint
   和确切 store incarnation；root 初始 voter 的磁盘身份必须仍符合 root descriptor。
   缺失、重复、外来 root、无效 incarnation 都拒绝。catalog 不是成员集合的来源。
3. 收集 V4 格式确认，最多 8 个并发请求；本地副本也必须提供实际 StoreGuard fence。
   总预算在 `(0, 30s]` 内，连接、RPC、目录复核和 apply ticket 共享一个绝对 deadline。
   重复回包不能补足缺失副本。中途失败会丢弃整个未提交的收集结果。
4. RPC 完成后重新建立 metadata 读屏障，复核所有 endpoint 的完整记录以及本地 range
   handle。目录变化即重新收集。两个 Raft group 之间没有新增原子事务：格式证据是
   对确切 store identity 的持久事实，不能据此声称具备原子的跨组身份撤销协议。
5. driver 先预留有界 apply waiter，再复核 applied range。在 append 的同一个 peer 锁中
   重查 owner 实例、term、完整配置、配置 apply index、deadline 和 pending ConfChange。
   配置改走再改回也不能复用旧证据；重建相同 region/node/term 的 peer 同样不行。
   使用 raft-rs 的 `has_pending_conf()` apply frontier，避免新 leader 的保守 tail fence
   被错误地与最后 ConfChange index 比较而永远不能解除。
6. 只允许该专用入口提交 Enable，初始化 clock 为 0，不产生时间健康声明。普通
   RaftGroup、driver、term-bound 和 async proposal 入口均拒绝直接提交 Enable，
   SingleNodeRaft stub 也沿用该检查。测试专用 raw-byte 注入仍用于独立 apply 故障测试。

收集结果无 Clone/反序列化入口，提交消费一次。返回的 `RawTtlActivationSubmission`
保留提案位置，由 `wait()` 提供实际结果；append 之后 owner 关闭、waiter 注册失败仍然
保留该位置，不能伪装成提交前拒绝。替换、超时或失败不会自动重提 Enable。重新确认必须
重新捕获配置，并继续遵守调用者的原始操作 deadline。已持久启用的组在恢复时由 clock
record 恢复模式；重复 Enable 仍经门槛，clock merge 不回退。

## 与旧 split 的顺序边界

旧 split 只复制 Default CF，尚未保留 Write CF 到期信息及 clock。现在 DataRange seal
在同组有序 apply 检查 TTL 模式：Enable 在先，seal 返回 `RawTtlRejected(LegacyCommand)`；
seal 在先，Enable 返回 FenceRejected。拒绝条目仍原子推进 applied position，不改变
range 或 clock。此措施防止旧 split 静默丢失 TTL，不代表 TTL split 已实现；已创建的
split intent 仍需要既有恢复/处置流程。

## 验证与剩余边界

- 新增 8 项 activation 测试：真实 raft-rs 五副本 joint 配置、漏确认/重复确认、替换磁盘、
  pending/已 apply/改走再改回的配置、leader term、peer 重建、过期证据、普通入口绕过、
  队列满与 stop、Enable/seal 两种顺序，以及实际 `(term,index)` apply 回执。
  StoreGuard fixture 产生真实本地持久格式证据，传输是 InProcHub。
- 新增 1 项 driver 测试覆盖 append 后关闭 owner、注册 waiter 失败时保留位置。
- 新增 2 项 runtime 目录测试覆盖单 snapshot、incarnation/地址变更、inactive/missing node
  和初始 voter 替换；已有旧 reader 隔离测试继续通过。
- Raft 本地回归：298 通过，8 项 MinIO 忽略，37 项网络测试显式过滤。
  workspace 全 target 严格 Clippy（含 write-stage-tracing）、格式与 diff 检查通过。
- 一次 `ttl` 模糊筛选意外包含名称中带 `settlement` 的两个 server socket 测试，均在
  bind 处被环境以 EPERM 拒绝；日志保留，随后使用确切模块/测试名称完成 3 项本地验证。
  这两项没有被算作通过，也没有重试监听。

以上是首次门槛组件的历史测试记录。后续 RuntimeBackend 的真实三副本与 HTTP/2 收集到
Enable、公开 TTL 读写、Linux 时钟提供者均已增加本地证据，见对应记录。实际网络与完整
NodeRuntime 启动、已有 TTL 组新增副本、迁移与 snapshot 恢复、旧二进制混部/降级、
部署时钟资格、后台清理和空间回收、TLC/Chaos 均未完成，第二阶段未完成。

原始日志与源码摘要见 [本轮验证记录](raw-ttl-activation-validation-20261004/manifest.json)。
