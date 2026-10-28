# 复制 capture 计划与跨 source 恢复

2026-10-05；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
接续 [本地持久 capture](RAW-TTL-DURABLE-CAPTURE.md)。本轮把固定计划登记到 metadata
Raft，使新 data leader 在原对象已完整上传时，可以返回原协议 record。第二阶段和
生产验收仍未完成。

后续：[V8 持久/服务隔离](RAW-TTL-CAPTURE-UPGRADE.md) 已排除旧 V7 writer/RPC，
下文版本段落保留本组件初始实现时的边界；完整旧 binary 与部署演练仍未验收。
再后续：[完整 payload 副本接管](RAW-TTL-CAPTURE-TRANSFER.md) 已支持从原 voter
保存的文件继续未完成上传。以下 GET-only 故障预期与“仍需完成”描述属于本包当时
的边界；当前 Plan 还要求 payload 多数副本，缺所有原字节时仍须拒绝。

## 实际入口

1. `plan_migration_image` 从实际 source driver/engine 原子持久化本地计划，释放文件锁，
   再把 descriptor 路由到 metadata leader。计划包含原 source/destination store、
   operation、range、cut、配置提交位置、manifest 和完整协议 record。
2. metadata 在 catalog 锁外，用鉴权 `InspectMigrationCapture` RPC 向精确 source
   store 核对现有文件。source 完整验证文件与 SST，检查实际 driver/engine、range 和
   applied position；不会重新规划，也不访问对象存储。随机 challenge 将回执绑定到
   本次请求，缺席、错误或超时均不产生确认。
3. metadata 获取 catalog 锁、排空先前提案，再核对 root、operation、range、目标身份、
   source endpoint generation 和现有 owners；在同一 term 提交不可变 descriptor。
   已提交的完全相同重试无需再联系 source。登记成功后 source 重载并核对同一文件才
   返回计划。metadata 回执丢失保留原计划和已提交记录。
4. `bind_migration_image` 要求 catalog 已有完全相同的 manifest，才绑定两个 owners。
   通用 retention Acquire/Share 也按 owner ID 检查，改换 owner 类型不能绕过绑定。
5. `capture_migration_image` 要求当前健康 data leader 已 apply 至原 cut，当前 range
   和 destination 一致，并在对象 I/O 前核对两个 Published owners。原 source 文件
   存在时继续原本的上传；其他 source 或原 source 文件缺席时，逐 SST 执行 GET 校验，
   全部通过才返回原 record。恢复分支不 PUT、不生成新镜像、不拼接当前配置。

目录记录使用直接 operation key `\0kv9\0migration-capture-v1\0<operation>`，编码为
`KV9MCP01 | endpoint_generation:u64 | descriptor | SHA-256`。descriptor 上限为
2 MiB + 328 B，沿用本地 `KV9CAP01` 数据；登记记录额外 48 B。解码数据本身不是
独立 source 确认、已提交元数据或对象存在证明。新登记必须持有 RPC 创建的确认值。
初始 endpoint generation 为 0 合法，比较仍要求完整 endpoint 相等。

内部 `CheckpointRetention` 新增有界 `CapturePlan` query，回复摘要绑定 query 类型
和全部字节；原 Apply 请求的大小上限不变。source 查询与 history 查询共享一个
实际阻塞任务许可，等待超时不会释放仍运行的任务。source 文件读取使用既有共享
pending/image 额度；GET-only 校验逐 SST 进行，并等待本地对象任务排空。

## 故障窗口和验证

[新证据包](raw-ttl-capture-catalog-validation-20261005/manifest.json) 保存本轮命令、
源码哈希及开发期间的失败日志，不改写旧证据。

- 真实三副本 metadata/data Raft、分离 leader、鉴权 HTTP/2 duplex：source 核对后
  提交 endpoint generation 变化，登记拒绝且无对象 PUT。这也验证外部 RPC 不持有
  catalog 锁。仅有本地计划时 bind 拒绝，缺独立确认时 catalog planner 拒绝。
- metadata 实际提交后丢失回执，随后拒绝所有 source 查询；相同登记重试成功，
  原 manifest 不变。通用 retention 的错误 subject 和改换 kind 均拒绝。
- 阻塞原 source 的首个 PUT，再转移 data leadership。新 leader 在对象缺席时拒绝，
  不追加 PUT；原上传完成后停掉原 data driver、移除 source 的活动目录，转移 metadata
  leadership，新 leader 返回字节一致的 record/cut/digest，不查询原 source。
- 删除一个对象后 GET-only 恢复拒绝，两个 pins 保持 Published。恢复对象后实际
  learner install/adopt、Raft/WAL 重开，核对原 value 与 expiry；关闭整个 fixture
  后重开新 metadata leader 的 WAL，登记记录保持一致。
- descriptor 截断、身份/operation/配置损坏、超界、对象缺失和 RPC 回执替换拒绝。
  retention RPC 的最大新 query 可通过；空/超界 query 及超界旧 Apply 拒绝。
  缺 ledger header 但存在 owner state 或 registration 的不一致状态不能被当作空记录。
- 原 900 个 TTL value、约 55.4 MB 镜像测试继续覆盖 plan 后写入、未知 PUT、本地
  uploader 重建、降低磁盘额度、完整 install/readback，以及 abort 后复用本地 slot。

测试的对象存储为 MemoryObjectStore，复制传输为 InProcHub/HTTP/2 duplex。
源端失效模拟和句柄重开不等于 kill/power-loss、真实 TCP、MinIO 或跨主机资格。
引擎 233、snapshot/install 23、capture RPC 2、retention RPC 8、metadata 157、
TTL integration 63、锁序 1 和拒绝伪造确认值的编译测试 1，共 488 项不同测试通过，
11 项既有测试忽略。新增绑定断言随后单独重跑通过，严格 workspace Clippy、格式
及 diff 检查通过。旧 TCP fixture 已更新拒绝预期并通过编译，受环境限制未执行。

## 仍需完成

原 source 永久丢失且仍有 SST 未上传时，复制 descriptor 不含缺失 payload，恢复
必须拒绝。当前可由原 uploader 完成后重试，或者走已提交 abort 与新 operation；
尚无向其他 source 转移未上传 payload 的协议。

读取依赖现有 Published Snapshot/Migration pins 和本地任务排空，本轮没有完成所有
Reader/Publisher 的分布式生命周期。源端超时、文件清理、catalog 登记或新 leader
GET 成功均不授权释放未知 PUT、旧 Version、负向 Pending 或远端对象。GC 仍未开启，
完整 portable successor recovery authority、容量稳态、时钟部署和形式化组合仍开放。

磁盘/内部服务 floor 仍为 V7，retention header 仍为 `KV9LED07`；没有新 Raft command
tag，但新增 metadata key 和内部 RPC/query。旧 V7 binary 不执行这些登记/绑定约束，
不能宣称混合版本升级已安全或已有生产部署资格；上线前仍须完成能力隔离与升级验收。
