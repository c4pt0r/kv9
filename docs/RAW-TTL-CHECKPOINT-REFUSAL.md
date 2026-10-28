# 精确负向 checkpoint 结算的持久记录

2026-10-05；接续[精确历史查询](RAW-TTL-CHECKPOINT-SWEEP.md)与
[已登记执行者闭合核对](RAW-TTL-REGISTERED-CLOSURE.md)。完整第二阶段仍在进行。

后台此前只在发现另一 identity 赢得同一 CAS 窗口后关闭 admission。Closed 本身不
包含 winner，后续访问或重启仍要查询历史。本次将已验证的负向事实与 gate 关闭
放入同一个 metadata Raft batch；后续扫描直接读取事实，历史来源退役后也可继续。

## 验证与提交

`RecordCheckpointRefusal` 请求携带 Pending token、来源 node 和预期 winner。
这些字段是待核验的声明。服务中的 metadata leader 先执行 ReadIndex，加载规范
intent 和现有保护；若没有持久事实，则定位 active source 的 store incarnation
与 endpoint generation，独立读取该 group 的 `predecessor + 1` 已提交历史。

本地来源须匹配运行时持久 store identity，并通过实际 driver/history 查询。
远端来源使用已有鉴权 History RPC，绑定 root/cluster/node/store/group/window
和新 challenge，单次查询最多一秒，共享调用总 deadline。自身候选胜出、Unknown、
不支持查询、来源缺失或回复不匹配都不能生成负向记录。

历史 RPC 在 catalog 锁外完成。取得 catalog 锁并以 Noop 排空前序不确定提案后，
重新核对完整 source endpoint、root、intent 和请求，再在同一 leader term 中
提交 gate 关闭与事实。验证期间发生的 endpoint generation 或 incarnation 变化
会拒绝本次提交；不会拿早先的回复补证当前目录。

普通 `plan_retention` 和持锁 admin Apply 路径拒绝创建该记录。runtime 私有
witness 只能来自上述验证；`plan_verified` 是要求调用方满足该契约的内部可信
planner API，不是接收网络声明的入口。Raft receipt 才表示持久完成。

## 恢复、幂等与保留边界

记录保存规范 intent 摘要、精确 winner、来源 node/store/endpoint generation、
gate 关闭 revision 与记录 revision。读取校验 root、Pending、intent、保护、
Closed gate、revision、编码和 checksum。已有行损坏或缺失依赖时报错，不能变成
“没有尝试”。缺少事实仍是 Unknown，不能从 Closed、EOF 或零执行者推断负向结果。

相同 Pending/source/winner 的重试确认原始事实，保留第一次记录的来源与 revision；
不要求该来源仍 active 或保留 group handle。重试在 catalog 锁内重新检查原记录并
取得新的精确 Raft 回执。不同 winner/source 或 Pending generation 拒绝。

后台持久化事实后才继续下一轮 Held sweep/closure。初次关闭会原子更新原有 change
stamp；向已经 Closed 的尝试补充历史事实不改变执行者集合，不让代际扫描重启。
`negative_records` 统计本进程收到成功回执的记录操作；丢回执后重启可直接发现行，
因此此计数不代表所有持久负向尝试的总数。

记录不释放 Pending、旧 Version 或 Reader，也不认证非零/未知 PUT 已终止。
负向结算与已登记零 PUT 闭合现在均可恢复，但全局释放仍需完整 publisher/部署
资格、恢复依赖和相应释放协议。对象 GC 尚未启用。

## 格式与兼容边界

| 内容 | 编码与边界 |
| --- | --- |
| 持久事实 | namespace tag 20 + Pending ID；`KV9CRF01`，200 字节 |
| 请求 | ledger request tag 17，137 字节；沿用现有鉴权 Apply RPC |
| Ledger header | 仍为 `KV9LED07`；新事实只能写入 07，旧布局先由既有 census 升级 |
| Service/store floor | 仍为 V7；不新增 RPC 类型或释放权限 |

这是一条独立、不可变的追加事实。旧 07 planner 不认识该行或请求，也不取得新的
释放权限；读取仍需校验现有保护。pre-07 header 下出现该行会被升级 guard 拒绝。
这些兼容约束不构成滚动升级、旧二进制混部或完整 publisher fence 的验收。

## 验证范围

[独立证据包](raw-ttl-refusal-validation-20261005/manifest.json)保存日志和源码摘要：

- 六项 metadata 专项覆盖严格编码、普通请求拒绝、原子关闭、精确重试、正向 winner
  拒绝、来源/代际错配、已关闭尝试补录、损坏/孤立记录/旧 header，以及真实 WAL
  五个尾部截断位置的原子恢复。Pending 全程保留。
- 实际三副本 Raft 与鉴权 HTTP/2 duplex 验证伪造 winner、未知窗口、查询不可用、
  admin 入口以及历史查询过程中目录变化的拒绝；目录变化后重新验证才能记录。
- 实际提交后丢回执，切换 metadata leader，移除全部运行时 group lookup 并让
  来源目录行 inactive；精确重试不查询历史，后台继续完成 closure。WAL 重开保留
  事实、Closed gate 与 Published Pending。来源退役使用测试内 Raft 目录更新，
  不代表完整节点退役流程已验收。

本轮 metadata 全套 **197 项通过、1 项既有 MinIO 用例忽略**，完整 TTL integration
**56 项通过**，合计 253 项不同测试；六项专项与十五项 sweep 回归包含于上述总数。
严格 workspace/all-targets clippy、格式和差异空白检查通过。

这是本地组合验证，不是完整 NodeRuntime 多进程、TCP/MinIO、物理断电或形式化
组合证明。物理 GC、完整 owner 生命周期与生产稳态验收继续保留。
