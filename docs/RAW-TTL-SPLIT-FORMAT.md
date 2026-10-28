# TTL split 的 V5 格式隔离

日期：2026-10-04。接续 [持久复制状态](RAW-TTL-SPLIT-STATE.md)，补齐旧 V4 owner
与新复制格式之间的磁盘、服务和逐 store 回执边界。生产 TTL split 提案仍关闭。

后续同日进展：[runtime 源核验与发布](RAW-TTL-SPLIT-RUNTIME.md) 已连接受保护入口。
后续 2026-10-05 [staging checkpoint](RAW-TTL-STAGING-CHECKPOINT.md) 已将当前 floor
提升为 V6，并以 tag 13 区分新 publication，保持旧 tag 7 语义。
下文保留 V5 格式隔离组件验收时的边界；完整生产资格仍未完成。

新 owner 使用 `KV9LIFE5` lifecycle header 和 `kv9.raft.v5` 内部服务。V5 读取
V1–V4 lifecycle，保留 node、store incarnation、phase、root，在已有独占锁下
原子持久升级后才允许 Raft/engine writer 启动。升级发生文件或目录持久化错误时，
原 guard 被置为失败；必须重新打开并稳定实际磁盘状态，不能把仅可见的新 header
当作已持久授权。离线 writer 也检查当前格式。

V4 包含 TTL，却不保证理解 tag 11 的 population 屏障、tag 12 的 transfer 指令
及其恢复记录。V5 header 被冻结的 V4 codec 拒绝；V5 dispatcher 不提供 V1–V4
服务路径，包括 Raft、注册、目录、格式确认和 checkpoint retention，不回退旧方法。
因此现有 V4 集群需要协调版本切换，不能将此变更描述为保持服务的滚动升级。
本轮没有部署或改写任何真实集群磁盘。

`ConfirmRawTtlFormat` 现在只接受精确 V5 回执；V4、未知未来版本和错误 root/
cluster/node/store incarnation 均未确认。现有 TTL activation 的单次门槛继续
覆盖 incoming/outgoing voters、learners、learners_next 的完整并集，包含本机，
重复回复不能代替缺失成员。原有 term、配置及 applied-conf index、driver 身份和
期限检查保持有效。新格式确认只证明 store 支持格式，不授予 split 源权威或空子组
导入权；尚未把这个门槛接到 Seal/Begin 的生产提案方法。

更新了版本探测 example 与现有 endpoint/replacement 校验脚本，使其检查 V5。
这些外部脚本本轮只检查语法，没有运行真实网络/Chaos 场景。既有历史证据包保留原
版本和哈希，没有重写。

## 验证

[证据包](raw-ttl-split-format-validation-20261004/manifest.json)保留命令和源码摘要。

- lifecycle：V1–V4 × Prepared/Bound/Active 均保持身份，升级幂等，重开仍要求 V5；
  冻结 V4 codec 接受旧记录并拒绝新记录。V4 升级每个已有 publication 故障切点
  均禁止旧 guard 继续授权，重开后才能取得实际持久格式回执。
- 实际 tonic dispatcher 拒绝全部 V1–V4 方法路径，当前方法进入认证边界；回执
  校验拒绝 V4、未来版本和不匹配的 store 身份。
- 五副本 joint 配置门槛、leader/配置/driver/期限变化及精确 apply 回执回归通过。
  server TTL activation 回归使用新生成的 V5 client/server，覆盖认证 HTTP/2 duplex
  和三副本 InProcHub；没有真实 TCP。

共 55 个不同测试通过（lifecycle 9、format RPC 4、activation gate 8、server 34）。
严格 workspace Clippy、格式、diff 与三个 Python 校验脚本语法检查通过。

尚未完成旧二进制进程验收、跨主机 rollout、真实 TCP/MinIO 和完整 NodeRuntime。
下一步是把 metadata split intent、源 Raft seal/cut、新格式门槛与实际空子组观察
组成生产受保护入口，再接通双 CF 复制和原子目录发布。merge 和完整第二阶段仍开放。
