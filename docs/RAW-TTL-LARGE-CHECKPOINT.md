# 大 checkpoint 格式、恢复与临时磁盘准入

日期：2026-10-05。接续 [逐 SST 落盘](RAW-TTL-SPOOLED-CHECKPOINT.md)。生产
metadata/data checkpoint worker 现在可规划、发布和恢复超过 48 MiB 的镜像。
V2 manifest、Raft tag 14 和 V7 store/service floor 明确区分新格式；旧 tag 7/13
仍只接受 V1。**这是组件进展，第二阶段和完整生产验收继续开放。**

后续 [持久 pending 磁盘准入](RAW-TTL-PENDING-DISK.md) 已补充 named journal/发布
临时文件、旧 inode reader 和启动盘点的独立额度；下文的资源边界记录本组件当时状态。

## 格式与升级

完整 SST 序列化字节总和不超过 48 MiB 时，manifest 继续输出原
`KV9CHECKPOINT\x01`，字段顺序、change ID 和旧字节不变。超过该值使用
`KV9CHECKPOINT\x02`；V2 解码拒绝小镜像，避免同一内容有两个版本身份。
两版都保持 1 MiB manifest frame 和每 SST 48 MiB 上限。V2 最多 64 个 SST，
直接复用已有 retention owner 的资源数量限制，没有扩大元数据 owner closure。
64 × 48 MiB 是 descriptor 的推导上界，并非已完成该容量的运行资格或内存保证。

实际 driver 在消费 PreparedFlush 派生的提案时为大镜像选择 tag 14。
有序 apply 继续检查 generation CAS、change ID、cut、当前 range 或实际 import
资格；tag 14 不授权普通写、目录发布或提前完成导入。tag 7/13 遇到 V2 保持
解码失败，不能在升级重放时将历史无效日志重新解释为成功。小镜像仍分别使用
原 tag 7 或 tag 13。成功 publication 的 retained-log 核对也识别 tag 14 并检查
其对应版本，不能只凭描述符或对象存在完成恢复。

生命周期持久格式为 `KV9LIFE7`，内部服务路径为 `kv9.raft.v7`。进程启动 owner
之前持久化 floor；离线 installer 继续要求当前格式。V1–V6 磁盘记录可升级，
已写 V7 后旧 reader 拒绝，旧内部服务路径没有 fallback。格式确认和既有全成员
门槛要求 V7。这要求协调升级；本地 codec/HTTP2 测试不等于旧 binary、网络混部
或跨主机滚动升级资格。

## 流式恢复和资源边界

规划、journal 重开和上传继续逐 SST 分配，共享 ImageBudget。planned journal
保留既有 V2 外层 framing、checksum 和发布顺序；内部 manifest 自带版本。
旧 resident planner 仍限制 48 MiB，resident journal loader 明确拒绝任何大镜像
planned journal，包括总文件长度尚在原“镜像加 metadata”包络内的边界情况。
prepared journal 只保留 descriptor，可通过原接口读取。恢复最终仍构建完整常驻索引。

本轮修正了一个此前遗漏：metadata 的实际 `prepare_remote` 启动入口直接创建
FlushJournal，之前没有打开流式加载。现在它与 uploader 创建的 journal 都逐 SST
加载。前一份说明关于 startup/recovery 路径的结论应以此修正为准；旧证据包保持原样。

新增共享 SpoolBudget，在创建匿名规划文件前预留完整布局字节，持有到该文件
句柄释放后。metadata、data groups 和 scoped uploader 共用 runtime 的额度，
拒绝立即返回；文件打开或 buffer 准入失败也归还预留。状态字段
`checkpoint_spool_disk` 显示当前/峰值预留、活动 spool 和拒绝次数。

配置 `KV9_CHECKPOINT_SPOOL_BYTES` 默认 **512 MiB**，runtime 最小 **48 MiB**。
它仅计算活动匿名规划文件：**durable pending、发布临时文件、WAL、OS page cache
和全部文件系统债务不在此额度内**。stage 期间仍可能同时持有 spool 和 pending。
ImageBudget 默认 96 MiB、runtime 最小 48 MiB 不变，也不是整个进程 RSS 上限。
源 capture/迁移安装的 resident 规划接口仍受 48 MiB 限制；本轮没有打通大镜像
的完整副本迁移路径。

## 验证与限制

[证据包](raw-ttl-large-checkpoint-validation-20261005/manifest.json)记录实际命令、
退出状态、原始输出、源码摘要和历史证据校验。
**265 个不同单元/集成测试及 5 个 compile-fail doctest 通过**，8 个既有测试
按原配置忽略；workspace 严格 Clippy、格式、diff 和三个相关脚本语法检查通过。

- engine 实际 **62,919,860 bytes** 镜像完成规划、stage、流式重开、上传、prepared
  转换和全部 CF 恢复；序列化 buffer 预留峰值 **7,340,128 bytes**。legacy decoder、
  resident 规划/加载拒绝。描述符测试另覆盖 64 个 SST 边界、超额数量、过大单 SST、
  小镜像伪造 V2、总和溢出，以及原 V1 字节/change ID。
- 共享 spool 额度在两个 uploader 与 scoped 副本间生效，第二份不足时在文件创建
  前拒绝；drop、打开失败、buffer 拒绝后无活动预留泄漏。
- 真实有序 apply 验证 tag 7/13 不接受 V2，tag 14 编解码、拒绝普通 WriteBatch
  lowering、成功 CAS 和重复提交幂等。冻结 V6 lifecycle codec 拒绝 V7；带有效
  认证的 HTTP2 请求也无法访问旧 V1–V6 服务路径。
- 三副本实际 runtime fixture 使用 **900 个合法 60 KiB 公开 TTL 值**，另含永久值
  和已过期值。导入未 Finish 时选中大镜像，engine WAL 每副本从 **55,392,577**
  减为 **8,130,473 bytes**，Raft 从 **55,374,365** 减为 **3,084 bytes**。
  缺少 catalog 授权、落后 learner 仍拒绝截断；三个副本重开、配置后缀重放、换主、
  Finish/目录发布和原始 expiry 保留通过。
- 发布后公开续期，再由实际 worker 为已绑定 range 选择 **55,327,204 bytes**
  镜像；停止 owner 后，三个副本通过正式 data-group 恢复入口重开，核对 Bound
  身份及续期后的 value/expiry。第二次维护仅访问重开后的子组 driver，遵守每个
  driver 只能 mint 一次 manifest seam 的生命周期约束。

大镜像场景显式传入 **17 MiB** buffer 额度，峰值 **8,358,192 bytes**，排空后
buffer/spool 预留归零。测试使用 debug build、100 ms 子组 tick，checkpoint 等待
上限 60 秒以容纳原生产 30 秒 pass 和 follower adoption；生产 timeout 未放宽。
这些数值不是吞吐、延迟、RSS 或长时间资源稳态资格。

使用磁盘 WAL/Raft、InProcHub、认证 HTTP2 duplex 与 MemoryObjectStore；未新增
真实 TCP/MinIO、进程 kill、旧 binary 或完整 NodeRuntime 跨主机验收，也未新增
形式化证明或 TLC 结果。后续优先推进 durable pending/WAL/磁盘债务及 CPU 准入、
大镜像 capture/迁移、常驻索引/RSS、长期历史与 Reader/Version/Pending 释放、
unknown PUT 隔离、TTL merge 和完整故障部署验收。
