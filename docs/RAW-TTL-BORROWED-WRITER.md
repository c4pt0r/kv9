# 从冻结视图直接生成 checkpoint SST

2026-10-04；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
接续 [SST 借用校验/恢复](RAW-TTL-SST-VIEWS.md) 与 [manifest 编码边界](RAW-TTL-MANIFEST-ENCODING.md)，
本轮移除生产 checkpoint 规划期间的 owned-entry 集合。此前 image 额度只覆盖最终
SST 对象，规划器仍把 key/value 复制进 SstWriter，再复制进序列化 buffer。

## 规划和编码

第一遍借用冻结视图，只为每个 SST 保存记录数与最终编码大小。按原来的 entry 字节
目标分片，默认 **8 MiB**，单条大记录仍单独成表；每个前缀的大小检查仍包括 header、
两个 footer key 和 CRC。总 image 仍限 **48 MiB**。布局元数据按 SST 数量增长，
不保留逐行 key/value 或逐行 offset。

实例 uploader 在布局完成后取得整个 image 的精确字节额度，随后第二遍借用同一份
不可变快照，直接写入每个 SST 的最终 Vec。成功输出的容量与预检大小一致，写入前
检查实际追加长度，最终校验记录数和完整 frame 大小；错误返回时丢弃部分输出并释放
reservation。额度继续随 PlannedFlush、journal 恢复和实际上传生命周期转移。

原静态兼容 planner 也改用借用布局/编码，但仍不自动获得实例预算；其结果在实例
upload_planned 的第一笔对象操作前准入，沿用已有兼容边界。

公开 SstWriter 的 owned-entry API 保留，finish 与借用编码器共享同一 serializer。
空表、记录数、字段长度、总大小、严格 key 顺序、大小加法溢出以及 layout 不匹配均
拒绝，不返回部分对象。原 SST v1 格式、CRC、对象哈希/路径、CF 顺序和分片策略不变。

一个容易误用的边界是 footer 长度：按字典序更大的 key 可以更短，导致最终 footer
比中间前缀的 footer 小。规划保留原有前缀限额检查；编码只按实际写入长度检查最终
buffer，不能用“假设现在结束”的前缀大小错误拒绝已经通过规划的合法表。

## 验证

[本轮命令、日志和源码摘要](raw-ttl-borrowed-writer-validation-20261004/manifest.json)
独立保存，前序原始日志与 manifest 不修改。新增八项测试包括：

- 独立固定 v1 字节 fixture（含 CRC）、三个 CF、空 key/二进制 value、owned API
  字节一致，以及输出容量等于精确对象长度。
- footer 变短时只按最终长度分配，保留原 prefix-limit 拒绝语义；只容纳最终 image
  的共享额度仍能完成实际规划，drop 后额度归零。
- 错误预检大小/记录数、少行、多行、重复或倒序 key 均拒绝；总大小和记录数限制在
  读取 iterator 前拒绝，key/value 字段边界继续检查。
- **168** 组数据、分片目标和总额度组合，对照原 owned-entry 规划器的分片字节、接受/
  拒绝结果、SHA256 与对象路径，包含空 CF、空 image、超大单行和长短 key 变化。
- 第二个 SST 编码的防御性校验失败时，前一个 SST 的 buffer 与 image lease 一并释放。

Engine **201 通过、25 忽略**；本地 Raft **327 通过、8 忽略、37 过滤**；TTL 激活及
三副本 checkpoint 组合 **19 通过**。严格工作区 Clippy、格式及 diff 检查通过。
既有多组共享额度、journal 重开、失败续做和实际 worker 取消排空继续回归。测试集合
与前序证据重叠；InProcHub、duplex HTTP/2 和 MemoryObjectStore 不替代真实 TCP/MinIO
或完整 NodeRuntime 验收。

## 仍开放的边界

消除暂存集合不代表 snapshot 的树节点被释放，也不代表最终常驻索引有预算。manifest
描述符/footer 副本、布局/容器/allocator 元数据、冻结版本保留、恢复索引、协议/TLS
和旧 Vec 接口等仍有独立成本；公开 owned SstWriter 仍由调用者承担其 entries。
本轮没有 heap/RSS 或吞吐测量，不据此宣称整机峰值或吞吐改善的数字。

完整维护预算和物理债务、Reader/旧 Version/负向 Pending 释放、对象 GC、历史有界、
48 MiB 全量 image 替换与驻留优化、TTL 范围转移及恢复锚点、部署时钟资格、独立时间
历史、真实网络/MinIO/旧二进制升级、完整 TLA/TLC/Chaos 和第二阶段生产验收继续开放。
