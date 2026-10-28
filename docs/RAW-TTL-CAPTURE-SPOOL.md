# 大 TTL 镜像的生产 capture 与安装

2026-10-05；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
接续 [恢复依赖记录](RAW-TTL-RECOVERY-DEPENDENCIES.md) 与
[大 checkpoint](RAW-TTL-LARGE-CHECKPOINT.md)。第二阶段与生产验收仍未完成。

核对继任恢复锚点的传输路径时发现：metadata/data checkpoint 已使用逐 SST spool，
生产 `plan_migration_image` / `capture_migration_image` 却仍调用整图 resident planner，
在规划时绕过共享 image 准入，并受 48 MiB 上限限制。较大 TTL 镜像因此不能走完现有
capture→install→recovery 链路。

## 实际入口和资源生命周期

两个 RuntimeBackend 入口现在都使用 `plan_capture_spooled`，要求已配置的 uploader，
共享该节点的 image buffer 与 spool disk 额度。规划先从同一 frozen view 确定 range、
scope、image cut，再查该 cut 的 committed configuration；逐 SST 编码写入本地文件，
不在规划期间 PUT/GET。完整 manifest 与 canonical snapshot record 在上传前已经固定。

生产 capture 使用独立匿名临时文件，不复用 checkpoint worker 的固定 scratch 名称。
引入已在 lockfile 中固定的 `tempfile = 3.27.0` 为直接依赖；没有更新包版本。共享 spool
准入在创建文件之前完成，image 准入先于每个 SST buffer。并发 capture、metadata/data
checkpoint 竞争相同额度，超额拒绝，不退回全图驻留或提高原 48 MiB 常量。

新 `upload_spooled` 逐 SST 校验、PUT、验证 GET；spool 的额度保持到实际文件关闭。
PlannedCapture 上传使用现有 local execution scope，返回/报错前等待本地对象任务
排空。报错不能证明远端 PUT 已终止；原 Snapshot/Migration Published pins 保留。
checkpoint worker 仍通过自己的 durable pending journal 上传，不改成匿名计划恢复。

Source capture 还检查传入 engine 确实属于该 driver、driver 未停止、range 属于该
Raft group。公开 resident `plan_capture` 保留为原有兼容入口；实际 RuntimeBackend
不再使用它。小镜像两种规划的 manifest 与 snapshot record 字节相同，大镜像使用
已有 V2 manifest / V7 installer，未新增命令、RPC 或磁盘格式。
既有边界仍生效：单 SST 至多 48 MiB，大镜像至多 64 个 SST、总上限 3 GiB，manifest
至多 1 MiB；实际还受配置的共享 spool 额度限制（默认 512 MiB）。

## 保留的发布约束

上传前仍从实际本地 applied metadata view 核对两个 owner 的完整绑定和 Published
状态；规划出的新 cut 不能借用旧 manifest 的 pins。metadata leader 和 data leader
可以不同。预算不足、未登记 owner、上传失败或身份不符不会产生捕获成功回执。

匿名 spool **不提供崩溃续做**。现有 plan→bind→capture 仍会重新规划，期间 source cut
前进时拒绝第二镜像；本次没有解决持续写入时固定原始计划、跨崩溃继续传输的问题。
这需要持久保存同一 cut 的 capture manifest、configuration 与协议 record，并绑定
实际发布者/目的地和恢复生命周期。它是继任恢复锚点的下一项必要工作，不能由本轮
成功安装一张镜像替代。

预算覆盖序列化 SST buffer 和活跃 spool 文件。frozen view、常驻索引、网络 buffer、
恢复对象与整机 RSS 使用各自现有路径；本次不宣称整个迁移内存或磁盘已形成硬上限。
Publisher/Reader 全集合、未知 PUT 隔离、旧 Version/负向 Pending 释放和 GC 仍开放。

## 验证

[独立命令、原始日志、开发失败与源码摘要](raw-ttl-capture-spool-validation-20261005/manifest.json)
与旧证据包分开保存。

- 8 个并发 transfer spool 保留不同 frozen image，逐一上传并还原独有 value；旁边的
  checkpoint scratch 原始内容不变。所有文件关闭后 image/spool 预留归零。
- 上传注入失败时，PUT 内仍可观察到 spool 被计费；失败和目录创建拒绝后正确归还。
- 相同小镜像的 resident/spooled manifest 与 protocol record 字节一致；foreign
  engine 与已经停止的 driver 都拒绝。
- 真实三副本 metadata/data driver、分离 leader、公开 TTL 写入，经实际 RuntimeBackend
  plan/bind/capture 得到 **55,384,729 字节**镜像。共享 image 额度 9 MiB，序列化 buffer
  峰值 **8,358,192 字节**；spool 额度 64 MiB。持有原计划时第二 capture 和普通 checkpoint
  都因同一 disk 额度拒绝。
- 未绑定 owner 时零 PUT；注入上传失败后 source/destination owners 仍 Published，
  同 cut 重试成功。source 再写入后新 cut 被拒绝，PUT 次数不增加。
- canonical capture record 在已绑定的 learner store 上实际安装、adopt、重开 Raft/WAL；
  核对全部 900 个 value、原 expiry bytes 和 clock，捕获之后的新 key 不混入旧镜像。
  恢复依赖记录指向同一 manifest，配置包含目标 learner。

引擎全量 229 项、capture/install 20 项、锁序 1 项、TTL integration 62 项通过；
专项集合有重叠，本轮去重 **312 项通过、10 项既有测试忽略**。严格 workspace Clippy、
格式和 diff 检查通过。使用 MemoryObjectStore、InProcHub 和已有 HTTP/2
duplex fixture；真实 MinIO 测试仍忽略，没有重试已知不允许的 TCP 绑定。没有完成
跨主机部署、旧 binary、断电、完整 NodeRuntime 进程故障或新形式化组合验收。
