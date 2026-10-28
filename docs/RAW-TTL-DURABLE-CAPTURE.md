# 固定 capture 计划与本地续传

2026-10-05；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
接续 [生产 capture spool](RAW-TTL-CAPTURE-SPOOL.md)。本轮修复持续写入期间
plan→bind→capture 镜像变化的问题；第二阶段与生产验收仍未完成。

## 生产行为

`RuntimeBackend::plan_migration_image` 与 `capture_migration_image` 共用持久计划。
首次规划从实际 group driver 的 engine 获取同一 frozen cut 的 range、manifest 和
committed configuration，再逐 SST spool。返回 manifest 之前，把协议 record、
配置提交位置、source/destination StoreIdentity、migration operation 和完整 SST
一起写入 `capture.plan`。range/root/creation/epoch 包含在 canonical record 中。

后续 plan/capture 加载原文件，source 新写入和较晚的配置变化不改变所选镜像。
当前 driver 必须健康、仍是该 engine 的实际 leader，source store、已提交 migration、
live/committed range 必须匹配，source engine 必须已恢复到该 cut 或更晚。
首次规划要求 image configuration 已包含目标 learner，避免绑定无法安装的镜像。

文件本身不是上传许可。capture 仍在任何对象 PUT/GET 之前，从 applied metadata
核对同一 manifest 的两个 Published owners；未绑定、身份不符、文件损坏都拒绝。
上传报错和未知 PUT 保留原文件及远端 pins，重试相同内容。配置和数据都来自原 cut，
不会把当前配置补到旧镜像上。

## 原子发布、预算与生命周期

- 新本地格式 `KV9XFR01` 包含有界 metadata、canonical manifest、逐 SST payload 和
  全文件 SHA-256；capture metadata 使用 `KV9CAP01`。普通 checkpoint journal
  拒绝把该文件解释为 checkpoint/reclamation authority。
- 一个 group engine 目录只有一个 slot。固定 `capture.lock` 使用非阻塞文件锁，
  规划、加载和上传持有同一锁；并发调用立即拒绝。成功/失败返回前排空实际本地
  object jobs，锁和实际 group owner 保持到排空完成。锁文件不在运行期间 unlink。
- 临时 `capture.pending.tmp` 写完、文件 fsync、rename、父目录 fsync 后才返回计划。
  尚未 rename 的 tmp 不选择镜像；rename 后发生不确定错误，重开校验并重新 fsync。
  损坏的 selected file 不回退到当前 engine 重新规划。
- 共享 `PendingDiskBudget` 在创建临时文件之前预留完整文件大小，失败遗留文件仍计债。
  打开的 inode 有独立读者租约；unlink 不会提前归还仍占用的磁盘额度。加载后上传使用
  同一个 FD，替换路径不能把上传重定向到另一份文件。
- SST 编码、校验和上传继续逐 SST 共享 image 额度。新建时 scratch spool 和 durable
  journal 同时存在，分别计入共享 spool 与 pending 额度。metadata 至多 2 MiB + 512 B，
  manifest 至多 1 MiB；原 SST/image 格式与上限不变。这不是整个进程 RSS 的硬上限。
- 启动盘点同时覆盖普通 group 与真实 `install-*` 目录中的 capture/checkpoint slots
  和临时文件，不跟随目录 symlink。已存在债务超过降低后的额度仍可加载和续传；
  不能据此创建新文件。此前遗漏的已安装副本 checkpoint journal 也纳入盘点。

另一个 operation 复用 slot 时，必须验证旧 binding 和 manifest 对应的 committed
abort 或 install evidence。只有完整匹配才清除旧本地 payload；没有超时释放，也不
修改远端 owner。已取消 operation 永久拒绝；已经安装且丢失本地 slot 的 operation
不能重新规划。若仍保存原计划，已安装 operation 可在 pins 仍 Published 时重试。

实际 group retirement 等待 owner 排空，再持久写入 Reclaimed fence，清理普通目录及
安装 generation 内的 capture payload，并对账磁盘预算；稳定锁 inode 保留。旧安装
记录和其他恢复依赖仍按各自现有协议保留，本轮不授予通用安装依赖删除许可。

## 验证与边界

[命令日志、源码和证据摘要](raw-ttl-durable-capture-validation-20261005/manifest.json)
独立保存，未重写旧证据包。

- 原子发布四个错误注入点只能恢复“无计划”或完整原计划；所有截断、损坏、超界长度
  拒绝；普通 checkpoint loader 拒绝新格式；并发锁、inode 路径替换、unlink 后计费
  和降低额度后的恢复通过。
- 真实 source driver 提交写入和 learner 配置后，关闭全部源端 driver/engine 句柄，
  重开 Raft/WAL、重新选举，恢复的 record/manifest/cut 与原计划字节一致。后来写入的
  value 和后来添加的 learner 都不混入镜像。不同 source、destination、operation、
  range、未完成旧 slot 和停止的 driver 均拒绝。
- 三副本 metadata/data fixture、不同 leader、公开 TTL API 的 **900 个 value**形成
  **55,384,729 字节**镜像；plan 与 bind 之间继续写入，随后完成实际 RuntimeBackend
  bind/capture。PUT 已写入对象后注入错误，pins 仍 Published，原计划重试成功。
- source 再写入及重建 backend/uploader 后，record 仍完全相同。把新 pending 额度
  降至 1 B、scratch 额度降至 1 B，原计划仍能续传；读取不再申请整图 scratch。
  序列化 SST buffer 峰值 **8,358,192 字节**。目标 learner 实际 install/adopt、重开
  Raft/WAL，核对全部 value、expiry、clock 和捕获之后两个 key 的缺席。
- 真实 metadata Raft 提交 abort 和下一个 migration 后，旧 operation 拒绝，新的
  operation 复用 slot、选择新的 cut；原两个远端 owners 仍 Published。
- 引擎 233、capture/install 22、TTL integration 62、磁盘盘点 2、退休回收 2、锁序 1
  项通过，共 **322 项不同测试通过，10 项既有测试忽略**。严格 workspace Clippy、
  格式和 diff 检查另行记录。

测试使用 MemoryObjectStore、InProcHub 与 HTTP/2 duplex。错误注入和句柄重开不替代
真实断电、跨进程/跨主机故障、MinIO 或混合版本部署验收。磁盘/内部服务 floor 仍是
V7；新增的是本地 retry 文件，没有新 RPC、Raft command 或 retention ledger 格式。
旧 V7 binary 不认识这个计划文件，不能宣称混合版本具有相同续传行为。

续传目前绑定原 source store。另一个 source leader/store 接手同一 operation 的
可移交恢复依据、全部 Publisher/Reader 生命周期、未知/非零 PUT 隔离、旧 Version 与
负向 Pending 释放及 GC 仍开放；本地 retry 文件不是 portable successor recovery
authority。容量稳态、时钟部署资格和新的形式化组合验收继续保留。
