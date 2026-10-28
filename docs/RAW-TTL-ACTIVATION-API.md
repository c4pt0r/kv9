# Raw TTL 公开激活与远端 metadata leader

2026-10-04；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
这是 [第二阶段](RAW-TTL-PHASE2.md) 的一项组件交付，阶段和生产验收仍未完成。

## 入口和结果

新增 `/kv9.v1.Kv9/ActivateRawTtl`，当前请求携带协议版本 2、精确的规范 `DataRange`
binding 和 1..30,000,000 微秒预算。请求上限为最大规范 binding 编码加 64 字节，已支持
完整 catalog 边界，见[v2 范围记录](RAW-TTL-DELETE-RANGE.md)。该独立管理 RPC 使用现有
client 认证和 MetadataWrite admission。node 身份不能调用公开管理入口；本次未新增
管理员角色或 TLS 协议。返回 `RawTtlResponse`，包含版本、binding digest 和本次
Enable 的实际 applied `(term,index)`，并沿用严格的响应与拒绝元数据校验。

激活是 range 格式的持久变化：必须先完成 [V4 升级](RAW-TTL-UPGRADE.md)，随后通过
[全副本门槛](RAW-TTL-ACTIVATION.md)。它不证明时钟健康，初始 clock 水位为 0。
启用后旧 Raw 读写路径拒绝该组，应使用 [独立 TTL 协议](RAW-TTL-PUBLIC.md)。
没有禁用接口或自动降级；TTL 组的旧 split seal 继续拒绝，以免丢失到期元数据。

Rust 入口为 `ScopedTtlClient::activate()`。即使数据 transport 选 point stream，
激活也调用该独立 unary RPC。CLI 沿用公开 TTL 客户端的配置、精确 binding 和
`KV9_CLIENT_TOKEN` 环境变量：

```sh
cargo run --offline -p kv9-server --bin kv9-ttl-client -- \
  --config ttl-client.json --activate
```

CLI 输出一条 `{"activation": <TtlReport>}`，不读取后续 NDJSON。
只有明确的提交前 NotLeader 可以在同一总 deadline 和尝试预算内跳转到已配置 peer。
超时、连接失败、丢失回复和不合法响应均不自动重试。部分未提交的控制面错误也保守地
报告 `UnknownWrite`，不能据此断言一定已写入或一定未写入。调用者再次显式激活会重新
收集所有证据并追加新的确认条目，返回的是新回执，不是找回原操作结果。
取消 RPC 不会释放仍在执行的后台操作配额；reservation 保持到准备/收集/精确 apply
等待结束。已经 append 的操作不会因客户端超时而被撤销。

## 两个 Raft group 的协调

metadata leader 和 data leader 可以位于不同节点，不要求迁移 leader：

1. data leader 从真实 driver 捕获 range、term、配置 apply index、四个成员集合与
   owner 身份；成员以完整 ConfState 为准，不能从目录推断投票配置。
2. 内部 V4 `ResolveRawTtlStores` 在 metadata leader 建立 ReadIndex，然后从同一个
   catalog snapshot 读取已认证 root、当前精确 range 和各 active store 的 incarnation、
   endpoint、generation、previous address。请求必须来自已认证的非零 node 身份。
3. 在实际 metadata leader 本地直接读取；其他节点通过已知 leader hint、当前 voters
   和初始 voters 的候选地址查找，最多 32 个候选、每个最多 1 秒，服从原始总 deadline。
   候选只是地址线索，每个返回仍须验证 root、cluster、binding 和完整有序成员列表。
   follower 上的陈旧目录不构成确认，不接受远端提供的任意重定向地址。
4. 并发度最多 8，逐个请求真正的 `ConfirmRawTtlFormat`。本地成员使用真实持久
   StoreGuard fence；远端必须返回确切 root/cluster/node/incarnation 的 V4 凭据。
   即使已有可写 quorum，缺少任意一个成员的格式证据也不能启用。
5. 收集后重新获取 metadata leader snapshot，逐字段比较目录结果。观察到 endpoint
   代际或身份变化即拒绝此次收集。目录重发的 `RawGroup` 包装对象可以变化，但实际
   driver、engine 和 binding 必须仍相同；换 owner 不可复用证据。
6. 沿用不可复制、单次消费的 prepared gate，在 append 的同一个 peer 锁中复核完整
   配置、配置 apply index、term、owner、deadline 和 pending ConfChange，随后等待
   精确 apply 回执。RPC/目录读取不在 peer 锁内执行。

内部目录查询最多 1024 个有序、唯一、非零成员；请求上限现由最大规范 binding、成员
编码和 envelope 计算，响应仍为 256 KiB。
每个 RaftGrpcService 有独立 4 个 blocking 查询配额；RPC 取消后，worker 退出前仍持有
配额。它没有占用注册请求的单独配额。此限制不是整机维护预算，也没有消除目录扫描成本。

两个日志之间没有新增原子事务。第二次目录读取只能拒绝已观察到的变化；它不能被描述为
原子的跨组身份撤销协议。安全门槛仍依赖确切 incarnation 的持久、单调格式事实，以及
data peer 在提交点对实际配置和 owner 的检查。新增副本、迁移及恢复的完整验收仍然开放。

## 验证

新增三节点 fixture 使用六个真实 raft-rs driver（metadata 与 data 各三个）、实际
DiskRaftStorage/WalEngine 和三个真实持久 StoreGuard。metadata leader 固定在节点 2，
data leader 在节点 1；公开管理、内部目录和格式确认均通过真实 tonic HTTP/2 handler
及认证运行。激活没有使用 test-only Enable 注入。Raft 复制用 InProcHub，HTTP/2 用
Tokio duplex I/O；fixture 直接组合 RuntimeBackend，未验证完整 NodeRuntime 启动。

新增 8 项测试：

- 4 项三副本组合测试：全 store 确认、缺少第三副本时拒绝、重复激活的新回执及后续 TTL
  读写；目录 generation 变化时无 Enable；metadata/data leader 间明确重定向且只 append
  一次；重复发布相同实际 owner 不误拒绝。
- 1 项客户端测试：unary/stream 两种数据 transport 下激活回复丢失不重试，取消后配额
  保留到 completion 结束，不调用旧 Raw 路径。
- 3 项内部目录测试：scope/成员/身份/endpoint 规范检查，认证及请求上限，4 个被取消的
  blocking 请求占满配额，第 5 个明确拒绝，worker 退出后恢复容量。

本轮回归：activation 6、客户端 5、公开数据链路 6、公开协议校验 2 项通过；Raft
本地 301 项通过，8 项 MinIO 测试忽略、37 项 socket 测试显式过滤。workspace 全 target
严格 Clippy（含 write-stage-tracing）、格式和 diff 检查通过。原始命令、日志、源码
摘要和未完成项见 [验证记录](raw-ttl-admin-validation-20261004/manifest.json)。

没有运行 TCP/TLS 或跨主机故障测试；公开链路时钟仍为注入 fixture，没有新增部署时钟
资格证据。后续已有[自动路由/跨 range cursor 组件](RAW-TTL-ROUTING.md)与
[长边界/分页删除组件](RAW-TTL-DELETE-RANGE.md)，[后台 TTL 调度](RAW-TTL-MAINTENANCE.md)
也已有组件。旧二进制升级/降级、完整多组生命周期、整机维护预算、每组 checkpoint/WAL
空间回收、split/迁移/merge 互通及
完整历史/TLC/Chaos 验收仍未完成。
