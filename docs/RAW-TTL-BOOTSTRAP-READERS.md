# Metadata 冷启动的持久读授权

2026-10-05；接续 [runtime Readers](RAW-TTL-RUNTIME-READERS.md)。本轮接通 metadata
checkpoint 的持久 bootstrap 保护，第二阶段与生产资格仍未完成。

## 选择与恢复

metadata 先恢复自身 engine，随后才能启动 Raft 和提供 ledger 服务，不能在冷启动时
等待自己的 Reader 登记。现有逐 store 选择观察仅覆盖 data group，也不能替代授权。

metadata checkpoint worker 现在先收尾已提交的 checkpoint/journal，完成精确
Pending→Version 交接，再处理本机选择。这个顺序避免 worker 等待由自己负责发布的
Version。对于即将选中的完整镜像，worker 核对 Published Version，通过已有 Share/
Publish 协议增加 `BootstrapReader` owner，再独立查询精确 Published 回执。
其完整资源集合与镜像相同，身份绑定 root、node、store incarnation、manifest 和
来源 publication window。它不是 process incarnation，也不是可转移恢复锚点。

只有确认后才原子保存 `catalog.bootstrap-reads`，然后调用 `checkpoint_applied`。
文件格式 `KV9MBR01`，包含 store 身份、至多两个 `(predecessor, manifest digest)`
槽位和校验和，总长不超过 **177 字节**。旧选择与新候选同时保留，覆盖选择前后崩溃；
下一次尝试保留实际旧选择、替换未选中的候选。临时文件不构成凭据；文件 fsync、rename、
目录 fsync 全部完成后才能切换。重试也重新同步，不能将失败的目录 fsync 当作成功。
同一完整镜像再次发布时可沿用原 grant，完整 WAL/Raft 检查仍独立验证实际获胜窗口。

`NodeRuntime` 在持有 StoreGuard 时使用专用 `BootstrapReader`。先完成已有无 GET 的
publication preflight，再核对本地凭据的精确 store 与镜像，通过 GET-only scope
恢复。实际对象任务全部退出后，才继续镜像语义、WAL 尾部和最终 publication 校验。
启动路径不查询在线 ledger，不发 PUT，也不把本地凭据转换成 portable anchor。
GET 超时、恢复失败或进程重启均不释放 grant；同步 scope 的真实排空先于 store 锁释放。

## 保护、兼容性与未完成部分

新 owner kind 为 **10**。它只能用于 metadata、generation 1、非 anchor、非 process
绑定。通用 QuiesceAfterTransfer/Release 和现有 Reader/publisher finish 都不能退役它。
因此确认过的持久读授权不会被一个普通 owner 转移请求撤销。Share 与 Publish 之间
中断只留下保守保护，缺少确认回执时不保存凭据、不前进本机选择。

这依赖本地持久文件与 Raft/StoreGuard 既有可信存储假设；checksum 不是签名，复制或
手写文件不能证明新 store 的授权。此文件仅是本 store 已确认授权的恢复凭据。

**已有远端 metadata checkpoint 缺少凭据时，本次 NodeRuntime 明确拒绝启动，且不
发 GET。** 不自动从 Published 来源、历史选择或磁盘 manifest 推断授权；旧 store 的
经过验证的 enrollment/升级流程仍待补齐。低层 recovery API 保留原 uploader 接口供
组件调用，生产 NodeRuntime 使用新接口。不能据此宣称既有集群无缝升级。

Store/service floor 仍为 V8，ledger header 仍为 `KV9LED08`；新增本地格式和 owner
kind，没有新增 Raft 命令或 ledger request tag。旧 owner decoder 会拒绝 kind 10，
相同 floor 不代表混合二进制或滚动升级已获资格。FrozenV7 fixtures 和旧证据未改。

本地凭据槽位有界，但全局 grant 数量目前随 store/镜像增长。旧 grant 仍持有完整
Published pins；丢弃本地旧凭据不释放它。尚需持久关闭旧读准入、实际排空与 store
退役/替换证明，以及可恢复的释放协议。本轮不证明全部 Reader census，不释放旧
Version/负向 Pending/未知 PUT，不开启物理 GC，不宣称容量稳态或完整阶段验收。

## 验证

- metadata 专项验证完整资源保护、身份/generation/范围约束，以及所有通用退役和
  finish 路径拒绝；公开 request codec 可往返新 kind。
- 本地凭据专项验证未确认/丢失 Publish 回执、连续 20 次候选更新仍至多两槽、未完成
  选择的替换、截断/错误 store/错误镜像/超长文件在 GET 前拒绝。
- 实际 MinIO 工作队列搭配暂停的 memory backend：GET 已超时但任务未退出时，
  启动线程仍持有 store 锁；解除暂停后才排空。不是实际 S3/MinIO 持久性验收。
- 三个真实 metadata Raft 副本、鉴权 HTTP/2 duplex，自动 worker 确认授权后选择；
  全部 driver/服务停止后，三个 store 分别进行两次完整 engine 重开，没有在线 ledger，
  无新增 PUT，grant 仍 Published。移除凭据的同一真实恢复路径在 GET 前失败。
- 丢失真实已提交 grant 回执时，metadata worker 不前进 checkpoint；恢复确认后继续。

相关全量回归、严格 workspace Clippy、格式与 diff 检查记录在
[独立验证包](raw-ttl-bootstrap-reader-validation-20261005/manifest.json)：metadata 217、
Raft checkpoint recovery 29、TTL runtime 78、bootstrap reader 3、region manager 21 项
通过，合计 **348 项通过、2 项忽略**，包括 7 个新增测试；8 项检查均通过。开发阶段曾有
测试 helper 名称错误及将上传校验 GET 错计为 bootstrap GET 的断言；修正与原日志均
保留。本轮没有真实 TCP、整进程断电、跨主机对象存储或完整形式化证明的新验收。

## 后续进展

[持久关闭与退役](RAW-TTL-BOOTSTRAP-RETIREMENT.md) 已接通新的 prospective 登记、V2
双槽状态和独立 store fence 核验，可回收正常在线 store 的旧 bootstrap grant。
本文的 V1 证据保持原样；缺凭据升级、旧孤立 grant、全局历史有界和生产资格仍开放。
