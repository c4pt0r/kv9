# 实际配置到历史磁盘的区间映射

2026-10-06 开始，2026-10-07 完成验证；接续
[实际配置时间线持久盘点](RAW-TTL-TIMELINE-CENSUS.md)。
本组件把已经保存的实际配置，与原始建组磁盘和经过独立核对的配置变更来源对齐。
它推进历史来源覆盖，不代表全体历史磁盘已盘点，也不授予 DATA 关闭、旧 Version、
负向 Pending 释放或对象 GC 权限。[第二阶段完整范围](RAW-TTL-PHASE2.md)保持开放。

## 映射依据

每个来源继续绑定 root、region、creation、node 和精确 store incarnation。
首条原生 initial 配置只有在完整 voters 与原建组一致时，才能使用已核验的原始
node/store 映射。推导时再次核对不可变建组 task；当前节点注册表不能代替历史磁盘。

对于带来源的单个 Auto 配置变更，先独立核对原始 task，再使用 raft-rs 的配置变更
语义，计算上一条实际配置应用该动作后的成员集合。只有结果与本条实际配置完全
一致、位置顺序合法时，才继承仍在配置内的已知磁盘；Attach/Promote 的目标磁盘
来自该条已验证的来源。Detach/Remove 不会把节点的新注册磁盘写回旧区间。
同一节点被重新绑定为不同磁盘，或不同节点出现相同已知磁盘，都会留下冲突缺口。

保存 incoming voters、outgoing voters、learners、learners_next 的完整角色并集。
已知来源当前只描述单个 Auto 变更；joint、旧 context、非法顺序、无法表达的配置
和实际变更结果不匹配都明确保留未知。snapshot 只证明该边界上的实际配置，不能
补造祖先配置或磁盘来源；snapshot 前一个区间的结束边界也标记祖先缺口。
后续可核验变更能说明其目标磁盘，不会清除已经观察到的源历史不完整标记。

## 持久进度与查询

metadata ledger 新增 `AdvanceConfigurationIntervals`（tag 41）。请求只含精确来源、
预期进度序号、输入 timeline 序号及原生映射的记录位置；调用者不能提交推导结果。
一次最多推导四条实际配置，结果与进度在 metadata Raft 的同一次更新中保存。
已保存的原请求回执可以重试，不依赖来源节点此时在线。

实际时间线追加时，从已处理位置继续，不重写之前的映射。若先前缺少原生映射、
后来补齐，则启动新一代推导，从首条开始有界重建；查询只返回本代已经处理的前缀。
每次进度变化都会使旧查询游标失效，不能拼接不同代或不同修订的结果。

查询最多返回四个区间，包含开始/结束的原始配置边界、已知 term/index、逐成员
磁盘和缺口位。最后一条的开放结束不代表未来配置不再变化。
`round_complete` 仅表示当次已保留来源前缀处理完；`inputs_current` 仅比较最新
持久输入修订，均不是在线新鲜度、全体历史完整性或可回收证明。

读取校验原始记录摘要、实际角色、代际、尾部及多余行；缺行或前缀改变拒绝继续。
成员数量在分配前受限。即使重新生成合法编码和校验和，重复磁盘、与实际角色不符，
以及给 snapshot/旧 context 填入已知磁盘或移除必要缺口，仍会拒绝。

## 格式与后台调度

ledger header 从 `KV9LED26` 升到 `KV9LED27`；namespace 59 保存 `KV9DIP01` 进度，
namespace 60 按来源和 ordinal 保存 `KV9DIE01` 结果。V26 初始化升级保留原有
时间线、来源和原生映射；旧 header 下出现新区间行会拒绝升级。V11 store/service
floor 不变，没有新增 Raft log kind 或 RPC。未增加滚动升级或旧 binary 混部资格。

现有 checkpoint sweep 在成员、来源、实际时间线、区间四类工作之间轮转，各有
独立游标，共用原有维护名额与期限。每次只处理一个来源的一页；未知或失败来源
不会固定占住后续来源。区间推导只消费持久 metadata，不发起远端查询或对象 I/O。
状态新增页面、失败、轮转及观察到的完成/缺口计数；这些累计计数不证明全局完成。

## 验证与剩余工作

验证覆盖实际四类动作、当前注册变化、同节点换盘、合法来源与实际结果不匹配、
原始 task 消失、joint/旧记录/snapshot 边界、丢回执、追加续做、新代重建、旧格式
升级、持久记录损坏及大成员集合。13/113 组目录的页面访问量保持相同。
三副本组合使用真实 raft-rs、磁盘 Raft/WAL、InProcHub 和鉴权 HTTP/2 duplex，
覆盖 metadata 换主、来源 Unknown/endpoint 变化、后台轮转和三个 metadata WAL 重开。

命令、开发失败、负向控制和最终结果记录在
[本轮证据包](raw-ttl-configuration-intervals-validation-20261006/manifest.json)。
负向控制仅移除实际配置结果比较，要求不匹配场景的独立断言失败，再恢复原代码。

最终相关回归去重 **555 项通过，1 项原有真实 MinIO 测试忽略**：完整 metadata
374 项、Raft 来源/时间线/成员/格式套件 42 项、四线程 TTL runtime 117 项及
Raft/server 文档 22 项；其中新增 11 项 metadata 和 2 项 runtime 测试。
严格 workspace 全 target Clippy（含 write-stage-tracing/checkpoint-testing）、
格式及 diff 检查通过。最终验证期间源码摘要未变化。
开发阶段误用 raft-rs 私有 API 的编译错误，以及三处严格 Clippy 错误已修正，
失败日志保留。TTL runtime 本轮约 279 秒，所选用例全部通过；这不解释此前其他
证据包中尚未确定唯一原因的 runtime-dev4 失败。

四条页面和受限成员集合限制单次数据处理量，不等于完整 CPU/RSS/磁盘预算。
raft-rs 配置重建随成员数增加，历史行及常驻索引仍持续增长；尚无长期稳态验收。
本组件不包含真实 TCP/MinIO、完整 NodeRuntime、进程 kill、断电、旧 binary、
跨主机、Chaos 或新增形式化组合验收。

下一步仍需覆盖旧/joint 配置、实际历史磁盘、snapshot 祖先和所有安装、迁移、
备份、恢复来源与可移交的恢复锚点，然后完成 DATA 关闭、Reader/Publisher/未知 PUT
生命周期、安全释放与物理 GC。历史和整机资源有界、merge、时钟部署、升级、
真实故障及持续负载验收仍属于原始目标，不能由局部区间无缺口替代。
