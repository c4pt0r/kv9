# 本地初始配置与原始建组 store 的持久对应

日期：2026-10-06。接续[来源持久盘点](RAW-TTL-ORIGIN-CENSUS.md)。实际来源查询
现在携带本地磁盘记录的初始成员集合；metadata 独立核对原始建组任务，并把对应的
精确 node/store 集合与分页进度原子保存。本组件不证明所有历史配置区间或所有
恢复来源已经覆盖，[完整第二阶段](RAW-TTL-PHASE2.md)继续开放。

## 观察与核对

DiskRaftStorage 从原有初始 ConfState 记录取得本地初始 voters，保留在成员历史中。
只接受 3/5/7 个唯一非零 voters，不接受 learner、joint、未知 protobuf 字段或
其他不支持的初始结构。缺失、重复/顺序有歧义和不支持的记录分别返回明确状态。
观察保留在日志重写与重放中，并参与历史指纹；旧游标不能跨观察变化继续。

snapshot 中的当前配置不能补造初始记录。本地曾观察到的初始集合在 snapshot 安装
后仍保留，同时 snapshot 祖先缺口也保留。未见初始记录的历史，或仅有不支持的
空初始配置的安装存储，不因为后来收到合法 snapshot 而变为已知初始来源。

内部 InspectConfigurationOrigins 请求版本改为 2，响应使用 KV9OPG02，在原有
每页四条配置变更来源之后追加最多 58 字节初始观察；整页最多 929 字节。V1 请求
被拒绝，V2 客户端也拒绝旧 KV9OPG01 回复。旧页只保留用于读取已经持久化的历史，
不会因解码成功成为新协议的确认。没有新增 RPC、Raft 日志种类或公开 KV API。

metadata 持久 planner 复核精确 placement 的原始任务，沿创建、迁移、安装证据、
移除或 abort 的不可变任务引用找到建组任务。比较磁盘实际初始 voters 与原始建组
集合，并要求提供观察的 source node/store 本身属于该原始集合；同一节点的替换
磁盘或迁移导入存储不能仅凭相同节点列表冒充 native source。保存的 store 来自
原始建组行，不读取当前 endpoint 来替换其他初始成员的旧磁盘。

## 持久化、升级与边界

V11 store/service floor 不变，retention header 从 KV9LED24 升到 KV9LED25。
复用 tag 39，新增 namespace 56：每个 exact source 一条不可改写的 KV9DOI01，
保存观察来源、首次 ledger 位置、原创建 task、规范创建行摘要和精确副本集合。
最多七个副本，记录最多 353 字节；来源进度的解码上限为 1264 字节。

映射与普通来源页/进度在同一 metadata Raft batch 中发布。丢回执可确认原提交；
后续页不重写首次观察位置。读取与重试检查映射和进度的对应关系；已知初始进度
缺少映射、孤儿映射或记录位置超前均拒绝，不能用重复请求默默补齐丢失记录。

旧 V24 可显式 Initialize，原页保持原字节；旧页缺少初始观察时保持未知，直到
新协议取得真实观察并完成核对。旧 header 下发现 namespace 56 拒绝升级。
实际观察到 Missing/Ambiguous/Unsupported 的来源缺口持久保留，后续正常观察不能
清除该历史缺口。这是格式与组件约束，不是旧 binary 混部或滚动升级验收。

后台复用已有来源盘点与共享 worker，无新增扫描所有任务的步骤或对象 I/O。
初始任务链只做有界点查；13/113 组 catalog 下的预算回归也覆盖新初始核对。
每页有界不等于全体历史、常驻索引或整机 CPU/RSS/磁盘已经有界。

## 验证

命令、实际输出、故障对照与源码补丁保存在
[验证包](raw-ttl-initial-origin-validation-20261006/manifest.json)。新增七项 metadata
测试、三项存储测试和一项 RPC 测试。覆盖 V24 旧页保留、错误初始集合、导入磁盘
拒绝、当前 node 注册替换、七副本编码、孤儿/缺失映射和未知历史保留；实际磁盘
测试覆盖 joint 配置、日志重写、snapshot 安装和重开。旧配置来源测试进一步核对
四种变更任务均能追溯到同一原始建组记录。

两项既有三副本 runtime 回归新增初始映射断言：真实 attach/detach 后，首次页提交
丢回执、metadata 换主、来源暂时拒绝、源历史变化，以及三个 metadata WAL 重开，
均保留同一初始映射。后台盘点为三个实际原始 source 各自登记；Unknown 来源恢复
后继续。使用实际 Raft/WAL、认证 HTTP/2 duplex 和 InProcHub，没有新增真实 TCP、
MinIO、完整 NodeRuntime、进程 kill、旧 binary、跨主机、Chaos 或形式化验收。

负向对照去掉精确初始成员比较后，合法但不同的三节点集合测试失败；原代码在
正式回归前恢复。首次开发编译暴露测试误用仅 OS 存储支持的日志重写接口，已改为
真实磁盘测试，失败输出保留。

最终相关回归去重 **588 项通过，1 项原有真实 MinIO 测试忽略**。包含完整 metadata
354 项、Raft storage 77 项、来源/成员/格式 RPC 相关套件、四线程 TTL runtime
111 项及 Raft/server 文档测试；套件重叠按测试全名去重。严格 workspace 全 target
Clippy（write-stage-tracing/checkpoint-testing）、fmt 和 diff 检查通过。验证期间代码
摘要保持不变；上述结果不代表完整 workspace 的网络测试或生产环境验收。

后续仍需完整旧/joint 配置区间、全体实际历史磁盘与 snapshot 祖先、安装/迁移/
备份/恢复来源和可转移恢复锚点。此记录不授予 DATA 关闭、旧 Version 或负向
Pending 释放、对象 GC 权限。Reader/Publisher/未知 PUT 生命周期、安全物理回收、
历史和资源稳态、merge、时钟资格及完整生产验收仍在原范围内。
