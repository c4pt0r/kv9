# 上传者终态回执与历史来源解耦

2026-10-05；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
接续 [V9 Reader 生命周期隔离](RAW-TTL-READERS-UPGRADE.md)。本轮修复旧镜像生命周期
收尾中的两项依赖：checkpoint Build 的结束报告已经提交后，重试仍要求原 Pending/
Version 保持 Published；迁移 Publisher 的已结束报告重试仍先读取原 capture catalog
及 source owners。即使原报告、注册和 pins 完整，这些依赖也会阻止 outbox 确认。

## 当前行为

`FinishCheckpointExecution` 和 `RecordCheckpointDrain` 先核对已保留的精确报告。
如果该 generation 确已结束且 issued/resolved 与原记录完全一致，则只确认，不写新
ledger revision，不修改当前代际、pins、索引或 closure stamp。即使同一 owner 的较新
generation 已 Published，也能确认旧报告；旧报告不能结束较新一代。

检查仍包括当前 Build owner、不可变注册、全部资源交叉引用、executor 索引、规范
checkpoint intent、绑定的原 admission gate，以及所请求 generation 的报告和已知来源。
错误 root、Pending token、binding、generation、计数，以及缺失或破损的自身证据仍
拒绝。Released 本身不是报告，旧来源缺失也不会被补造成零 PUT。

Build 的历史识别与当前准入分开：owner/index/enrollment 的一致性检查、drain 查询及
逐代 history 查询使用保留的规范 descriptor/gate。新 Share/Publish、取消、新代登记、
首次 Finish/Record 仍使用原来的完整 Published Pending 或精确 Version 检查。
公开的 checkpoint intent 查询也保留其原 Published 保护要求。Build 的 intent/gate
仍是自身历史的一部分；本轮没有解除这些记录的保留要求。

`FinishMigrationPublisher` 对已完成代际先核对自身 owner/注册/pins、原 operation、
binding 摘要和不可变 PUT 报告，再确认精确重试。确认不再依赖原 capture catalog 或
Snapshot/Migration source rows。首次结束仍验证完整 capture、精确 owner pair 和
至少一个完整 Published source；未知 entry 的零 PUT fence、连续代际和新准入条件
保持原要求。

store/service floor 保持 **V9**，ledger 保持 **KV9LED19**，请求、报告和 owner 编码
不变。没有新增自动 release、历史删除或对象 GC。成功确认允许删除已确认的本地
outbox 副本；metadata 中的未知 PUT 计数继续保留。

## 验证证据

- 两个新回归先实际失败：Build 在同镜像保护转移后拒绝终态确认；Publisher 在模拟
  capture 历史归档后拒绝终态确认。原始失败日志随包保留。
- Build 的三项新测试覆盖原 Pending→Version→相同完整镜像 Snapshot 的真实 catalog
  转移、较新 Published generation、不改任何行的旧 Finish/Record 确认、错误计数/
  token/binding、自身证据损坏、Held/Published/Released 缺报告拒绝，以及新 I/O 不
  因历史观察而获准。删除旧 source rows 只用于模拟未来归档；真实 WAL 关闭重开仍
  保存原报告和较新活跃代，查询与确认不改写 WAL。
- Publisher 的两项新测试覆盖旧代在新代活跃时确认、原 capture/source 历史缺失、
  错误 operation/计数、自身注册/pin/report 缺损与 phase 不一致。首次结束无法借
  终态快速路径绕过来源检查。
- 扩展实际三副本 checkpoint outbox 测试：真实作用域上传产生非零、未获远端终止
  资格的 PUT 报告，结束已提交但回执丢失；随后原 Pending/Version 将保护转移至同
  镜像 Published Snapshot，再切换 metadata leader。重新创建进程 context 和磁盘
  outbox 后，无 uploader 的 follower 后台确认原报告并清空队列。三份 WAL 全部
  重开，已保存报告、未知计数及 Snapshot 保护保持；另一个未保存报告的 scope
  仍 Published/未知，未被补发终态。
- 扩展实际迁移 Publisher 测试：真实 PUT 后记录 `(issued=1, resolved=0)`，Finish
  回执丢失；通过测试 Raft batch 模拟归档 capture catalog，保留两个 Published
  image owners。新进程 context、换主和 follower outbox 重放成功，WAL 重开保留
  原计数与两份保护。此测试不授予生产环境归档 capture 的权限。

所有来源归档均为隔离测试 fixture；没有生产归档器。运行证据使用 MemoryObjectStore、
InProcHub、鉴权 HTTP/2 duplex 和真实本地 WAL，不是 kill/断电、TCP/MinIO 或跨主机
部署验收。命令、日志、增量补丁与源码哈希见
[本轮证据包](raw-ttl-terminal-publisher-validation-20261005/manifest.json)。

本轮 **451 项不同测试通过，2 项依赖真实 MinIO 的既有测试忽略**；新增 5 项 metadata
回归，扩展 2 项真实 Raft runtime 场景。严格 workspace Clippy（含诊断/测试 feature）、
格式和 diff 检查均通过。

## 继续保留的阶段要求

这一步解除已完成报告的确认依赖，不证明全体上传者已结束，也不证明未知远端 PUT
已终止。完整 Publisher census/admission closure、所有 store 与恢复引用、portable
recovery anchor、跨新旧镜像的 Version/负向 Pending 安全释放、历史总量与物理 GC、
资源稳态、部署时钟和形式化组合仍按 [第二阶段](RAW-TTL-PHASE2.md) 的完整目标推进。
