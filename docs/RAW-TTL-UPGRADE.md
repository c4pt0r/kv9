# Raw TTL 格式升级隔离

2026-10-04，本地工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
这是 [TTL 第二阶段](RAW-TTL-PHASE2.md) 的升级组件进展，公开 TTL 仍未启用。

后续 2026-10-05：当前 owner/service floor 为 **V9**，见
[Reader 生命周期升级隔离](RAW-TTL-READERS-UPGRADE.md)。此前 V6 连接未绑定导入的
checkpoint 与恢复，V7 支持大镜像，V8 增加 [capture catalog 隔离](RAW-TTL-CAPTURE-UPGRADE.md)；
V9 排除可绕过 Reader/Bootstrap 准入、排空与镜像读 gate 的旧进程，并补齐冷启动检查。
下文保留最初组件记录，完整旧 binary/部署升级、容量资格仍未完成。

## 已实现的边界

- store lifecycle 当前格式为 `KV9LIFE4`，仍是带 checksum 的 97 字节记录。
  V1/V2/V3 可以读取并升级；node、incarnation、phase 和 root 均不改变。
  发布顺序为写临时文件、文件 sync、rename、目录与祖先 sync。
  任何步骤报告失败都会使当前 guard 拒绝后续授权；重开先稳定恢复出的文件和目录，
  再允许重试。已发布 V4 后，旧 V3 reader 拒绝打开该 store。
- 内部服务路径升级为 `kv9.raft.v4.Kv9Raft`。metadata、data、discovery、registration
  和 endpoint confirmation 均无旧代回退。V3/V4 不能混用通信，保持已有的停机全节点
  升级要求，不支持混合版本滚动升级。
- `NodeRuntime` 在启动 owner 前完成格式发布；独立的 offline snapshot installer
  也要求当前持久格式。CLI 安装入口先验证 store identity，再发布格式 fence，
  然后才能打开 group 文件。此检查覆盖安装和 runtime adoption 的公共入口。
- `StoreGuard::raw_ttl_format_fence` 只为当前格式、Active、健康且匹配 root 的 store
  产生不可随意构造的证据。Bound/Prepared、旧格式、错误 identity 或发布失败都不能产生。
  该证据说明旧 owner 被持久格式排除，不代表组成员身份、TTL 激活、时钟健康或存储健康。
- 新只读 RPC `ConfirmRawTtlFormat` 要求经过认证的调用者、匹配的 root/node/
  cluster/incarnation、本地 Raft receive 授权及上述格式证据。
  客户端严格匹配全部身份和格式版本 4，在一个总 timeout 内完成连接及 RPC，
  控制消息编码/解码上限为 256 字节。超时、UNIMPLEMENTED、旧地址/替换磁盘、未知版本
  或矛盾回包均为未确认，没有 fallback。该 RPC 不修改 catalog 或 group。

升级后不能直接切回旧二进制；旧 reader 的拒绝正是 TTL 元数据不被绕过的边界。
手工删除/改写 lifecycle、克隆整个 store 身份，或不遵守目录锁的外部写者不在此契约内。
网络鉴权继承当前共享 token 的信任边界；这不是新增 TLS 或对恶意持 token 节点的证明。

## 全组激活组件进展

单个 `ConfirmedRawTtlFormat` 不能授权 `RawTtl::Enable`。
[全副本激活组件](RAW-TTL-ACTIVATION.md) 现已接入下列 1–3 项的运行路径和本地测试：

1. 从该组 Raft 的同一个配置快照确定 **全部**参与副本，包含 voters、
   voters_outgoing、learners、learners_next；不得只取多数。当前
   `RaftPeer::membership()` 只返回 incoming voters/learners，不能直接用于这个门槛。
2. 从权威目录解析每个 node 的确切 store incarnation，取得逐副本确认；
   重复确认不能增加覆盖率，缺一项或错误身份都不能继续。
3. 把这批确认绑定到组的 root/creation/range epoch、当前配置及提案时的 leader term。
   在提交 Enable 的同一受保护入口重新检查配置，处理尚未 apply 的 ConfChange 和
   收集确认期间的成员变化；不能把一次过去的全组结果永久缓存为启用权限。
4. 以持久状态区分新激活、恢复和重试；新加入副本的安装/能力必须继续满足 V4 门槛。
   公开 API、客户端、TTL 时间观察和读取协议仍须按完整阶段清单接入。

第 4 项以及真实跨节点组合验收仍未完成。嵌入式 Runtime 控制接口从 catalog 解析确切
身份并并发确认，专用 driver 入口在 append 锁内重新检查；普通提案入口已禁止直接
Enable。公开端点仍没有 `Enable` 或 TTL 请求字段。

## 首次 V4 组件证据

- common 全部 53 项通过，其中 lifecycle 8 项覆盖旧格式升级、发布故障点、
  格式证据条件以及恢复。测试中的旧 V3 codec 是从上述基线复制并核对的真实代码片段；
  它不是一次旧服务二进制启动/降级演练。
- 新格式 RPC 4 项测试通过，包括实际生成的 tonic 分发器对三代旧路径的拒绝、
  当前路径的鉴权、请求/回包身份替换，以及无效请求在连接前拒绝。
  handler 和 dispatcher 测试不使用 socket；不能替代 HTTP/2 端到端验收。
- Raft 本地回归 289 项通过，含 offline installer 在旧格式下不创建 group 文件的测试。
  37 项网络相关测试显式过滤，8 项 MinIO 测试保持忽略。
- workspace 全 target 编译及严格 Clippy（含 write-stage-tracing）通过。

真实旧二进制对照、跨节点 V3/V4 互斥、超时/重连/地址替换的 HTTP/2 测试，以及
与全组激活控制器组合的 TLC/Chaos 仍未验收。已有 socket 权限限制没有被当作通过。
可在允许监听的验证环境执行实际二进制路径检查：

```sh
cargo run --offline -p kv9-raft --example check-wire-floor -- OLD_ADDR NEW_ADDR 3
```

本轮原始日志和源码摘要见 [验证记录](raw-ttl-upgrade-validation-20261004/manifest.json)。
