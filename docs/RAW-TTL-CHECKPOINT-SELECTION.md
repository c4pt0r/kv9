# Checkpoint 选择一致性与在线观察

2026-10-05；接续 [Reader 生命周期](RAW-TTL-CHECKPOINT-READERS.md) 与
[checkpoint 历史保护](RAW-TTL-CHECKPOINT-HISTORY.md)。本次修复本地恢复引用检查，
第二阶段与生产验收仍未完成。

## 修复的问题

旧 WAL 的 `checkpoint_applied` 主要比较 image index：同一个 index 的不同 manifest、
不同 cluster/region、回退的 term/epoch，以及与保留 WAL term 冲突的 cut 都没有获得
与 segmented WAL 相同的检查。这可能让错误的内部调用先覆盖恢复引用或改写日志，
直到下次恢复才发现矛盾。实际 runtime 仍有自己的提交历史及镜像验证；本次补齐的是
engine 层的拒绝条件，不表示公开 API 可直接提交任意 manifest。

两种布局现在共用同一套 manifest/进度/前驱检查：

- 先验证规范 manifest，要求本地 applied position 覆盖 cut，精确位置的 term 一致。
- stream 的 cluster/region 不可改变，同一个 index 必须是同一个完整 manifest。
- 新 image 的 term、conf_ver、version 不可回退；同 scope 的更旧 image 返回未采用。
- legacy 在覆盖引用之前逐项核对已有 positioned WAL 的 term 与 cut。unpositioned
  历史仍不能授权回收；没有新增可绕过 committed publication 的入口。

## 在线观察

`WalEngine::checkpoint_reference(path)` 只解析文件。rename 后文件可见，不代表整个
选择流程成功；它也不与在线 compaction/选择互斥。因此它继续用于独占恢复和诊断，
不能拿来证明活跃 owner 已成功采用某镜像。

新增 `selected_checkpoint(&self)` 在 history → WAL 的锁顺序下检查实际活跃 owner：
历史不能处于不确定状态，WAL/布局必须健康，所选 cut 必须与 history 相同，并受
当前 applied position 覆盖。segmented 使用已成功发布的内存 topology；legacy 在
持锁期间读取引用。选择、压缩、布局切换或 append 的不确定失败都要求先恢复。
查询不获取 peer 锁，不改变普通 apply 的锁顺序。

data worker 的“已有更新 checkpoint 覆盖此次采用”分支以及 import compaction 的
本地 image ceiling 已接入该查询；前者同时要求新 checkpoint 的 epoch 覆盖原镜像。
三副本测试中的在线观察也使用这一入口。

这个返回值只是本机的瞬时选择观察：它不验证完整远端 SST、publication/configuration
与 WAL 尾部，不记录跨节点副本集合，也不成为可转移 recovery anchor 或 ledger 释放
凭据。查询返回后仍须由实际 peer 在执行 compaction 时重查原有授权与限制。
恢复时仍执行现有完整 image、提交历史、配置和尾部验证。

## 验证

新增 6 项 engine 测试，包含两种布局的 20 个冲突场景：拒绝前后实际 WAL 和所选引用
保持一致，重开后旧 image 与尾部仍可恢复。其余覆盖正确重复采用/epoch 推进、legacy
保留 term 冲突、并发 compaction 的失败、布局/append 失败，以及引用已经发布但后续
tail rewrite 或 obsolete segment unlink 失败。后者明确检查普通文件读取能看到新
manifest，而在线观察拒绝，清除故障并实际恢复后才允许返回选择结果。

engine 各 suite 共 242 项通过、25 项忽略；Raft data checkpoint 恢复专项 18 项、
driver 锁顺序 1 项、三副本 TTL 组合 59 项通过。本轮去重合计 320 项通过，新增专项
已经包含在 engine 回归中。严格 workspace Clippy、格式和 diff 检查通过。

完整命令、测试数量、日志和本轮源码摘要见
[独立验证包](raw-ttl-selection-validation-20261005/manifest.json)。旧验证包保持不变。
测试使用实际本地文件、Raft/InProcHub 与 HTTP/2 duplex；MemoryObjectStore 和故障注入
不能替代远端对象持久性、进程断电或真实跨主机验收。本次没有执行新的形式化证明。

## 剩余工作

后续仍须把每个实际副本/安装/恢复路径的依赖与可验证、可转移的继任恢复锚点组合，
再判断旧 Version 能否退役。完整 Reader/publisher 资格、非零或未知 PUT 终止/隔离、
负向 Pending 释放、对象 GC、历史与容量稳态仍开放。本次不改变磁盘/服务 V7、ledger
header 07 或任何 owner 的释放规则。
