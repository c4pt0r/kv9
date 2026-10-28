# Capture catalog 的 V8 升级隔离

2026-10-05；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
接续 [复制 capture catalog](RAW-TTL-CAPTURE-CATALOG.md)。此前旧 V7 writer/service
仍可能绕过新计划登记和 owner 绑定规则，本轮将这些约束纳入持久格式和内部服务版本。
第二阶段、完整迁移恢复和生产部署验收仍未完成。

后续：[payload 副本接管](RAW-TTL-CAPTURE-TRANSFER.md) 已增加两个 V8 RPC，
从原 voter 保存的完整文件继续未完成上传；未改变本轮持久编码或升级要求。
下文测试数量与当时未完成项保留为本包的历史记录。

## 行为变化

- lifecycle 当前 header 为 `KV9LIFE8`，确认 RPC 返回格式版本 8。仍是原 97 B
  checksum 记录；V1–V7 可读取并单向升级，node、store incarnation、root 和 phase
  保持不变。V7 reader 不认识 V8，不能重新打开已升级 store。
- 已有 lifecycle 的 NodeRuntime 启动继续先发布格式，再打开 Raft/WAL；只有持久
  当前格式、Active、健康的实际 StoreGuard 能产生格式确认。缺 lifecycle 的既有
  certified-recovery 路径仍须核对恢复出的根证书，再持久采用当前格式后启动服务。
- 内部服务为 `kv9.raft.v8.Kv9Raft`。所有 metadata/data Raft、discovery、registration、
  endpoint、TTL 目录/格式确认、retention、history/selection/pressure 和 capture
  RPC 共用该版本路径，没有 V7 或更早版本回退。V7/V8 在不同磁盘上运行也不能互相
  注册、复制或确认格式；持有相同 cluster token 不会绕过服务版本。
- offline installer 对 V7 guard 也在创建 group 文件前拒绝。CLI 继续先验证身份、
  持久发布 floor，再进入安装；升级发布失败会使 guard 拒绝授权，必须关闭重开。
- `KV9CAP01`、`KV9XFR01`、`KV9MCP01`、`KV9LED07`、manifest 与 Raft command 编码
  不变。不会因升级重新规划 capture、重写镜像、释放 pins 或自动开启 GC。

这是停机升级边界，不支持混合版本滚动升级。升级部署仍要求停止所有旧 owner、
保留原数据和身份、用新版本逐 store 启动；一旦发布 V8，不能直接回退旧 binary。
不能通过改写 lifecycle header 来实现降级。代码中的旧 header 写入仅用于完全停止
的隔离测试 fixture，不是运维恢复步骤。

## 验证覆盖

冻结的 [V7 codec](../crates/common/src/store_lifecycle/pre_capture_codec.rs) 来自上一
证据包的实际 source excerpt；[V7 proto](../crates/raft/src/grpc/ttl_format/pre_capture_wire.proto)
是上一包的完整原文件，SHA-256 对应关系保存于新包。旧 proto 仅在测试模块中
include，没有旧生产服务注册。为 HTTP/2 duplex 测试增加两个已经锁定的 dev 依赖
`hyper-util`/`tower`，未引入新的解析包版本。

- Prepared/Bound/Active 的 V7 升级保持身份；冻结的旧 codec 升级前可读、升级后拒绝。
  重复升级不重写，真实关闭重开后仍是 V8。
- V7→V8 发布的 create/write/fsync/rename/各级目录 fsync 前后注入错误。每个失败
  都使当前 guard 无法授权，重开后才可稳定原状态并重试；不是断电测试。
- 当前生成 dispatcher 对 V1–V7 的全部 12 类内部 RPC 路径返回 UNIMPLEMENTED，
  即使带有效鉴权。V8 自身路径进入现有鉴权检查。
- 冻结 V7 与当前 V8 的真实生成客户端/dispatcher，经 HTTP/2 duplex 双向交叉调用。
  各自正常格式确认成功；跨版本格式、registration、retention、metadata/data Raft
  拒绝，旧后台调用计数不增加，当前 Raft inbox 无旧消息。生产格式和 retention
  客户端也不会回退。V7 后台是计数 fixture，不能据此声称已运行完整旧服务 binary。
- 扩展实际三副本 capture 故障切换用例：全部 owner/服务关闭后构造 V7 lifecycle，
  在重新打开 source payload 和新 metadata leader WAL 前执行真实 StoreGuard 升级。
  原 `capture.plan` 文件 hash、descriptor、catalog record、两个 Published pins
  均保持一致；原对象仍能校验并返回相同协议 record。
- 原 offline installer 测试覆盖 V3/V7 未升级时不创建 group 文件；全组 TTL 激活、
  capture/install 与 900 个 TTL value 的大镜像回归使用新的 V8 内部服务。

验证脚本的三处 lifecycle header 断言已更新为 V8；`check-wire-floor` 默认以 V7 为
旧对照，也保留更早版本选择。脚本只做语法检查和示例编译，未伪装成实际跨进程演练。
原始日志、source/fixture hashes 与历史证据校验见
[本轮证据包](raw-ttl-capture-upgrade-validation-20261005/manifest.json)。

本轮 common 73、格式 RPC 5、snapshot/install 23、Raft TTL 激活 11、retention RPC 8、
capture RPC 2、TTL integration 63，共 **185 项不同测试通过，7 项既有测试忽略**。
严格 workspace Clippy（含两个诊断/测试 feature）、格式和 diff 检查通过。

## 尚未验收

本轮建立并测试版本拒绝边界，不提供滚动升级能力。完整旧 binary 的启动/拒绝演练、
真实 TCP/MinIO/跨主机故障、部署停机恢复、容量稳态与形式化组合仍需验证。
原 source 永久丢失且 payload 未上传完成时仍须拒绝；payload 接手、全部 Publisher/
Reader 生命周期、旧 Version/负向 Pending 释放、portable successor recovery
authority 与 GC 继续按第二阶段完整目标推进。
