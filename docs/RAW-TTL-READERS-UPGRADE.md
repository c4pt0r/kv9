# Reader 生命周期的 V9 升级隔离

2026-10-05；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
接续 [metadata 跨来源 Reader 盘点](RAW-TTL-IMAGE-READER-CENSUS.md)。此前 Reader、
BootstrapReader、读准入关闭及逐代盘点逐步加入 ledger，但早期 V8 服务仍可能绕过
这些规则。本轮将这些约束纳入磁盘和内部服务的最低版本，并修复已有离线授权的
启动快速返回缺少格式检查的问题。完整第二阶段仍未验收。

## 行为变化

- 当前生命周期 header 为 `KV9LIFE9`，格式确认返回 9；长度仍为 97 字节。
  V1–V8 可由新版本读取并持久升级，node、incarnation、root、Prepared/Bound/Active
  phase 保持不变。旧 V8 codec 拒绝 V9；升级失败的 guard 继续拒绝授权，须关闭重开。
- 内部服务只有 `kv9.raft.v9.Kv9Raft`。全部 15 类内部 RPC 共用新路径，包括
  metadata/data Raft、登记、retention、bootstrap fence、selection 和 capture
  传输；没有旧服务回退。已有相同 cluster token 也不能跨 V8/V9 注册或复制。
- metadata 冷启动组合在 preflight 和引擎 WAL 打开之前检查已有 lifecycle 的当前
  格式和 store 身份。选中 checkpoint 后，enrollment 在加载授权及已有 receipt
  快速返回之前核对锁持有的目录、精确身份和 Active 格式 fence。因此已有离线授权
  也不能绕过持久升级。缺 lifecycle 的无 checkpoint 路径保留原 certified local
  recovery 流程；此检查不提前创建或采用身份，选中 checkpoint 仍要求有效 fence。
- 已有授权在正确升级后继续支持全组离线恢复；不需要重新登记或联系在线 quorum。
  未确认授权仍走原来的在线登记、Published 来源核对和实际 GET 生命周期。
- ledger 保持 `KV9LED19`。Reader/Bootstrap receipt、read gate、image census、
  manifest、Raft command 及 capture 编码不变；升级本身不修改 pins、历史来源或
  回收资格。offline installer 在创建 group 文件前也拒绝未升级的 V8 guard。

这是停机升级边界。部署必须停止所有旧 owner，保留原数据与身份，由新版本持有
独占 StoreGuard 后持久升级，再启动服务。不能混合版本滚动运行；发布 V9 后也不能
直接回退旧 binary。测试中的旧 header 构造只用于已完全停止的隔离 fixture，不能
作为运维降级或恢复操作。

## 验证范围

冻结的 [V8 proto](../crates/raft/src/grpc/ttl_format/pre_readers_wire.proto) 是上一
证据包完整 proto 的原始字节；[V8 codec](../crates/common/src/store_lifecycle/pre_readers_codec.rs)
是上一包真实源文件的常量、StorePhase、StoreRecord 和 codec 原文摘录，另加测试包装。
来源及 fixture hashes 随新包记录。它们只在测试中 include，没有旧生产服务注册；
计数 backend 和停止后的格式 fixture 均不等于运行完整旧版本 binary。

- Prepared/Bound/Active 的 V8→V9 升级保留身份，重复调用不重写，真实关闭重开后
  保留新格式。V7/V8 升级发布的 create/write/fsync/rename/目录 fsync 前后注入错误，
  所有失败均使当前 guard 失效；不是实际断电验证。
- 当前 dispatcher 对 V1–V8 的全部 15 类 RPC 路径拒绝，即使携带有效鉴权。
  冻结 V8 与当前 V9 生成客户端/dispatcher 经 HTTP/2 duplex 双向交叉调用；各自
  正常格式确认成功，跨版本格式、登记、retention、metadata/data Raft、bootstrap
  fence 和 selection 拒绝。旧 backend 未收到新调用，当前 Raft inbox 未收到旧消息。
  原 V7 交叉版本验证也保留。
- 冷启动回归先复现“已有授权绕过新 floor”，再验证修复：拒绝时不打开引擎 WAL、
  不调用 ledger、不改 receipt；实际持久升级后可离线复用原授权。
- 扩展真实三副本 Raft/retention 场景：独立 Snapshot GET 在超时后仍保持实际执行，
  census 页面丢回执、换主、真正 GET 排空并完成 image census。全部 owner 停止后，
  对三份存储构造 V8 header，逐份验证 production startup 在 GET/PUT 前拒绝且
  原 WAL/receipt 不变；实际升级再从新 checkpoint 和 Raft tail 离线恢复。精确
  census、Snapshot 与旧 Version 的 Published 保护均保持，原授权不重写。
- 原三副本 migration capture 恢复测试继续覆盖从停止的 V7 格式到当前 V9；原
  source capture/catalog/pins 保留。installer 的早期拒绝扩展到 V3/V7/V8。

原始失败与通过日志、当前 source/fixture hashes、相对上一包的补丁及精确检查清单见
[本轮证据包](raw-ttl-readers-floor-validation-20261005/manifest.json)。三处部署脚本的
header 断言和 `check-wire-floor` 示例同步到 V9；这些脚本的语法和示例编译验证不代表
真实部署演练。

本轮共 **572 项不同测试通过，9 项既有 MinIO 测试忽略**。覆盖 common、格式 RPC、
metadata、93 项 TTL runtime、bootstrap/outbox、恢复、snapshot/install、Raft 激活及
retention RPC。严格 workspace Clippy（含两个诊断/测试 feature）、格式与 diff 检查通过。
依赖 TCP listener 的整套 wire transport 测试因先前已确认的环境限制未执行；本轮
版本拒绝使用真实生成协议的鉴权 HTTP/2 duplex 测试，不能替代跨进程网络验收。

## 仍需推进

版本隔离不补造旧 Reader 来源，不证明旧 GET/PUT 已结束，也不撤销已有离线恢复授权。
全体 store/Publisher 生命周期、data group/joint membership、backup/replacement、
portable recovery anchor、旧 Version/负向 Pending 安全释放、未知 PUT 隔离、终止历史
与实际空间回收仍须完成。真实 TCP/MinIO、多进程旧 binary、断电/跨主机部署、时钟
资格及形式化组合仍未验收。没有启用自动旧 Version/负向 Pending 释放或对象 GC。
