# Raw TTL 公开协议组件

2026-10-04；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
本记录覆盖公开读写链路，不表示 [第二阶段](RAW-TTL-PHASE2.md) 或生产验收完成。

## 入口与能力

新增 `/kv9.v1.Kv9/RawTtl`，point stream 使用独立操作码 **6**；旧操作码、旧 Put
请求和旧 `RoutedRaw` 均不降级执行 TTL。`RawTtlRequest` 现在必须指定协议版本 2、
规范 `KV9RNG01` binding，以及 1..30,000,000 微秒的服务端预算。响应必须回显
协议版本和精确 binding digest。客户端同时验证响应类型、条目数量、值和游标边界。
旧端点不支持方法/操作码时，能力探测失败，客户端不会转发为旧 Put。

`Capabilities` 在目标组通过 ReadIndex，并从快照验证已启用 TTL、range 身份和 epoch。
它只确认协议和持久组状态，不会启用 TTL，也不证明当前时钟健康。组启用仍要求
[V4 格式隔离](RAW-TTL-UPGRADE.md) 和 [全副本激活门槛](RAW-TTL-ACTIVATION.md)；
现已增加[公开激活 RPC/CLI](RAW-TTL-ACTIVATION-API.md)，嵌入式
`NodeRuntime::activate_raw_ttl_format` 继续保留；两者共用全副本门槛。
已有 metadata fallback 和 legacy reader 不获得 TTL 权限。

`ScopedTtlClient` 在每个逻辑调用前完成能力确认。确认和数据调用共享客户端总 deadline、
请求配额与尝试次数。只有能力探测的明确 NotLeader 可以跳转；数据请求至多发送一次，
包括收到明确拒绝之后也不自动重新解析 TTL。不同节点的单调时钟不能直接共享 Instant；
线上传递的是剩余预算，服务端从入口开始固定自己的 deadline，覆盖排队、准备、
提交、apply 和读取。客户端在原始 deadline 到达后停止等待，并如实报告未知写。

## 数据语义与边界

- `Put` 的 `pairs` 覆盖单 key 和单 range 原子 BatchPut；`optional ttl_secs` 保留 presence。
  未指定清除旧期限；零值在 apply 时过期；正数在首次准备时解析为时钟区间上界加秒数。
  一个 batch 共用一次解析，重复 key 按输入顺序覆盖。apply 不读时钟。
- `Delete` 同时删除值和期限。永久覆盖、零 TTL、删除只沿用已提交水位，时钟未配置或
  已拒绝时仍可执行；它们仍受一致性、range fence、配额与日志背压约束。
- v2 新增[分页 DeleteRange 与长范围边界](RAW-TTL-DELETE-RANGE.md)，每页至多一个
  原子 TTL Write，返回精确回执与可显式继续的位置；未知页不重放。
- `Get`（一个 key）和 BatchGet 使用同一 keys 接口。正 TTL 写以及 Get/Scan 必须获得
  [合格时钟样本](RAW-TTL-CLOCK.md)。读取先提交 Observe 并等精确回执，再建立 ReadIndex；
  值、期限、水位来自同一快照。旧 resident 快速路径仍拒绝已启用组；新 TTL 路径统一
  经过上述协议，目前没有 TTL 专用的 resident 性能优化。
- `Scan` 仅在指定 range 内分页。每页最多返回 256 行、检查 4096 行，另限制返回字节，
  为协议与游标留出空间。响应不超过 1 MiB；按字节停止时保留首个未返回 key 的**包含式**
  游标。按行数停止时使用最后检查 key 后追加零字节的游标。即使没有可见值也必须继续
  处理 `next`；范围边界覆盖 catalog 的 65536 字节，后继位置另允许追加一个零字节。
- 沿用有界客户端的 key 4096 字节、value 65536 字节、batch 256 项、请求/响应 1 MiB
  上限。Get/BatchGet 不拆快照；超出总响应上限返回失败。Scan 可在字节边界自动分页。
  物理扫描行预算与返回字节预算不是 engine snapshot 的总内存预算。

新增 TTL 命令提交返回接受位置，即使 append 后等待注册与 shutdown 竞争也保留该身份。
本路径不自动重提被替换的条目，避免改变期限或把未知结果变成重复写。明确的 clock、
TTL、scope 和背压拒绝使用严格 `kv9-ttl-refused` 元数据；重复、混合、未知或不合法
控制元数据不构成“没有写入”的证明。未确认、apply/engine 失败或不合法回包的写入保留
`UnknownWrite`。应用层显式发起的新调用是新逻辑操作，不应据未知结果自动重试。

TTL 读取的 Observe 也是日志写入，因此受保留日志背压约束。所有 TTL 操作（含读取）
在排队、apply 等待、最终快照操作和 RPC 取消期间保留同一 admission reservation。
目前每次读取都会复制观察，尚未合并时间推进或做吞吐调优。

## 使用

Rust 入口为 `client::ttl::ScopedTtlClient`，配置沿用 `ClientConfig`，另传入从可信
root-pinned `LookupRawRoute` 取得的 `DataRange`。选择 `TonicUnary` 或 `TonicStream`；
实验 tarpc transport 明确不支持。此精确 scope 客户端不会自动刷新或跨 range 执行。
需要自动发现与跨范围 Scan 时，使用后续新增的
[`RoutedTtlClient` 和路由配置](RAW-TTL-ROUTING.md)，每个数据调用仍独立确认 scope。

CLI：

```sh
cargo run --offline -p kv9-server --bin kv9-ttl-client -- --config ttl-client.json
```

认证 token 使用 `KV9_CLIENT_TOKEN` 环境变量。配置结构为：

```json
{
  "client": {
    "version": 1,
    "peers": [{"node_id": 1, "address": "127.0.0.1:19001"}],
    "keyspace_id": 100,
    "epoch_conf_ver": 1,
    "epoch_version": 1,
    "max_in_flight": 4,
    "max_attempts": 4,
    "deadline_ms": 5000,
    "retry_backoff_ms": 10
  },
  "binding": [],
  "transport": "tonic_stream"
}
```

`binding: []` 是必须替换的占位符：填入实际 route 响应的规范字节数组，不是自行生成
range 身份。配置现在最多 1 MiB，支持完整 binding 的 JSON 数组；输入按行有界，ID 必须递增。CLI 为每次调用输出
独立 JSON report（包括能力阶段、确认回执、明确拒绝和未知写），不重放失败操作。
输入示例：

```json
{"id":1,"operation":{"kind":"put","pairs":[[[107],[118]]],"ttl_secs":60}}
{"id":2,"operation":{"kind":"get","keys":[[107]]}}
{"id":3,"operation":{"kind":"put","pairs":[[[107],[118]]]}}
{"id":4,"operation":{"kind":"scan","start":[],"end":[],"limit":128}}
```

## 验证及未完成项

本地测试覆盖真实单副本 raft-rs、DiskRaftStorage 和 WalEngine：精确写入身份，
未指定/零 TTL，按区间避免提前到期，重复 key、空 value、任意 value 字节，旧快照，
实际 WAL 重开，回拨拒绝，过期空页，最大 key 游标，响应字节分页和共享 deadline。
格式启用在这些读写测试中是显式 test-only fixture；不能作为生产激活协议的证据。

两个实际 tonic HTTP/2 adapter 经 Tokio duplex I/O 接入客户端和真实 Raft fixture；
另有不支持能力、回包丢失、取消后配额保持、严格拒绝和回包校验测试。没有绑定 socket，
不代表 TCP/TLS、完整 NodeRuntime 多组启动、跨节点激活或时钟部署资格验收。
时钟由可控 provider 注入；真实 Linux provider 的独立组件验证见时钟记录。

原始命令、日志、源码摘要和未通过的完整验收项记录在
[公开协议验证记录](raw-ttl-public-validation-20261004/manifest.json)。历史验证目录不覆盖。

后续的[公开激活管理验证](RAW-TTL-ACTIVATION-API.md) 增加了三个真实副本、分离的
metadata/data leader 和实际格式凭据的 HTTP/2 组合证据，不改变上述历史读写测试边界。

后续的[路由组件](RAW-TTL-ROUTING.md) 已接入自动发现和跨范围 cursor，并保留单页预算、
每页一致性与未知写不重放规则；不代表实际 TTL split/merge 已完成。

v2 的边界和分页删除实现及新证据见[范围操作记录](RAW-TTL-DELETE-RANGE.md)；上面的
首次公开链路验证目录保留原 v1 记录。

后续[后台 TTL 调度组件](RAW-TTL-MAINTENANCE.md)已有共享 TTL 配额、轮转、取消排空与退避。
仍须完成：跨节点部署资格、TTL resident 优化、整机维护预算、每组 checkpoint/WAL 空间回收、
split/迁移的到期元数据与水位传递、完整历史/TLA/Chaos 验收。旧 split seal 继续拒绝
TTL 组，防止仅复制 value 导致期限丢失。multi-Raft runtime 已存在；上述工作需要在
其完整多组生命周期中验证，不能把本地单组协议测试当作生产可用结论。
