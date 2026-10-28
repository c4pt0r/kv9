# Reader 终态回执与旧 Version 解耦

2026-10-05，接续 [bootstrap 在线登记](RAW-TTL-BOOTSTRAP-ENROLLMENT.md)。此前 Reader
已经在 metadata 中 Released、本地尚未收到成功回执时，重试仍要求旧来源保持
Published；bootstrap 还要求旧 intent 可读。这会把未来的 Version 回收与本地 outbox
清理互相卡住。本轮将确认已完成结果与首次释放分开，没有放行 Version 或对象回收。

## 已完成结果的确认

`FinishCheckpointRead` 和 `FinishImageRead` 先从请求中的 source ID、固定协议 domain
和完整 node/store/process lifetime 重新派生 Reader ID，要求 source generation 1。
只有请求的完整 binding、generation、不可变注册以及全部 resource cross-links 都与
当前 Released Reader 一致，才返回已有结果。checkpoint 与 image 协议的 ID domain
保持分离，不能拿另一类请求确认同一个 Reader。

该分支不读取旧 Pending/Version/Snapshot/Migration owner，不重新登记 Reader，
在当前 08 格式下也不改变 ledger revision。错误来源、不同 lifetime/subject/resource closure、
旧代报告遇到新代，以及损坏的 Reader 自身注册/交叉引用都不能获得确认。缺少或
损坏的来源元数据不再妨碍一个独立存在的 Reader 终态回执；这不会修复来源数据。

`RetireBootstrapRead` 则从 root、精确 Pending generation 1、node/store 重新派生原有
bootstrap owner ID 和 operation，核对 owner kind、generation、完整不可变注册、
resource cross-links 与 Released 状态。该 kind 的通用 release 和 generation 2
仍禁止，所以这个精确终态来自既有受核验关闭路径。确认不再读取旧 intent/Version，
也不要求原节点在线；metadata runtime 在 ReadIndex 后观察，并在原 catalog 锁内
再次核对终态与原请求。首次关闭仍走独立 store fence RPC、endpoint 重检和同 term
提案。两种证明在 runtime 中以不同 enum 分支表示，不能把“未找到终态”当作已释放。

## 首次释放约束继续生效

尚不存在、Held、Published 或已经进入更新 generation 的 Reader，不能使用终态
确认分支。原有首次结束仍验证精确 Published 来源；Pending 正向交接仍只接受同一
镜像的 Published Version。实际 GET-only scope 排空和持久 outbox 顺序未变。
bootstrap 的首次退役仍要求完整 Published Version、规范 intent、持久读准入关闭
和独立 store 观察，普通调用者的声明仍被拒绝。

后续 Version 生命周期仍必须等首次结束的依赖满足。释放成功但回执未送达的情况，
现在只需保留已 Released owner、注册与 resource links，不必继续把旧 Version 保持
Published。如何安全压缩这些终态历史、判断所有旧进程/本地 outbox 已不再依赖它们，
仍是独立的未完成要求。

## 验证与证据边界

最终 8 项检查全部通过：352 项不重复测试通过，2 项真实 MinIO 测试忽略。
检查覆盖完整 metadata、TTL runtime、bootstrap Reader、持久 outbox 与 checkpoint
恢复回归；workspace 严格 Clippy、格式和差异检查通过。

新增四项 metadata 测试和一项真实三副本组合测试：

- Pending/Version 经完整同镜像覆盖转移后，旧 Reader 已 Released 的请求仍可无变更
  确认；进一步模拟来源记录不可用，仍只接受完整匹配的终态。
- bootstrap 的 intent/Version 不可用时，原 Released owner 可确认；不同 Pending、
  store/node、generation、kind 或 operation 拒绝。bootstrap 首次退役仍拒绝。
- 尚未结束的 Reader、旧请求遇到新 generation、跨协议请求及 Reader 自身损坏拒绝。
- image Reader 测试显式移除来源行，验证仅终态确认不依赖该行。这是元数据缺失夹具，
  **不是**合法 Snapshot/Migration 退役流程。开发中尝试通用转移这两类 owner 被原有
  安装证据门槛拒绝，失败日志保留；该门槛未修改或绕过。
- 三副本实际发布/选择 metadata checkpoint，执行真实受保护 GET 与排空；Reader
  finish 和旧 bootstrap retirement 都真实提交但丢失回执。随后经实际 Raft 把旧
  Version 转移给完整同镜像 Snapshot，再切换 metadata leader。
- 重新打开持久 Reader outbox，并重放本地 bootstrap 关闭记录；原 source inspector
  被拒绝时仍可确认已有终态，无额外 GET、无重新授权。全体 driver/服务停止后，实际
  WAL/checkpoint 重开仍保留 Released Version、Released Reader 与 Published Snapshot，
  两种旧请求仍是无变更确认。

上述 Version 转移使用相同对象集合的完整 Snapshot 保护，**没有**证明不同的新镜像
能替代旧 Version，没有使任何对象可删除。来源行的移除仅用于组件测试，不是新增的
生产历史 GC。组合测试使用进程内 Raft、鉴权 HTTP/2 duplex、MemoryObjectStore 和
真实文件恢复，不替代完整 NodeRuntime/TCP/MinIO、进程断电或部署验收。

源码摘要、完整命令、开发失败与最终结果见
[验证包](raw-ttl-terminal-read-validation-20261005/manifest.json)。store/service floor
仍为 V8，ledger 仍为 `KV9LED08`；owner ID、请求编码和磁盘格式均未变化。
全体 Reader/Publisher census、旧 Version/负向 Pending 释放、未知 PUT 隔离、全组
缺凭据冷启动、全局历史/空间稳态与物理 GC 仍开放，第二阶段和生产资格尚未完成。
