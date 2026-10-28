# TTL 镜像安装、替换及 learner 尾部恢复

2026-10-04；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
本轮推进第二阶段的同组迁移/安装格式互通。原 JointInstaller 的镜像校验只允许
Default CF 的 Raw value 与 range record，因此合法的 clock、Write CF expiry 和
checkpoint history 也会被拒绝。仅让上传保留所有 CF 不能解决这个接收端阻断。

## 镜像内容与替换约束

安装、未采用 generation 的重开验证和首次 runtime adoption，现在复用普通 data
checkpoint 的完整镜像检查。检查针对 restore 实际还原的同一 snapshot，包含：

- 精确 destination cluster、group/creation、range 与 manifest epoch；每个 SST 的
  实际记录数，全部 CF 的 Raw namespace 与范围。
- clock 的规范编码；Write CF expiry 必须有 activation、水位以及同 key 的 Default
  value。expiry 编码必须合法，Lock CF 和不支持的系统/外部记录拒绝。
- checkpoint pair 与连续 manifest history 一致，历史 scope/cut 合法，compaction
  confirmation 不越过镜像位置。合法系统记录保留，不再被当成 foreign user key。

安装不重算期限，不以目标节点时钟过滤，也不重写 expiry 的 source creation/term/index。
已过期但尚未物理清理的 value/expiry 仍原样转移；来源 group 的 index 可以大于目的
镜像的 cut，跨组位置不能直接比较。`InstalledImage.records` 继续只统计 Raw value，
新增 `ttl_watermark` 是实际镜像的诊断观察，不授予时间、服务、投票或 owner 释放权限。

替换一个已选、尚未 runtime-adopt 的 generation 时，原有 cut/term/vote 单调约束
继续存在；另外，已启用 TTL 的旧镜像要求新镜像保留 activation，水位不得降低。
相同或更高水位允许；旧镜像尚未激活时允许转为有效的 TTL 镜像。这一检查发生在
新私有 generation 的 engine 验证中，早于 ready/selector 发布。失败保留旧 selector
及未选文件，并要求重开 installer；同一 digest 的幂等重试沿用既有验证。

V4 store fence、目的成员身份、同组锁、完整 engine/protocol pair、fsync/selector
顺序、最多 8 个保留 generation 和单向 adoption 规则不变。已经 runtime-adopt 的
generation 继续禁止 installer 替换。

## 验证

[命令、日志与源码摘要](raw-ttl-install-validation-20261004/manifest.json)另存。五项新增测试：

- 合法 TTL 镜像安装、重开及 adoption：三个用户值、二进制/空 value、已过期/未过期
  记录、另一 source creation 的原始版本位置逐字节保留；重建状态机拒绝旧写入绕过。
- newer cut 移除 activation 或降低水位时拒绝，旧 selector 不变且可重开；legacy→TTL、
  相同水位和更高水位分别成功。
- 八类损坏/不支持镜像：坏 clock、坏 expiry、缺 clock、孤立 expiry、越界 key、Lock CF、
  未知系统记录及缺失历史的 manifest pair；都不能替换旧选择。
- 五个实际安装发布点注入 EIO，分别带/不带旧选择，共十个窗口；重开只返回完整旧镜像、
  完整新镜像或未选择状态，且 TTL 字节和水位正确，原计划可续做。这里是进程内错误注入，
  不是进程 kill、物理断电或远端对象持久性证明。
- 真实 raft-rs **一个 voter 加一个 learner** 的运行链路：源组 TTL 写入并发布一次
  checkpoint，添加 learner 后捕获含合法 manifest history 的精确 cut；源组继续续期与
  Observe，目标安装旧镜像，再通过 InProcHub 追赶尾部。Cleanup 删除过期版本，同时
  保留被续期的值。目标实际 WAL/协议文件重开并重新 adoption 后状态一致；engine 先
  恢复尾部，driver 从 installed cut 重放已提交历史后才推进自己的 applied 观察。

最后一项为隔离 transfer 直接注入 Enable 测试入口，但命令仍经实际 Raft/apply；不将
其计为全副本激活或新增副本升级资格验证。对象后端为 MemoryObjectStore。

专项集合 **6 通过**（含原有 V4 fence 测试）；Raft 本地回归 **332 通过、8 忽略、37 过滤**；
RegionManager **16 通过**；TTL 激活/三副本 checkpoint 组合 **19 通过**。严格工作区
Clippy、格式和 diff 检查通过。集合存在重叠，既有真实 MinIO/TCP 忽略项没有被替代。

## 剩余阶段工作

这是同组镜像内容及安装生命周期的组件支持，不是完整可转移恢复授权。源捕获的真实
维护预算、源/目标 owner 与历史释放、新副本资格和完整迁移协调、adopt 后完整恢复
authority、真实 NodeRuntime/TCP/MinIO/旧二进制及 Chaos 验收仍开放。

后续增量见 [采用后的恢复校验](RAW-TTL-ADOPTED-RECOVERY.md)：已接入本地 committed
WAL/publication、逐项 TTL/range 检查、pending 保护和受目录决策约束的新 base 重启。
这关闭了 adopt 入口绕过普通恢复的缺口；完整可转移恢复授权与 owner 释放仍未验收。

现有 split population 仍只复制 Default CF；旧 seal 对已启用 TTL 的组仍在有序 apply
拒绝。需要实现持久可续做的 child 复制/发布协议，再转移 expiry 与水位；merge 还须
继承两侧最大水位。不能靠本轮放行 TTL seal。整机内存/CPU/磁盘与物理债务、常驻索引、
Reader/旧 Version/负向 Pending、对象 GC、历史有界、48 MiB 全量 image、时钟资格、
独立分布式时间历史及完整 TLA/TLC/第二阶段生产门槛继续开放。
