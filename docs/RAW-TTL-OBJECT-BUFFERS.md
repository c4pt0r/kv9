# 对象 worker 共享缓冲额度与引用生命周期

2026-10-04；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
本轮继续[checkpoint image 配额](RAW-TTL-IMAGE-BUDGET.md)，覆盖对象客户端持有的 PUT
payload 与 GET 收集 buffer。两份配额独立：image 表示计划对象集合，buffer 表示客户端
额外持有的序列化数据。它们的合计不包含全部内存，不能当成节点 RSS 上限。

## 实际缓冲所有权

原 PUT 先把调用者 slice 复制进 worker job，执行时再 clone 一次 Vec 交给 SDK。
现在先取得 buffer 额度，再复制一次，并用 `Bytes::from_owner` 将 Vec 和 reservation
绑定到同一个共享 owner。队列、比较路径与 SDK 的 PutPayload 只 clone Bytes 引用，
不再复制完整 payload。owner 中先释放 Vec 再释放 reservation；最后一个引用消失才
归还额度。调用者超时、异步 future 取消或 PUT future 返回，都不能单独证明 SDK 已
释放引用，因此不在这些事件上手动归还额度。

GET 收到响应头后先检查 descriptor 上限与完整 range，按声明长度取得 buffer 额度，
再分配精确容量的收集 Vec。逐 chunk 检查上限和声明长度，在追加前拒绝越界；截断、
错误、取消都归还该 buffer。完成后同样将 Vec 和额度装入 Bytes owner，reply channel、
调用者的 clone/slice 都保留额度。上游 HTTP/TLS 解码及单个网络 chunk 的分配不由这个
Vec 的额度覆盖；GET 请求已发出也不等于已经取得收集 buffer 的额度。

ObjectStore 增加对象安全的 `get_shared_bounded`。checkpoint 上传后的读回、Prepared
能力恢复及 SST restore 验证全部调用这个共享接口，在哈希/SST 校验结束前保持 owner。
SST parse 产生的 key/value 副本和恢复 MemEngine 仍是另一类尚未预算的成本。

原 `get` / `get_bounded` 的 Vec 接口保留兼容：它们从共享结果复制出一份**未计费的 Vec**
再交给调用者。第三方后端的默认共享接口也只是包装旧返回值，没有这个预算。
因此本次证据不证明所有旧库调用或任意 Bytes→Vec 转换都受配额限制；生产 checkpoint
已迁移到共享接口，测试让旧 GET 直接 panic，以检查这三条路径不会退回旧接口。

## 配置、压力与指标

`KV9_OBJECT_BUFFER_BYTES` 默认 **100,663,296 bytes（96 MiB）**，runtime 最低也是
**96 MiB**，允许在没有其他占用时同时容纳最大 48 MiB PUT 和幂等比较 GET。配置在
创建对象 worker 前解析，非法值拒绝启动。这个最低值不是整轮或排队时必定成功的保证。

runtime 用 `connect_with_budgets` 创建一个客户端，供 metadata/data checkpoint 和恢复
共用。独立的 `connect` / `connect_with_io_budget` 现在也有每客户端默认 96 MiB buffer
额度；不同客户端只有显式传同一 Arc 时才共享，不存在自动的进程级全局单例。

PUT 在复制/入队前进行非阻塞准入；失败可明确分类为 AdmissionRefused，不发送 PUT。
GET 收集额度不足时返回错误，不轮询 body。队列已满、过期任务拒绝、reply 被丢弃、
错误或取消通过共享 owner 自动释放；额度不足不会阻塞等待其他已持有 buffer 的请求。
持久 pending、owner 登记、未知结果、prepared 验证和回收授权沿用原协议。

节点状态 `object_buffer_memory` 输出 `limit_bytes`、`retained_bytes`、`peak_bytes`、
`active_buffers`、获准与拒绝次数；没有 MinIO uploader 时为 null。clone/slice 不重复
计费，仍持有完整底层分配的额度。字段衡量客户端 owned payload buffer 容量与预留，
不包括 Bytes/Vec 元数据、allocator 开销、协议/TLS 内存、第三方 SDK 自行产生的其他
副本、LIST 结果、兼容 Vec 返回值或 resident/恢复索引。

## 验证

[命令、原始日志与源码摘要](raw-ttl-object-buffers-validation-20261004/manifest.json)另存。
新增五项 engine 测试和一项 runtime 配置测试：

- 实际 worker 使用 upstream InMemory transport；SDK 成功返回后，它仍保留原 payload
  Bytes，额度也继续保留。幂等 PUT 的比较 GET 另占 buffer，GET clone/slice 持有完整
  额度，删除测试后端对象并释放最后一个结果引用后归零。这个 transport 刻意保留
  payload，是引用生命周期证据，不是远端 MinIO 的内存或持久性证据。
- 两个实际 worker 共用 128 字节，各上传 64 字节并由后端保留；第三笔 PUT 在入队前
  被拒绝且没有对象副作用，释放一个持有者后恢复。
- 收集 buffer 不足时不轮询 body；截断与超限响应拒绝，额度没有泄漏。
- 生产 MinIO worker 在确定性 deadline barrier 处持有真实 PUT job：调用者已超时，
  64 字节仍计费并阻止下一笔复制；释放 barrier 后，worker 拒绝过期 job 才归还额度。
  此测试只构造客户端，过期 job 不发网络请求，不需要 TCP/MinIO 服务。
- 上传读回、Prepared 恢复、三个 CF 的 restore 均通过共享读取接口；旧 GET 被设为
  panic。九次真实读取在同一小预算下完成，结束无保留 buffer。
- 默认值、最低配置值、无效值与整数溢出边界。

Engine、Raft 本地回归、三副本激活/自动 checkpoint 组合及严格 Clippy 继续验证；
集合与前序证据重叠，忽略项保持明确。共享 owner 测试和 MemoryObjectStore 三副本测试
没有拼成完整 NodeRuntime 与真实 S3/MinIO 的端到端验收。

完整 codec/恢复临时内存、CPU、本地 WAL/磁盘字节、长期公平性与物理债务仍未验收。
Reader/旧 Version/负向 Pending 生命周期、对象 GC、协议历史有界、48 MiB 全量 image
限制、TTL resident 优化与范围转移、恢复锚点、部署时钟资格、独立时间历史、真实网络/
MinIO/旧二进制升级、完整 TLA/TLC/Chaos 与第二阶段生产门槛继续开放。

同日后续：[SST 借用校验与直接恢复索引](RAW-TTL-SST-VIEWS.md) 已去掉上述生产路径中的
解析 key/value 副本和完整 restore WriteBatch，且在迭代期间保持原 buffer owner。
SST 写入端、最终常驻索引及完整 RSS 仍待处理；本页原始证据 manifest 保持不变。
