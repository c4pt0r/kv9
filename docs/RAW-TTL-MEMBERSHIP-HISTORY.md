# 本地 Raft 历史节点盘点

日期：2026-10-06。接续 [placement 任务盘点](RAW-TTL-DATA-STORE-HISTORY.md) 与
[交接竞争回归](RAW-TTL-HANDOFF-CONTENTION.md)。此组件补充实际持久配置的本地观察；
不把 node ID 等同于 store incarnation，不授予全局关闭、owner 释放或对象删除权限。

## 数据来源与缺口

`DiskRaftStorage` 从实际写入及重放的初始、indexed configuration、snapshot 与
compaction base 记录构建节点角色并集。节点退出配置后，已经观察到的 voter、
outgoing voter、learner、learners_next 角色保留。普通恢复配置查询仍使用原来的
`ConfigurationHistory`，不以这个集合替代精确 cut 的配置或安装授权。

安装 protocol snapshot 仍按原协议重置恢复用配置历史，但新的盘点保留本地已见
节点，并记录最新 snapshot cut 和不可清除的前缀未知标志。源端 snapshot 没有携带
完整祖先配置，不能凭新镜像或当前成员补造这些历史。重开时从原持久记录重建同一
观察。缺初始记录、重复/冲突顺序、未知 protobuf 字段、不合法成员集合和容量越界
都有明确的缺口状态；后来的健康配置或 snapshot 不能清除已有的不确定性。

物理日志重写在原有测量步骤中，用准备保留的 base/初始/indexed 记录重建一份
同样有界的观察，并核对摘要。若较早 base 独有的节点或不确定性会丢失，在创建
临时文件前拒绝重写，原文件和历史保留。正常原始/压缩日志的重写及重开保持相同
观察；这不是以最后一份健康 base 覆盖早期来源。重建的临时集合也受 4096 项限制。

此处“未观察到缺口”只描述本地配置记录，**不证明已覆盖尚未 apply 的 committed
配置、每个曾运行的 store、其他副本的历史或任何外部恢复来源**。返回的最后配置
位置不是全日志 apply cut。后续协议必须另外绑定 group/store、cut、来源与完整性。

## 容量和分页

每个本地 Raft store 最多保留 4096 个 node ID 及四位角色并集。超出后保留确定性的
最小 ID 子集并永久标记不完整；不会靠拒绝配置 apply 中断共识，也不把子集当作全体。
集合顺序不影响重开/重写结果。未知或超出单配置成员预算的旧记录不新增完整 protobuf
临时副本；原有日志解码与配置历史的内存成本仍存在。

每次分页复制最多四个成员，并额外查看一个条目决定是否有续页。opaque cursor
绑定观察摘要及上一个 node ID；角色、位置、snapshot 或缺口变化使旧 cursor 失效。
摘要在配置观察变化时从有界集合计算，分页不扫描完整历史。存储失败后拒绝查询；
peer 入口还检查 fatal 状态，查询与持久化共用原 writer/peer 锁序。

这是每组盘点集合和单页的边界，不是全节点 RSS、CPU、配置历史总量或恢复时间的
资格。当前 API 为本地组件，尚未接入跨节点 RPC、metadata 历史来源核对或 DATA
全 store 关闭门槛。没有新增日志 record、磁盘 floor、内部 wire 或 ledger 格式。

## 验证

当前已通过七项存储定向测试和一项实际三副本测试：joint/已移除成员、重复 snapshot、
冷重开、变化游标、未知/缺失/冲突来源、4096 项容量边界、sync 失败后的拒绝及实际
磁盘压缩重写。三副本测试通过真正 add/remove learner 配置提交和各自 Raft 文件
重开核对历史节点。它没有创建第四个实际 store，不能证明第四份磁盘已完成退役。

另一个反例在两份低层 compaction base 之间留下只有旧 base 提及的节点。修正前
重写没有拒绝，新断言失败；修正后重写在文件变化前拒绝，磁盘重开仍得到原观察。
配置位置的断言使用真实 conf_applied，而非只跟踪数据命令的 engine applied_index。

反例控制在 snapshot 安装时故意清空节点集合，新恢复测试按预期失败；原实现已恢复。
加入重写保护后重新运行 storage 66 项、rawnode 54 项、driver 52 项、snapshot/install
24 项及四线程 TTL runtime 100 项，共 **296 个不同测试通过**，7 项既有安装测试
忽略。严格 workspace Clippy（含 write-stage-tracing、checkpoint-testing）、格式与
diff 检查通过。先前一套通过结果保留，不重复计数。初次测试代码使用错误的文件系统
模拟 API，以及以数据 apply 等待配置变更的失败，也保留在
[本轮验证包](raw-ttl-membership-history-validation-20261006/manifest.json)。

完整第二阶段继续开放：源端祖先历史转移、node→store incarnation 的可信绑定、
实际历史/joint/replacement 成员与 Reader/install/backup/recovery 来源、DATA 全
store gate、可转移恢复锚点、未知 PUT、安全释放、物理 GC、全局资源/历史有界、
merge 与完整部署、时钟和形式化联合验收。
