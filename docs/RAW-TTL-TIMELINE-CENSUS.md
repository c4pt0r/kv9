# 实际配置时间线的持久盘点与后台续做

2026-10-06。接续[逐次配置查询](RAW-TTL-CONFIGURATION-TIMELINE.md)，现在每个精确
source 的实际配置记录及分页进度一同提交到 metadata Raft。后台轮流推进成员、
变更来源和完整配置时间线。完整历史区间/store 对齐、所有恢复来源和生产验收仍
未完成；本组件不授权 DATA 关闭、对象保护释放或 GC。

## 来源与持久状态

新请求 tag 40 `RecordConfigurationTimeline` 仅包含 region/creation/node/store、
placement task、预期 sequence 和任务修订；请求不能携带自称真实的配置页。普通
ledger planner 拒绝该请求。Runtime 在 catalog 锁外取得鉴权的远端配置观察，
随后取得锁并排空当前 metadata term，再核对完整 endpoint generation 和来源任务。
每条已知变更来源在持久 planner 中重新对照原始不可变 task、operation、动作、
node/store、creation、范围与规范 task 行摘要，不能仅凭远端描述写入已确认来源。

namespace 57 保存 `KV9DTP01` 进度，58 保存 `KV9DTE01` 配置记录。记录按连续观察
序号排列，完整保留 initial、applied 和 snapshot 边界以及四种成员角色、auto_leave
和已知来源。Raft index 不是唯一键：旧记录可能缺少 term，snapshot 也可能与某次
apply 共享位置；这些不确定性不能通过覆盖或合并消失。

已保存记录不可改写，新增项必须紧接已保留前缀。每页最多四条；进度与记录、首次/
最近观察位置和轮次在同一批 metadata Raft 写入中提交。读取进度同时核对其实际
持久页、保留尾部和无超前记录；缺失页、孤儿行、记录冲突均拒绝，重试不静默修复。
分页读取逐条检查连续序号，最多返回四行并读第五行判断后续；查询游标绑定来源和
进度 sequence，不能跨持久进度变化续用。

完整 joint 配置页可以超过 64 KiB，因此页/记录长度采用 u32 framing。进度记录
上限 147,685 字节，单条记录上限 33,188 字节；外层校验后仍执行配置 wire 的
长度/成员数约束。每页有界不等于全局历史、常驻索引或整机 CPU/RSS/磁盘已受限。

retention header 从 `KV9LED25` 升到 `KV9LED26`，V11 store/service floor 不变。
V25 可 Initialize 升级，既有来源页和原生初始 store 映射保持原字节；旧 header 下
存在任一新 namespace 则拒绝升级。没有新增 Raft 日志类型或公开 KV API，亦不
构成旧 binary 混部或保持服务的滚动升级资格。

## 重试、未知历史与维护

相同最近请求的丢回执重试可根据持久请求摘要确认，不再依赖来源仍可达。换主和
WAL 重开保留原进度与记录。任务修订或 endpoint generation 变化从首屏重查；
远端历史变化使持久游标失效时，在原总 deadline 内最多重查一次首屏。错误与
Unknown 不能转换为成功空页或完整状态。稳定首屏确认已有完整轮次时可以不写入，
但仍核对前缀，且不刷新原历史观察时间或授予新的新鲜度保证。

`round_complete` 仅表示一个来源的一轮枚举结束。缺失初始、snapshot 祖先、异常
配置以及没有已知来源的旧 apply 都使 `source_history_unknown` 持久保留；后续
看似正常的页面不能清除它。即使没有已知缺口，也仍须把实际配置与独立核对的
初始 store/变更来源逐次对齐，才能形成历史区间；页里的 node ID 不自行成为 store
映射。旧磁盘与同节点替换磁盘拥有不同的来源记录。

现有 sweep 在 placement 盘点完成后轮转三类历史，每类各有调度游标，每次只访问
一个来源；拒绝或失联也推进调度游标。重启/换主可重新调度并沿各来源持久页继续。
使用既有维护名额、读取 worker 与原请求预算，没有新增对象 I/O；取消仍不提前
释放实际 worker。新增 timeline 成功、失败、回绕、完整轮次观察和缺口计数均是
累计诊断数，不是全局完成比例或可删除对象证明。

## 验证与剩余工作

[证据包](raw-ttl-timeline-census-validation-20261006/manifest.json)保存源码清单、补丁、
命令、原始日志与负向对照。新增九项 metadata 测试覆盖规范请求、独立原任务校验、
两次实际 WAL 重开、原回执、revision/endpoint 变化、不可改写前缀、joint/snapshot/
旧 context、不可清除缺口、损坏/孤儿行、同节点不同磁盘、V25 升级和大 joint 页。
13/113 组 placement catalog 的单页读取成本相同，保留查询实际只读五条索引行。

新增两项 runtime 测试使用真实三副本 Raft/WAL、认证 HTTP/2 duplex 与 InProcHub。
生产 attach/detach 产生六次配置变化，记录包含初始配置；首次页提交丢回执后换
metadata leader，来源拒绝时仍确认原提交。增加旧 context learner 后游标失效，
从首屏重查并保存全部八条配置；三份 metadata WAL 重开后分别读取完整两页。
后台测试同时验证三类盘点轮转、来源 Unknown、endpoint 变化和单维护并发上限。

负向对照仅移除持久 planner 的原任务核对，伪造不存在 task 的测试必须失败；
代码随后原样恢复。首次开发编译的三个错误位于新增测试辅助代码，修正后日志
仍保留。迁移目标使用已有离线 guard fixture，不是第四个运行副本。没有新增
真实 TCP/MinIO、完整 NodeRuntime、kill/断电、旧 binary、跨主机、Chaos 或形式化
验收。

最终相关回归去重 **542 项通过，1 项原有真实 MinIO 测试忽略**：完整 metadata
363 项、Raft 来源/时间线/成员/格式套件 42 项、四线程 TTL runtime 115 项及
Raft/server 文档 22 项。严格 workspace 全 target Clippy（含 write-stage-tracing/
checkpoint-testing）、格式及 diff 检查通过。最终验证期间源码摘要保持不变；
上述结果不代表完整 workspace 网络测试或生产资格。

下一步将持久时间线与原生初始映射、变更来源对齐，形成保留明确缺口的实际
配置/store 区间。所有旧/joint 配置、snapshot 祖先、不可达历史磁盘及安装/迁移/
备份/恢复引用、可转移恢复锚点仍须覆盖。Reader/Publisher/未知 PUT 生命周期、
DATA 关闭、安全释放、物理 GC、历史及资源稳态、merge、时钟与部署升级故障验收
继续按[完整第二阶段](RAW-TTL-PHASE2.md)和[生产范围](RAW-KV-PRODUCTION-SCOPE.md)推进。
