# 持久 bootstrap grant 的有界盘点

2026-10-05，接续 [BootstrapReader catalog 准入](RAW-TTL-BOOTSTRAP-ADMISSION.md)。新准入
已有精确 Pending、Version 和 store 来源，但旧 grant 的 operation 混入了 store 身份，
不能套用进程 Reader 的 operation → Pending 投影。新增独立 bootstrap 索引，保留所有
已登记 grant 及其原始状态，为恢复依赖检查提供可分页的目录。

## 索引和观察语义

索引按 root namespace、完整镜像 subject 和 owner ID 排序。它保存不可变 binding 摘要，
查询时核对完整 owner、registration、每个资源 pin 和已有 admission 来源。subject 是
分组键，不是“拥有正确 source”的证明：来源未知、没有对应 canonical intent 的旧
subject 仍能通过全局 `catalog` 枚举，不能因为不能反推 Pending 而丢弃。

`catalog` 按 `(image, owner)` 游标列出全部登记的 bootstrap grant；`grants` 按精确 image
subject 和 owner 游标列出该镜像的 grant。每页最多四个结果，含 lookahead 最多核对五条
索引。调用者必须在同一稳定 view 上完成多页观察。不同 Pending 恰好引用同一镜像时，
它们仍是独立 grant；不把 subject 分组解释为某个 Pending 的专属生命周期。

每项包含完整 owner 和可选 admission。有 12 格式 typed admission 的记录原样保留精确
来源；没有记录的 grant 明确返回未知。Held、Published、Released 都纳入目录，Released
不能补造准入或旧代历史。普通进程 Reader、Build、Publisher 和 Version 不进入这个索引。

## 盘点、并发和恢复

历史补齐分三个持久阶段：owner 行、immutable registration 行、typed admission 行。
每次最多处理四行，并读取一行 lookahead。前两轮发现不完整的 owner/registration 组合；
第三轮发现只剩 admission、owner/registration 都丢失的孤立来源记录。各轮都校验完整
资源交叉引用，遇到不一致拒绝推进，不修复或重分类原有保护。

调用者只提交预期序列、阶段与游标，metadata 在同一 view 重算实际下一页。跳页和旧
游标拒绝；紧邻一次的已提交请求可无变更确认。每页新增索引与进度原子提交。新 grant
在扫描游标之前出现也不会漏掉：统一在 ledger 批次出口，从受批次上限约束的 owner
写集中增加 bootstrap 索引，覆盖 typed entry、Publish、独立退休，以及未进入即被
独立 store fence 建立的终态。没有按 attempt 扫描整个 owner 表。

目录完成后，单个 owner 检查会拒绝其索引缺失；查询会拒绝遇到的损坏/孤立索引与
来源不一致。完成标记依赖这些原子写入和持久存储完整性，不是针对任意静默删行的
全库审计证书。后续释放判定仍需处理未来准入、来源/生命周期变化和稳定观察范围。

后台先完成 Build、再完成进程 Reader、最后推进 bootstrap 目录，复用原 metadata 维护
槽位。每次最多提交一个目录页，保留 leader、ReadIndex、root、取消和 deadline 检查。
没有新增后台任务或逐镜像常驻集合；增加 bootstrap 页数、扫描行数及完成状态统计。
目录完成仅表示已登记范围的索引覆盖，不表示实际对象任务已排空。

## 格式与兼容

ledger header 为 `KV9LED13`。key tag 29 的 `KV9BRI01` 索引记录固定 152 字节；tag 30
的 `KV9BRP01` 进度最多 116 字节；request tag 27 是 `AdvanceBootstrapIndex`，stage
0/1/2 分别对应三轮扫描，持久 stage 3 表示完成。

12 的 admission、owner 身份与资源 pin 原样升级。旧格式下存在新索引/进度会拒绝；
新 header 不会将旧 grant 自动变成 typed admission。新旧 grant 的查询和退休规则沿用
原协议，既有 bootstrap 本地 receipt 与离线恢复路径不变。store/service floor 仍是 V8；
本轮不认证混合旧二进制部署。

## 验证与边界

新增七项 metadata 测试和一项真实三 metadata Raft 组合测试。覆盖：

- 12 格式升级，旧任意 operation/未知 subject、三种原始 phase 和新精确来源同时保留。
- 游标之后写入但 ID 位于游标之前的新 grant，以及目录完成后的未进入终态，均原子索引。
- 跳页/旧请求拒绝、紧邻重试确认、稳定 snapshot 隔离、孤立注册和孤立 admission 拒绝。
- 损坏索引/进度、查询遇到的孤立索引、header 降级及完成后单 owner 索引丢失检查。
- 13 与 113 个旧 grant 的全局/镜像分页迭代数和点读次数相同，测试禁止 retention 全表 scan。
- 三个扫描阶段的真实 WAL 尾部截断：重开后索引和进度同生效，原请求安全重试。
- 鉴权 HTTP/2 duplex 中丢失已提交页面回执、换 leader、拒绝旧 leader/取消请求、原请求
  重试、共享维护槽位继续扫描，以及所有 driver/service 停止后的 WAL 重开。

最终八项检查通过：396 项测试通过，2 项真实 MinIO 测试忽略；完整 metadata、TTL
runtime、bootstrap、outbox、checkpoint 恢复、workspace 严格 Clippy、格式及差异检查
通过。

初始测试构造旧记录时错误使用受 install evidence 保护的 Snapshot 释放路径；已改用
显式旧格式 fixture，保持生产门槛，相关失败与编译错误日志保留。完整结果与源码摘要见
[验证包](raw-ttl-bootstrap-index-validation-20261005/manifest.json)。

该目录只覆盖登记在此 ledger 的 grant，不证明外部读者不存在，不认证本地 prospective
receipt 已独立落盘，也不说明所有 bootstrap store 已关闭读准入。它没有开启负向 Pending、
旧 Version 或物理对象释放。下一步要结合精确 store read fence、选中镜像、Reader/
Publisher 连续历史及部署来源检查，形成可持续验证的恢复依赖判定；未知 PUT、历史总量
有界、可搬迁恢复、时钟和完整生产部署验收仍属于
[第二阶段完整范围](RAW-TTL-PHASE2.md)。
