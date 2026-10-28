# 离线安装的 Reader 生命周期

2026-10-05；基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
接续 [迁移 Publisher](RAW-TTL-MIGRATION-PUBLISHER.md)。本轮把实际 CLI 安装中的
旧镜像校验、新镜像恢复和发布前重验纳入独立 Reader；整个第二阶段继续开放。

## 安装入口

```sh
# 先设置 KV9_CLIENT_TOKEN 及 KV9_OBJECT_STORE_* 环境变量
kv9 install-migration-image \
  --data-dir <stopped-store> --record-file <capture-record> \
  --addr <metadata-leader-ip:port> --owners <destination-owner-id,...>
```

`--owners` 使用 bind/capture 回执中的 owner ID，最多 16 个且不能重复。通常使用
当前迁移的 destination owner；替换安装需要同时提供旧选中镜像与新镜像的 owner。
调用先读取已提交的 Snapshot/Migration owner，验证 Published、root、连续首代、
非本地全局来源。它们只授予读保护，不改变原有目的 store、range、Raft 配置、cut、
TTL 水位和安装 selector 检查，也不提供投票、公开服务或迁移完成权限。

CLI 继续独占停止节点的 store。连接 metadata 使用现有 RetentionClient 与
`KV9_CLIENT_TOKEN`；地址需指向当前 metadata leader，错误不自动重试未知写入。
缺少新参数会在安装前拒绝。9 份现有端到端脚本已提供对应回执 owner 和 metadata
地址；本轮只验证这些脚本语法，没有重新执行真实 TCP/MinIO 场景。

## 每一次实际 GET

引擎新增只读 `CheckpointRestore` 依赖。相同的 WAL 恢复、原始/分段布局和镜像
检查通过该接口调用 restore；安装器的旧 generation 检查、准备和最终校验全部
使用同一外部 reader。旧低层 uploader API 保留供尚未接入的调用者使用，不能把
接口存在解释为所有恢复路径已经受保护。

每次 restore 先用完整 manifest 编码摘要、cluster、region、epoch 和完整 SST
closure 匹配一个已确认 owner。没有精确覆盖时不发 GET。随后按源 owner 和
node/store/process 派生 Reader，上一代必须 Released；预留共享 drain outbox
后再次读取 owner，排除其他请求在首个查询与槽位预留之间推进代际的窗口。
这项复核也用于已有 data checkpoint Reader。

读取 scope 先绑定真实 identity，再确认 Share 与 Publish，之后才能 GET。使用
GET-only `CheckpointReader`；没有 PUT 方法。所有已排队/运行的实际对象任务退出
后才得到 `DrainedRead`，超时、取消或内层返回不能提前得到报告。恢复接口在返回前
完成此等待，因此 installer group lock 和外层 store lock 覆盖真正的 I/O 生命周期。

## 结束与恢复

新 ledger 请求 tag **22 `FinishImageRead`**，包含原 source token 和完整 Reader
binding。与 checkpoint tag 18 分开：它仅接受 Snapshot/Migration、generation 1、
非 anchor 的全局 source，并要求同一 snapshot 中的完整来源仍 Published。
按密集 Reader generation 原子结束全部 pin；未确认 Share 也可建立同代终态 fence，
防止迟到请求重新获得读权限。不同身份、资源、跳代和过时报告不能结束后继代。
本轮不允许失去原来源后替换一个较新镜像来释放该 Reader。

真实排空后，原 `FinishImageRead` 请求先 fsync 到 1,024 槽/16 KiB 共用 outbox，
再提交 metadata。回执丢失保留原文件，阻止同进程开启后续读；下一次离线安装在
新准入前有界重放原报告。节点的后台 sweep 同样认识该请求。新进程不会改写保存
的旧 process/generation，未保存报告的崩溃 Reader 也不会被重建成已排空。

安装成功可以结束自己的 Reader，但原 Snapshot/Migration pins 继续 Published。
只读报告不能替代 Publisher PUT 终止证明、全体读者清单、继任恢复锚点或 GC 权限。

retention header 仍 **KV9LED08**，store/service floor 仍 **V8**。Owner/资源行以及
capture、安装 image、selector 编码均不改变。旧请求 tag 18 的来源限制和语义保持
不变；不认识新请求的程序应拒绝，也不能清理带完整 checksum 的未知 outbox 报告。
尚未完成旧二进制、混合部署或滚动升级资格验证。

## 验证边界与后续

[本轮证据包](raw-ttl-install-reader-validation-20261005/manifest.json) 保存命令和原始
日志。metadata 覆盖精确来源、代际/身份拒绝、未确认准入 fence、编码以及五种实际
WAL 尾部截断。运行态使用三副本 Raft、独立 metadata/data leader、鉴权 HTTP/2
duplex，覆盖 TTL 安装与重复校验、旧/新双镜像保护、零 GET 拒绝、GET 超时后的
实际排空、丢回执阻止后继读取及新 context/outbox、换主后的原报告重放。

本轮 **603 个不同测试通过，12 个既有测试忽略**：engine 236、metadata 214、
snapshot/install 24、checkpoint recovery 24、TTL runtime 72、RegionManager 18、
outbox 8、引擎类型约束 doctest 7。新增 metadata 测试 3 个、安装运行态测试 5 个；
定向重复运行不重复计数。workspace 全目标严格 Clippy、格式与 diff 检查通过。

对象数据为 MemoryObjectStore；超时测试使用真实 MinIO 工作队列和暂停的内存后端。
context 重建与 WAL 重开不是实际进程 kill、断电、跨主机或 MinIO 部署验收。

尚未接入的 **runtime adoption、普通 data 启动恢复与 metadata 自身启动恢复** 继续
保留。metadata 恢复发生在其 Raft owner 启动前，不能直接以“先提交 Reader”形成
循环依赖，须与持久恢复授权和启动顺序一起解决。安装源的后继交接、无报告遗留
Reader、Publisher census/关闭、旧 Version/负向 Pending 和物理 GC 仍开放。

后续进展：[恢复预检查](RAW-TTL-RECOVERY-PREFLIGHT.md) 已在共同恢复入口中提前核对
成功发布，零 GET 拒绝非法本地选择，并提供精确 predecessor；不改变上述 Reader 接入边界。

再后续：[运行态 Reader](RAW-TTL-RUNTIME-READERS.md) 已接入 adoption 和普通 data 启动，
并将 data 恢复延迟到 metadata 就绪之后。metadata 自身启动 Reader 仍未接入。
