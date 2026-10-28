# 可跨节点读回的 checkpoint 精确意图

2026-10-04；基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`，第二阶段与生产验收仍开放。

Pending owner 原本只有 operation/subject 摘要。确定失败的尝试清除本地 journal 后，
其他节点不能从这些摘要重建完整 manifest 和前驱 generation，因而缺少继续核对
历史和协调关闭发布者的信息。本轮将两者持久化到原 metadata Raft ledger；这是
发布者关闭协议的前置数据，不是关闭完成、负向释放或对象删除的凭据。

## 生产路径和存储

data/metadata checkpoint 在 Pending Published 已确认后，提交 `RecordCheckpoint`
并取得精确已应用回执，随后才登记执行 Build 和进入对象 I/O。失败/未知结果停止
当前调用，保留原 journal，下一轮仍提交相同 identity。正向交接也先确认这份记录，
覆盖旧 journal/已发布 checkpoint 的增量补录。Register/Acquire/Publish、完整后继
覆盖和 executor 本地排空语义不变。

新记录在原 retention namespace 的 tag 8 下，以 Pending owner ID 为 key：

| 字段 | 编码 |
| --- | --- |
| 版本 | `KV9CPI01` |
| root / Pending ID | 32 / 16 字节 |
| 前驱 generation | u64 big-endian |
| manifest 长度 / 原始规范字节 | u32 big-endian / 最多 1 MiB |
| 校验和 | 前述全部字节的 SHA-256 |

总记录上限为 1 MiB + 100 字节。新 ledger request 使用 tag 7；请求总上限为
1 MiB + 256 字节，仅该请求可使用扩大的区间，其余 owner 请求仍限制在 16 KiB。
完整记录通过一个 metadata Raft batch 提交，8 MiB batch 上限不变。

planner 从完整 descriptor 重新计算 operation、subject、Pending/Version ID 和全部
SST resource identity，核对 root 对应的 cluster、scope、generation、owner kind、
anchor 标志和完整 closure。descriptor 必须重新编码后逐字节相同；普通 checkpoint
JSON reader 能接受的空白/未知字段也不能形成第二个意图表示。同 key 已有不同或损坏
字节时拒绝覆盖。owner 或 forward link 不完整也拒绝。

首次记录或重试均要求精确 Pending Published；如果 Pending 已 Quiesced/Released，
则要求精确 Version Published 继续覆盖同一 closure。记录不会创建资源 pin、授权
上传、构造 PreparedFlush、产生 manifest 成功/失败结论或解除任何全局 pin。

## 远端读取与恢复

内部、带节点认证的 CheckpointRetention RPC 新增独立 `Intent(Pending ID)` 查询。
metadata leader 经 ReadIndex 后，在一个 snapshot 内读取记录并核对全部 owner/link；
响应仍绑定 root、query kind 和具体 ID。Intent 与 Owner 响应不能互相代用，Owner
观察的 8 KiB 上限保持不变。没有记录只表示观察不到，绝不表示尝试失败或执行者为空。

这是已知 Pending ID 的点查询，没有新增全历史枚举、任务接管调度或已关闭发布者
集合。新节点可通过既有内部客户端 `on_channel` 读回规范 bytes，再使用
`CheckpointIntent::decode` 核对预期 root/ID；公开 Admin API 没有新增读取端点。
旧节点不认识新请求/query，会拒绝；没有完成旧 binary 混部或完整 backfill。

单条有界不等于整机内存已验收：增加的 descriptor 编解码、RPC 副本和 archive 总量
尚未纳入独立共享字节额度。不同尝试的记录仍会累积，原 48 MiB checkpoint image
上限不变。本轮没有声称磁盘/RSS 已达稳态或压缩历史已经安全。

## 协议定义与证明范围

[TLA+ 模型](../proofs/tla/checkpoint_intent/CheckpointIntent.tla) 定义 durable archive、
protected closure、local journal、回执确认、对象使用和清理；记录提交与收到回执
是两个动作，允许其间丢回执和 restart。模型不包含 DELETE 或释放全局保护的动作。
存储原子性、Raft term/drain 与精确回执、身份摘要无碰撞及 published successor 的
完整覆盖是组合前提；本轮不重新证明这些底层协议。

安全不变式：archive 只能为空或存放该 operation 的规范 descriptor；对象使用或
journal 清理后仍有该精确 archive 和 published protection。记录不可替换，restart
保留 durable projection。条件进展依赖存活的 metadata quorum、稳定后继保护、原
descriptor 可得、重试最终获得回执，以及独立 manifest 协议最终产生 settlement。
无限失败、持续重启或历史缺失不满足这些前提。

[Lean 组件证明](../proofs/lean/checkpoint-intent/Intent.lean) 对任意 descriptor 类型
证明初始化、单步保持、可达状态、清理/使用的 archive 保留、不可替换和 restart
投影，共 **7 条定理**；归纳遍历任意长度的 transition history。四个错误变体
（错误 descriptor、无回执使用、无回执清理、覆盖已有记录）必须使检查失败，另外
拒绝 proof hole 和自定义 axiom。其单 operation 投影对应 TLA+ 的逐 operation 更新；
没有自动提取 Rust，也没有证明 caller/存储/Raft 的完整 refinement。

TLA+ 已通过 SANY 解析及语义检查；**TLC 安全/时序检查仍未完成**，当前 sandbox 的
socket/RMI 限制沿用此前边界，未重复启动已知无法运行的 TLC。Lean 组件定理不能
替代完整 TLA/TLC、协议组合与 Chaos 验收。

## 验证

[独立证据包](raw-ttl-intent-validation-20261004/manifest.json)保存命令、日志、证明输出
和源码摘要；历史证据不修改。

- metadata 测试验证缺少 Pending、仅 Held、错误 root/operation/subject/epoch/kind/
  anchor、损坏数据和后继缺失均拒绝；同一规范记录幂等，正向交接后仍可读取。
- 大于 16 KiB 的 descriptor 使用独立有界路径并逐字节读回；截断、校验和错误、
  长度/前驱/owner 篡改、非规范 JSON 和超限输入拒绝。新 wire 测试核对独立查询、
  认证、响应相关性、空值与大小边界；不扩张原 Owner 观察大小。
- 真实三副本、分离的 metadata/data leader 和认证 HTTP/2 duplex 验证：上传/交接
  现有丢回执测试从八处扩展为十处，包括首次 RecordCheckpoint 和交接前幂等确认。
  前五处未知结果均不执行 PUT；随后沿原 journal 重试并验证 canonical intent。
- 已有确定失败的旧 Pending 测试现在在清除本地 journal 后经远端读回原 intent；
  停止实际驱动、重开 metadata WAL 后，descriptor、前驱 generation 和 Published
  Pending 仍保留。正向交接的恢复测试也核对 Released Pending 对应的 intent。
- 自动 data worker 的关闭失败测试保持原故障位置：新增归档调用后，执行 Quiesce
  位于调用 14；先前选中调用 13 的尝试在 adoption archive 确认处停止，随后修正
  注入位置，原“新写入不能越过未知关闭”的断言保留。另一个正向 Share 故障从 7
  移到 9。调整的是调用坐标，没有放宽待验证行为。

common/meta **192 通过、1 忽略**；TTL 组合 **25 通过**；内部 retention RPC **6 通过**。
严格 workspace Clippy、格式与 diff 检查通过。网络为 duplex/InProcHub，对象为
MemoryObjectStore/InMemory；不是完整 NodeRuntime、真实 TCP/MinIO 或 process-kill。

下一步仍需持久关闭新的 executor 准入、索引并核对全体执行者、精确处理未知进入和
崩溃遗留记录，再与实际 manifest 历史组合，才可能释放负向 Pending。Reader/旧
Version、继任恢复锚点、迁移、TTL split/merge、部署时钟及所有生产门槛继续开放。
