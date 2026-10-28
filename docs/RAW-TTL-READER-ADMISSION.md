# Reader 来源准入、逐代记录与负向 checkpoint 读关闭

2026-10-05，接续 [Reader 目录索引](RAW-TTL-READER-INDEX.md)。此前通用 `Share`
只核对来源的资源集合和 subject，Reader 的 operation、完整范围和协议身份没有与
来源绑定；已提交 checkpoint 拒绝事实也没有阻止 Reader 的新登记或 Publish。本轮
将来源校验和逐代记录接入实际读流程，并关闭这个读准入缺口。

## 精确来源与两步准入

进程 Reader 必须通过 `Share` 指定一个精确的 Published 来源；直接 `Acquire Reader`
拒绝。metadata 从来源完整 binding、固定 checkpoint/image 协议 domain 和
node/store/process lifetime 重新派生 Reader，逐项核对 generation、operation、范围、
subject 与全部资源。不能靠更改 operation、ID、范围或借用另一类来源绕过检查。

有效 Share 原子保存 Held owner、不可变注册、resource links、反向索引和该代来源。
Publish 在同一 metadata 观察中重新检查原来源仍 Published，以及该 operation 没有
已提交的负向拒绝事实，再原子更新 pin 和该代发布位置。重复 Share/Publish 仍必须
通过当前准入检查；成功重试本身不能绕过后来提交的拒绝。

runtime、adoption 和离线安装继续使用既有两步调用，只有 Publish 得到成功回执后才
开始 GET。请求编码、Reader ID、outbox 格式及本地实际排空顺序不变。

## 拒绝事实与正在执行的读

新 Share 和 Publish 都从来源 operation 查找规范 Pending 的拒绝事实，校验 intent、
关闭 gate、revision 和完整拒绝记录。存在合法拒绝时不再授予 I/O；拒绝记录或依赖损坏
也不能开启准入。这里只将精确拒绝作为负向读关闭：正向交接同样会关闭 Build gate，
因此单独的 Closed gate 不阻止已发布 Version 的正常恢复读。

已得到成功 Publish 的本地 scope 可以继续完成；读关闭不撤销已运行的 GET，也不等于
实际排空。原进程取得真实 `DrainedRead` 后，仍通过完整 Published 来源覆盖路径提交
结束报告。Held、未提交的 Share 和已释放但回执丢失的情况仍可结束或确认。负向
Pending 本身继续 Published，保持这些首次结束的完整来源保护。

## 持久代际事实

ledger 升为 `KV9LED10`。新增 tag 25 的 `KV9RDG01` 记录，单行 178 字节，绑定
root、Reader token、source token/kind、完整不可变 binding 摘要，以及 Share、Publish、
finish 三个提交 revision。每代独立保存，不用当前 generation 覆盖早期记录。

记录区分三种 origin：

- `Entered`：该代通过本协议 Share，明确记录进入时刻；可以尚未 Publish。
- `Unentered`：该代没有登记成功，结束请求先为它建立终态，阻止迟到的准入请求。
- `Legacy`：旧目录中的活跃 Reader 在升级后结束。记录不补造它的早期准入或发布事实。

受管理 Reader 使用 `KV9RDO01` owner/registration 编码，读取时必须与该代记录、阶段、
反向索引和资源交叉引用一致。缺失 generation 行不能降级成旧 Reader。其通用
Quiesce/Release 路径拒绝，必须提交原协议的排空结束报告；旧格式 Reader 的一般转移
路径不因此产生受管理准入或实际排空证明。

完成后一代只能在前一代 Released 后登记。已有终态回执继续只依赖 Reader 自身事实，
不重新要求旧来源在线或 Published。历史点查询缺少某代时返回未知；调用者不能跳过
空洞，不能从一个 Released owner 推断所有历史代都合格。

## 升级与验证边界

01–09 的既有读取和检查后升级路径保留；09 的 Reader 索引、08 的 Publisher 历史及
未知 PUT 原样保留。旧活跃 Reader 不允许通过新 Share/Publish 补领 I/O 权限，需要先
结束原代。已经 Released 的旧记录仅确认既有结果，不自动升级成具备来源证明的历史。
新代际记录出现在旧 header 下时拒绝；旧 planner 无法读取 10。

store/service floor 仍为 V8，本轮没有证明混合旧二进制部署可用，也没有把旧的
checkpoint birth 追认为全部 Reader 都受此协议约束。单次登记和点查询成本受单 owner
资源上限约束，代际历史总量仍需后续稳态回收协议。

最终八项检查全部通过：372 项测试通过，2 项真实 MinIO 测试忽略，新增 8 项测试。
完整 metadata、TTL runtime、bootstrap、outbox 与 checkpoint 恢复回归，以及 workspace
严格 Clippy、格式和差异检查通过。

验证涵盖精确来源、准入重试、负向拒绝与在途读、正向 Version 恢复、旧记录转换、
历史空洞、损坏与降级拒绝，以及 Share/Publish/finish 的真实 WAL 尾部截断。
三 metadata Raft 副本和鉴权 HTTP/2 duplex 组合测试暂停真实对象任务：GET 超时后仍
保持 Reader Published；提交拒绝并丢失回执、切换 metadata leader 后，新读被拒绝；
实际任务退出后原 Reader 才能结束，丢失的 finish 回执通过持久 outbox 重放确认。
全部停止后，真实 WAL 重开继续保留拒绝、每代来源/排空记录和 Published Pending。

详细命令、开发失败与最终结果见
[验证包](raw-ttl-reader-admission-validation-20261005/manifest.json)。旧 Reader 盘点测试
显式构造旧 owner 和缺少代际记录的字节夹具；它不是生产降级流程，也不会被当作历史
准入证明。

## 完整阶段仍开放

这提供了新读准入和单代生命周期事实。全体 Reader 的连续代际闭合、旧/外部读者、
BootstrapReader、Publisher 与恢复引用的完整盘点、部署来源资格、负向 Pending 与
不同镜像间 Version 替代、未知 PUT 隔离、终态历史压缩和物理 GC 仍需完成。
[第二阶段](RAW-TTL-PHASE2.md) 的完整生产目标和验收范围保持不变。
