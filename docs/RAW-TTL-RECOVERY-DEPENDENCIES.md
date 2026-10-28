# 运行态保留实际恢复依赖

2026-10-05；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
接续 [逐 store 选择记录](RAW-TTL-REPLICA-SELECTION.md)。第二阶段与生产验收仍未完成。

此前普通 data group 启动与安装后启动都会丢弃成功 open 返回的 checkpoint 观察。
运行态只有当前选择与简化的安装回执，不能回答该次恢复实际使用的镜像、发布位置、
image 中的 range/import/TTL 状态，以及原始安装镜像和本地记录的关系。

现在 `open_with_dependencies` 与 `AdoptedGeneration::open_engine_with_dependencies`
在原有完整恢复成功后生成私有、固定大小的 `DataRecoveryDependencies`。RegionManager
两条实际启动路径都保留它，和 engine 一起归属同一个 `PreparedLocal`。恢复失败或
缺对象不会留下可查询的成功观察。新建但未完整重开的组明确没有
active recovery 观察，不能把它解释成空依赖。

## 记录的事实

- cluster/root/group/creation、实际恢复完成位置、当时的 protocol compacted base。
- selected manifest 的规范编码摘要与 image cut；image 中 Bound DataRange 或完整
  ImportState 的编码摘要；image TTL presence/水位。
- image cut 生效的全部配置 protobuf 摘要及其已知 apply 位置。None 保留“初始或
  snapshot base 配置”的含义，不伪造配置变更位置。
- 普通已发布 checkpoint 的真实 ManifestChange publication term/index、generation
  和 change ID。它们来自同一次成功恢复，不使用 image cut 代替 publication。
- 安装来源的真实 store、generation、sealed image record 摘要及原始 image 的
  manifest/range/configuration/TTL 下界。来自私有 adoption anchor；修改公开诊断
  receipt 的 path、range、cut、generation 或 digest 不能改变这些事实。
- 分别记录 engine open 是否使用原始安装镜像，以及此前 adoption 是否读取并验证
  原始镜像。首次 adoption 和 V1→V2 升级属于后者；V2 重开通常不需要该次读取。
  因而“新 checkpoint 已采用”不能掩盖升级期间实际发生过的旧镜像读取。

只保留固定字段和摘要，不在每个 group 复制完整 SST 列表、range bounds 或 import
cursor。类型没有堆拥有字段，测试限制大小不超过 1 KiB；255 个本地 group 的新增
观察数据上界为 255 KiB（不含既有 group/engine 与编码临时内存）。没有按 checkpoint
代数增长的观察历史。

这些摘要定位已验证 descriptor，**不是完整 descriptor、可转移恢复锚点或释放凭据**。
本地 WAL、protocol continuation、pending journal、原始 installation selector / image /
group record / V2 marker 的持久性和授权仍必要。原始本地记录在新镜像恢复后也不能删除。
该观察不是所有 Reader、migration、backup、Version 或对象 closure 的清单。

## 运行态接口和失效边界

`NodeRuntime::data_group_recovery(region)` 返回本 store 的历史 open 观察，以及从实际
engine 取得的当前 healthy selection。比较状态明确区分 `NoCompletedOpen`、
`SameImageAsOpen` 和 `ChangedSinceOpen`。SameImage 仅表示镜像身份相同，不认证后续
WAL/配置、此刻远端对象可用性或恢复可转移性；Changed 不会给新选择补造恢复事实。
普通组完成本地恢复时可以尚未核验目录中的压缩授权、尚未启动 driver；接口单独返回
`driver_started`。成功 open 和 driver 已启动都不等于公共 range 已获准服务。

当前选择读取沿用 history→WAL 锁与不确定 compaction/poison 检查。查询 failed、
retired、reclaimed 或不存在的 group 返回错误，不返回“没有依赖”。观察没有独立缓存，
不会从被替换/退役的 group 留下一条可被新 owner 借用的记录。调用者保存的历史副本
自然也不能证明 group 仍在线。

现有 group status JSON 增加 `recovery_at_open`，明确输出历史事实；定期 status 不会
重新 GET 对象或扫描镜像。当前 selection 比较只在上述按需接口执行。没有新增 RPC、
ledger owner、磁盘格式或 service floor，V7 / `KV9LED07` 不变。

## 验证

[本轮独立命令、日志和源码摘要](raw-ttl-recovery-dependencies-validation-20261005/manifest.json)
保存实际结果；此前证据包保持不变。

- 普通 WAL 与 checkpoint+tail 两条实际 open；image 水位不被 tail 水位替代，真正
  publication 晚于 image cut；丢失当前对象后新 open 拒绝，没有成功依赖观察。
- 私有返回类型 compile-fail；修改公开 adoption 诊断字段不能替换来源。
- 未激活、已激活零水位、非零水位；V1 升级与 V2 重开的原始镜像读取区分。
- 实际 learner 安装/后续 checkpoint/日志压缩，V1 升级仍读取原始镜像，随后删除
  仅原始镜像引用的测试对象，V2 能从新镜像恢复并保留原始来源事实。
- RegionManager 三个副本完整恢复，运行中选择前进明确标为 Changed；不确定
  compaction 查询拒绝。缺目录授权、失败 open、退役后的查询也拒绝。
- 未完成 TTL import 的三个实际 child 重开，观察保留实际 image 的 ImportState
  摘要；之后换主、继续复制并发布。完整 TTL integration 集合覆盖该路径。

恢复集合 24 项通过、1 项忽略；安装专项 13 项、RegionManager 18 项、compile-fail
1 项、TTL integration 61 项通过，合计 **117 个不同测试通过、1 项忽略**。
严格 workspace Clippy、格式与 diff 检查结果一并存档。开发阶段的类型推导、测试
compaction helper 调用和无效 drop 警告保留为诊断，不计为成功验证。

这些是本地磁盘恢复、MemoryObjectStore、InProcHub 和已有 HTTP/2 duplex fixture
证据，不是完整 NodeRuntime 进程故障、MinIO 持久性、断电、旧 binary 或部署验收。
本轮没有新增完整形式化证明。

下一步仍需将实际恢复输入与可转移 protocol continuation、完整副本/store fence、
Reader/publisher 终态组合成持久继任恢复锚点。旧 Version、负向 Pending、非零/未知
PUT 隔离与物理 GC 均保持未完成；本轮没有释放这些保护，也不缩减第二阶段范围。
