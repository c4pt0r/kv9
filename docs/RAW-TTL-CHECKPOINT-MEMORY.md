# Checkpoint 规划与 pending journal 的临时副本

2026-10-04；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
本轮延续[对象 I/O 配额](RAW-TTL-OBJECT-IO.md)，消除 checkpoint 路径中不必要的完整
image 副本，并把规划的大小检查移到复制数据之前。整机内存准入和稳态验收仍未完成。

## 修正的路径

原先 `stage_planned` 复制 plan 的全部对象，再编码为完整 journal buffer；为了验证
编码结果又 decode 出一份对象集合。恢复时 `load` 先保留完整文件，再复制每个对象。
自动 data/metadata worker 在 journal 重新加载和上传期间还保留原 plan。

现在 staging 借用 plan 的 manifest 和对象，验证后逐段写临时文件并增量计算 SHA-256。
它不再 clone 对象集合或创建完整 journal 编码 buffer。已有 pending 的幂等比较使用
解码后的 generation、manifest、对象字节，保留排他性与精确身份；这个比较仍会加载
旧对象集合，不能宣称所有分支都只保留一份 image。

`load` 根据同一已打开文件的长度先拒绝超限文件，再用有界 manifest 和各 SST 长度
分别分配最终返回的对象 buffer，避免同时保留完整文件副本。每个 SST 仍验证哈希、
格式、CF、范围和条数；返回前必须通过完整 journal 的 SHA-256 和精确 EOF 检查。
截断、增长、额外尾部、损坏校验和、伪造长度均拒绝。流式读取过程中会解析有界数据，
但在最终校验完成前不返回 PendingFlush，不授予上传、publication 或回收能力。

落盘格式仍是原 v1 prepared / v2 planned，包括 magic、generation、manifest 长度、
对象排列、字节序和 SHA-256 覆盖范围。发布仍依次执行临时文件写入、文件 fsync、
rename、父目录 fsync；临时文件不成为恢复授权，任何错误不清除原 pending。
已准备的能力只能把相同 generation/manifest 的 planned 文件升级为 prepared 文件。
两个自动 worker 在 plan 已持久化后显式释放它，再 load 原 journal；owner 与历史 pin
的持有和释放规则不变。

## 规划前置检查

checkpoint 现在借用冻结 snapshot 的 key/value。SstWriter 维护已接收条目的编码长度，
在复制下一条记录之前核算 SST header、全部 entry、两份 footer key 以及 checksum。
每个 SST 使用全 image 尚余的字节额度，跨 CF 共享原有 **48 MiB** 上限。
超大的单条记录和累计超限都在复制该记录之前拒绝；拒绝不改变 writer 的内容或计数。

分片仍使用原先的 8 MiB entry 目标和键顺序。合法 SST 的字节、哈希、对象 key 与恢复
身份不变。这里限制的是序列化字节：Rust Vec 容量、条目索引、SST 验证、resident map、
快照引发的写时复制、上传/读回和恢复 MemEngine 的开销不是同一个量。

## 验证与边界

新增七项测试覆盖：

- 独立的旧格式参考编码，与真实磁盘 v1/v2 load/stage 逐字节比较；幂等保存与升级身份保持。
- writer 确实借用原对象 slice；短写正确继续，途中错误返回，未完成文件不能恢复。
- 超限文件在读之前拒绝，文件长度变化、伪造对象长度和最终 checksum 损坏均拒绝。
- 无效对象数量、内容或 generation 不创建临时文件，不覆盖原 durable identity。
- 三个 CF 的完整 SST 大小累计边界与长 footer key 的精确边界。
- 借用规划与原 writer 的分片及每个 SST 字节完全相同。
- 大 value 或乱序 entry 拒绝后，writer 可继续加入合法记录并正常解析。

已有全截断遍历、未知 PUT 原计划恢复、候选历史保护、三副本自动 checkpoint 和公开
TTL 激活组合继续回归。命令、原始日志与源码摘要另存于
[本轮验证](raw-ttl-checkpoint-memory-validation-20261004/manifest.json)，旧证据不改写。

测试不构成全节点 RSS 峰值或长期稳态证明。本轮没有新增共享内存额度：现有两槽重 I/O
准入继续有效，但还需给 checkpoint/恢复/compaction 等路径统一临时内存成本与额度，
并测量持续写入时的快照保留、磁盘、FD 和对象债务。完整 48 MiB image 限制和全驻留
index 仍在；Reader/旧 Version/负向 Pending 释放、对象 GC、历史有界、TTL 范围转移、
部署时钟资格、完整 NodeRuntime/TCP/MinIO、旧二进制升级及 TLA/TLC/Chaos 继续开放。

后续[共享 image 字节准入](RAW-TTL-IMAGE-BUDGET.md) 已按 plan/pending 的实际 SST 字节
限制同时持有量，额度随上传对象生命周期转移。它不关闭上述完整临时内存/RSS 边界。
