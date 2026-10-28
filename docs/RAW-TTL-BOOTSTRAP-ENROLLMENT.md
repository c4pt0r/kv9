# 缺少 bootstrap 凭据的 metadata store 在线登记

2026-10-05，接续 [bootstrap grant 退役](RAW-TTL-BOOTSTRAP-RETIREMENT.md)。
生产 `NodeRuntime::start_core` 已接入启动登记：原 store 已具有有效 V8 Active 格式、
本地已选中的 checkpoint 和精确 Published Version，但尚无可读 bootstrap 凭据时，
可向仍在服务的 metadata quorum 申请持久保护，再执行恢复。第二阶段仍未完成。

## 授权与启动顺序

整个路径持有原 `StoreGuard`。在 GET、创建本地 driver 或发布 serving 之前：

1. 原有只读 preflight 扫描实际 checkpoint selector、WAL 中的成功发布 batch 和
   committed Raft，取得精确 manifest 与 predecessor。目录必须属于同一 StoreGuard。
   preflight 本身不构成对象保护，也不替代恢复校验。
2. 若没有选中 checkpoint，不创建登记 client。若存在精确、可读的本地 V1/V2 凭据，
   检查 Pending 身份并同步文件/目录，直接使用已有授权；这一分支不要求在线 quorum。
   同一镜像再次发布时保留原授权窗口，不按新的 predecessor 重新生成身份。
3. 缺少可读凭据时，先确认本地 Active 格式 fence，再向远端查询精确 Version。只有
   完整 binding、全部对象 closure 和 Published 状态匹配，才允许继续。缺失来源、Held
   或 Released 来源都不能通过，也不会按本地 manifest 自动补建 Version/Pending。
4. 在 `KV9MBR02` 中先保存精确 prospective 槽位，标为不可读，fsync 文件、rename 并
   fsync 目录。随后通过现有 ledger Share/Publish 申请同一 store 的 BootstrapReader，
   再独立查询完整 Published 回执，最后持久保存可读状态。回执未知时不执行 GET。
5. 使用原 GET-only BootstrapReader 恢复，等待实际对象任务排空；继续执行完整 base、
   root、WAL、publication、配置和最终 engine 校验。只有整个正常启动完成后，才发布
   运行态服务。登记成功本身不是恢复完成的证明。

启动登记不删除、关闭或退役其他槽位。已有 V2 候选或待退役债务在总数不超过两个时
原样保留，待完整启动后交给正常 worker 重放。选中的镜像已经在待退役列表、槽位满、
不同镜像占用同一 predecessor/Pending、身份损坏或另一个 V1 槽位尚无精确历史时，
保留原文件并拒绝登记。它不能恢复已 Released 的 owner generation。

不确定 Share/Publish 的重试使用已持久的同一 Pending、node/store、manifest 和
predecessor；再次提交前重新同步整个状态。若之后完整恢复失败，已确认的授权继续
保护镜像，下一次启动可重试恢复。没有超时释放或“恢复失败就回收”的旁路。

## 启动专用路由

`StartupLedger` 复用原鉴权 retention RPC，使用 root 中除自身外的 seed 地址和
cluster credential。它没有本地 backend、engine、driver 或 listener 依赖，避免为
取得启动授权而先启动尚未恢复的 Raft。原 gRPC runtime 提前创建并继续用于正常服务。

一次登记共用 10 秒网络 deadline，每次连接最多 250 毫秒；只有连接失败（尚未提交
请求）或明确的 NotLeader 拒绝允许尝试下一 seed。超时、鉴权失败、格式错误和未知
写入结果立即返回，不在其他 seed 上自动重放。下一次启动按持久槽位显式重试。
既有 RPC 的 root/query digest、消息大小和服务端实际任务约束继续生效。

## 验证和未完成边界

最终 8 项检查全部通过：145 项不重复测试通过，1 项真实 MinIO 恢复测试忽略；
workspace 严格 Clippy、格式及差异检查通过。检查包括 bootstrap 状态、完整 TTL
runtime、region manager，以及 raft/metadata checkpoint 恢复回归。

新增五项本地状态/启动组合测试和一项真实三副本组合测试：

- 缺少 quorum、错误来源 phase、身份损坏、关闭中的镜像和冲突槽位不能产生读取授权。
- 不确定 Share/Publish 只留下同一不可读槽位；保留其他债务时仍满足两槽上限。
- 新鲜 WAL 不访问 ledger，目录不匹配拒绝；已有可读凭据不要求网络。
- 三个真实 metadata Raft driver 完成镜像发布与 Version handoff，测试显式模拟旧
  worker 直接选择镜像而未写凭据。随后停止目标节点的 metadata/data owner 线程及
  RPC 服务，并在同一 StoreGuard 下使用生产启动组合打开其实际文件。
- 首次 Share 已提交但回执丢失时，另一个已连接 seed 的 handler 没有收到重放，
  对象 GET 数没有增加。metadata 换主后，路由跨过旧 leader 的明确拒绝，仍使用原
  prospective 槽位；Publish 回执丢失仍没有新增 GET。确认后完整恢复通过，无新增 PUT。
- 停止所有原 Raft owner 和服务后，同一生产恢复组合重复离线重开两次，不创建 ledger
  client；原 Version 和新 grant 的 Published 保护保留。

此证据使用 MemoryObjectStore 和鉴权 HTTP/2 duplex，运行真实 Raft/apply 与文件
恢复；没有执行完整 NodeRuntime TCP 启停、MinIO、独立进程断电或混合版本滚动升级。
源码摘要、开发失败与最终检查见
[验证包](raw-ttl-bootstrap-enrollment-validation-20261005/manifest.json)。

本轮只闭合**具有精确 Published Version 且另有可达 serving quorum**的旧凭据缺失路径。
全组都没有凭据时的冷启动、单节点缺凭据、所有 seed 已迁址且无可达引导地址、早于
ledger 的镜像、尚无 Active 格式的旧 store，仍须独立设计与验收，不能伪造文件放行。
当前内部 service/store floor 仍是 V8，ledger 仍是 `KV9LED08`；没有新请求或文件格式，
也不构成旧二进制/网络混部资格。

V1 保存前崩溃形成的孤立 grant、全体 Reader/Publisher census、store 退役/替换、
终止 owner/resource 历史有界、Version/负向 Pending 生命周期、未知 PUT 隔离与物理 GC
仍开放。后续[终态确认](RAW-TTL-TERMINAL-READ-ACK.md)已使已 Released 回执不再要求
来源保持 Published；自身终态记录和交叉引用仍须保留，首次结束的来源依赖未放宽。
在线登记本轮没有释放任何 Version 或物理对象，也不声称全局存储稳态或生产部署验收完成。
