# Raw TTL 分页 DeleteRange 与完整范围边界

2026-10-04；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
这是 [第二阶段](RAW-TTL-PHASE2.md) 的公开范围操作组件；完整阶段和生产验收仍未完成。

## 协议与边界

公开 RawTtl/ActivateRawTtl 协议版本现在为 **2**。能力确认和数据响应都必须返回该版本；
v1 回包不能授权新操作，新服务拒绝 v1 请求而不进入数据 backend。point stream 继续
使用独立 opcode 6，不降级到旧 Raw。此次没有新增 Raft 日志格式，store/wire 仍为 V4；
删除复用现有 `RawTtlAction::Write` 中的 Delete mutation。

范围起点、终点现在覆盖完整 catalog 的 **65536 字节**界限。起点/续页位置另允许最大
长度 key 后追加零字节形成的 65537 字节位置；终点仍为半开范围的排他上界。普通 Put/
Get/Delete 的用户 key 仍最多 4096 字节，value 仍最多 65536 字节，没有扩大数据负载限制。
Scan 的返回字节预算现在为最长续页位置留出空间，整体数据 RPC 仍受 1 MiB 限制。

长 binding 的激活和内部目录路径也已补齐：

- `DataRange::MAX_ENCODED_BYTES = 117 + 2 * MAX_BOUND`，最大为 131189 字节。
- 公开激活请求上限为该长度加 64 字节协议空间。
- 内部目录请求上限再加 1024 个 u64 成员的最大编码空间及 64 字节 envelope；成员数量、
  身份和响应 256 KiB 限制不变，4 个 blocking 查询配额也保持不变。
- CLI 配置文件上限从 64 KiB 调整到 **1 MiB**，足以容纳最大 binding 的 JSON 字节数组；
  文件仍按有界读取处理，不放开输入内存限制。

## DeleteRange 的确认语义

`Operation::DeleteRange { start, end, limit }` 每次处理一个 range 内的**一个有界删除页**。
`limit` 为 1..256；不是整个范围的事务，也不会在服务端循环到范围结束。

1. 用 data leader 的 ReadIndex 和同一不可变 snapshot 检查 TTL 模式及精确 range。
2. 扫描 Default CF 的物理 Raw key，不按 TTL 过滤。因此永久值、未到期值和已过期但
   尚未回收的值均可被显式删除。无需时钟采样，时钟未配置或已拒绝时仍能执行。
3. 至多选择 `limit` 个 key，另限制 Delete mutation 的编码字节为 `1 MiB - 256`。
   迭代只多观察一个可能因字节预算未被选中的 key，并保留该 key 的包含式游标。
   不收集整个范围，不累计 value 内容；engine 迭代器仍可能逐项读取 value，这不是
   整个 engine snapshot 或底层 I/O 的总预算证明。
4. 将选中的 key 放进一个已有 TTL Write 命令，使用已提交水位，不读取 apply 本机时间。
   有序 apply 原子删除每个 key 的 Default CF 值和 Write CF 到期记录，并保存 applied
   position。提交前继续检查总 deadline，proposal/apply 沿用现有 scope fence。
5. 只有精确 `(term,index)` 回执确认后，才返回已处理数量和续页位置。空页不追加日志，
   返回 `processed_keys=0, applied=None`；非空页必须带非零 applied receipt。

`processed_keys` 是本页已经应用的 Delete mutation 数，不是执行瞬间“实际存在并发生
变化”的计数。显式 DeleteRange 与后台 TTL Cleanup 不同：本页在快照中选中的 key 若
在 apply 前被覆盖，删除仍按日志顺序删除该 key 的当前版本。后续写入可重新创建它。
扫描之后插入到已经越过的位置的 key 不承诺被此次分页遍历删除；没有范围 tombstone
或跨页 snapshot 保持，也不承诺一次全局线性化的范围删除。

每次 RPC 至多一个提案，服务端不会隐藏已提交的前缀页。每个已确认页都有自己的
`Reply::Deleted { processed_keys, applied, next }` 和路由报告中的 scope。
不同 group 的日志 index 不能直接比较；调用方应保留各页报告，而不是合并成一个
虚构的全局回执。任何结果未知的页都停止，不自动重提，也不产生新的成功续页位置。
未知页可能随后应用，客户端超时不撤销已 append 的命令。

## 跨 range 与 CLI

`ScopedTtlClient` 要求整个请求范围落在指定 binding 内；`RoutedTtlClient` 将每页上界
裁剪到当前 range，沿用 root/namespace 固定、能力确认和数据调用至多一次的规则。
到达当前范围末尾时，`delete_continuation` 指向下一范围；空范围也可以推进，不误报全局
结束。下一页重新发现当前位置的 owner，不把旧 group/epoch 固定在游标里。

扫描和删除使用不同的续页字段和调用：

| 操作 | 报告字段 | Rust 调用 | CLI 输入字段 |
| --- | --- | --- | --- |
| Scan | `continuation` | `continue_scan` | `cursor` + `limit` |
| DeleteRange | `delete_continuation` | `continue_delete_range` | `delete_cursor` + `limit` |

删除游标 `DeleteCursor` 保存查询 root/tenant/keyspace、原始 start/end 和 next；可用
Serde 保存和恢复。它是调用者的查询状态，不是签名授权或崩溃后自动重试凭据。

首个删除页：

```json
{"id":1,"operation":{"kind":"delete_range","start":[],"end":[],"limit":128}}
```

成功后，若报告的 `delete_continuation` 非空，把它原样放入下一条输入：
`{"id":2,"delete_cursor":<返回的对象>,"limit":128}`。
`operation`、`cursor`、`delete_cursor` 互斥；混用会在发送请求前拒绝。
明确选择下一页只继续尚未覆盖的位置，不是重放前页。收到 UnknownWrite 后必须保留
当前页的不确定状态，不能仅凭超时重发旧起点或假定可以跳到后面。

## 验证与未完成项

本轮新增 9 项专项测试，并扩充已有三副本实际链路：

- txn 规划器：过期/永久 key、prefix key、行数和编码字节停止点、最大内部 key，
  失败不改变原快照。字节边界不会跳过未选中的 key。
- 真实单副本 Raft/WAL：两 CF 同时删除、准确回执、时钟拒绝时删除、空页无提案、
  页间实际 WAL 重开与续删；unary/stream 均接受完整 catalog 边界。
- 路由控制：完整长边界跨空范围、不同删除游标、已确认页后遇到未知页停止、
  取消后 admission 保持；跨范围返回页为受控 fixture，不作为 TTL split/merge 证明。
- 回包控制：缺失/零位置、错误 scope/版本、错误数量和不前进游标不能伪装成成功。
- 公开激活和内部目录 handler：最大规范 binding 可通过预算；旧公开 v1 请求不能
  进入 v2 数据 backend。这是 handler/codec 验证，未运行旧版本二进制升级。
- 既有三副本 fixture 在真实格式激活后，通过自动路由完成 TTL 范围删除；确认所有
  三个副本应用后，逐副本检查 Default/Write CF 均无原值及期限。复制为 InProcHub，
  控制/数据 RPC 为 Tokio duplex 上的真实 HTTP/2，未绑定 TCP 或启动完整 NodeRuntime。

验证结果：common 53、txn 25、Raft 本地 302、server 专项 36 项通过；Raft 另有 8 项
MinIO 忽略、37 项 socket 测试显式过滤。CLI 新增 3 项及原有 4 项实际二进制配置/输入
检查通过，均未发起网络请求；全 workspace/全 target 严格 Clippy（含 tracing）、格式
和 diff 检查通过。原始命令、日志和源码摘要见
[验证记录](raw-ttl-delete-validation-20261004/manifest.json)，历史目录保持原样。

后续[后台时间推进与条件清理组件](RAW-TTL-MAINTENANCE.md)已接入。下一步仍需整机维护预算、
每个 group 的 checkpoint/WAL 空间回收、
TTL resident 优化及 split/迁移/安装/新增副本/恢复的格式互通。独立并发历史检查、完整
TLA/TLC、真实多组和网络故障、部署时钟资格与 Chaos 验收仍未完成；逻辑删除没有证明
物理空间已回收，第二阶段继续保持开放。
