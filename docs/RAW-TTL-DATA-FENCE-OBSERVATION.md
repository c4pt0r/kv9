# DATA store 读下限的独立确认

2026-10-05；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
接续 [DATA 本地持久读下限](RAW-TTL-DATA-READ-FENCE.md)。本轮让 metadata 独立
核实并持久记录指定 store 的 prospective DATA 读下限。它仍不是全体历史成员的
读关闭证书，不授权释放 Pending、Version 或删除对象。

## 查询与身份

新增内部 `InspectDataReadFence`，与 checkpoint history、selection 和 metadata
bootstrap fence 分离。查询绑定 cluster、root、node、store incarnation、DATA
region、group creation 和新生成的 16 字节 challenge；响应回显完整查询，包括预算，
只允许规范的 224 字节 `KV9DRF01` 或明确 Unknown。客户端不能把历史 selection
返回值转换成这项确认。帧上限 512 字节，RPC 预算至多两秒；metadata 的单次远端
观察至多一秒，并与整个 retention 操作共享原有总 deadline。

服务端在读取前后都核对有效节点身份和当前 store authority。实际读取由 runtime
调用持有独占 store 文件锁的 `DataReadFences`，读取并同步持久文件；不依赖该
DATA group 仍有运行中的 handle。缺少记录返回 Unknown，文件损坏、目录缺失、
错误 creation 或身份拒绝；不支持和超时也不能成为确认。

读取与 checkpoint history/selection 共用一个有界 blocking-reader 槽位。
RPC 超时、调用方取消或监听停止均不会提前归还实际 I/O 持有的槽位；工作实际退出
后才释放。返回的 floor 还须与请求的 store、group 和 creation 完全匹配。

## metadata 固化

`RecordDataReadFence` 是新的 request tag **36**。普通公开 retention planner
拒绝此请求；调用者不能提交一个自称有效的 floor。metadata leader 先验证候选
intent，再通过独立查询确认实际 floor 精确等于候选镜像和 winning generation。
本机观察也走实际持久 provider，不使用 selection 记录替代。

观察在 catalog 锁外进行；取得锁、排空 metadata 当前 term 后，重新核对完整
endpoint 行（含地址代际、incarnation 和 active 状态）及 intent，再在同一 term
提交。查询期间发生地址变化，即使新旧地址相同，也拒绝这次旧观察。

ledger 提升为 **KV9LED21**。namespace **48** 为每个 `(node, store, region)`
保存一条最新记录：完整 floor、观察时 endpoint generation 和 ledger revision。
记录为固定 280 字节，使用规范编码与校验和；creation、generation、镜像位置或
epoch 不允许回退或变成不可比较的内容。V20 可随原子操作升级，但旧 header 下
出现新 namespace 行会拒绝，不能把降级或孤立行解释为空状态。

精确已记录的 floor 或覆盖旧请求的较新 floor 可以确认丢回执重试，不再查询原
store。这只确认持久的历史事实，不宣称该 store 当前在线，也不重新签发读租约。
重试仍复核 metadata 快照中的事实，不能把其他 store、creation 或 group 的记录
借用到本次请求。此操作不改变任何资源 pin。

## worker 与验证边界

DATA checkpoint worker 完成真实镜像采用并推进本地下限后，分别报告 selection
和独立 floor。两项确认都成功才完成本地报告；丢失 floor 回执沿原报告重试，不
重复对象 GET。进程重启仍按原 checkpoint 恢复与采用流程重新建立报告状态。

新增测试覆盖请求规范编码、普通 planner 伪造拒绝、与 selection 的独立性、
单调更新、pin 保持、V20 升级/降级孤立行拒绝以及实际 WAL 重开。RPC 测试使用
真实 StoreGuard/provider，覆盖鉴权、scope/challenge/预算关联、坏文件、Unknown、
超时后的实际槽位保持及读取期间 authority 撤回。

新增三副本测试使用真实 raft-rs、磁盘 WAL/Raft、路由 ledger 和鉴权 HTTP/2
duplex：只有 selection 时拒绝确认；实际 worker 推进 floor 后仍必须得到独立
回复；Unknown、不支持、endpoint generation 改变不产生事实。另一个用例在
提交后丢 floor 回执，验证无重复 GET、metadata 换主后重试、移除 DATA handles
后的实际持久查询和停止全部 owner 后的 WAL 重开；Version pin 始终保留。

最终 10 项检查通过：529 个不同 Rust 测试通过，1 个既有测试忽略，其中新增
7 个测试；runtime 97 个用例使用四个测试线程全部通过。严格 workspace Clippy、
格式和 diff 检查通过。负向对照只绕过独立 floor 相等检查，随即错误接受只有
selection 的请求，新测试按预期失败；原检查恢复后完成上述最终验证。开发中的
类型修正和 WAL 测试初始化失败日志保留，不计入通过数。

本轮保持 store 生命周期及内部服务 **V10**。服务新增方法不提供旧路径 fallback；
旧 V10 服务若不支持方法只能得到未知结果。没有新增旧二进制混部、TCP/MinIO、
进程强杀、断电、长期资源稳态或形式化联合验收。此前八个保护脚本/冻结夹具及
封存证据保持原字节。

仍须完成完整 DATA 历史成员、迁移/移除/joint configuration、store 替换与所有
恢复来源盘点；远端确认当前在线 store 不能代替这项盘点。还须补齐 DATA 镜像
未来 Reader/Publisher 入口关闭、实际全体终态、portable recovery anchor、未知
PUT 终止资格、负向 Pending 与旧 Version 释放、物理 GC，以及历史空间有界和
完整第二阶段验收。一条最新记录的固定大小不等于 store/group 历史总量有界。

原始日志、开发失败、负向对照、增量 patch 和源码摘要保存在
[本轮证据目录](raw-ttl-data-fence-observation-validation-20261005/)。
