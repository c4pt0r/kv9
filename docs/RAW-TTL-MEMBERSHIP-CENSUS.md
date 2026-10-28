# DATA 历史成员的持久盘点与来源轮转

日期：2026-10-06。接续[独立成员查询](RAW-TTL-MEMBERSHIP-RPC.md)和
[保留 placement 历史](RAW-TTL-DATA-STORE-HISTORY.md)。本增量把实际来源的查询页、
分页进度和已见节点保存在 metadata Raft，并接入原有 checkpoint sweep。
本地来源一轮完成不表示所有历史 store、snapshot 祖先或恢复依赖已经覆盖；
不授予 DATA 镜像关闭、旧 Version/负向 Pending 释放或对象 GC 权限。

## 持久记录与信任边界

retention header 从 `KV9LED22` 升至 `KV9LED23`，磁盘/内部服务 floor 保持 V11。
新请求 tag 38 仅包含精确 region/creation/node/store、来源 task、预期 sequence 和
placement task revision；调用者不能提交成员页作为证据。普通 ledger planner
拒绝该请求，实际 runtime 独立查询来源后，在 metadata catalog 锁内再次核对完整
endpoint generation 和当前任务修订，再产生原子 ledger 写入。

namespace 52 保存每来源的进度，namespace 53 保存成员分行。来源区分同一 node 的
不同 store incarnation；节点今天的 endpoint 不会替代其历史磁盘。旧 ledger 可显式
Initialize 升级；旧 header 下若已有新记录则拒绝。没有增加写入格式或 GC 权限，
也没有新增内部 RPC。这里的格式检查不构成完整旧 binary 或滚动升级资格。

一次记录至多处理四个节点，保留角色并集、首次/最近观察位置和轮次。每个节点可
关联一个经过原 task 独立核对的保留 placement witness；这只是已知来源线索，
**不是该 node 所有历史配置区间与 store incarnation 的映射**。未找到 witness 的
节点原样保留并计数，不用当前节点目录补造历史来源。

每来源最多保留 4096 个不同节点；超出后保留已有行并持久记录 overflow，不能据此
声称完整。源历史缺失、snapshot 未知祖先等 gap 一旦观察到就持久保留，后续正常页
不能自行清除。配置位置回退也会拒绝。进度和成员规范记录分别最多 347、354 字节；
这些局部上界没有限制整个集群的来源数量、历史总量或 RSS。

## 重试、查询和后台续做

分页进度来自 metadata 持久记录。task revision 或 endpoint generation 改变时，
从头重新核对；源本地历史变化使旧 cursor 失效时，runtime 允许在同一总 deadline
内进行一次独立首屏读取。错误/Unknown 不会变成空页或完成状态。新轮保留旧节点
观察，不删除之前的缺口。

如果上次提交成功但回执丢失，完全相同的最近一次请求可以从持久摘要确认；换主、
来源失联或 catalog 后续变化不影响这个历史回执。重试不刷新来源事实。完整轮次的
相同 fingerprint、placement revision 和 endpoint 首屏确认也不新增持久事实。

嵌入式查询 `data_membership::members` 每页四行、最多读五行，游标绑定精确来源和
进度 sequence，终页必须满足保留行计数。`placement_current` 单独说明当前保留任务
盘点是否仍与观察相符；它不证明实际远端成员仍然新鲜。`round_complete` 与
`has_known_gaps` 仅描述这个来源，缺少已知 gap 也不是全体 store 完整性证书。

既有 Build/Reader/bootstrap/placement census 完成后，checkpoint sweep 每个名额
处理一个不同来源；prefix seek 跳过同一 store 的重复 task。即使来源拒绝，调度游标
也向前，避免阻塞其他 store。来源进度持久化，内存游标仅决定下次调度位置；遍历到
末尾只表示调度回绕。新增统计为累计处理/失败/回绕/已见完整轮次与已知 gap 次数，
不是剩余工作或全局完成比例。

实际 retention worker 持有原准入名额，内部来源读取不重复获取同一 outbound 名额；
公开独立检查仍在派发前获取名额。RPC 读取使用原共享容量，取消/超时不提前释放
实际 worker。此增量不进行对象存储 I/O。

## 验证

新增六项 metadata 测试覆盖规范请求、旧请求确认、修订变化、未知节点保留、查询数量
校验、WAL 重开、同节点两个 replacement store 与重复 task 去重、升级孤儿记录、
snapshot 未知前缀的持久保留，以及 13/113 组 catalog 下相同的分页读成本。

两个新增三副本测试使用实际 raft-rs、磁盘 WAL/Raft 和鉴权 HTTP/2 duplex。一次页
提交后丢回执，metadata 换主且来源拒绝查询时仍确认原记录；实际移除更多 learner
使旧 cursor 失效，新轮重新核对且保留未知节点，三份 metadata WAL 重开结果一致。
另一个测试拒绝查询期间变化的 endpoint，随后验证 Unknown 来源不阻止其他 store
自动完成，恢复该来源后继续原盘点。附加 learner ID 没有对应第四台真实磁盘。

最终相关回归 **448 个不同测试通过**：metadata 339、成员 RPC 5、四线程 TTL runtime
104；一个既有真实 MinIO 测试忽略。严格 workspace 全 target Clippy（含 tracing 与
checkpoint-testing）、fmt 和 diff 通过。原始命令、失败、源码摘要和补丁见
[验证包](raw-ttl-membership-census-validation-20261006/manifest.json)。

开发中三处测试夹具错误及一次 lint 失败均保留：空扫描未复制 WAL 恢复数据、替换
store 时重复插入节点、修改 fingerprint 却沿用旧 cursor、以及整除判断 lint。
第一次完整 runtime 有三项旧测试失败：Build/Reader/bootstrap 盘点完成后不能继续
断言整个 metadata Raft applied index 不变。共享通道会执行成员确认 Noop；修正后
逐字节比较全部 owner、索引及 placement 记录，原阶段/pin/WAL 恢复断言保留。

下一次 runtime 中，共存测试在已发布 generation 1 后累计六次交接失败，诊断包含
实际 catalog busy 与 metadata 换主。生产的指数退避在此时会把后续尝试推迟到旧
20 秒检查窗口之后。仅该测试第二代 checkpoint 的恢复窗口改为 90 秒，覆盖现有
最大 30 秒退避与两个 30 秒 worker pass；初始及末尾等待仍为 20 秒，并补充副本角色
诊断。生产超时/退避没有变化。这是恢复检查，不证明 20 秒进度 SLO 或持续竞争下
的收敛上界。最终四线程运行中该共存用例实际超过 60 秒后通过；catalog 竞争与退避的进度代价
继续作为待解决的稳态/延迟问题。定向与最终并发运行均保留，单次通过不能替代长期负载资格。

没有新增真实 TCP/MinIO、完整 NodeRuntime、进程 kill、旧 binary、跨主机时钟、
Chaos/TLC 或完整协议证明。更早的未知交接错误仍无唯一确立原因，本轮日志不反向
证明历史错误原因。

## 后续完整范围

下一步仍需把实际历史配置的 node 与各历史 store 区间关联，处理缺失磁盘和 snapshot
祖先，覆盖安装/迁移/备份/其他恢复来源及可转移恢复锚点。只有来源资格、所有 reader
和 publisher 生命周期、未知 PUT 隔离与关闭协议成立后，才可推进旧 Version/负向
Pending 的安全释放和物理 GC。历史总量、整机资源稳态、merge、时钟和真实部署/故障/
形式化验收继续按[第二阶段](RAW-TTL-PHASE2.md)与[完整范围](RAW-KV-PRODUCTION-SCOPE.md)
推进。本次新增盘点不关闭其中任何完整验收项。
