# Checkpoint 对象 payload 配额与读回上限

2026-10-04；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
本轮在[共享任务配额](RAW-TTL-SHARED-MAINTENANCE.md)之后，为 runtime 的对象客户端
加入共享 payload 字节准入，并修复 SST GET 在检查大小前完整收集响应的问题。
这是[第二阶段](RAW-TTL-PHASE2.md)的资源控制组件，完整生产验收继续开放。

## 配额位置与配置

`prepare_remote` 在创建对象 worker 前解析 `KV9_CHECKPOINT_IO_BYTES_PER_SEC`：默认
**67,108,864 bytes/s（64 MiB/s）**，接受不小于 **16,777,216 bytes/s（16 MiB/s）**
的 `u64` 整数。非法、溢出或非 Unicode 值启动拒绝。最低值避免人为把最大合法单个
image 的一次 payload 等待拉长至默认对象 deadline 之外；这不是完整 pass 时延保证。
burst 固定为 **50,331,648 bytes（48 MiB）**，容纳现有最大合法 SST，不扩大 image 上限。

同一 runtime 的 metadata/data checkpoint、读回验证、对象恢复与其他复用 uploader 的
访问使用同一 ObjectIoBudget。它位于 MinIO worker 执行实际 S3 PUT／GET 方法之前，
不为每组创建新的 credit。单独使用 `MinioObjectStore::connect` 的外部调用者保留原有
无此限流器的行为；runtime 使用 `connect_with_io_budget`，状态可观察。

节点状态中的 `object_io_budget` JSON 提供配置速率和 burst、获准的 PUT/GET 字节数与
操作数、当前/峰值等待数、准入前取消数、超 burst 拒绝数及累计等待微秒。获准字节数
包含失败请求，不能用作成功上传量；未配置该客户端时字段为 `null`。

- PUT 按即将提交的 body 长度扣费；失败或结果未知不退回已扣 credit。
- 显式有上限的 GET 按允许的最大 body 长度扣费，缺失、较短响应或拒绝也不退款。
- 幂等 PUT 返回 AlreadyExists 后的内容比较 GET 单独扣费；不同内容仍返回原有
  `ObjectContentMismatch`，不能以限流或大小不同掩盖不可变对象冲突。
- 没有 descriptor 的普通 GET 先 HEAD 取得长度，再取得 credit 并 GET；响应长度仍
  要重新核对。HEAD 不是数据、发布或保留授权。
- 使用配额的 S3 客户端设置 `max_retries=0`，避免 SDK 在计费边界之下重传 body。
  上层继续沿原有持久 journal 和未知结果协议恢复；没有新增隐式 mutation 重放。

异步 mutex 只串行化 credit 等待和扣除，网络调用前释放。等待在原有 worker wire
deadline 内；超时丢弃等待 future，释放等待位置，不扣未获准 credit。超过 burst 的
请求立即拒绝，不进入永远无法满足的等待。时间采用单调 Instant，积分用饱和 `u128`，
空闲后最多仍只有一个 burst。服务率控制的是**应用 payload 的保守准入计量**，不是
实际网卡流量计；HTTP/TLS 头、HEAD、LIST、TCP 重传和服务端提前发送的拒绝响应不在该
字节计量中，不能据此宣称网络出口瞬时硬限速。

原有任务 permit 仍覆盖等待和实际同步操作，不能因取消 future 提前释放重 I/O 槽位。
配额未改变共识确认、owner 登记、上传读回验证、候选历史保护或删除授权。

## GET 收集边界

ObjectStore 增加对象安全的 `get_bounded`。checkpoint 上传后的校验 GET、恢复
prepared 能力以及 restore 都以实际 plan/manifest 的大小调用它。

生产 MinIO 后端先检查响应声明大小与完整对象 range，再逐 chunk 收集。若声明大小
超过上限，不轮询 body；若后续 chunk 超限，在追加到结果 Vec 之前拒绝并停止轮询；
EOF 字节数必须与声明相符。之后仍执行完整 SST 哈希、大小、CF、范围、条数检查。
超大对象不能因限流器已经扣费而被当作合法 SST。

MemoryObjectStore 在 clone 前检查；第三方 ObjectStore 的默认方法仅在其旧 `get`
返回后检查。该兼容默认方法不证明第三方实现的分配有界。上游网络栈的单 chunk 分配、
队列中的 PUT 副本、完整 image 与 resident index 也没有由本次改动形成整机内存预算。

## 验证

[命令、原始日志与源码摘要](raw-ttl-object-io-validation-20261004/manifest.json)
另存，前序验证文件不改写。

新增 7 项 engine 测试与 1 项配置测试：

- 共享速率、burst、取消等待不扣 credit，零值/超 burst 拒绝及 `u64::MAX` 速率算术。
- 真实对象 worker 的普通 PUT、幂等 PUT 比较读、有上限 GET、HEAD 后 GET、内容冲突
  和过大读回都按预期计费；错误不退款，原对象内容保留。
- 两个独立对象 worker 共用一个 budget，累计 512 字节、初始 64 字节 burst、
  1000 bytes/s 时，完成时间至少 448 ms；没有每 worker 偷增一个 burst。
- 声明过大时不轮询 body；流中途超限时不轮询后续 chunk；截断 body 拒绝。
- 实际 worker 的配额等待超时后，checksummed pending 文件字节不变；关闭并重建
  journal 和客户端后，沿原 manifest/generation，使用同一个 budget 完成上传读回，
  得到真正 PreparedFlush，再 restore 三个 CF。该测试不假称已提交 ManifestChange。
- 配置边界与 budgeted SDK retry policy 单独检查。

对象 worker 测试通过 upstream InMemory transport 驱动生产 `drive_jobs`、wire deadline、
PUT/GET 和字节准入，不经过 TCP，也不构成 MinIO 持久性证据。三副本公开激活和自动
checkpoint 回归仍使用 MemoryObjectStore；本轮没有把这两套证据拼成完整 NodeRuntime
和真实 MinIO 的端到端验收。engine、Raft 本地回归、server 激活组合及含 tracing /
checkpoint-testing 的严格 Clippy 结果见 manifest，集合重叠且忽略项明确保留。

## 仍需完成

WAL/本地磁盘字节、临时内存与 CPU 成本、物理垃圾债务和持续负载下公平性尚未验收。
自动 split、迁移/安装/启动恢复等路径虽然可复用对象字节配额，其完整 job/控制扫描
预算仍需统一。低速、排队、网络耗时和 ledger 调用可能耗尽 pass deadline；原 pending
继续恢复，不能把默认速率视作每轮必定完成的承诺。

Reader/旧 Version/负向 Pending 的安全释放、对象 GC、manifest/configuration/owner
历史有界、全量 48 MiB image 限制、resident TTL 优化及范围转移仍开放；实际 MinIO、
完整 NodeRuntime、部署时钟资格、独立时间历史、TLA/TLC/Chaos 与生产门槛继续保留。
