# DATA checkpoint 交接竞争与读下限回归

日期：2026-10-06。接续 [DATA placement 历史验证](RAW-TTL-DATA-STORE-HISTORY.md)
中保留的四线程失败。本增量修正测试的单次交接成功假设，并验证实际后台重试；
不修改生产交接协议、RPC 错误分类、超时、存储格式或回收权限。

## 已确认的问题与修正

`read_floor` 测试在三个 DATA checkpoint worker 运行期间发布第二份镜像，随后
辅助函数另行调用 `after_positive_settlement` 并直接 unwrap。每个真实 worker
也会执行同一份正向交接。metadata 事务锁采用 try-lock；竞争失败被内部 RPC
合并为 outcome-unconfirmed，因此一次调用成功不是这个并发协议提供的保证。

辅助函数现在只完成原 Pending 登记、真实对象上传和 DATA Raft publication。
三个已有 worker 负责原镜像的正向交接与采用。原有 20 秒测试进度期限和全部
冷 WAL 回退、旧镜像 GET 拒绝、换主及持久读下限检查保留。完成检查额外核对
精确 intent 的 Pending=Released、Version=Published，不能仅凭 floor 数值通过。

测试 RPC hook 在错误折叠之前记录节点、查询类别、请求 tag、剩余预算和原始错误，
另统计 catalog busy；不记录请求 payload。这些记录仅存在于测试模块。

## 可控竞争

新增三副本场景先让原 worker 采用 generation 1，再准备 generation 2。持有真实
metadata leader 的事务锁后，第二份镜像仍通过 DATA Raft 提交。单次额外交接调用
必须返回 outcome-unconfirmed，并且 hook 必须观察到原始 catalog busy。

全部三个 worker 都发生实际失败后，验证：

- 第二份镜像的 Pending 保持 Published，Version 尚未创建；对象 GET 计数不变。
- 每个副本的已选镜像与持久读下限仍为 generation 1，采用计数不增加。
- 释放锁后，无新的上传或替换意图，由原三个 worker 完成 generation 2 的交接、
  精确镜像选择与读下限推进；取消排空后维护占用归零。
- 第一份镜像的 Version 仍 Published；新读下限不授权旧 Version 释放。

对象存储的 delete/list 接口在这个 fixture 中直接 panic，因此测试不能靠删除或
重新枚举对象完成。全部实际 Raft/WAL 与鉴权 HTTP/2 使用既有进程内 fixture。

## 证据与限制

新增竞争场景与原冷恢复场景的两项定向测试通过。修正前加入原始错误记录的一次
四线程全套运行也曾 98/98 通过，说明原失败并非每次运行都会出现。修正后四线程
TTL runtime 全套 **99/99 通过**，耗时 246.6 秒；严格 workspace Clippy（含
write-stage-tracing、checkpoint-testing）、格式和 diff 检查通过。新增测试 1 项，
两项定向测试已包含在 99 项内，不重复计数。初次重构遗漏一个局部 region 引用的
编译失败也保留在[本轮验证包](raw-ttl-retention-diagnostics-validation-20261006/manifest.json)。

历史 `runtime-dev-4.log` 只有合并后的错误，没有原始 backend/transport 原因。
本轮证明了原辅助函数假设存在可控反例，**不宣称已确定历史失败的唯一原因**，
也不以一次全套通过宣称持续并发负载资格。全部历史失败和封存验证包保持原样。

完整第二阶段仍开放：实际历史成员与恢复来源覆盖、DATA 全 store 读写关闭、
可转移恢复锚点、未知 PUT、旧 Version/负向 Pending 释放、物理 GC、历史与资源
有界、merge，以及真实部署、时钟和形式化联合验收均未由本轮关闭。
