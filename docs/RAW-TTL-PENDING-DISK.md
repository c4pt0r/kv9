# Checkpoint 持久 pending 磁盘准入

日期：2026-10-05。接续 [V2 大 checkpoint](RAW-TTL-LARGE-CHECKPOINT.md)。生产
metadata/data worker 现在共享 pending journal 与发布临时文件的字节准入，启动时
盘点已有文件，文件被替换或删除后仍计入未关闭的读取句柄。**这补齐一类持久磁盘
债务，不代表 WAL、整个文件系统或完整第二阶段已经完成容量验收。**

## 计费范围与配置

`KV9_CHECKPOINT_PENDING_BYTES` 默认 **1 GiB**，runtime 最小 **48 MiB**。它独立于
现有默认 512 MiB 的匿名 spool 配额，覆盖 metadata 的 `catalog.pending`、
`catalog.pending.tmp`，以及每个 data group 的 `data.pending`、`data.pending.tmp`。
runtime 为这些文件和所有 scoped uploader 使用同一个 PendingDiskBudget。
普通 `FlushJournal::new` 兼容接口不隐式绑定节点配额；生产创建路径明确接入共享额度。

新增 `checkpoint_pending_disk` 状态：额度、当前/峰值计费字节、登记路径数、活动
reader/writer、发布拒绝数，以及因磁盘配额保留 planned journal 的次数。
字节按文件逻辑长度或写入前的完整预留计算，包含 framing、manifest、SST 和 checksum；
不是文件系统实际 block 数、空闲空间探测或整个卷容量保证。文件系统 metadata、
block rounding、匿名 spool、WAL/Raft、安装临时文件和 OS page cache 另有成本。

规划文件仍受 spool 配额控制。发布新的 pending 或替换 prepared journal 时，先
清理并同步该组固定的发布临时文件，再为**完整新文件**申请额度；旧文件尚存或旧
reader 仍打开时不能提前抵扣。额度不足在创建/写入新临时文件前返回。成功后保持
原 write → file sync → rename → parent sync 顺序；没有新增磁盘格式、Raft tag
或 store/service 代际，当前仍为 V7。

## 文件与句柄生命周期

Linux/Unix 用实际 device/inode 识别文件，同一 inode 的多个观察和读取不重复
计算其字节。不同 inode 的旧 plan 与新 prepared 文件同时存在时分别计费。reader
lease 随加载后的实际 FD 保持，上传继续读取已验证的 inode，不重开路径；FD 关闭
后才归还其引用。无稳定 inode 标识的平台采用保守的逐打开计费，本轮仅验证 Linux。

named debt 保存在共享 budget 内，不随 FlushJournal、worker 或计划对象 drop
而消失。失败的部分临时写入保守保留完整预留，直到后续清理并同步该文件；重启
则按磁盘实际长度重新盘点。rename 或目录同步处失败也不能凭函数返回错误当作
“文件没有发布”。原 candidate/history pin、全局 owner 和实际 checkpoint 恢复
授权规则继续独立执行；磁盘额度不是这些权限的凭据。

typed settlement 删除 journal/发布临时文件并同步目录后才去掉 named charge。
旧 inode 仍有 reader 时继续计费。删除同步失败保留额度，重试成功后归还。
RegionManager 的既有授权退役/重开删除路径也在删除和同步之后核对文件确实不存在，
再移除 named charge；磁盘 quota 本身没有获得删除任何未授权数据的能力。
同一组仍要求唯一 journal owner；这项额度管理不替代 driver/seam 的生命周期约束。

## 重启与满额下的恢复

实际 `prepare_remote` 在创建 uploader 或启动 worker 前盘点 metadata 和全部本地
data-group 固定 journal slots，包括不能恢复的组留下的普通文件和部分临时文件。
它不读取完整镜像，不赋予目录中的组任何恢复资格，也不跟随 data-group 目录 symlink。
错误类型的目录仍交给组的既有校验隔离；盘点不会因为这种目录把其他组一并隔离。
盘点是启动工作，目前仍遍历整个 data-groups 目录，尚未建立启动时长的规模资格。

已有债务可能超过新配置，例如下调配额。此时仍允许加载、校验和结算已有 pending，
但拒绝新增文件字节。否则，满额时无法写入很小的 prepared journal，也就永远无法
完成 checkpoint 来释放原来的大 plan。

生产 worker 使用 `record_uploaded` 处理这个边界：如果只有**创建文件前的配额
拒绝**，且当前磁盘仍是已经完整校验并同步的**同一 manifest/generation 的 durable
plan**，则明确返回 `PlannedRetained`，保留原文件并继续使用真实 PreparedFlush
提交和结算。所有 PUT/GET 已经验证，原 plan 也足以在崩溃后恢复同一身份；重启时
可能重新上传同一内容。错误 generation、另一身份、损坏和 I/O 失败均不能走这个
分支。旧 `stage` API 仍严格要求 prepared journal 写入成功。

这不会提前释放 unknown PUT、Pending/Version 或 executor owner；已有 remote
admission、执行排空和 publication/settlement 规则继续生效。

## 验证与未完成项

[证据包](raw-ttl-pending-disk-validation-20261005/manifest.json)保存实际输出和源码
摘要。**167 个不同单元/集成测试和 5 个 compile-fail doctest 通过**；8 个需要
真实 MinIO 的既有测试按原配置忽略。workspace 严格 Clippy、格式和 diff 检查通过。
新增验证包括：

- 两个 uploader/scoped 使用同一额度；另一个组在首组写盘仍阻塞时被拒绝，没有
  创建第二个临时文件。journal 对象 drop 后 named debt 保留。
- planned → prepared 替换期间，同时计算旧打开 inode 和新文件。路径删除后，
  实际上传仍从旧 FD 成功完成；只有该 FD 关闭才归还最后一份额度。
- 每个 publication 故障窗口、部分写入、删除同步失败保留应计债务；重新盘点
  得到实际文件长度，重试清理后恢复准入。未发布 candidate 的配额拒绝能够安全
  取消本地 history pin。
- 满额时只保留精确 durable plan；严格 stage、错误 generation 和 I/O 错误不会
  冒充 prepared 更新成功。节点启动盘点 metadata、多组、partial tmp 和隔离组
  文件，允许已有债务高于新额度。
- 实际 metadata/data worker 采用显式 **1 byte** 测试额度，分别恢复已有
  **3,554 / 5,510 bytes** 的 pending。发布/采用后计费归零，峰值不高于原债务；
  三个 data 副本采用同一镜像并保留公开 TTL 值。测试直接注入额度，不改变 runtime
  的 48 MiB 配置下限。
- 实际 RegionManager 的退役删除和崩溃后续删都释放对应 journal debt；保留原
  tombstone、授权校验和 owner 排空规则。

测试使用磁盘 WAL/Raft、InProcHub、认证 HTTP2 duplex 和 MemoryObjectStore。
一次过宽的开发测试过滤器还匹配了四个 TCP point-stream 测试，均在 socket 创建处
因 sandbox `EPERM` 失败；已记录原始日志，没有把它们计入通过测试，也未重试同一
受限路径。真实 TCP/MinIO、完整 NodeRuntime、跨主机/旧 binary、进程 kill 及稳态
负载仍需部署环境验证。本轮没有新增形式化模型或 TLC 结果。

接下来仍要把 WAL/Raft 写入及物理债务纳入容量与背压，完成 CPU/整机公平性、
大镜像 capture/迁移、常驻索引/RSS、长期历史与 Reader/Version/Pending 释放、
unknown PUT 隔离、TTL merge 和完整生产验收。这里的逻辑字节配额不能替代它们。
