# Metadata 镜像上传准入与逐代盘点

2026-10-05；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
接续 [上传者终态回执](RAW-TTL-TERMINAL-PUBLISHER-ACK.md)。同一 metadata 镜像可以
拥有多个 Pending attempt。原有镜像读 gate 拒绝新 Reader，但另一 Pending 的
checkpoint Build 仍可进入上传。本轮增加独立的写 gate 和跨 attempt 的上传者盘点。

## 准入与历史

只有完整镜像的 all-store 读 gate 已关闭，才能提交 `CloseMetadataImageWrites`。
新的不可变记录绑定 root、完整 manifest digest、读 gate 来源 Pending、原读 gate
字节摘要及两次关闭 revision，并与摘要 marker 在同一批提交。旧 ledger19 的读 gate
本身不被解释为上传关闭。写 gate 对同一镜像已有和以后创建的 Pending 别名生效。

Build 的新 Acquire/Share、Held→Published、Published entry 重试和较新 generation
都检查写 gate。关闭不撤销已经进入的作用域；它们继续提交实际结束报告。未进入
作用域的零 PUT 终态也可登记。迁移 Publisher 的 typed entry 同样接入检查；现有
迁移协议的 DATA 图像不因此获得 metadata 关闭资格。新镜像、不同 region 和 anchor
身份保持独立。

反向目录按 `(region, image, owner)` 收录非 anchor Build 和迁移 Publisher。新 owner、
取消或结束路径与索引在同一事务写入；旧 owner rows 与不可变 registrations 分两轮
回填，每页最多四行加一行 lookahead。第二轮防止单独丢失 owner 被误认为不存在。
完成目录后，缺少索引的 owner 变更拒绝自动修复。

只有写 gate 关闭且全目录完成后，才允许 metadata 镜像 census。每次最多处理一个
owner 的四个连续 generation，缺失代际也占一项。Build 使用保留的 intent、gate、
origin 和 drain 历史；没有合格来源的 legacy Build 一律记为未知。已进入 generation
的 admission revision 必须早于镜像写 gate，不能以关闭后的登记补造旧上传来源。

汇总保留 owner/generation 数、零 PUT、缺来源、活跃、缺终态、非零 PUT 及 unresolved
PUT 数。后续零报告不覆盖旧代未知上传；即使 issued 全部 resolved，非零 PUT 也不
成为零 PUT 证明。此汇总仅描述登记历史，不能授权删除远端对象。

metadata 镜像的索引数量、变化 stamp 与 lifecycle 写入原子提交。census 结束核对
owner 数；丢失索引不能变成“空集合已关闭”，丢失 stamp 不能由后续变更重建为零。
迟到 Finish、Record 或未进入终态使先前进度失效并从头扫描；精确终态重试不改写
stamp。旧 snapshot 的观察保持一致。同镜像来源保护转移后仍可读取保留的 Build
历史，不要求原 Pending 继续 Published。

## 格式与调度

store/service floor 仍为 **V9**；retention header 升为 **KV9LED20**。旧格式只有在
没有新 namespace 数据时才能升级；新记录位于旧 header 下时拒绝写入和新盘点。
旧 Reader ledger19 stamp 仍可在升级过程中读取。没有删除旧记录或自动转换未知来源。

| Key tag | 用途 | 编码 magic |
| --- | --- | --- |
| 42 / 43 | 不可变镜像写 gate / 配对摘要 | `KV9MIW01` / `KV9MWM01` |
| 44 / 45 | 上传者镜像索引 / 两轮目录游标 | `KV9IPI01` / `KV9IPP01` |
| 46 / 47 | metadata 镜像变化与数量 / census | `KV9IPM01` / `KV9IPL01` |

请求 tag 33/34/35 分别关闭上传、推进目录和推进 census。目录成为现有全局维护的
第五类扫描；每次仍只推进一个有界页面。写 gate 和逐镜像 census 使用原 attempt
维护槽位及 leader/deadline 检查，没有增加每镜像任务或常驻内存表。持续变更下的
长期公平性、历史总空间上限和稳态吞吐资格仍待验证。

## 验证与边界

新增回归先在缺少准入 hook 时实际失败，证明另一 Pending 能绕过镜像级关闭；补齐
hook 后通过。metadata 测试覆盖跨别名新进入与 Held Publish 拒绝、旧活跃上传排空、
零报告不消除未知 PUT、旧格式回填、缺失注册/索引/stamp/gate、降级拒绝、关闭后
origin 拒绝、游标后方新终态、13/113 owner 和 generation 的固定页面工作量，以及
实际 WAL 尾部截断下的 gate、目录数量、census 和迟到排空原子性。迁移 Publisher
另验证 DATA 目录、连续代际和旧未知计数保留。

运行测试使用实际三副本 raft-rs、WAL 和带身份认证的 HTTP/2 duplex RPC。阻塞的
ObjectStore PUT 已进入真实 execution scope；当前内存后端不提供远端终止保证，
排空报告因此保留非零 issued、零 resolved。另一 Pending 在 gate 后进入失败，
报告保持零 PUT；旧上传不因关闭而提早结束。分页请求从 follower 路由，提交后丢失
回执，再切换 leader 确认与续扫；三份实际 metadata store 离线恢复后保留 gate、
原上传报告、完整 census、Published 旧 Version 和别名 Pending。

这不是 TCP/MinIO 部署、进程强杀、断电或未知远端 PUT 已终止的证明。目录中的 DATA
Publisher 没有获得 data-group 上传关闭或安全释放资格。旧部署、外部未登记作用域、
完整恢复依赖、可转移 recovery anchor、negative Pending/旧 Version 安全释放、物理
GC、历史有界、时钟和完整阶段验收继续开放。

最终验证：metadata 318、runtime 94、bootstrap 13、outbox 8、recovery 29，共 **462**
项不同测试通过；两项既有 MinIO 测试忽略。新增十项 metadata 测试和一项三副本
运行测试。带 tracing/checkpoint-testing 的 workspace/all-targets 严格 Clippy、
`cargo fmt --all --check` 和 `git diff --check` 通过。开发期间的失败与修正保留原始
日志，未计入最终通过数，也没有重写之前的封存证据。

原始运行记录、开发失败、增量 patch、源码摘要及明确的未完成项见
[本轮证据目录](raw-ttl-image-publisher-validation-20261005/)。
