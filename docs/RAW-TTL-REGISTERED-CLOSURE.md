# 已登记 checkpoint 执行者的有界闭合核对

2026-10-05；接续[原子 Begin](RAW-TTL-ATOMIC-BEGIN.md)、
[逐代来源](RAW-TTL-EXECUTOR-ENROLLMENT.md)和[后台换主](RAW-TTL-CHECKPOINT-FAILOVER.md)。
完整第二阶段与生产验收仍开放。

## 新的行为

此前 caller 只能分别读取 Build 索引页和某个 executor 的历史页，EOF 本身无法回答
是否保留了先前页面的阻塞项。现在 metadata 保存每个 Closed 尝试的聚合进度，后台
在 Held sweep 完成后，每次访问推进一个 owner 的最多四代历史。caller 只提交
Pending token 和预期 sequence；planner 在同一已建立 snapshot 上重新计算页面、
计数和下个游标，客户端不能上传“全部完成”或零阻塞的结论。

扫描前必须具备完整 Build catalog census。它遍历反向 Build 索引，包含 gate
创建前的通用/旧 Build，并对每个选中 owner 从 generation 1 开始稠密核对到当前
代。每页保留 owners/generations、已证明零 PUT、缺来源、Active、缺结束事实、
非零 PUT 代数和 unresolved PUT 数；后面成功的代际不会覆盖前面的缺口或 unknown。
原子 Begin 的 birth revision 也绑定到进度，旧尝试没有 birth 就不能取得合格结论。

`Work::Complete` 表示该尝试已登记集合的核对已完成，不代表没有阻塞。
`registered_zero_put_closed()` 仅在当前 Complete、具有原子 birth、所有被检查代际
均证明零 PUT 时为 true。Cancelled、合格的 RetiredUnpublished 和实际 `(0,0)`
Drained 计为零 PUT；`issued == resolved > 0` 仍计入非零 PUT 阻塞，不替代真实对象
后端的终止资格。返回值是观察，不是资源释放 capability。

## 跨页并发与重启

每个尝试有一个持久 change stamp，值为最后一次相关 metadata mutation 的 ledger
revision。`Planner::finish` 检查当前命令已有的有界 writes，把以下事实变化与新
stamp 放进同一个 batch：Build owner/registration、Build 反向索引、intent/gate、
逐代来源/排空/未发布退役和原子 birth。没有 gate 的早期 generic Build 不单独生成
stamp；Begin 在创建 gate 时初始化 stamp，随后完整 catalog census纳入这些旧 owner。

该检查不扫描完整引擎、不复制未提交 overlay，也不以 overlay 授予 I/O 或释放权限。
closure 自己的游标、Held sweep 游标和全局 census 进度不更新 stamp，避免扫描让
自己不断失效；其他尝试的变化也不会使本尝试重扫。

每个页面绑定 source revision。迟到报告、新取消代际或插入到已经过游标之前的新
owner 会更新 stamp，旧 Complete 立即变成待推进状态。下一次提交清空聚合结果并从
头核对，不能只继续尚未扫描的尾部。缺失旧代记录依然占据阻塞项。Closed gate 后
允许的未进入取消同样参与 invalidation，不依赖“后来的取消大概无害”推断成员完整。

重复同一 sequence 的紧邻重试为幂等确认；旧到不能证明是最后一次提交的请求拒绝。
重新加载进度、换主与 WAL 重开保留 sequence、source revision 和累计阻塞项。丢失
进度可从头重扫；已有进度却丢失 stamp、stamp 回退、错误 root/gate/birth/编码或
新行出现在旧 header 下均拒绝，不用空值补证。

## 格式与成本

| 内容 | 编码 | 上限 |
| --- | --- | --- |
| Ledger header | `KV9LED07` | reader 接受 01–07，旧布局成功写入时原子升级 |
| 聚合进度 | namespace tag 18，`KV9CCL01`，Pending ID key | 最多 235 字节，decoder 上限 256 |
| 变更水位 | namespace tag 19，`KV9CCM01`，Pending ID key | 96 字节 |
| AdvanceCheckpointClosure | ledger request tag 16 | 原有小请求限制；无新 RPC 类型 |

保持 V4 Build 索引、V5 enrollment、V6 birth 的原字节与含义；06 下没有 stamp 的
已关闭尝试在首次扫描时原子建立 stamp 与进度。固定 V4 fixture 去掉在旧版本中不
存在的 closure stamp，继续验证旧 V1/V2 不能补造来源。header 读写和升级 guard
已同步，内部 service/store floor 仍为 V7。混合二进制或滚动升级未由本轮验收。

单次至多读取五条 Build 索引（四条加 lookahead）、消费一个 owner 的四代，并写
一条进度；相关事实 mutation 另加常数大小 stamp。资源 closure 和旧 resource base
的原有上限仍适用。测试测得 13/113 代首页面均为 211 次点读、一条索引；13/113 个
owner 首页面均为 312 次点读、五条索引。它验证页成本与总历史大小解耦，不是吞吐、
RSS 或物理总历史有界的验收。

## 后台和验证

已有 metadata leader sweep 发现 Closed/Done 后调用该观察。需要推进时提交一条
受当前 leader/取消/deadline 检查的 Advance；有阻塞的 Complete 在 stamp 未变时不
反复追加，事实变化后再重扫。维护统计新增页面、重扫、零 PUT Complete 观察和
阻塞 Complete 观察计数；后两者是访问次数，不是去重尝试数。全局 census 未完成
时等待其既有轮询；原有按尝试轮转和每节点任务启动继续使用。

[本轮独立证据](raw-ttl-closure-validation-20261005/manifest.json)包括：

- 八项 metadata 专项：65 代跨五个 owner、全部阻塞类别、游标后插入与迟到报告、
  不相关尝试和幂等操作不重扫、旧尝试不补 birth、损坏/降版拒绝、V6 升级、WAL
  截断下报告与 stamp 的全有或全无，以及 13/113 规模下固定页面读取量。
- 实际三副本 Raft/鉴权 HTTP/2 duplex：九代真实零 I/O scope、另一个实际对象
  上传但无合格响应的 scope；首个聚合提交后丢回执，换主后精确重试与后台续扫。
  第十代迟到取消使旧 Complete 失效，后台重启从原事实重算；另一尝试的 unknown
  不变，metadata WAL 重开保留两种结论，两个 Pending 均仍 Published。

本轮 metadata 全套 **191 项通过、1 项既有 MinIO 用例忽略**；完整 TTL integration
**54 项通过**，合计 245 项不同测试。严格 workspace/all-targets clippy、格式与
差异空白检查通过。专项与完整套件重叠的测试不重复计数。

## 尚未授予的权限

本轮没有释放负向 Pending，没有删除对象、释放 Reader/旧 Version，也没有把
无报告崩溃遗留者、旧 binary、未登记外部上传者或任意非零 PUT 认证为终止。
零 PUT 登记集合的结论仍需与完整 uploader/部署协议、精确负向 settlement 和
恢复依赖共同组合，才能成为全局释放证书。可信成员报告和 metadata/engine 的
正常原子写入、恢复完整性是该观察的前提；没有宣称任意底层无痕删除都可被
一个常数大小聚合独立检测。

完整 publisher 资格、非零/未知远端 PUT 终止或隔离、继任恢复锚点、Reader/旧
Version、物理 GC、历史回收、稳态容量、部署时钟以及完整形式化/真实 TCP/MinIO/
进程故障验证继续保留。新聚合是组合证明的实际输入，不代表完整第二阶段完成。
