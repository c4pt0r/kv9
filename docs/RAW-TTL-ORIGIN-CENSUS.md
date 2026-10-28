# 精确配置来源的持久盘点与后台轮转

日期：2026-10-06。接续[配置来源分页查询](RAW-TTL-ORIGIN-QUERY.md)。现在已查询的
配置来源和分页进度保存在 metadata Raft，后台按来源续做。完成某个来源的已知记录
枚举，不证明初始/旧配置区间、snapshot 祖先或所有恢复来源完整；第二阶段继续开放。

## 持久记录与原始来源

retention header 从 `KV9LED23` 升为 `KV9LED24`，V11 store/service floor 不变。
新请求 tag 39 只包含精确 region/creation/node/store、placement task、预期 sequence
及 task revision，不接受调用者提交的配置页。旧 header 可显式 Initialize 升级并
保留原成员记录；旧 header 下存在新 namespace 则拒绝，不能补造空的新盘点。
这不构成旧 binary 混部或保持服务的滚动升级资格。

runtime 在 catalog 锁外通过已有鉴权 RPC 查询真实来源，取得完整 endpoint generation
和页关联。取得 catalog 锁并排空同一 metadata term 后，再次核对整个 endpoint，
按当前任务修订确认来源，然后逐条独立核对原始 task 的 operation、node/store、
动作、creation、范围和规范行摘要。普通 ledger planner 不能直接接受新请求。

namespace 54 保存来源进度，namespace 55 按来源及 Raft index 保存每条记录。
记录包括原始 term/index、完整配置来源、连续序号、首次/最近观察位置和轮次。
同一 index 的已提交来源不可替换；新记录只能追加到已保留前缀之后。当前 node
目录中的新 store 不会替代旧记录里的 store；同一 node 的两个 store 分别计数。
源配置位置或已知记录数量回退、重排或跳过前缀都会拒绝。

每个提交最多处理四条来源。进度规范记录最多 1206 字节，单条记录 353 字节；查询
返回四行并最多读取第五行作为续页依据，同时核对连续序号与末页总数。查询游标绑定
精确来源和进度 sequence，记录变化后不能沿用旧查询游标。历史记录总量和常驻内存
仍未有界；这些分页上限不是整机内存/CPU/磁盘稳态证明。

## 恢复、重试和调度

进度、实际来源页和已知缺口在同一批 metadata Raft 写入中提交。旧 context、未知
初始配置和 snapshot 前缀等不确定性一旦观察到便持久保留，后续正常页不能清除。
`round_complete` 只表示一轮已知记录枚举完毕；`has_known_gaps` 没有已知缺口也不等于
全体历史来源已覆盖。查询单独返回 placement revision 是否仍有效，不声称来源实时新鲜。

丢回执后，完全相同的最近一次请求可从持久摘要确认；来源失联、catalog 后续变化和
metadata leader 切换不改变旧回执，也不刷新来源事实。task revision 或 endpoint
generation 变化时从首屏重新核对；源历史变化使游标失效时，runtime 在原总 deadline
内最多再独立查询一次首屏。RPC 错误和 Unknown 都不能作为空页或完成状态。

现有 checkpoint sweep 在 placement 盘点完成后交替执行成员盘点和精确来源盘点，
每次只选择一个不同来源。两类使用独立调度游标，即使来源拒绝也推进游标，避免一个
失联 store 阻塞其他来源或另一类盘点。分页进度持久化，内存游标只决定访问次序。
换主或重启可以从头调度，并沿每个来源的持久进度继续。

复用原维护名额、请求期限与实际 RPC worker 容量，没有新增对象 I/O。取消/超时仍
等待真实 worker 退出才归还容量。新增累计页面调用成功/失败/回绕/已见完整轮次/缺口
统计不表示全局完成百分比；稳定首屏确认可以不新增持久事实。

## 验证与剩余范围

独立命令、负向对照、源码摘要和补丁记录在
[本轮证据包](raw-ttl-origin-census-validation-20261006/manifest.json)。新增七项 metadata 测试
覆盖规范请求、独立 task 核对、WAL 重开续页、旧回执、前缀不可改写、修订/endpoint
变化、缺口保留、同 node 不同 store、V23 升级与孤儿记录拒绝，以及 13/113 组
placement catalog 下相同的单页读取成本。保留记录查询实际最多读五行。

两项新 runtime 测试使用实际三副本 raft-rs、磁盘 WAL/Raft、鉴权 HTTP/2 duplex 和
InProcHub。生产 attach/detach 产生六条来源，页提交丢回执后换 metadata leader，
拒绝来源查询仍可确认旧记录；未注释的新配置使旧游标失效后从头核对并持久保留缺口。
三份 metadata WAL 重开后的进度和精确来源一致。后台测试验证 endpoint 变化拒绝、
Unknown 来源不阻塞其他 store、成员/来源交替推进及同一维护并发上限。

负向对照只移除持久 planner 的原 task 核对，新测试即在不存在的 task 上失败；
原始代码已恢复。真实 TCP/MinIO、完整 NodeRuntime、进程 kill/断电、旧 binary、
跨主机、Chaos 和新增完整形式化验收不在本轮验证范围。

最终相关回归去重 **510 项通过，1 项原有真实 MinIO 测试忽略**；包含完整 metadata
347 项、四线程 TTL runtime 111 项、Raft 来源/RPC/格式 30 项和文档 22 项。
首次严格 Clippy 报告分页条件可简化，改为等价显式分支后重跑九项新增测试通过，
严格 workspace 全 target Clippy（tracing/checkpoint-testing）、fmt 和 diff 均通过。
完整 runtime 未因该等价查询条件改写而重复执行；改写前后的源码清单与失败日志保留。

下一步仍需建立初始配置与完整旧配置区间的精确 store 关联，处理 snapshot 祖先和
不可达历史磁盘，覆盖所有安装/迁移/备份/恢复来源及可转移恢复锚点。本轮盘点没有
授权 DATA 关闭、旧 Version/负向 Pending 释放或对象 GC。Reader/Publisher/未知 PUT
生命周期、历史/CPU/RSS/磁盘有界、merge、部署时钟和升级/故障/生产验收继续按
[完整第二阶段](RAW-TTL-PHASE2.md)和[生产范围](RAW-KV-PRODUCTION-SCOPE.md)推进。
