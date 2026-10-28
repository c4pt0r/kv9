# Manifest 编码在扩张 buffer 前拒绝超限

2026-10-04；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
接续 [SST 借用校验与恢复](RAW-TTL-SST-VIEWS.md)，修正 checkpoint manifest 的另一处
内存边界：原代码先 `serde_json::to_vec` 编码全部 JSON，再复制到含 magic 的 Vec，最后
才检查 **1 MiB** 上限。超限错误不能限制这之前已经发生的分配。

现在 magic 和 JSON 直接写入同一个 `BoundedOutput`。每次写入先检查长度加法和整个
frame 的上限，再扩张容量、追加字节；容量按倍增增长但请求值最高为 1 MiB，避免普通
Vec 的额外预留跨过边界。超限 chunk 不追加、不申请更多输出容量，序列化立即返回
原有的 manifest-too-large 错误，不继续遍历后面的字段，也不返回部分编码结果。

编码字段顺序、转义、magic、对象格式和 `change_id` 算法不变；**恰好 1 MiB 接受，
多一个字节拒绝**。该限制覆盖 magic，不只是 JSON。planning、journal、restore 和
Prepared 恢复继续通过同一个 `CheckpointManifest::encode` 使用此路径。

[源码摘要、命令与原始日志](raw-ttl-manifest-encoding-validation-20261004/manifest.json)
另存。新增五项测试覆盖：

- 三 CF、二进制 footer key、转义/Unicode cluster 和整数边界的字节兼容、decode 和
  三个 generation 的 change ID 一致。
- 有效 manifest 的实际 1 MiB 边界，以及少一字节容量、多一字节输入的拒绝行为。
- 多次扩容后输出容量不超过额度；超限写入不改变容量、指针、长度或已写内容。
- 大字段超限后不访问后续字段，避免完整序列化再检查。
- 实际 upload 规划中 SST 本身能通过 image 准入，但 footer key 展开成 JSON 后超限；
  此时没有远端对象，image reservation 归零，随后正常的小 checkpoint 可上传成功。

Engine **193 通过、25 忽略**，本地 Raft **327 通过、8 忽略、37 过滤**，TTL 激活及
三副本 checkpoint 组合 **19 通过**；严格 Clippy、格式和 diff 检查通过。Clippy 首次
发现新增测试中的手动整除判断，改为标准方法后重跑通过。没有真实 TCP/MinIO 或完整
NodeRuntime 端到端验收；集合与前序测试重叠。

这是每次编码输出的大小边界，不是整机共享内存额度或 manifest 历史数量上限。已有
输入结构、decode 后的对象、allocator 元数据/重分配期间的物理峰值、SST writer、
常驻索引和 snapshot 保留仍须分别处理。本轮没有 RSS/吞吐测量，没有解除 48 MiB
全量 image 限制，也没有关闭 GC、TTL 范围转移及第二阶段其他生产验收项。
