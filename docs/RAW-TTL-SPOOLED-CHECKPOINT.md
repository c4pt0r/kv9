# Checkpoint 按 SST 落盘与上传

日期：2026-10-05。接续 [导入 Raft 回收](RAW-TTL-IMPORT-COMPACTION.md)。生产 metadata
和 data-group checkpoint worker 不再同时持有整份镜像的 SST buffers：规划先逐个
SST 写入本地临时文件，持久 journal 重开和上传也按 SST 处理。这是扩展大镜像前的
必要步骤；**48 MiB 格式上限、驻留索引和完整生产验收仍未完成**。

后续 [V2 大 checkpoint](RAW-TTL-LARGE-CHECKPOINT.md) 已扩展镜像格式并增加活动
spool 配额，也修正了实际 metadata 启动 journal 未启用流式加载的遗漏。下文
记录本组件当时的实现与证据，当前边界以该后续说明为准。

## 生产路径

`RemoteUploader::plan_spooled` 先用冻结 snapshot 做原有借用布局预检。每个 SST
编码前申请共享 ImageBudget，直接编码到最终 buffer，写入临时文件后释放该 buffer
和额度。计划保留 manifest 与独占文件句柄，不再保留整个 `Vec<Vec<u8>>`。冻结
snapshot 在规划完成后释放，不需要一直保留到远端上传结束。

临时文件在打开后立即 unlink；正常结束、失败或取消后的 FD drop 释放文件空间。
open/unlink 崩溃窗口可能遗留一个固定名称的 scratch，后续规划重用该位置；data
group 的既有授权退役回收也删除这个明确命名的本地 payload。临时文件本身既不
是 durable pending，也不授权远端写、manifest 提交或任何 WAL/对象回收。

`stage_spooled` 按原 V2 planned journal 格式写入 generation、manifest、每个精确
SST 与完整 checksum，再执行 file sync、rename 和 parent sync。data worker
通过原 CheckpointCandidate 在可能出现模糊结果的发布前保持历史 pin；无文件且
删除/目录同步确认成功时才取消失败的本地候选。metadata worker 继续保留其原
所有者和执行协议。远端 I/O 前仍先登记真实 owner、intent/admission 和 executor。

由 uploader 创建的 journal 使用流式重开：逐项验证全部 SST、完整文件 checksum
及精确 EOF，完成文件和目录同步后才返回 PendingFlush。返回值保留所验证的已打开
inode 与 descriptor，空闲时不保留 SST buffer。路径随后被 rename/替换不会把上传
重定向到另一文件；上传每个对象前仍从原 inode 读取并重新校验实际 SST 字节。
原 inode 被原地破坏时拒绝，不会凭旧校验结果构造 PreparedFlush。

上传维持原顺序：精确对象 PUT、受限 GET 字节比较，全部对象成功后才返回 prepared
capability。budget 拒绝、损坏、未知 PUT 或失败不会清除 durable journal 或全局
owner。逐 SST 上传继续使用原 scoped object executor，超时后的队列/实际任务仍由
既有排空协议负责。

## 兼容性与资源边界

manifest、SST、planned V2 和 prepared V1 journal 的字节及 change ID 不变，没有新
Raft tag 或 store/service 代际；当前为 V6。旧 `plan` / `plan_checkpoint` 与普通
`FlushJournal::new` 的 resident 接口保留，独立调用者仍可能分配完整镜像。两个生产
后台 worker 和 uploader 创建的 startup/recovery journal 已使用新路径。
迁移/安装的独立 capture 接口也仍使用原 resident 规划，未在本轮一并替换。

ImageBudget 现在同时计量旧 resident plan 和新路径的单个活动 SST buffer。
状态字段 `active_images/admitted_images/refused_images` 保留原名，但它们统计的是
reservation 次数/数量；一个流式镜像有多次逐对象 reservation，不能将这些字段
解释为累计完成镜像数。`retained_bytes` 和 `peak_bytes` 仍表示实际预留序列化字节。
配置默认 96 MiB、runtime 最小值 48 MiB 未变，所有申请仍不等待。

8 MiB 是原有分片目标，超大单行仍可能生成更大 SST，受原 48 MiB 镜像上限和实际
buffer 配额限制。这里不是 8 MiB 的整个进程内存保证：descriptor、布局、读取器
小缓冲、allocator、常驻索引、快照旧版本、网络独立对象 buffers 与 OS page cache
另有成本。恢复虽逐 SST 验证，最终仍构建完整 resident index。

规划临时文件和 journal 发布会增加本地 I/O，并短暂保留两份磁盘字节。现有任务
准入和 48 MiB 上限继续限制这些工作；新的完整磁盘/FD/CPU/RSS 稳态预算尚未建立。
本轮不根据 reservation 指标宣称进程 RSS 或吞吐性能提升。

## 验证

[证据包](raw-ttl-spool-validation-20261005/manifest.json)保存源码摘要和原始输出。
**146 个不同 Rust 单元/组合测试和 5 个 compile-fail doctest 通过**；8 个既有测试
按原配置忽略。workspace 严格 Clippy、格式和 diff 检查通过。
新增 engine 回归验证：

- **12,583,998 bytes** 的完整镜像，在 **8,389,632 bytes** 配额下完成规划、stage、
  journal 重开、上传、prepared 升级和全部 CF 恢复；最大 SST 与预留峰值都是
  **7,340,128 bytes**。旧 resident 规划在同一额度下拒绝。
- 与旧规划和 journal 写入器逐字节比较，manifest/generation/change identity 不变；
  精确重复 stage 接受，另一 generation 拒绝。
- journal 路径替换后，已经加载的 pending 仍读取原 inode；完整 checksum 损坏在
  load 拒绝，load 后原 inode 被修改则在上传校验时拒绝，首对象损坏时没有 PUT。
- staging 写盘失败只取消已确认未发布的候选；重试后 durable pin 在能力 drop 后
  保留。匿名 scratch 正常不留下路径，现有退役/重开测试覆盖遗留 scratch 的删除。
- 两项 compile-fail 检查保证 spool 不能 Clone，也不能转为 prepared capability。

三副本导入 checkpoint 与 Raft 回收场景也改用显式 **17 MiB** 测试额度，小于每份
选中镜像；真实生产 worker 的序列化 buffer 预留峰值为 **8,358,192 bytes**，排空后
归零。engine WAL/Raft 实际回收、三个副本重开、配置重放、换主、Finish/publication
以及原 expiry 和后续公开 TTL 读写通过。该额度由 fixture 显式传入，不改变生产
环境配置下限。已有 metadata 配额拒绝后恢复、阻塞 PUT、取消排空及 lost-receipt
组合场景继续回归。

仍使用磁盘 WAL/Raft、InProcHub、HTTP/2 duplex 和 MemoryObjectStore。没有新增
完整形式化模型、TLC、真实 TCP/MinIO、进程 kill、旧 binary 或跨主机资格。
接下来仍须扩展大镜像的版本化 manifest/有序 apply/恢复边界及资源准入，保留
legacy 拒绝语义；长期 GC、历史/Reader/Version/Pending 释放、unknown PUT 隔离、
merge 和完整第二阶段生产范围继续开放。
