# Checkpoint 执行者逐代排空记录

2026-10-04；基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
本轮继续完整第二阶段，补齐执行者的历史事实；没有开启全局 Pending 释放或对象删除。

2026-10-05 更新：生产 worker 已改用原子执行者收尾，消除 Release 与报告之间的
提交窗口；成功 checkpoint 同时关闭上传准入。新增验证见
[原子收尾证据](raw-ttl-finish-validation-20261005/manifest.json)。下文区分当前路径与
保留兼容的旧分步操作；旧证据包保持原样。

同日后续：[本地持久报告队列](RAW-TTL-DRAIN-OUTBOX.md)已在 metadata 提交前 fsync
实际排空请求，固定槽位于 I/O 前预留；重启可按原 process/generation 自动补交，
取得成功回执后才删文件。它补齐已保存报告的进程恢复，不认证未保存报告的崩溃
遗留者，也不改变旧 unknown 或全局 Pending/Version 的释放条件。

同日后续：[逐代原始准入](RAW-TTL-EXECUTOR-ENROLLMENT.md)新增 05 ledger header
和原子来源行。稠密历史查询逐代保留缺来源、缺报告和旧 unknown；已证明未进入/
未 Publish 的代际拒绝非零 PUT 报告。旧记录不因新一代成功而取得完整关闭资格。

## 修复的缺口

此前 Build owner 在本地排空后通过 Pending/Version 转交保护并进入 Released。
Released 既可能来自实际排空，也可能来自通用转交；新一代还会覆盖当前 owner 的
generation。因此不能从最新 Released 推断旧代际没有未知远端写入。

现在 data/metadata checkpoint 为每个实际 scope 绑定精确 root、Pending ID、
executor ID 和 generation。绑定在 Share/Publish 之前，拒绝重复绑定、已关闭 scope、
仍有 job 或已经发出 PUT 的 scope。外层关闭准入并等待实际 job 全部退出后，才产生
无公共构造器、不可 Clone 的 `DrainedExecution`。嵌套 scope 不能提前出具报告。
worker 只接受自己的精确 scope/generation；报告尚未产生时，不能走该 scoped executor
的收尾。已有无报告旧路径不会因此被自动认证。

## 从实际 job 计数

报告记录 `issued` 和 `resolved` 两个单调计数：

- `issued` 在 MinIO worker 已通过路径和 payload 配额检查、即将调用可能写入的
  SDK `put_opts` 前增加。队列拒绝、过期 job 丢弃、buffer 拒绝或配额等待超时不会
  增加。计数耗尽在发出下一次调用前拒绝，避免回绕。
- `resolved` 只在禁用 SDK 重试的 budgeted client 收到本次 PUT 成功或
  `AlreadyExists` 条件失败后增加。计数发生在后续比较 GET 之前，GET 错误不改变
  已收到 PUT 响应的事实。其他错误、future 取消、panic 保持该调用未确认。
- 未禁用重试的 client 和任意测试 `ObjectStore` 返回成功，也不据此增加
  `resolved`。后一次成功不能证明更早一次超时调用已经结束。

这记录的是实际 job 的响应，不能用 caller 的返回值替代。caller 超时后 job 仍可能
完成或被拒绝；报告必须等实际 job 退出再汇总。`issued=0` 表示本 scope 未进入
可能写入的 SDK 调用；`issued=resolved>0` 表示每次均有允许计入的响应；二者不同
则保留未知写入。后续成功不会消除已经结束但未确认的调用。

AWS 的 PutObject 文档说明，成功响应对应完整对象写入，`If-None-Match: *` 已存在
时返回 412；这是响应分类的协议依据。[AWS PutObject](https://docs.aws.amazon.com/AmazonS3/latest/API/API_PutObject.html)
本实现实际面向可配置 S3/MinIO endpoint；这些计数本身不能证明部署的服务器、代理、
SDK transport 或条件写语义满足终止契约，仍需真实后端资格与故障验收。它们不把
未知结果转换成服务器端取消，也不证明其他执行者或旧 binary 已停止。

## 持久化和重试

新增 `RecordCheckpointDrain {pending, executor, issued, resolved}`（request tag 12）。
metadata 核对精确当前 token、canonical intent、gate、indexed executor、完整
Published Pending/Version 覆盖，且当前 executor 必须 Released、`resolved ≤ issued`。
这是由可信内部执行者提交的观察，不是 metadata 可以独立验证的远端 I/O 事实；
既有内部 RPC 认证和非 Byzantine 成员模型仍是前提。

`KV9CDR01` 位于 retention namespace tag 13，key 包含 Pending、executor 和
generation。固定 176 字节绑定 root、身份、不可变 binding 摘要、gate opened
revision、记录 revision、两个计数及 checksum。每代的第一条记录不可变，精确重试
幂等，不同计数拒绝。查询旧代记录时允许当前 owner 已进入更新代的 Published，
但必须保持相同不可变绑定和完整交叉引用；旧代请求不能修改更新代。

旧 tag 12 仍允许精确 Released executor 追加实际保存的报告，以兼容原分步操作。
它不会从 Released 推断报告。旧路径在 Release 与报告之间崩溃留下的缺失记录，
仍不能用新 process identity 或新一代成功推断补造。

当前 data/metadata worker 在实际 scope 排空后调用
`FinishCheckpointExecution {pending, executor, issued, resolved}`（ledger request
tag 13，与 Raft manifest command tag 不同）。planner 在同一 metadata snapshot
核对 canonical intent、精确 executor binding、index、全部资源 pin，以及完整
Published Pending/Version 保护，再将 Released owner、index、资源 pin 和逐代报告
放入同一个 Raft/engine batch。Held、Published、Quiesced 均无需提交中间态；Held
只接受零 issued，Published/Quiesced 保留实际计数，包括未知 PUT。

从未进入的第一代、Released 后未进入的紧邻下一代只接受零 issued，沿已有取消
规则原子写入终态和报告，阻止延迟 Share/Publish。同代 Released 的精确报告重试
幂等，计数改变、错误 binding、代际跳跃或回退拒绝；已存在的旧代 unknown 或缺失
报告均不变。旧非 indexed owner 不会被直接认证。所有转换继续保留全局保护。

生产收尾只需一次 mutation 回执；回执未确认，worker 保留 exact handle 和实际
计数，阻止它开始新上传。提交前崩溃仍可能留下 Published 且无报告的旧执行者；
提交后则不会由此操作产生 Released 但缺报告的窗口。planner 仍信任内部执行者
对实际 scope 的报告，本地排空和一次原子提交均不证明远端取消或全体 publisher
已经关闭。未升级的旧 writer 仍可使用原分步路径，不能据此宣称历史覆盖完整。

报告丢失不回溯重建，旧 unknown 报告不被新代报告覆盖。记录只提供以后逐代核对所需
的输入；不能仅看最新代或所有 owner 的当前 phase 就得出全体终止。Opened/Closed
gate 与记录查询互相绑定，但本轮不要求报告时 gate 已关闭，正常成功的上传也记录。
所有 owner、报告与未知债务保留；新增常数大小行不代表物理历史总量已经有界。

## 2026-10-05 原子收尾验证

- 五项新增 metadata 测试覆盖各 phase、关闭后的取消和延迟进入、跨代 unknown、
  旧报告缺失、Version 覆盖、错误身份/计数拒绝、codec、snapshot 隔离和实际 WAL
  重开。丢弃未提交 plan、截断 WAL batch 后仍是 Published/无报告；完整提交后
  重开同时看到 Released 和原始 `(issued, resolved)`，重复请求不增加 ledger revision。
- 实际三副本 Raft/认证 HTTP/2 duplex 验证原子提交前拒绝和提交后丢回执；worker
  均停止新上传，恢复确认后继续。Published 时无报告、Released 时已有报告来自
  同一 metadata snapshot 的观察。成功 gate 在本地排空前已关闭。
- 原逐代排空组合增加 Share/Publish 前后四种失败和 gate 关闭后的下一代拒绝；
  实际 scope 零 PUT 报告连同旧 unknown 共八条记录在 metadata WAL 重开后保留。
- 正向交接扩展为全部十一个丢回执前缀，包括 Close；延迟执行者不能重新上传，
  data 和 metadata WAL 重开后 gate 仍 Closed。

本次 metadata 全套 **111 通过、1 项真实 MinIO 测试忽略**；三副本组合 **41 通过**。
扩展后的五项原子测试、跨代排空恢复和两项 worker 故障用例另行复验，去重后共
**152 项通过**。严格 workspace Clippy、格式和 diff 检查通过；完整 TCP runtime
测试仅编译，未重复触发已知 socket 权限限制。

新增 tag 13 沿用已有小请求/owner/batch 上限和 `KV9CDR01` 记录，没有新增存储
格式或提升 V7 floor。旧 decoder 拒绝未知操作，worker 不回退到分步收尾；这不
替代混部升级资格。历史证明没有自动扩展为新增原子操作的完整 refinement 证明。
测试仍使用本地 InProcHub/duplex 和对象测试后端；真实 TCP/MinIO、进程强杀和
旧二进制部署验收继续开放。

## 2026-10-04 验证

[独立证据包](raw-ttl-drain-validation-20261004/manifest.json)记录命令、输出及源码摘要。

- 五项新增 engine 测试覆盖实际排队 job 尚未退出时无法出具报告、嵌套/重复/事后绑定
  拒绝、opaque backend 不认证、成功及条件失败计数、比较失败不撤销已收到的 PUT
  响应、配额超时发生在 issued 增加之前，以及 SDK future 超时后重试成功仍有未知。
- 三项 metadata 测试覆盖非 Released 拒绝、精确 token/codec/截断、幂等与不可改写、
  新代 Published/成功后旧 unknown 仍可读、错误计数和破损交叉引用拒绝。
- 新增实际 Raft/认证 duplex 组合跨三个代际：第一代无 PUT 的报告提交后丢回执，
  精确重试不改变记录；第二代 opaque backend 上传成功仍记未知；第三代无 PUT
  不覆盖第二代。受控重开 metadata WAL 后三份记录和全局 Pending 均核对成功。
- 自动 data worker 新测试在报告提交后丢回执、继续拒绝确认：owner 已 Released
  仍不能开始新上传；确认恢复后继续。已有 data 和 metadata 对象队列阻塞测试增加
  报告核对，实际过期 job 丢弃后两条生产路径均持久记录 `(0,0)`。
- engine **210 通过、25 忽略**；common/meta **208 通过、1 忽略**；TTL 组合
  **32 通过**。严格 workspace Clippy、格式和 diff 检查通过。

网络仍为 InProcHub/duplex，SDK 后端为 InMemory/受控延迟；这些测试没有验收真实
MinIO、TCP、完整 NodeRuntime、process-kill、旧 binary 或部署环境的远端终止语义。

## 组件证明和后续

[Lean](../proofs/lean/checkpoint-drain/Drain.lean)对任意长度的一个精确 scope/generation
历史，证明计数不变式、issued 单调、永久 unknown 单调、报告只在关闭且实际 job
排空后产生、零 issued 排除该 scope 的写入债务、相等计数排除 unknown、报告不变和
成功重试不能抹去 unknown，共十条组件定理。七个错误变体和两个 proof-policy 控制
被拒绝，axiom audit 通过。

模型的 Reply 指满足单次请求终止契约的响应，Abandon 指实际 attempt 退出且没有
该响应；不从超时推断远端取消。唯一 attempt token、真实 I/O 与步骤的对应、可信
执行者、后端终止契约、所有旧代报告完整性都是组合前提。没有 Rust 自动提取、
跨代 ledger 的完整 refinement 或全局闭包证明。最初泛化 simp 触及 Lean heartbeat
上限，已改为逐转移证明；没有提高限制或引入承认项。

[TLA+](../proofs/tla/checkpoint_drain/CheckpointDrain.tla)通过 SANY 解析和语义检查；
TLC 沿用已知 socket/RMI 环境限制未运行，没有再次启动已知失败的服务。

下一步必须解决没有报告的已 Published 崩溃遗留者、远端未知 PUT 的可靠终止/
写入隔离，以及旧代报告与 legacy publisher 的完整集合核对，才能安全释放负向
Pending。Reader/旧 Version、TTL split/merge/迁移、完整恢复锚点、历史容量、
整机预算、部署时钟和完整 Chaos 仍在完整第二阶段范围中。
