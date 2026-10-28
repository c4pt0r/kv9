# Checkpoint 结果收敛的历史扫描预算

2026-10-04；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
接续 [持久采用记录](RAW-TTL-ADOPTION-FACTS.md)。第二阶段与生产验收仍未完成。

## 修复的问题

原 `ManifestSeam::converge` 在查询精确 predecessor 的历史结果前，先调用
`manifest_effect`。后者持 state-machine 锁从当前 generation 遍历全部历史。
即使一条精确历史就能决定旧 pending 的结果，也要先付出随历史增长的扫描成本，
其间阻塞同一 state machine 的 apply。

现在先查询原 attempt 的精确窗口：当前 pair 可直接决定时直接返回；已超过窗口时，
读取 `expected_generation + 1` 的不可变 transition。自己的 change ID 得到
`AlreadyApplied`，不同的 winner 得到 `WindowRefused`。精确 identity 的结果优先于
其他 checkpoint 对内容的覆盖，不会把已知失败的 identity 报成它自己的成功。

只有没有精确结论时才查询正向 effect，每次最多 **4 条历史**。生产 driver 每页持锁一次；
真实 state-machine 测试计数为一条 current pair 读取加最多四条 history 读取。
这是 effect page 的读取界限，不包含 converge 之前的 pair/transition 查询。

## 续做和并发

- 原 attempt 的进程内 slot 保存向旧 generation 移动的游标。不断追加新 checkpoint
  不会将尚未完成的扫描重置到最新一代，因此旧匹配仍可到达。
- 页读取失败保留原游标；缺行、扫描完毕或没有 effect 都不是负向证明，slot 保持
  Unknown。一个完整 pass 结束后，下次调用从新 head 开始，以便观察后续覆盖。
- 每次 converge 最多一页；当前 predecessor 尚未被消费时，仍可重发同一 identity。
  重启从 durable pending 重建相同 attempt，游标从 head 重来，不持久化扫描进度。
- 同一 slot 同时只有一个调用驱动查询/发送/等待；其他 converge 返回 typed Busy。
  不在全局 slot 表锁内等待 driver。不同 region 可以继续。清理 slot 校验实际 Arc
  身份；旧调用退出不会清除或解锁后来创建的 slot。
- canonical checkpoint 历史必须通过解码、region 与外层 term/index 绑定检查；损坏
  不被当成正向覆盖。非 checkpoint 历史不提供 effect 证明。

内部 Rust 查询接口改为 `manifest_effect_page`；RPC、Raft command、WAL 和 journal
格式没有变化。每页行数有界不等于 CPU 时间或总解码字节已有硬预算：canonical manifest
有既存 1 MiB 编码边界，旧 opaque history 可能更大，输入分配与历史总量仍需治理。
这也没有回收历史或证明整机持续负载下的公平性。

## 负向 Pending 的边界

实际三副本组合测试先持久登记并上传一个 predecessor 为零的 pending，再通过真实
data Raft 连续发布九个其他 checkpoint。metadata 与 data leader 分离，owner 请求走
带节点认证的 HTTP/2 duplex，SST 使用 MemoryObjectStore。

旧 attempt 随后从 generation 1 的精确历史得到 `WindowRefused`，本地候选完成并清除
journal。测试同时确认全局 Pending owner **仍为 Published**，没有创建对应 Version，
原对象仍存在。测试显式驱动发布步骤，没有模拟自动 worker 的并发上传竞态。

负向 identity 结论只能证明该 CAS 不会生效，不能证明所有同 identity 上传者、队列和
超时后仍执行的对象请求都已排空。本轮没有新增全局 owner quiesce/release 或 GC。
后续仍需持久、可重试的执行者关闭证明，以及所有副本、Reader、旧 Version、迁移等
实际恢复引用的覆盖证据。

## 验证

[命令、日志和源码摘要](raw-ttl-settlement-budget-validation-20261004/manifest.json)另存。

- 真实 state-machine 新增 3 项测试：每页实际读取上限；持续追加时第 11 页找到旧匹配；
  空/缺失历史、`u64::MAX`、游标边界和 corrupt/misbound 历史。
- region 新增 5 项测试：精确历史优先、错误后原页重试、扫描完毕继续 Unknown、重启
  重扫、并发 Busy 与其他 region 进展、旧调用不能清除/解锁替代 slot。
- 三副本新增上述旧 pending 的精确负向收敛与全局 pin 保留测试。

Raft 本地回归 **342 通过、8 忽略、37 过滤**；region **15 通过**；TTL 激活组合
**20 通过**；Raft doc tests **12 通过**。专项集合与回归有重叠，不相加。
严格 workspace Clippy、格式和 diff 检查通过。真实 TCP、MinIO、完整 NodeRuntime、
跨节点时钟资格、旧二进制、完整 TLA/TLC 与 Chaos 仍未验收；TTL split/merge、完整
恢复移交、对象生命周期、常驻索引和整机资源预算继续保留在完整第二阶段范围中。
