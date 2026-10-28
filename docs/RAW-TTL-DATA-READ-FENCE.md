# DATA store 的持久化镜像读下限

2026-10-05；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
接续 [metadata 镜像上传准入与盘点](RAW-TTL-IMAGE-PUBLISHER-CENSUS.md)。
DATA checkpoint selection 的 metadata 记录描述已观察到的选择，并不能约束该 store
以后的读取。本轮为实际选择增加本地持久化读下限，作为后续 DATA 全成员关闭协议的
前置能力；尚未提供跨 store 的读关闭证书。

每个 DATA group 使用 store 根目录 `data-read-fences/<region>` 中的 224 字节记录。
记录绑定 cluster、root、node、store incarnation、group creation、完整 manifest
摘要、term/index、conf/version 和 winning generation，使用规范编码与 SHA-256 校验。
同一 store 的各调用方共享 provider、故障状态和实际文件锁；guard 先释放时，
仍存活的 provider 继续保持独占权。读取不建立随历史增长的内存索引。
单次读取有固定字节上限；累计历史文件数量及空间仍未设全局上限。

下限只能单调前进。相同记录重试不写盘，回退 generation、改变 creation、同一 cut
换成另一镜像以及非单调 epoch 均拒绝。原子发布按临时文件写入、文件同步、rename、
目录同步执行；发布失败使该 provider 拒绝后续读取和更新，须停止使用并重新打开
store。恢复读取先同步可见文件和目录；未发布的临时文件不成为下限。文件损坏、
身份错误和目录缺失不会被解释为空记录。延续 StoreGuard 的合作进程独占模型，
不覆盖绕过文件锁的外部删改或替换。

DATA worker 在确认 committed publication、完成正向 ownership handoff、读取校验并
实际排空 GET、成功持久选择精确镜像后推进下限，再上报 metadata selection。已经被
较新选择覆盖的旧 attempt 不能借用新镜像的 generation 来推进。选择成功到下限
发布之间的崩溃保持原下限；下限发布失败则本地 Reader 拒绝继续使用该 provider。
这不把 metadata 中旧 selection 记录升级为新的 fence 证明。

checkpoint 校验、运行态恢复和 retained Snapshot/Migration Reader 在 GET 之前检查
下限。作用域先绑定实际执行身份，因此本地拒绝仍可记录真实的未进入终态；不会
伪造已进入请求的排空。已进入的读取不被下限推进强制取消，仍受已有 Reader pin 和
实际完成报告约束。新 image 的读准入仍需原有精确 owner、publication 和恢复校验。

冷恢复在 WAL/Raft preflight 后、WAL 修补和 SST GET 前检查选中的镜像；若已有下限，
没有选择镜像的纯 WAL 恢复也拒绝。运行时普通 group 入口另有独立检查，以覆盖未配置
uploader 的路径。底层 JointInstaller 的准备、旧选择校验、原始镜像 V1 adoption，
以及 AdoptedGeneration 的恢复入口也检查同一 provider。V2 adoption 若选择覆盖下限
的新 checkpoint，可保留原安装记录而不重新 GET 旧原始镜像；这不释放原镜像 owner。

store 生命周期与内部 RPC service 提升到 **V10**；retention ledger 保持 **KV9LED20**。
生命周期记录仍为 97 字节，magic 为 `KV9LIF10`。发布 Active V10 前先持久化下限目录；
V9 升级保留 node、incarnation、phase 和 root。当前 Active V10 若丢失目录，打开及
writer 检查都拒绝，不能自动重建一个空目录。Prepared/Bound 的初次创建允许初始化。
内部服务仅注册 `kv9.raft.v10`；冻结的 V9 codec/proto 为测试夹具，未注册兼容服务。
跨 V1–V9 的各旧 RPC 路径拒绝分发。

验证覆盖规范记录的截断/损坏/越界、单调性、精确重试、五个发布故障切点、共享故障
状态和独占 store 重新打开；也覆盖 V9 三种 phase 升级及生命周期发布前后各故障切点。
新三副本测试以真实 raft-rs、WAL 和路由 ledger 选择两代 DATA checkpoint，逐 store
确认旧读取零 GET、真实未进入终态、新镜像可读；切换 metadata leader 后，旧镜像与
无镜像 WAL 副本恢复均在修改文件前拒绝，新镜像完整恢复成功。全部停止后重新取得
三份 store 独占锁，重建 provider 并核对持久下限。测试框架同时清理缓存 duplex
连接，并等待仍由 fixture 持有的 store 对应旧服务/上下文实际释放；显式交给离线
恢复者的 guard 不在等待集合中。中止监听不能代替 owner 实际退出。关闭 Reader
准入检查的负向对照会触发旧镜像 GET，修复版本拒绝该读取。

已有迁移 V2 恢复测试扩展为：实际选择新 checkpoint 后推进目的 store 下限，旧
retained 原始镜像在 GET 前拒绝；新 Pending/Version 来源恢复仍能完成。V9/V10
生成的客户端与 dispatcher 通过认证 HTTP/2 duplex 双向验证无协议回退。
这些是组件与进程内故障注入证据，不是完整旧二进制混合部署、TCP/MinIO、断电或
多进程强杀验收。此前六个保护脚本/冻结夹具及封存证据保持原字节。

默认测试并发度的一次运行出现三个既有集成用例失败，分别涉及 metadata 回执
未确认、非预期换主和 checkpoint 等待超时，原日志保留为 `runtime-parallel-dev.log`。
最终 runtime 检查使用四个测试线程，不改变生产期限或测试断言；此验证安排不能
证明高并发或持续负载稳定性。

最终 12 项检查通过：602 个不同 Rust 测试通过，9 个既有测试仍忽略；其中新增
11 个测试。runtime 的 95 个用例全部通过，workspace 严格 Clippy、格式与 diff
检查通过。重叠筛选执行到的同名测试只计一次；此前失败与负向对照日志单独保留。

仍待完成：独立远端 DATA fence 查询与 metadata 固化；完整历史成员、迁入/迁出、
joint configuration、store 替换和恢复来源的盘点；DATA 镜像所有未来 Reader/Publisher
入口关闭与全体终态；可转移 recovery anchor；negative Pending/旧 Version 释放、
未知 PUT 终止资格、物理 GC、长期历史空间界限和完整第二阶段验收。
当前读取下限不授权删除对象或释放任何 Version/Pending。

验证日志、开发失败、增量 patch、源码摘要和最终检查结果保存在
[本轮证据目录](raw-ttl-data-read-fence-validation-20261005/)。
