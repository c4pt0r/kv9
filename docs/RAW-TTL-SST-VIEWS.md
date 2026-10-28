# SST 借用校验与未发布恢复索引

2026-10-04；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
接续[对象 buffer 额度](RAW-TTL-OBJECT-BUFFERS.md)，本轮去掉 checkpoint 批量校验产生的
完整 SST key/value 副本，以及 restore 中的完整 WriteBatch。对象格式、image 上限和
发布/采用协议保持兼容；这项改动不代表整机内存或第二阶段生产验收完成。

## 共享校验器与 buffer 生命周期

`SstView` 完整检查输入后只保存 CF、数量和 entry/footer 的字节范围，成功路径不建立
entry 集合，也不复制 key/value。迭代时从已验证的不可变 buffer 返回借用 slice。
总长度、CRC、magic/version/CF、记录数、字段长度、严格递增顺序、非空、footer 范围和
尾部无多余字节的检查继续保留。footer 必须与实际首尾 key 一致。

`SharedSst` 持有原始 `Bytes` owner 和同一套校验结果；restore 迭代期间保持对象 buffer
额度，最后一个 owner 释放才归还。损坏对象在迭代前拒绝，解析错误也会释放它持有的
buffer。这里没有 unsafe、指向自身的引用或逐行 offset 集合。

生产 checkpoint 规划、pending journal 校验、远端读回、Prepared 恢复和 SST restore
使用借用或共享视图。公开的 owned `Sst` 接口继续保留，其解析也通过同一个校验器，
随后显式复制成 owned entries，维持原有二分查找和迭代行为。旧接口仍有分配成本。

## 恢复只构造最终索引

restore 先验证 manifest，再逐个验证 SST，将借用的 key/value 直接复制进尚未发布的
`CheckpointBuilder` 索引。它不能提供读视图或已应用位置；只有所有 SST 完整验证后，
才设置 manifest 的 applied position 并返回 MemEngine。后续对象损坏、读取失败或
校验不匹配时，整个 builder 随错误丢弃，不返回可读的部分恢复状态。

原先的 owned SST rows、完整 WriteBatch 以及 batch 再写入索引的复制已从此路径移除。
恢复后的 `data_revision` 保持原先单个 batch 的语义：只有内部 maintenance 记录时为
0，包含任意数据记录时为 1；同样的系统前缀在非 Default CF 中仍计作数据。后续写入
正常递增 revision，避免恢复改变自动 checkpoint 的触发条件。

## 验证及边界

[命令、源码摘要和原始日志](raw-ttl-sst-views-validation-20261004/manifest.json)独立保存。
新增五项测试覆盖：

- 三个 CF、空/二进制 key/value、前缀 key；entries 与 footer slice 确实指向原 buffer，
  借用和 owned 解析内容一致，重复迭代结果相同。
- 释放原调用者引用后，SharedSst 仍持有完整额度并拒绝下一次分配；释放 SST 后归零。
- 所有截断位置，以及重新计算合法 CRC 后的 magic/version/CF/count/字段长度、重复
  key、尾部垃圾损坏。重复 key 的 footer 同时改为匹配，单独验证顺序检查有效。
- metadata-only 和含数据恢复的 revision 0/1、准确 applied position，以及后续写入。
- 第二个 SST 损坏导致 restore 失败且 buffer 额度归零；修复读取后重新恢复全部三个 CF，
  返回准确位置和完整数据。

Engine **188 通过、25 忽略**；本地 Raft **327 通过、8 忽略、37 过滤**；TTL 激活及
三副本 checkpoint 组合 **19 通过**；RegionManager **16 通过**。工作区严格 Clippy、
格式和 diff 检查通过。测试集合与此前证据重叠；未执行真实 TCP/MinIO、完整 NodeRuntime
或进程 kill，不能把本地组件证据合并成这些验收。

`SstWriter` 规划时仍保存 owned entries；恢复后的常驻索引、持久树节点、snapshot
保留、manifest 编解码、协议/TLS 和旧 Vec 接口副本仍不受 image/buffer 额度完整覆盖。
本轮没有 RSS 或吞吐 benchmark，不能从少了副本推断整机峰值或性能数字。全量 image
仍限 **48 MiB**，索引仍完全驻留内存。

Reader/旧 Version/负向 Pending 释放、对象 GC、协议历史有界、其余维护预算及物理债务、
TTL 范围转移和恢复锚点、部署时钟资格、独立时间历史、真实网络/MinIO/旧二进制升级、
完整 TLA/TLC/Chaos 和第二阶段生产门槛继续开放。

同日后续：[manifest 流式有界编码](RAW-TTL-MANIFEST-ENCODING.md) 已将编码输出的
1 MiB 检查移到扩容前，去掉独立完整 JSON Vec；输入、decode 和整机内存仍未全覆盖。

同日后续：[冻结视图直接编码 SST](RAW-TTL-BORROWED-WRITER.md) 已移除生产 planner
中的 owned-entry 暂存集合，直接生成受 image 额度保护的最终 buffer；公开 owned
writer、常驻索引、snapshot 保留和完整 RSS 仍有独立成本。
