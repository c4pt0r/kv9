# 已提交 split 的后台续做与共享准入

日期：2026-10-04。接续 [TTL split runtime](RAW-TTL-SPLIT-RUNTIME.md)。本组件修复
`KV9_AUTO_SPLIT_BYTES=0` 同时停止已提交 split 的问题，并将协调工作移出节点控制循环。
完整生产范围和第二阶段仍未验收。

## 行为与生命周期

节点启动一个 split dispatcher，初始不具备执行资格。控制循环根据 bootstrap serving、
endpoint recovery 和 Raft 接收资格更新它；失去资格后撤回等待需求，不派发新工作。
阈值 0 只关闭新触发。每个已提交 split intent 的 seal、两侧 populate 和 publication
仍根据实际 metadata、父子组状态及当前 leader 继续推进；重启不依赖调度器内存游标。

续做优先于新触发。一次续做任务只调用 seal、单侧 populate、publish 之一。新触发
沿用原有稳定 operation 派生、两子组创建和 intent 提交流程，仍需 metadata leader。
父组/子组的实际 leader 承担对应操作；metadata leader 发布。所有协议检查仍在
既有 AdminApi 及有序 apply 中执行，调度器读到的状态不构成授权。

新增第九类共享维护任务 `split`。每个节点最多运行一个，计入既有 4 个任务、2 个
heavy 任务的总预算。实际 `spawn_blocking` 任务持有非克隆 permit；取消 dispatcher
不会提前归还该名额，也不会中止或回滚已经接受的 Raft 提案。节点关闭时先取消派发、
停止 data-group owner，再等待实际 split 任务退出，最后释放 group/store 资源。

循环每 100 ms 观察完成状态；失败后退避 200 ms 至 3.2 s。intent/stage 和触发
range 使用独立的轮转游标，拒绝后也向前移动；每次至多访问 32 个续做候选、32 个
新触发候选。游标只是调度位置，不记录复制进度。节点状态输出 `split_maintenance`
包含运行资格、阈值、在途任务、pass、seal/populate/publish/trigger 次数、失败数和
最近一轮错误。意外退出会进入节点健康检查错误。

## 验证

[证据包](raw-ttl-split-maintenance-validation-20261004/manifest.json)包含实际命令、
日志和源码摘要。server TTL activation/runtime 36 项、共享预算及 TTL maintenance
12 项，合计 **48 个不同 Rust 测试通过**；严格 workspace Clippy、格式及 diff 检查通过。

新增回归运行实际 metadata、parent、low/high child 三副本 raft-rs、磁盘 WAL，
内部认证 HTTP/2 duplex 与 InProcHub。直接启动生产 `Maintenance`，保持阈值 0：

- 每个节点两个 heavy 名额被占用时，split 等待且不封存；失去资格撤回等待，恢复
  资格后重新等待。公开 TTL 写入仍可完成。
- 缺少第三个 store 的格式确认时，后台 seal 拒绝且记录失败；恢复确认后自动续做。
- 父组 leader 为 node 1，高子组 leader 为 node 3，metadata leader 为 node 2；
  无手动 seal/populate/publish 调用，最终三个 metadata 副本均观察到发布。
- 两侧原始 expiry 字节、已过期行和永久值保持正确；没有新 split intent 或 trigger。
  取消并排空实际 worker 后，节点维护名额和等待需求全部归零。
- 独立预算测试确认 split 仅占一个 heavy 名额；dispatcher drop 不能释放实际任务
  的名额，TTL 仍可使用其余容量。

上一组件的中途复制/WAL 重开/换主续传与发布后读写测试也在本轮回归中通过。
本轮没有新增形式化证明或重新运行未改动的 Lean/TLA 组件。完整 NodeRuntime、TCP、
MinIO、进程 kill、Chaos 和跨主机验收仍未执行；测试没有替代这些要求。

后续 2026-10-05：[staging checkpoint](RAW-TTL-STAGING-CHECKPOINT.md) 已连接未绑定
导入的镜像、publication、worker 和恢复，并验证实际 engine WAL 段回收。以下保留
本组件记录时的后续清单；导入 Raft 截断和完整容量要求仍未完成。

## 容量边界与下一步

候选访问限制不是完整的扫描或延迟预算：发现仍把全部 binding/intent 读成 Vec 后
排序/过滤；磁盘统计仍遍历文件树。源摘要重读封存 snapshot，TTL 单侧 populate
可在 30 秒控制期限内复制多页；普通 Raw population 仍沿用原有整侧复制调用。
因此本轮只限定并发和选中候选数，尚未限定总 CPU、RSS、磁盘带宽、WAL 增长或
单任务延迟；也没有证明满负载公平性。intent 提交前遗留的已创建子组仍需生命周期
协议处理；阈值 0 的续做契约目前针对已经提交的 split intent。

未完成 import 仍只能依赖 WAL。当前 checkpoint scheduler 从 RawDirectory 发现
已绑定组；`checkpoint_scope`、image inspection、adoption 和恢复入口也要求真实
`RANGE_KEY`。不能通过提前绑定或伪造 range 把未完成子组加入这个流程：这会混淆
导入状态与可服务所有权。

下一步需要显式定义 staging checkpoint：绑定实际 child creation、operation、
source seal、import spec/cursor/hash、clock 和 applied cut；镜像检验与 WAL tail
必须保留“尚未绑定”的状态。随后连接 staging 的发现、持久 Pending/owner、发布、
恢复与安全日志回收，并验证 Begin/page/Finish 各切点重启。恢复锚点和 retention
释放必须成立后才能截断 WAL。超大历史 value、abort/回收、merge、历史物理增长、
完整预算与部署资格继续按 [完整范围](RAW-KV-PRODUCTION-SCOPE.md) 推进。
