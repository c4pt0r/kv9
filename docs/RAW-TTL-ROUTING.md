# Raw TTL 自动路由与跨范围分页

2026-10-04；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
本轮推进 [第二阶段](RAW-TTL-PHASE2.md) 的公开客户端，完整阶段及生产验收仍未完成。

## 入口与路由

新增 `client::routed::ttl::RoutedTtlClient`，沿用 `RoutedConfig` 中的固定 root digest、
tenant、keyspace 和至少两个 metadata seed。支持 unary 与 point stream，数据端始终
调用独立的 [RawTtl 协议](RAW-TTL-PUBLIC.md)。没有旧 Raw fallback，也不会自动启用 TTL；
激活仍走[显式绑定的管理入口](RAW-TTL-ACTIVATION-API.md)。

Get/BatchGet、Put/BatchPut、Delete 自动发现目标 range。批量请求的所有 key 必须属于
同一个 range；发现跨范围时整批拒绝，不拆成多个写入或多个读取快照。缓存与地址仅为
线索，精确 root、tenant、keyspace、binding 和当前服务端 fence 仍是每次调用的要求。
若缓存的范围不能容纳 batch，会先刷新一次目录，避免旧边界永久造成错误拒绝。

复用现有路由器的目录验证、endpoint 解码、连接和缓存边界：最多 32 个候选/缓存连接、
128 个 range 缓存、256 个在途调用、16 次尝试，总 deadline 不超过 30 秒。本客户端
要求至少 3 次尝试预算，为目录、能力确认、数据调用留出空间；发生重定向时可能需要
更多尝试。被淘汰的连接可由已获准的在途请求短暂持有，不据此宣称整个进程的内存上界。

目录探测使用较短的 probe timeout；follower 可提供 metadata 候选，不能授权 data
ownership。旧 leader hint 不会跳过尚未尝试的已知候选。每次数据调用前，对实际目标
副本执行 Capabilities；此步骤可处理 NotLeader、过期 scope 和暂时的探测失败。
不支持 TTL、未启用、异常控制元数据和不合法回包均不能触发旧协议降级。

修复了 Runtime 的一个路由提示问题：`LookupRawRoute` 现在仅从目标 range 对应且
binding 匹配的本地 group 获取 leader，并确认 leader 属于返回的副本集合。没有该
本地 group 时不返回 data leader hint，不能拿同一 keyspace 的另一个 group 代替。

## 写入与预算

一个逻辑调用从入口到 lookup、capability、data 共享总 deadline、尝试次数和客户端配额。
数据请求一旦发出，**至多调用一次**；即使返回明确拒绝，也由调用者决定是否发起新的
逻辑请求。请求发送后的超时或丢失回复仍报告 `UnknownWrite`，不刷新 route 后偷偷重放，
不重新计算 TTL。服务端继续保留首次准备的期限和取消后的 admission 契约。

报告区分 Lookup/Capability/Data，包含 node ID、scope 观察、耗时和结构化失败。
只有能力确认成功、但预算不足以发送数据时，返回 `NotDispatched` 与停止原因，不能
把它记成写成功。未通过输入/编码大小检查的调用返回 ClientRejected；把 binding 与
protobuf framing 计入 1 MiB 请求上限。独立的 Capabilities 没有用于路由的 key，
因此自动路由客户端拒绝这种用户操作；内部每次调用的能力确认仍自动执行。

## Scan 与游标

`call(Operation::Scan { start, end, limit })` 开始扫描。每个逻辑页最多读取一个 range，
服务端上界裁剪为用户 end 与当前 range end 的较小者；页内沿用复制 Observe、ReadIndex
和同一 snapshot 上的 TTL 过滤。每页最多返回 256 行、检查 4096 行，数据 RPC 响应
不超过 1 MiB。不为了凑满可见行而循环遍历多个 range 或所有过期行。

- 页内还有工作时，保留服务端的包含式 `next`，包括字节预算留下的首个未返回 key。
- 当前 range 已扫描完、整个查询尚未结束时，`next` 是当前 range 的排他上界，下一页
  从该位置查询新 owner。一个空 range 可以返回零行、零 examined，同时仍带续页游标。
- `continuation: None` 才表示本查询已结束；不能根据 rows 为空停止。
- 报告中的 `ScanCursor` 保存版本、root、tenant、keyspace、原始 start/end 和下一位置。
  可通过 Serde 保存和恢复，然后调用 `continue_scan(&cursor, limit)`。继续前验证格式、
  namespace 和前进/边界约束，不把原 group/epoch 固定在游标内。
- 一个最大长度 4096 字节 key 的后继位置可为 4097 字节、末字节为零。目录查询新增接受
  这一合法位置，普通 Raw mutation 的 key 上限没有扩大；畸形的 4097 字节回包被拒绝。

每页是各自的一致性读取，整个扫描不是全局 snapshot，也不冻结 TTL 时间。并发更新、
过期和拓扑变化遵循每页观察；已经越过的位置不回头。游标是调用者保存的查询状态，
不是签名凭据或所有权证据；服务端始终重查 scope。后续 v2 已覆盖完整 catalog 边界，
并增加了分页 DeleteRange，见[范围操作记录](RAW-TTL-DELETE-RANGE.md)。首次路由组件的
4096 字节边界限制已修复；对应历史验证目录仍保留原记录。

## CLI

`kv9-ttl-client --config FILE` 现支持两种互斥配置：原来的 `client` + `binding`，或
新的 `routing`。仍从 `KV9_CLIENT_TOKEN` 读取 token，配置文件现最多 1 MiB；例如：

```json
{
  "routing": {
    "version": 1,
    "root_digest": [1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1],
    "tenant_id": 1,
    "keyspace_id": 100,
    "seeds": [
      {"node_id": 1, "address": "127.0.0.1:19001"},
      {"node_id": 2, "address": "127.0.0.1:19002"}
    ],
    "max_in_flight": 4,
    "max_attempts": 12,
    "deadline_ms": 5000,
    "probe_timeout_ms": 500,
    "retry_backoff_ms": 10,
    "cache_capacity": 16
  },
  "transport": "tonic_stream"
}
```

以上 root digest 和节点仅为示例，必须替换为实际可信集群配置。输入沿用有界 NDJSON：

```json
{"id":1,"operation":{"kind":"put","pairs":[[[107],[118]]],"ttl_secs":60}}
{"id":2,"operation":{"kind":"scan","start":[],"end":[],"limit":128}}
```

下一行使用报告中原样保存的 `continuation` 对象，形如
`{"id":3,"cursor":<continuation对象>,"limit":128}`。每行只能选择 operation，或
cursor + limit，ID 严格递增。自动路由配置不接受 `--activate`，不会把激活扩展到查询
途中发现的其他范围；原显式 binding 配置的激活方式保持可用。

## 验证和开放项

新增 9 项路由/分页测试通过，覆盖两种 transport、候选故障切换、过期 leader hint、
scope 刷新、跨范围 batch 拒绝、错误 namespace、旧协议、不重放未知写、取消后配额、
共享 deadline/尝试预算、protobuf 总字节检查、过期空页/空范围、游标序列化、最大 key
后继位置及拓扑替换。这些客户端边界测试的目录和返回页受控；它们没有运行 TTL split
或 merge 协议，不能作为范围数据转移正确性的证据。

另外新增两项 RuntimeBackend 测试：三副本实际 metadata/data driver、真实持久格式
激活后，通过客户端发现分离的 leader、执行 TTL 读写与过期 Scan（unary/stream 均覆盖）；
以及目标 group 缺失时不借用其他目录项的 leader。后者注入一个其他范围的目录包装对象，
仅验证选择逻辑。组合 fixture 采用真实 WalEngine/DiskRaftStorage、InProcHub 复制和
Tokio duplex 上实际 HTTP/2；时钟为受控 provider。未启动完整 NodeRuntime，未绑定 TCP。

本轮路由测试 9、activation/runtime 测试 8、显式 TTL 客户端 5、公开数据链路 6 项通过；
CLI 实际二进制的 4 项配置/输入检查无网络请求。workspace 全 target 严格 Clippy
（含 write-stage-tracing）、格式与 diff 检查通过。原始命令、日志和源码摘要见
[验证记录](raw-ttl-routing-validation-20261004/manifest.json)，历史记录保持原样。

分页 DeleteRange 已增加独立的删除游标与 CLI 入口，见上述 v2 记录。
后续仍须完成后台公平时间推进与条件清理、各组 checkpoint/WAL
空间回收、TTL resident 优化，以及范围转移/安装/新增副本/恢复的格式与水位互通。
完整 NodeRuntime 多组跨范围读写、部署时钟资格、实际网络故障、
独立历史、完整 TLA/TLC 和 Chaos 验收继续保留，不因客户端路由组件通过而关闭第二阶段。
