# Metadata 副本的持久选择记录

2026-10-05，接续 [bootstrap 目录](RAW-TTL-BOOTSTRAP-INDEX.md)。之前的副本选择记录只
覆盖 data group，metadata 的本地恢复选择缺少经 leader 独立核实的 catalog 记录。本轮
把实际 metadata selector 和对应的持久 bootstrap grant 接入既有选择上报协议，作为
后续恢复依赖判定的输入。

## 首次记录的证据

`RecordCheckpointSelection` 仍使用 request tag 19。metadata leader 先取得 ReadIndex，
检查精确 Pending、Published Version、source node、store incarnation 和当前 endpoint，
再通过既有鉴权 RPC 查询源节点。源节点读取实际 metadata engine 与 metadata driver，
核对选中镜像、指定发布窗口的 canonical manifest、已提交配置历史，以及该镜像在本地
bootstrap receipt 中的 active、authorized 槽位。缺少任何一项都不能建立新记录。

本地 receipt 必须绑定同一 root/store、完整镜像和原始 predecessor；带 Pending 的新
receipt 还须精确匹配 Pending。旧 V1 已授权 receipt 保留原始发布窗口，不补造 typed
admission。相同镜像在后续窗口重发时，也不能把原 grant 解释成新窗口的授权。

返回前重新读取实际 selector 和本地授权；leader 在 catalog 锁内重检 endpoint。catalog
核对精确 Published bootstrap owner、完整 binding、registration、资源 pin、索引及已有
admission 来源，才写入新选择记录。普通请求 planner 不能自行制造独立观察结果。目前
metadata 源限定为 root 中已 provision 的 voter；该功能没有认证 store replacement。

RPC 沿用有界请求/响应、root/node/store/region/window/challenge 绑定、实际任务槽位和
超时后排空规则。metadata 与 data 的回复不能互换；未选择、无授权、源拒绝、未知回复、
历史不健康或 endpoint 在查询期间变化，均不建立新事实。

## 历史记录和重试

每个 `(node, store incarnation, metadata region)` 只保存最新一条记录。精确重试或被
更高发布窗口、镜像位置及不回退 epoch/term 覆盖的旧请求，可由已持久化事实确认，无须
原源节点继续在线。相同位置的冲突镜像不能推进记录，旧重试也不能覆盖新记录。

bootstrap grant 可能已在本机切换镜像后独立退休，而下一次选择报告尚未确认。因此读
历史事实允许其精确 grant 为 Published 或 Released；首次建立新事实仍要求 Published。
这项区别不构成新的读授权。grant 自身交叉引用、intent 和 Published Version 仍须保留，
没有开启历史删除或 Version 释放。

metadata worker 每轮给实际 execution 结束、bootstrap 退休和选择上报各一次执行机会；
未确认的报告阻止开始新的候选工作。只在确认后缓存一个 Pending token。丢回执时从
本地 selector 和持久 receipt 重建同一请求，无额外对象 GET/PUT、逐 attempt 常驻集合
或新增后台任务。缓存丢失也走同一路径。尚未登记的旧 selector 沿用已有 bootstrap
登记路径，不把缺失 receipt 当作已授权。

## 格式与验证

ledger header 升为 `KV9LED14`。metadata 使用 key tag 31、`KV9CMS01`、固定 192 字节，
额外保存精确 bootstrap owner ID 与 generation。data tag 21、`KV9CSR01`、168 字节
保持原格式。13 的 bootstrap index 与既有 admission 原样升级；旧 header 下出现新
metadata 记录会拒绝，不尝试修复。store/service floor 仍为 V8，未认证混合旧二进制部署。

新增九项测试：五项 metadata、两项真实三 metadata Raft 组合、一项本地 receipt、一项
RPC。覆盖首次证据缺失与独立核验、13 格式升级保留原记录、旧 grant 退休后的历史读取、
单调推进和旧重试、损坏/错代 grant、真实 WAL 尾部截断，以及 selector/receipt 的窗口
关联。三节点测试还覆盖鉴权 HTTP/2 duplex、endpoint 变化、丢失已提交回执、leader
切换、缓存丢失后的报告重建、GET/PUT 计数不增长、其他 store 不能借用事实，以及所有
service/driver 停止后的 WAL 重开。

缓存丢失测试保留同一活跃节点的唯一 manifest seam；它不代表完整进程重启。初始测试
误尝试在同一活跃节点创建第二个 proposer，被既有保护拒绝；已改用现有 worker 测试
入口。升级测试的初始 fixture 误查询尚未完成的全局 bootstrap 目录，已改为核对实际
单 owner 和原始持久记录。开发失败日志均保留。

最终九项检查通过：411 个不同测试通过，2 项真实 MinIO 测试忽略。完整 metadata、
TTL runtime、bootstrap、drain outbox、checkpoint 恢复、选择 RPC、workspace 严格
Clippy、格式及差异检查均通过。检查结果与源码摘要见
[验证包](raw-ttl-metadata-selection-validation-20261005/manifest.json)。

## 尚未关闭的范围

这里记录的是逐 store 的历史选择下界，不是当前租约、全体副本 census、可搬迁恢复锚点
或释放许可。没有关闭未来 Reader/Bootstrap 准入，也没有汇总全体恢复依赖、外部/旧协议
部署来源、Reader/Publisher 连续历史。负向 Pending、旧 Version、未知 PUT 终止证明、
历史总量有界、物理 GC、时钟及完整生产部署/形式化验收仍属于
[第二阶段完整范围](RAW-TTL-PHASE2.md)。
