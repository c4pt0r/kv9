# 实际 Raft 成员变更的精确 store 来源

日期：2026-10-06。接续[成员持久盘点](RAW-TTL-MEMBERSHIP-CENSUS.md)。
迁移任务原本已保存精确 store，但实际配置变更只有 node ID。仅保留最终 ConfState
不能把一次加入、提升或移除关联到原磁盘，更不能用今日 endpoint 替代历史身份。
本增量把原始元数据依据带入实际 Raft 变更，并在日志压缩后保留。

## 生产路径

Runtime 的 attach learner、promote voter、remove source、detach aborted learner
分别从实际已 apply 的 103/104/106/108 task 读取来源。固定深度验证原 creation、
迁移及证据链、region、operation、node 和 store；摘要覆盖 canonical task 整行。
这个过程不查询当前 endpoint 来推测旧磁盘，不依赖 placement 索引已完成扫描。

`ConfChangeV2.context` 携带固定 169 字节 `KV9CFO01`：root、creation、region、
operation、task/kind、node/store、来源行摘要和动作。提案及有序 apply 都核对真实
group；已识别的 context 必须对应同一个单步节点和动作，拒绝未知字段和 joint
transition。普通 metadata 配置及已有无来源的 Raft 接口保持原语义。

DiskRaftStorage 在原 writer lock 内从实际已持久、已提交的配置 entry 提取来源。
新 record kind **9** 在一次 checksum/write/sync 中保存 term/index、来源与完整
post-change ConfState；同步成功后才发布配置、replay guard 和观察。只有已提交、
尚未 apply 的 proposal 不会产生来源回执。失败后的 writer 不能提供观察。

日志重写保留 kind 9，并在写临时文件前检查所有已有来源均被保留。协议 snapshot
安装保留本地已知变更来源；导入 prefix 的 ancestry 仍明确未知。精确位置查询遇到
不同 term 拒绝，旧记录或导入位置没有来源时返回 Unknown，而不是“没有历史 store”。
相同 node 先后使用不同 store 的记录不会互相覆盖。

## 升级与权限边界

新 record 是 **单 store 的前向日志格式变更**：旧实现遇到 kind 9 会拒绝重开。
现有 V11 store/service floor、KV9LED23 和 RPC 格式未变；旧在线副本可能正常 apply
配置但只写旧记录，不能据此认定整个配置均已支持来源持久化。没有运行旧二进制升级
资格测试。本轮没有把降级可运行性或全副本来源完整性作为保证。

来源是描述，必须与 certified catalog 的原 task 独立核对；它不授权成员变更、
读准入关闭、pin 释放或 GC。历史无 context 的配置不补造来源；初始成员的 creation
依据、完整配置区间、snapshot ancestry、跨节点分页和持久汇总仍需组合成完整盘点。
已有 membership census/RPC 继续报告原来的缺口，不会因单条新来源而宣称全部覆盖。

新来源随既有配置历史保留，尚无历史数量或常驻内存硬上限；文件写入和重写复用
原 WAL/Raft 字节计费。完整恢复来源、可转移恢复锚点、Reader/Publisher/unknown PUT
生命周期、安全 Version/Pending 释放、物理 GC、merge 和生产资格继续开放。

## 验证

最终相关回归 **648 个不同测试通过**，其中新增 8 项；metadata 原有 1 项真实
MinIO 测试跳过。四线程 TTL runtime 107 项全部通过，严格 workspace Clippy
（all-targets，write-stage-tracing/checkpoint-testing）、格式及 diff 检查通过。

独立命令、失败记录、负向对照与源码摘要保存在
[本次证据目录](raw-ttl-configuration-origin-validation-20261006/)。

新增测试覆盖 canonical context、异组拒绝、四种 task 来源及 replacement store、
未 apply/未提交记录、真实压缩与重写、snapshot 未知前缀、写入和同步各切点的
LoseUnsynced/KeepUnsynced/Seeded 恢复，以及真实三副本 attach/detach 和三盘重开。
配置重写负向对照把 kind 9 降为旧 kind 4，测试在重开后准确发现来源消失；源码已恢复。
三副本使用实际 raft-rs、磁盘 WAL 和进程内传输，不能替代 TCP、MinIO、完整
NodeRuntime、进程 kill、跨主机或 Chaos 验收。
