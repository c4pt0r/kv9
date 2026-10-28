# 共享 checkpoint image 字节准入

2026-10-04；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
在[消除 journal 重复副本](RAW-TTL-CHECKPOINT-MEMORY.md)之后，给 plan/pending 持有的
序列化对象集合加入共享字节准入。它不是整个 checkpoint 操作的内存上限，也不是 RSS
上限；[第二阶段](RAW-TTL-PHASE2.md)的完整资源与生命周期验收继续开放。

2026-10-05 后续：[按 SST 落盘与上传](RAW-TTL-SPOOLED-CHECKPOINT.md) 已替换两个生产
worker 的全 image 持有，uploader journal 逐 SST 校验/恢复。下面记录原版整体 image
准入及其历史证据；当前逐对象计费、兼容接口和剩余上限见后续文档。

## 配置与实际接线

`KV9_CHECKPOINT_IMAGE_BYTES` 默认 **100,663,296 bytes（96 MiB）**，runtime 接受不小于
**50,331,648 bytes（48 MiB）**的 `u64` 整数；非法、溢出和非 Unicode 值启动拒绝。
配置在 metadata pending 加载之前解析，因此未配置远端存储时也不能用非法值启动。
最低值容纳一份最大合法 image；不承诺同时容纳多个 image 或幂等比较时的新旧两份。

metadata 的 startup journal、其自动 worker、data-group 自动 worker、配置对象访问时的
数据组 pending 恢复与未激活组检查共用 uploader 关联的 ImageBudget。生产 runtime 的
各组复用同一 uploader。不同 uploader 也可显式注入同一 Arc budget，测试覆盖共享计数。
ImageBudget 不持有跨文件、对象或共识调用的锁。

- 新计划先遍历冻结 snapshot 的借用 entry，按实际分片和 SST 编码计算完整对象字节，
  再申请额度，获准后才复制 key/value 和构造对象。预检增加一次 O(N) 只读遍历，不在
  engine 写锁内执行；未给 CPU 成本或快照引起的写时复制建立预算。
- pending 先验证有界 manifest，在分配对象 buffer 前按其总 SST 大小申请额度。
  全部对象和完整 journal 校验通过后，额度与 PendingFlush 一起返回。
- 非 Clone 的 reservation 随 PendingFlush 转移到 PlannedFlush，再由实际同步上传过程
  持有到整个上传/读回成功或错误返回。丢弃 dispatcher、请求 future 或 journal 句柄
  不释放仍由实际 worker/plan/pending 持有的额度。
- 完成 prepared 转换后不再保留对象集合，因而释放 image 额度；prepared descriptor
  不被算作 image。错误、损坏 journal 和中途失败同样通过 RAII 归还额度。
- 准入不等待：不足时返回错误，由后台原有退避和原 journal 恢复。启动恢复不足时
  返回启动错误，不把有效 pending 当作不存在。没有持有一份 image 并等待第二份的
  阻塞死锁，也不凭配额结果修改 owner、candidate pin、manifest 或回收权限。

静态 `RemoteUploader::plan` 和独立 `FlushJournal::new` 保留原来的不计费接口，用于
未配置共享预算的调用者；runtime 使用实例 `plan_checkpoint` 和 `uploader.journal`。
普通实例 `upload` 也使用计费规划；`upload_planned` 对此前未计费的计划在第一笔 PUT
之前补充准入，但不能追溯约束调用者已经分配的缓冲。来自另一 budget 的已计费计划
继续持有其原 reservation。不能因此宣称所有独立库调用都受 runtime 的同一个上限控制。

## 状态与限制

节点状态 `checkpoint_image_memory` 输出配置上限、`retained_bytes`、峰值、活跃 image、
获准次数和拒绝次数；没有 uploader 时为 null。`retained_bytes` 包含已经为正在规划的
image 预留的额度；整个上传结束前保守地持有完整 image 大小。它不表示成功上传字节。

计量对象是 plan/pending 的 SST 序列化字节，不包括 Vec 额外容量和对象描述符、SST
writer/parse 的条目结构与临时副本、MinIO 队列/PUT/GET 缓冲、恢复 batch/MemEngine、
resident index 或被快照保留的旧版本。对象 worker 超时后可能仍持有的独立请求副本也
不在这个计量内；它们不能借用已释放的 image reservation 来宣称被同一上限覆盖。
这些成本需要继续接入临时内存预算，并通过持续负载和 RSS 观察验证。

## 验证

[原始日志、命令与源码摘要](raw-ttl-image-budget-validation-20261004/manifest.json)单独保存。
新增五项 engine 测试、一项配置测试、一项真实 metadata worker 组合测试：

- 两个 uploader 使用同一 68 字节额度，各持有一份真实 34 字节 SST；第三份拒绝，
  丢弃持有者后可再次获得，结束无泄漏。
- 跨真实 8 MiB 分片边界和两个 CF 的预检结果等于生成对象的实际字节总和。
- 配额不足时 pending 文件保持逐字节不变；释放 plan 后重开原 journal，按同一
  manifest/generation 上传、验证并升级 prepared，额度归零。
- 最终 journal checksum 错误和真正创建对象后丢 PUT 回执均归还额度，错误不清空计划。
- 未计费兼容计划在第一笔 PUT 前受实例 uploader 配额控制，拒绝时未创建对象。
- runtime 配置默认值、最小值、非法值和整数溢出边界。
- 实际 metadata leader worker 因另一个真实 image 占满额度而拒绝规划；没有 journal、
  对象或 publication。释放后走真实 owner 登记、上传、ManifestChange、采用及 journal
  结算，额度归零。测试使用较小的显式 budget，生产环境配置下限仍为 48 MiB。

既有双 data-group 阻塞 PUT 测试还验证：两个实际 worker 持有两份 image，TTL 在三个
副本清理后，取消 dispatcher 的排空等待期间额度不减，两个 PUT 真正返回后才归零。
同节点 metadata worker 复用该 uploader。组合使用 InProcHub、HTTP/2 over Tokio duplex
和 MemoryObjectStore，不构成完整 NodeRuntime/TCP/MinIO 持久性或进程 kill 验收。

共享维护的其余路径、本地 WAL 字节、CPU/完整临时内存、长期公平性和物理债务仍未验收。
Reader/旧 Version/负向 Pending 释放、对象 GC、历史有界、48 MiB 全量 image 限制与驻留
优化、TTL 范围转移、可转移恢复锚点、部署时钟资格、独立时间历史、旧二进制升级、完整
TLA/TLC/Chaos 与生产门槛继续保留。

后续[对象 buffer 配额](RAW-TTL-OBJECT-BUFFERS.md) 独立计量 MinIO 客户端的共享 PUT
payload 和 GET 收集结果，额度随最后一个引用释放；不把 image 额度提前释放当作这些
副本已经消失的证据。兼容 Vec 接口、codec 与恢复 index 等边界仍保留。

后续[借用 SST 编码](RAW-TTL-BORROWED-WRITER.md) 已让生产规划在精确 image 准入后
直接写最终对象，不再建立 owned key/value 暂存集合。布局/manifest 元数据、常驻
索引与 snapshot 保留等仍未纳入这份 serialized-image 额度。
