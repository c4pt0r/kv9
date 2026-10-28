# 已登记 checkpoint 尝试的后台发现与续做

2026-10-04；基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
第二阶段、完整 owner 生命周期和生产验收仍开放。

2026-10-05 更新：后台发现已接通精确提交历史判定，不再只处理已 Closed 的尝试。
正向历史可完成 Pending→Version 交接，负向历史只关闭准入并保留 Pending。
新验证见[无 journal 的结果收尾](raw-ttl-discovered-settlement-validation-20261005/manifest.json)。

同日后续：本地没有目标组历史时，已接入有界、鉴权的远端精确窗口查询。
见[远端历史验证](raw-ttl-remote-history-validation-20261005/manifest.json)。

## 本轮解决的问题

[未 Publish 执行者回收](RAW-TTL-UNPUBLISHED-RETIREMENT.md)此前依赖本地恢复 worker
持有 journal。现在 metadata leader 可从持久 admission gate 的 key 区间发现尝试，
即使原进程和 journal 已不存在，也能继续处理 Closed gate 下的 indexed Held 代际。
它不从对象 LIST 猜测 owner，不依赖配置 remote uploader，也不重新上传 checkpoint。

发现覆盖已有 gate 的尝试。没有 gate 的旧 intent/owner、完整 legacy backfill
仍待处理。单个 gate 的存在不能证明所有历史 publisher 均已登记。

## 精确历史收尾（2026-10-05）

扫描器读取同一 metadata snapshot 的 intent、gate 和 Pending phase，然后在
本节点实际 group driver 查询 `predecessor + 1` 的已提交历史。metadata 用自身
driver；data 使用 lifecycle 登记的 group handle 或 Raw directory 的精确 region，
包括有实际 handle 的 importing/sealed group。不会把 metadata 历史或另一个
region 的 leader 当作目标组的历史，也不重新 mint 单实例 ManifestSeam。

当前 pair 必须已达到目标窗口。历史必须是可解码的 checkpoint，cluster、region
和记录的 `(term,index)` 都须一致；当前 pair 恰在该窗口时，其 change ID 也必须
与历史相符。规范 descriptor 和 predecessor 推导的 change ID 相同，才表示这次
identity 已应用；不同则表示该精确 CAS 窗口由其他 identity 占用。较新的当前 pair
单独不能证明拒绝，缺历史、legacy opaque 历史、未消耗的窗口都保持 Unknown。
这条路径不扫描后续 effect coverage，也不从 SST 缺失或进程退出推断结果。

正向结果调用既有 owner 交接：Record、Close、Share、Publish、Quiesce、Release，
完整 Version 覆盖先于 Pending 释放。负向结果只 Close，随后由原有分页回收未
Publish 的执行者，负向 Pending 和已 Published 的遗留执行者都保留。所有写入和
owner 读取前再次检查取消、统一 deadline、serving 状态及 metadata leader 身份。
任何未确认回执立即结束本 pass，下一次重新读持久 phase；不推断该操作没有提交。

`Closed` 和 Held sweep 的 `Done` 均不能跳过未完成的正向交接，因此在 Close、
Share、Publish 或 Quiesce 后失去原 publisher，也能由后续扫描器继续。完整交接后
的 Closed/Released 尝试不再重提正向操作；Done 页也不重复写进度。

本地历史缺失时可查询远端实际副本；本地历史损坏仍报错，不被远端结果掩盖。
这不是全体 publisher 的关闭证书，不能据此释放旧 Version、Reader 或未知远端
PUT 的保护。

## 远端精确历史（2026-10-05）

新增只读内部 RPC `InspectCheckpointHistory`，返回精确 `predecessor + 1` 窗口
中已提交的规范 change ID 或显式 Unknown。请求和回复绑定 root、cluster、node、
store incarnation、region、predecessor 及每次新生成的 challenge。调用方自行
比较 canonical intent 的 change ID；缺回复、错误、缺失窗口或未知方法均不能
当作负向结果。服务端在实际读取前后都检查现有 V7 StoreGuard 格式凭据和接收资格，
runtime 另检查 serving、root 与 deadline。实际 follower 也可提供已提交的不可变
历史；仅凭本地尚未看到窗口不能证明窗口未提交。

目录暂没有供该维护链路使用的完整 placement 索引，因此从同一 metadata snapshot
的 admitted node 行有界寻找候选，每页至多八条，跳过本节点与非 active 节点。
同一轮 attempt 扫描保持同一个候选节点，扫描环回才轮换节点，避免两个游标同时
递增而漏掉某个 attempt/node 组合。每次查询重新读取 endpoint；地址只用于定位，
接收方必须持有目标 region 的实际 driver。没有增加全 task/range 扫描或随 attempt
数增长的 map，只多保留当前候选和节点游标两个字段。

每个 pass 最多一次远端 RPC，连接和请求共享至多一秒预算，并受原五秒 pass
deadline 限制。服务端最多接受两秒预算，业务消息上限 512 字节；客户端 codec
也限制 512 字节。服务端仍先经过现有共享 gRPC decoder，512 字节业务检查不等于
独立的传输层内存上限。每个 service 实例只有一个历史读取槽，真实阻塞 job 持有
permit；超时/取消不会提前释放槽或让新请求叠加实际读取。

已知结果返回后，调用方再做 metadata ReadIndex 并核对相同 root 和完整 endpoint，
包括 generation、store incarnation、地址和 active 状态。即使地址不变，只改变
generation，也丢弃本次证据。后续 owner 操作仍逐次检查 metadata leader、serving、
取消和 deadline。Unknown 不写 gate；候选拒绝或网络错误记入 `remote_errors`，
保留保护并允许下一轮继续寻找其他节点。

该 RPC 是 V7 内的只读增量，未增加磁盘格式或 ledger request，也没有旧方法回退。
老 V7 实例不支持此方法时保持 Unknown。有限且稳定的目录/attempt 集合、持续持有
历史的可达副本和有进展的调度是遍历前提；全 placement、不断增长的历史、持续
leader 抖动以及固定收敛时限尚未验收。

## 持久分页与安全边界

新增 `AdvanceCheckpointSweep {pending, expected, after, next}`（request tag 11）。
metadata 在同一规划 snapshot 中验证精确 Pending token、规范 intent、Published
Pending/Version 覆盖和 Closed gate，然后重读当前四条 executor 页：只有该页已无
Held 且 `next` 等于真实分页结果时，才能按 sequence/after CAS 前进。关闭后不能
新增 Held；后来的 Cancel tombstone 只有 Released，因此不会在游标后方插入待回收
的 Held。并发本地 worker 的回收可使 CAS 重试，但不能跳过尚未处理的 Held。

`KV9CSW01` 位于 retention namespace tag 12，以 Pending ID 为 key。单条最多
139 字节，绑定 root、Pending、gate 关闭 revision、sequence、前后游标、Done
及 checksum。精确上一条 CAS 可幂等重试；更早的请求不能覆盖新进度。游标损坏拒绝；
进度行缺失只会从头重扫，既有 gate/owner/回收证据仍需完整。此处不是对任意元数据
丢失的自动修复协议。

完成页写入持久 Done，此后不为重复扫描不断追加进度。未完成的正向交接仍可续做。
Done 仅表示这次
indexed Held sweep 已完成；Published、Quiesced、普通 Released 可能依然存在。
它不证明远端 PUT 已终止，不授权负向 Pending、旧 Version、Reader 释放或对象删除。
新增进度与原 owner/evidence tombstone 均保留，物理历史容量仍待治理。

## 后台调度

`NodeRuntime` 启动一个后台任务；全局 gate 扫描只由 serving 的 metadata leader 派发。
后续[持久报告补交](RAW-TTL-DRAIN-OUTBOX.md)复用该维护槽：全局 gate 扫描仍只由
metadata leader 执行，其他 serving 节点也可补交本地保存的排空报告。leader 在
本地报告和全局扫描之间交替，每个 pass 只选一种工作。
每个获准 pass 最多发现一个 gate。需要结果收尾时，最多执行一次正向交接（六个
mutation 请求）或一次负向 Close，并返回；其余 pass 至多处理四个 owner，提交
四个回收和一个进度请求，不在同一 pass 叠加交接和分页工作。
读取页与提交前复核各最多访问五条 index（四条加 lookahead），不是跨页固定 snapshot。
节点保留一个 attempt 发现游标及两个远端节点轮询字段，不建立随 attempt 总数
增长的内存 map。

发现游标每处理一个 key 就前进，包括该 key 的 value 校验失败时。每次全局循环中
每个尝试至多处理一页，大尝试不会独占扫描；重启/leader 切换丢失全局发现游标时，
各尝试仍从已提交的分页进度继续。合法 key 下的破损 gate/intent/cursor 会报错，
后续 key 仍能得到处理；header 或 key 本身损坏则拒绝发现，不能安全猜测跳过位置。
没有宣称无限 leader 抖动或无限历史增长下的固定收敛时间。

调度间隔 250ms，每个实际阻塞 job 共用 5 秒请求 deadline；ReadIndex 单次至多
1 秒。deadline 约束协议等待，不是文件系统阻塞的硬时限。新增第八类共享维护
`checkpoint_retention`，同节点最多一个，属于轻任务；原总并发四个、重任务两个、
burst 四个和每 10ms 补一个令牌的限制不变。实际 job 持有 permit；取消会停止后续
操作，关停等待真实 job 返回后再释放 backend/store 生命周期。

状态输出新增 `checkpoint_retention`：运行/并发、发现数、环回、Open/Done、页数、
已观察 owner 数、确认回收数、失败及最近错误。异常退出会进入节点现有健康检查。
新增 `positive_handoffs`、`negative_closures`、`history_unknown`；前两者只计本次
调用取得完整回执的次数，不能当作所有历史结果的精确总数。
远端查询另计 `remote_queries` 和 `remote_errors`，不把 Unknown 当作查询错误。
该任务已接入产品启动/关停代码；测试使用实际 runtime backend，完整 NodeRuntime
进程启动、故障注入和 RSS/CPU/磁盘预算验收仍待完成。

## 2026-10-05 验证

- 无本地 journal/uploader 的 metadata/data 两组，每组覆盖旧窗口 winner、更新
  winner、旧窗口 loser 和未消耗窗口；后台完成四个正向交接、两个负向关闭，
  回收两个 Held，保留两个 Published 执行者及负向 Pending。
- 正向交接六个提交位置逐一丢回执；即使先把 Held cursor 推到 Done，metadata
  leader 转移后仍能继续。实际 metadata WAL 重开保留 Closed/Done、Released
  Pending 和 Published Version，终态扫描不再追加日志。取消发生在第一个 ledger
  mutation 后时，后续 handoff 调用停止。
- 本节点无目标副本、历史行缺失或 opaque 时保持 Unknown；截断记录、watermark
  不符、pair/history 不符拒绝，不关闭 gate、不释放 Pending；恢复合法记录后续做。
- 实际 metadata/data checkpoint worker 与全局维护同时运行，继续提交和采用
  checkpoint；关停排空后已发布的最新两组镜像均有 Published Version，公开 TTL
  写入仍可读取，共享任务占用归零。

新增四项组合测试，与原两项 sweep 回归共 **6 项通过**；严格 workspace Clippy、
格式和 diff 检查通过。验证范围为该维护链路，完整阶段验收继续开放。

上述本地历史收尾未增加 RPC、ledger request 或磁盘格式；后续远端查询新增了
一个只读 RPC，当前 floor 仍为 V7。组件测试使用真实
Raft、认证 HTTP/2 duplex、内存对象后端和受控 WAL 重开；未据此宣称真实 TCP、
MinIO、进程强杀或旧 binary 部署已验收。原 Held sweep 证明保留，新增历史收尾
尚未完成组合 refinement 证明。

远端查询新增四项 RPC 测试，覆盖鉴权、活动 store 和格式隔离、所有关联字段、
精确窗口与 challenge、边界/错误 oneof、读取中撤销资格，以及超时/取消仍占用
真实读取槽。另四项 runtime 组合测试通过认证 HTTP/2 duplex 验证：

- metadata leader 无目标组查询 handle，首个候选也无 handle；一轮完整遍历后
  转向真正持有历史的 follower，正向交接、负向只关闭和未来窗口 Unknown 各自成立。
- 候选拒绝时不修改 gate/owner，后续候选继续完成正向交接。
- 回复期间实际提交同地址 endpoint generation 变更，本次结果不能修改 owner，
  使用新 generation 的新查询可以继续。
- 错误 token、未准入 caller 和错误 store 被拒绝；不存在的 group/窗口为 Unknown。

这些 fixture 中 Raft driver 仍存活，仅从 backend 移除目标 group 的查询 handle；
没有据此宣称部署了完全不重叠的副本 placement。完整进程、真实 TCP、MinIO、
混合二进制和资源稳态验收仍开放。

本轮完整 TTL 集成 **49 项通过**，RPC **4 项通过**，V7 格式隔离 **4 项通过**，
去重共 **57 项通过、0 忽略**；其中后台 sweep 的 10 项已包含在完整集成中。
严格 workspace Clippy、格式和 diff 检查通过。原始日志、命令和源码摘要保存在
[远端历史证据包](raw-ttl-remote-history-validation-20261005/manifest.json)，既有
51 份历史 manifest 及其 433 份独立检查日志的摘要已核对，未刷新历史源码摘要。

## 2026-10-04 验证与证据

[本轮证据包](raw-ttl-sweep-validation-20261004/manifest.json)保留命令、输出和源码
摘要，历史证据包保持原样。

- 四项 metadata 测试覆盖未关闭拒绝、禁止跳过 Held/伪造下一页、规范请求与截断、
  精确 CAS 重试和旧请求拒绝、Done 后仍有 Published/Quiesced、取消下一代不重入、
  游标破损拒绝、缺失进度安全重扫、发现区间排他性和破损值后的下一条发现。
- 真实 Raft 和认证 HTTP/2 duplex 组合：两个没有 journal 的尝试各含九个 Held 和
  一个 Published。首个回收提交后丢回执，下一轮先服务另一个尝试；分页进度提交后
  再丢回执，重新创建扫描器从持久进度续做。leader 转移到节点三，旧 leader 拒绝
  派发，新 leader 完成后重复扫描不追加命令。受控停止并重开节点三 metadata WAL，
  两条 Done、十八份回收证据、两个 Published executor 与两个 Published Pending
  均核对成功。
- 自动任务测试在无 uploader 的三个 backend 上运行：第一个尝试的 gate value
  损坏时报告错误，第二个仍完成；只有 leader 发现和派发，单任务并发峰值为一，
  cancel/drain 后共享占用归零。额外预算测试验证重任务占满两个槽时仍能与 TTL
  共存，且取消调度器不提前释放实际 job 的 permit。
- common/meta **205 通过、1 忽略**；TTL 组合 **30 通过**；维护测试 **5 通过**。
  严格 workspace Clippy、格式和 diff 检查通过。

开发中损坏值测试最初使用了错误的 namespace 字节，存在性断言阻止了无效注入；
修正为真实 retention prefix 后测试通过。所有组合测试仍是 InProcHub/duplex，
受控 WAL 重开不能代替真实 TCP、MinIO、process-kill 或旧 binary 滚动升级验收。

## 模型范围

[Lean](../proofs/lean/checkpoint-sweep/Sweep.lean)以任意有序 ID 域和任意长度历史，
归纳证明已扫描前缀不含 Held、Done 排除域内 Held、游标/sequence 不倒退、全局
保护保留，并构造“Done 但 Published 仍在”的可达反例，共九条组件定理。七个语义
错误变体和两个 proof-policy 控制均被拒绝；axiom audit 通过。

前提是 gate 已关闭、没有新的 Held、页覆盖完整、Raft 原子提交且持久进度未丢失；
`visits ≤ 4` 是调用约束，不是从 Rust 提取的成本证明。模型不覆盖删除破损进度后的
重扫，不将 legacy 全集合完整性、实际 RPC/RSS 成本或全局调度活性作为已证结论。
Lean 校验器已识别实际 `rfl` 和 application type mismatch 两类语义拒绝输出。

[TLA+](../proofs/tla/checkpoint_sweep/CheckpointSweep.tla)显式保留各尝试持久进度，
volatile 发现游标、损坏尝试和 Published 集合。SANY 解析/语义检查通过；TLC 沿用
已知 sandbox socket/RMI 限制未运行，没有重复失败启动，也没有 Rust refinement。

下一步继续取得已 Published 遗留者和远端未知 PUT 的可靠终止凭据，再组合精确
manifest 历史与完整 publisher 集合推进负向 Pending 释放。Reader/旧 Version、
TTL split/merge、迁移、整机资源、历史容量、部署时钟和完整 Chaos 均继续保留。

2026-10-05 后续：[Build catalog 回填](RAW-TTL-BUILD-INDEX.md)已接入同一维护槽。
leader 的非报告任务交替处理一页全局回填和一个 checkpoint 尝试，避免为每个
尝试扫描全部 owner 历史。旧 Build 独立登记，原 executor Held sweep 的可回收
集合保持原含义；新增 `build_index_*` 指标不代表 publisher 关闭或 GC 许可。
