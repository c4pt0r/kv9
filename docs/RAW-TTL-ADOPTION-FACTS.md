# 持久采用记录与后续 checkpoint 的独立恢复

2026-10-04；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
接续 [采用后恢复校验](RAW-TTL-ADOPTED-RECOVERY.md)。第二阶段与生产验收仍未完成。

## 移除的旧对象依赖

此前，即使后续 checkpoint 已经通过完整发布历史校验，恢复仍需还原原始安装镜像，
才能知道原 TTL activation 与水位。这不仅多读取一次完整镜像，还会让旧 SST 永久成为
该副本的恢复依赖，阻碍后续对象回收。

现在首次 adoption 在完整 engine/protocol/image 验证后，将实际还原镜像的 TTL presence
和水位与安装绑定一起持久化。后续恢复从这个本地记录取得原始下界，再验证当前选中
checkpoint 的对象、发布历史、配置和 WAL。后续选择合法时，仅被原始镜像引用的 SST
不再是此副本的恢复依赖；原始镜像的本地 `image` record、selector、group record、
adoption marker 仍必需，不能删除或用当前 engine 的状态替代。

这没有新增对象删除入口，没有释放任何 ledger owner，也没有把本地 checksum 当成
可跨节点移交的恢复凭据。多个副本、迁移 source/destination、Reader、旧 Version、
pending 或 backup 仍可能引用同一对象。

## 格式、升级和发布

`runtime-adopted` 从 120 字节 `KV9ADP01` 升为固定 **137 字节 `KV9ADP02`**：

| 字节偏移 | 内容 |
| --- | --- |
| 0–7 | `KV9ADP02` |
| 8–39 | 原 destination store/group binding digest |
| 40–55 | 原 installed generation |
| 56–87 | 原 immutable image record digest |
| 88 | TTL presence，只有 0 或 1 |
| 89–104 | 未激活必须全零；已激活为规范 `KV9CLK01` clock 编码 |
| 105–136 | 前 105 字节的 SHA-256 |

零水位的已激活组与未激活组保持区别。解码先限制整个 record 大小，再检查 envelope、
精确绑定和规范 clock；未知版本、尾随字节、损坏或 foreign binding 拒绝，不自动修复。
这是已有本地文件完整性/独占锁模型下的持久事实，不是抵御任意重写磁盘的签名证明。

- 首次 adoption：沿用真实 generation pair 和全部 SST 验证，从该观察提取 clock。
- 旧 V1：精确校验旧绑定，重新还原并检查原始镜像，再升级为 V2。缺对象或内容无效时
  保持 V1 不变并拒绝。不能读取较新的 engine/镜像来猜原始水位。
- V2 重开：检查本地 marker 与 sealed image record 的绑定。若仍选择原始 checkpoint，
  必须还原它，并要求实际 range/clock 与记录完全相等；缺 SST 仍拒绝。
- 较新 checkpoint：保留全部普通 committed ManifestChange、winning pair/history、
  configuration、当前对象及逐项 WAL 校验；clock 不得移除或低于持久记录的原始值。
  不再读取原始 SST。实际 protocol base 的目录授权规则不变。

发布顺序是临时文件写完并 fsync → rename marker → fsync 目录，完成后才返回 adoption。
任何一步报错都不返回 runtime owner。rename 后 sync 结果未知时，重开验证 V2 后再次
同步目录，再交出 owner；不能用“文件可见”代替持久发布。

旧 adoption reader 有 120 字节读取上限且要求精确 V1 字节，因此会拒绝 V2。此处是组的
显式降级隔离；store lifecycle 仍为 V4，group record、selector 和 engine/protocol 编码
不变。测试保留旧 reader 的读取/比较规则，**没有运行旧二进制或证明滚动升级可用性**。

## 验证

[完整命令、日志和源码摘要](raw-ttl-adoption-facts-validation-20261004/manifest.json)另存。
新增四项测试：

- 未激活、已激活零水位、非零水位分别保存、解码、从 V1 升级并实际打开 engine。
- 两种 presence 的全部截断、逐字节单 bit 损坏、尾随字节，以及重算 checksum 后的
  未知版本/foreign binding/不规范 clock，全部拒绝；损坏 marker 不被修复覆盖。
- V1 升级缺原始对象时保留旧记录；恢复对象后升级成功。V2 下如果实际选择仍是原始
  镜像，删除其对象后仍拒绝打开 engine。
- 3 个实际发布步骤 × 首次 adoption/V1 升级，共 6 个 EIO 窗口：只留下完整旧记录、
  完整 V2 或尚未 adoption，重试可恢复。这里是进程内错误注入，不是进程 kill 或断电。

扩展实际 raft-rs 一 voter/一 learner 用例：后续 checkpoint 发布、选择和目标日志压缩后，
用旧 V1 记录重开，证明升级仍记录原始 1000 水位而不是当前 2000；随后删除只属于原始
镜像的 MemoryObjectStore SST，原始 restore 确实失败，但目标能从新 checkpoint 重启、
重放 publication，TTL 和续期版本保留。再删除当前 selected SST，恢复明确失败。

RegionManager 的实际 `adopt_installed` 测试也删除了仅旧镜像引用的对象：缺少 committed
compaction floor 时拒绝；持久写入并读回该决策后，从较新 base 启动成功。这些删除是
隔离测试中的依赖验证，不是生产 GC 的安全证明。

专项集合 **13 通过**；Raft 本地回归 **339 通过、8 忽略、37 过滤**；RegionManager
**18 通过**；TTL 激活组合 **19 通过**。集合有重叠。严格 workspace Clippy、格式和 diff
检查通过。真实 TCP/MinIO、旧二进制与完整 NodeRuntime 未在本轮验收。

## 后续约束

下一步对象回收仍必须证明所有副本的恢复引用已切换，已暴露 reader/旧 Version 与执行者
已排空，未知 pending 得到正确终态，并保留 durable settlement/retention authority。
本轮记录只消除 adopted 副本的一条旧 SST 读取依赖，不能据此 quiesce/release migration
或 Version owner。完整可转移继任恢复锚点、全对象 closure、负向 Pending、GC 与历史
回收、整机预算、TTL split/merge 和第二阶段其余资格仍开放。
