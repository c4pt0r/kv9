# 逐次实际配置历史与有界远端查询

日期：2026-10-06。接续[初始配置对应](RAW-TTL-INITIAL-ORIGIN.md)。原有成员盘点只
保留累计节点角色，来源盘点只保留带新 context 的变更；两者都不足以重建配置区间。
本组件从实际磁盘记录保留初始配置、每次已 apply 的配置和 snapshot 边界，并提供
有界、鉴权的来源查询。这些是区间核对的输入，尚未成为完整区间/store 对应证明；
[第二阶段](RAW-TTL-PHASE2.md)继续开放。

## 保留的事实

配置记录按本地观察顺序编号，类型包括 Initial、Applied(index)、Snapshot(term/index)。
每条保存 voters、outgoing voters、learners、learners_next 和 auto_leave。已知
kind 9 来源与该次实际 ConfState 同时保留；旧 kind 4 没有来源时明确返回 None，
不从当前节点注册推测旧磁盘，也不补造已被压缩的旧配置 term。

状态先规范排序，拒绝重复/零节点、非法角色重叠和未知字段。每类成员最多 1024 个。
不支持或超大的状态保留为 None，并带未知标记；不会把缺失字段截短成一个完整配置，
也不会因为盘点无法表示旧状态而拒绝原本可 apply 的共识记录。

初始记录继续与 snapshot 区分。snapshot 保存当时选中的成员，但所有先前本地
观察仍保留，导入祖先的缺口保持未知。重复、乱序或无法定位的记录保留歧义标记；
相邻完全相同的 indexed apply 只占一个观察。已提交、尚未 apply 的配置不提前出现。

现有 fsync 成功后才发布内存观察；失败 writer 不提供查询。重开从原有磁盘记录
重建同样的历史。历史链摘要纳入成员指纹，物理日志重写必须保留相同观察；不能
因为新选中的 compaction base 看起来正常就丢弃早期状态。没有新增日志 kind、
磁盘格式 floor 或 retention namespace；V11 和 ledger25 不变。

## 分页与独立核对

新增内部 InspectConfigurationTimeline RPC，协议版本 1；旧服务不支持时拒绝，
不降级到只有累计节点角色或部分变更来源的查询。KV9CTP01 每页最多四条，144 KiB
上限；KV9CTR01 单条编码上限 33,024 字节，KV9CTC01 游标 48 字节。游标绑定完整
历史指纹与连续观察序号，变化后必须重新开始。页和记录解码在分配前检查长度与数量，
拒绝跳号、伪造结束、错误 scope 和被去掉的 snapshot/未知标记。

RaftPeer 在原互斥边界内读取实际持有的 DiskRaftStorage。Runtime 复核真实 group
owner、root/creation 和 range/import scope，不能打开另一个同名文件来提供观察。
远端请求绑定 exact store、随机 challenge 与原始期限，鉴权入口在读取前后都
确认 store 的当前执行资格。与已有观察共享实际 worker 名额；超时/取消不能提前
归还仍在工作的读取容量。

metadata leader 通过 ReadIndex 和目录快照选中精确 endpoint，收到回复后再次
确认完整 endpoint generation。每条已知来源独立验证原始不可变 task、operation、
node/store 和创建身份。没有来源的旧配置仍可作为描述性观察返回，但不能因此
取得它没有的 store 证明。Rust 入口为 NodeRuntime::inspect_configuration_timeline。

查询只访问四条缓存记录，不扫描整个历史。10/10,000 次配置下的查询 scratch
测试，以及每类 1024 成员的 joint 大页测试覆盖此局部预算。缓存历史总量、恢复
总成本、维护发现、公平性和整机 CPU/RSS/磁盘稳态尚未有界或验收。

## 验证与后续

[验证包](raw-ttl-configuration-timeline-validation-20261006/manifest.json)保存命令、日志、
源码与负向对照。新增六项存储、五项 RPC、两项实际三副本 runtime 测试：

- 真实磁盘上的 initial、来源配置、旧 joint 配置、snapshot 边界和后续配置，跨页
  及重开保持一致；另一场景验证 compact/rewrite 不改变历史与指纹。
- 写入/fsync 故障与崩溃恢复后，完整配置和来源一起可见或一起不存在；失败 writer
  拒绝查询。未知字段、乱序记录、游标过期、编码损坏和成员上限有覆盖。
- 鉴权、query 每个字段关联、未知来源、畸形回复、旧 V11 方法缺失、超时后的实际
  容量保持、取消，以及读取期间身份撤销均拒绝不完整确认。
- 三副本真实 attach/detach 产生六次变化，远端返回初始记录与逐次 learner 集合；
  endpoint 变化和替换磁盘拒绝，新增旧 context learner 使游标失效。三个实际
  data Raft 存储重开后逐页比较完整历史。
- 带不存在目录 task 的已 apply 配置不能成为确认来源。负向对照移除独立 task
  核对后该测试失败，原代码在最终验证前恢复。

首次开发编译发现分页方法对同级存储模块不可见，已限定为 crate::storage 可见并
保留失败输出。相关回归共 601 项不同测试通过，含新增 13 项；原有一项真实 MinIO
测试忽略。完整 metadata、Raft storage、TTL runtime 和相关 RPC/doc 测试通过后，
严格 Clippy 指出一个多余的 unit 表达式；仅删除该表达式，重跑 11 项 timeline 测试、
全 workspace 全 target 严格 Clippy（含 write-stage-tracing/checkpoint-testing）、
格式和 diff 检查通过。两次源码清单及失败日志均保留，未重复计算通过的测试。
测试使用真实 Raft/WAL、认证 HTTP/2 duplex 和 InProcHub；迁移目标
仍是已有的离线 fixture guard，不是第四个运行副本。本轮没有新增 TCP、MinIO、
完整 NodeRuntime、kill/断电、旧 binary、跨主机、Chaos 或形式化验收。

下一步需要把逐次实际配置和进度持久盘点到 metadata，再与已核对的初始 store、
逐次变更来源对齐，形成带明确缺口的历史区间；不能把“拿到一页”当作全历史完整。
所有实际旧磁盘、joint/旧 context、snapshot 祖先及安装/迁移/备份/恢复来源、可转移
恢复锚点仍需覆盖。本组件不授权 DATA 关闭、旧 Version/负向 Pending 释放或对象
GC。Reader/Publisher/未知 PUT 生命周期、安全回收、历史及资源稳态、merge、时钟
资格和完整生产验收继续按原范围推进。
