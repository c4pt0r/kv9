# Raw TTL 时钟提供者与时间观察

2026-10-04，基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8` 的未提交工作树。
这是 [第二阶段](RAW-TTL-PHASE2.md) 的组件进展，尚未完成生产时钟资格验证或公开 TTL API。

## 已接入的路径

`Config.raw_ttl_clock` 默认 None，不自动选择 SystemTime 或把本机时间当作零误差时间。
显式配置后，NodeRuntime 在启动 owner 前构造一个共享的 LinuxTtlClock，每个节点仅持有
一个 timerfd 和一个采样状态；没有每 group 的时钟线程或定时器。配置与内核资源初始化
失败会拒绝该配置；健康检查在每次采样时执行。

- 源读取 `adjtimex(modes=0)`，只观察，不设置时间。`maxerror` 与 `precision` 按微秒
  处理，`time.tv_usec` 按 STA_NANO 决定单位；不以 esterror 替代最大误差。
  只接受 TIME_OK，并检查同步、硬件、PPS、闰秒及未知状态位。
  接口定义见 [Linux adjtimex 手册](https://man7.org/linux/man-pages/man2/adjtimex.2.html)。
- 采样前后读取 CLOCK_BOOTTIME，计算包含声明漂移和两端采样误差的最大经过时间。
  BOOTTIME 包含主机 suspend 时间，但其速率准确性仍需部署验证，见
  [Linux clock_gettime 手册](https://man7.org/linux/man-pages/man2/clock_gettime.2.html)。
- 一个非阻塞 CLOCK_REALTIME timerfd 使用 ABSTIME/CANCEL_ON_SET，采样前后检查。
  ECANCELED 和 timer 到期均拒绝，只有 EAGAIN 表示本次未观察到通知；不重置 watch
  来隐藏跳变。通知行为见 [Linux timerfd 手册](https://man7.org/linux/man-pages/man2/timerfd_create.2.html)。
  其远期到期值为 8,000,000,000 UNIX 秒，不能表示它的 time_t 平台拒绝初始化。
- 按原始启动锚点和前一条有效观测同时检查区间一致性，避免每次更新基准后掩盖累计
  漂移。至少两个一致观测且达到 warmup 时长才发出样本。已观测异常使对象进入 Fenced；
  不因下一次读回正常值自动恢复。显式重建需重新 warmup，且不能降低任何 group 的水位。

状态文件增加 `raw_ttl_clock=<JSON>`，包含 policy、health、last_refusal、observations
和最近 uncertainty 上界；未配置为 null。它报告最近观测，不授权使用缓存时间。
目前按请求采样，没有后台轮询；空闲时状态不会自动刷新。

`NodeRuntime::observe_raw_ttl_time(region, deadline)` 先建立目标 group 的 ReadIndex，
读取实际 applied range 与持久 clock，再取得健康样本、检查它不落后于已提交水位，
提交固定的 Observe 命令。clock provider 不持有 Raft/engine 锁，不在 apply/replay 中运行。
`RawTtlSubmission` 保留真实提案位置和精确 apply wait；提交后 owner 丢失仍保留位置。
一次调用沿用同一个绝对 deadline，没有自动重试或更换时间源。

调用者必须先检查观察提交的实际结果，再建立新的读取屏障与 snapshot；单独收到
submission 或 sample 都不能授权 TTL 读取。公开 Get/Put/Batch/Scan 还没有调用此链路。

## 必须成立的误差模型

令内核给出的 UNIX 时间为 `x`。部署必须保证：报告误差、precision、量化余量与
`additional_uncertainty_ms` 的总和覆盖该观测相对于真实 UNIX 时间的误差。
Linux 的 maxerror 可以由时钟守护进程设置、由内核推进；它本身不是独立时间认证，
参见固定版本 [Linux v6.12 ntp.c](https://raw.githubusercontent.com/torvalds/linux/v6.12/kernel/time/ntp.c)。
还必须保证 BOOTTIME 的理想速率在 `1 ± ρ` 内，单次读数绝对误差不超过 `ε`，覆盖
支持的 slew、调度、suspend、虚拟化和恢复场景。clock_getres 只是必要检查。

对观测到的 BOOTTIME 差值 `d`，以纳秒计算：

```text
elapsed_min = floor(max(d - 2ε, 0) / (1 + ρ))
elapsed_max = ceil((d + 2ε) / (1 - ρ))
radius = reported_error + additional_error + sample_elapsed_max
sample = [floor((x - radius) / 1ms), ceil((x + radius) / 1ms)]
```

减法在 UNIX epoch 处截为 0，其余运算溢出拒绝。候选区间分别与锚点/前一次观测按上述
elapsed 边界投影后的区间求交；无交集拒绝。只要输入区间与 elapsed 假设成立，求交与
向外取整不会移除真实时间。有效样本仍走原有 `sample.advance(committed)`；新节点或
重启节点落后于已提交水位时不能靠重新构造 provider 绕过。

检测器能发现部分假设违例，不能证明上述假设。未通知的共同 clock 错误、虚假的同步
状态/误差报告，以及不满足额外余量的 leap smear 都不在已验证保证内。闰秒状态明确
拒绝，但当前环境没有进行真实时间调整或闰秒演练。小规模 CPU 测试不能关闭此门槛。

## 显式配置

节点启动参数 `--raw-ttl-clock-policy '<JSON>'` 接受完整 policy，最长 4096 字节；
缺字段、未知字段、重复 flag 和越界值均拒绝。下面是测试用参数，生产值需由部署误差
模型和验证确定，不能直接视为推荐配置：

```json
{
  "max_uncertainty_ms": 100,
  "additional_uncertainty_ms": 2,
  "max_sample_span_ms": 10,
  "boottime_drift_ppb": 500000,
  "boottime_error_ns": 100,
  "warmup_ms": 10
}
```

任意正常采样都要同时满足耗时与 uncertainty 上限。配置不提供自动资格判断；平台
不支持、warmup、同步失效、跳变、过大误差和落后水位有独立的 ClockRefusal。内核查询
失败后也持续拒绝；这可能牺牲可用性，需要在确认来源恢复后显式重建。

## 本地验证与未完成项

- 时钟组件 10 项测试通过，其中独立真实时间模型覆盖 729 个速率、两次 UTC 误差、
  BOOTTIME 端点误差与采样耗时组合；覆盖向外取整、累计漂移、回拨、快跳、超时、
  格式异常、丢失同步与持续拒绝。真实 Linux adapter 测试只读取主机时间并建立进程
  自己的 timerfd，不调整主机时间；首次
  采样必须拒绝，绝不根据本机结果宣称 UTC 准确。
- Runtime 时间观察 2 项通过：拒绝后驱动 Raft 仍无新增 applied 条目；实际 WAL 重开
  保留水位并拒绝落后采样；观察经精确 apply 后，新的 ReadIndex/snapshot 隐藏过期键，
  旧 snapshot 保持自己的时间水位。该测试使用注入的样本与进程内传输。
- CLI 配置专项 1 项、common 全部 53 项、本地 Raft 298 项通过；Raft 的 8 项 MinIO
  保持忽略，37 项网络测试显式过滤。workspace 全 target 严格 Clippy 通过。

未完成：实际多主机的时间源/硬件资格验证、真实 clock step/suspend/VM/闰秒故障，
新 provider 与跨节点 leader 切换的组合测试、完整 TLA/TLC/Chaos 与独立 TTL 时间历史
检查器。公开 TTL wire/客户端、所有读写路径、后台清理、split 复制和空间回收仍待推进。
第二阶段保持未完成。

原始日志和源码摘要见 [验证记录](raw-ttl-clock-validation-20261004/manifest.json)。
