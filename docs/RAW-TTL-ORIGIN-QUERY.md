# 精确配置来源的分页与跨节点核对

日期：2026-10-06。接续[实际配置来源持久化](RAW-TTL-CONFIGURATION-ORIGIN.md)。
现在 metadata leader 可向一个精确历史 store 查询已 apply 的配置来源，并逐条核对
原始 catalog task。查询是只读观察；完整历史来源、持久盘点、DATA 关闭及 GC 仍开放。

## 本地分页和不确定性

每页至多四条 kind 9 已知记录，保留 term/index、root/creation/region、task、
operation、node/store 和原始行摘要。普通 kind 4 表示该来源没有精确 context；
`legacy_contexts_unknown` 一旦观察到便保留。旧记录不会被当前 endpoint 或新记录
补成已知，页尾只表示已知记录枚举结束，不证明历史 store 全部覆盖。

本地维护记录摘要链与实际插入序号。56 字节游标绑定摘要、最后 index 和已访问数；
返回下一页前做实际 index→序号核对，不能伪造跳过的条数。页摘要还绑定成员历史
摘要和旧 context 缺口，因此新配置、角色变化、snapshot 和新来源都会使旧游标失效。
查询只做树定位和四项复制，不遍历全部历史来重新算摘要或 rank。

页面编码 `KV9OPG01` 最多 871 字节，严格检查数量、顺序、位置、续页与 ancestry
缺口；scope 在 RPC 两端独立核对。日志重写核对已知来源与旧 context 缺口均保留；
snapshot 保留本地已知来源，导入前缀仍未知。数量与常驻内存尚未有界，单页 scratch
有界不代表历史总量或整机 RSS 有界。

## 只读内部协议和 runtime

新增独立 V11 方法 `InspectConfigurationOrigins`，请求/响应上限 2048 字节，
单次远端观察最多两秒。沿用真实 node 认证、store 格式 fence、完整 root/cluster/
node/store/region/creation/challenge scope 和完整请求回显；旧服务缺方法时拒绝，
不降级调用成员、选择记录或 read floor 方法。V11 store/service floor、ledger23
以及上一增量的磁盘 kind 9 格式不变。

提供者只读取实际持有的 group driver 和 RANGE/IMPORT engine；前后重检 serving、
ownership 和创建身份，不重新打开运行中的 Raft 文件。没有本地来源时返回 Unknown，
Unknown 不能作为空页或完成盘点。读取复用现有内部观察 worker 配额；超时和取消
不提前释放仍在运行的任务名额，最终响应前再次确认 store 接收资格。

`NodeRuntime.inspect_configuration_origins` 在派发前取得原有 retention 配额，原请求
期限最多 30 秒并覆盖所有阶段。metadata leader 先 ReadIndex，再验证请求指定的
历史 store 与当前 endpoint 精确匹配；远端返回后再 ReadIndex 并重检整个 endpoint
及代际。对每条来源，从同一 metadata view 读取并验证原 task/creation/迁移/证据链、
operation、动作、node/store 及完整行摘要。远端已 apply 不替代这些独立来源检查。

## 验证和后续

独立日志、源码摘要和负向对照见[本轮证据包](raw-ttl-origin-query-validation-20261006/manifest.json)。
新增 10 项测试；相关回归去重 **352 项通过，所选测试无忽略**，其中四线程 TTL runtime
109 项通过。严格 workspace Clippy（全部 targets，tracing/checkpoint-testing）、格式
和 diff 检查通过。这是所选组件回归，不是全量 workspace 测试或真实部署验收。
新增测试覆盖三页顺序、伪造 rank、格式损坏、新配置使游标失效、重写与 snapshot 后
缺口保持、10 与 10,000 条历史的单页临时分配、请求全部字段回显、权限撤回，以及
超时/取消后的实际容量占用。旧 V11 dispatcher 和新服务使用真实 HTTP/2 duplex，
没有使用真实 TCP。

三副本 runtime 测试通过生产 attach/detach 生成六条来源，由分离的 metadata leader
查询第三副本；验证不同 store 拒绝、endpoint 代际变化拒绝、Unknown 和旧游标拒绝。
另一个用例向测试 harness 注入已 apply、但没有对应 catalog task 的来源，独立查询
必须拒绝。负向对照只跳过 catalog 核对，这一断言即失败；临时修改已恢复。

下一步把此入口接入后台有界轮转和持久来源盘点。初始配置依据、完整旧配置区间、
snapshot 祖先及所有恢复来源仍须核对；当前节点列表和本轮已知来源页都不能作为
全体历史 store 的证明。完整恢复锚点、生命周期释放、GC、历史/CPU/RSS/磁盘稳态、
merge、真实网络/MinIO/进程故障、部署时钟、升级及形式化验收仍未完成。
