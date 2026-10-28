# 已采用 generation 的 TTL 恢复校验

2026-10-04；工作树基线 `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`。
这是第二阶段的本地恢复组件修复，完整阶段与生产验收仍未完成。

## 修复的问题

原 `RegionManager::adopt_installed` 在验证 adoption marker 后直接调用通用 engine
打开入口，没有接上普通 data group 的 committed Raft 历史、checkpoint publication
和 pending candidate 恢复检查。原普通恢复还只比较最终 range/clock：例如镜像水位
1000、尾部依次写入 2000→1500→3000，最终状态高于镜像，因而会漏过中间回退。
此外，adopt 后合法发布新 checkpoint 并压缩日志，再启动仍传最初安装的 cut，导致
Raft constructor 拒绝实际已经前进的 protocol base。

## 恢复入口与约束

`AdoptedGeneration::open_engine` 现在复用普通 data group 恢复入口。adoption 时从
durable selector、校验过的 image record、group record 提取私有恢复输入；修改公开
诊断字段不能替换恢复路径、身份或原始镜像的例外。它不代替调用方的 migration/store
权限，也不允许提前释放 group lock。

- 必须存在 selected checkpoint 和不早于原始 cut 的 protocol base。只有**精确等于
  原始安装 manifest** 的选择，才能以该 adoption 代替本地 ManifestChange publication；
  原始配置仍需与 committed protocol history 相符。
- 其他选择必须是更晚的 cut，并通过普通恢复的精确 committed ManifestChange、原子
  winning pair、连续 history 与实际配置验证。相同 cut 的不同镜像、缺失选择、未发布
  的新镜像、配置冲突均拒绝。新的镜像不得移除原 TTL activation 或降低原始水位。
- 未被镜像覆盖的 WAL 必须有合法 committed term/index。range 与 clock 按 mutation
  顺序逐项检查，包括合批内多个命令；删除后补回、回退后升高、替换 range 后修复都
  不能掩盖违反约束的中间写入。最终还核对观察状态和实际 engine 状态。
- 普通 data group 同样使用逐项检查。当前范围规则仍只有首次合法 descriptor、相同
  descriptor，以及未激活 TTL 时的单向 seal；TTL 旧 seal 继续拒绝。
- pending journal 经现有 scope、committed configuration 检查后，在返回 engine、
  绑定 driver 或允许压缩前恢复 candidate 历史保护。错误不清除 pending。
- RegionManager 启动采用实际 protocol base。等于原始安装 cut 时沿用 adoption；若
  已前进，必须有 committed migration truncation 的精确 floor，或覆盖它的 committed
  group compaction floor。正常 Active group 和 adopted group 共用这个判定。

原始镜像被后续 checkpoint 替换时，恢复先还原并检查原始镜像中的 TTL 水位，再释放
临时索引、还原当前选择，避免两份完整还原索引同时存活。代价是增加原始镜像读取，
并继续依赖其远端对象可读；本轮没有设计允许释放原始对象的继任恢复锚点。GC/owner
释放协议必须保留这个依赖，不能凭本轮通过就删除原始对象。格式、selector 与 adoption
marker 编码没有变化。

后续 [持久采用记录](RAW-TTL-ADOPTION-FACTS.md) 已将原始镜像的已验证 TTL presence/水位
写入 V2 adoption marker，较新合法 checkpoint 重开不再读取原始 SST。上述额外读取
与远端依赖描述的是本记录对应的增量；完整 owner 释放与可转移恢复授权仍开放。

## 验证

[命令、日志与源码摘要](raw-ttl-adopted-recovery-validation-20261004/manifest.json)另存。
新增三项 Raft 测试、两项 RegionManager 测试，并扩展原有用例：

- 十类实际 WAL 异常：越过 commit、term 错误、无位置写、降低水位、删除后恢复 clock、
  foreign range 后恢复、单 batch 与跨 batch 中间回退、删除后恢复 range、TTL seal。
  使用合法 WAL framing，并对需要的负例提供实际已提交位置，验证位置校验不够时
  状态迁移检查仍会拒绝。
- 六类 selected image/configuration 异常：无选择、同 cut 不同镜像、未发布新镜像、
  新镜像水位降低、移除 activation、配置历史冲突；均拒绝完成 open。
- pending reservation 重建、单一执行 owner，以及修改 receipt 公开字段不能更换
  私有恢复输入。普通分段 WAL 回归增加三个“中间损坏、最终修复”的合批变体。
- 实际 raft-rs voter/learner 链路在原 capture/install/追赶/重启之后，再发布 checkpoint、
  选择并压缩/回收目标日志，实际文件重开后从新 base 重放 publication，TTL/expiry 保持。
  此用例通过测试字节提供 compaction authority；单独的 RegionManager 测试验证真实
  committed catalog floor 的拒绝与放行。
- RegionManager 在原始 cut 上也会拒绝超出 commit 的 engine WAL，且没有创建 driver。
  后续 cut 在缺少目录授权时拒绝；持久写入并读回实际 compaction 决策后启动成功。

专项集合 **9 通过**；Raft 本地回归 **335 通过、8 忽略、37 过滤**；RegionManager
**18 通过**；TTL 激活相关组合 **19 通过**。测试集合有重叠。严格 workspace Clippy、
格式和 diff 检查通过。对象后端是 MemoryObjectStore，复制传输为 InProcHub；实际
RegionManager 使用 GrpcTransport 无 listener，未进行 TCP/MinIO 持久性验收。

## 尚未证明的边界

WAL 检查包含 committed 位置和 range/TTL 不变量，**没有逐字节重新执行所有 Raft
命令并比对全部 WAL effect**。一致的 term/index 不是任意业务值的完整语义证明。
后续镜像以实际 publication 为依据，也没有从已释放的旧 WAL 重建并审计每个中间状态。

没有新增完整 NodeRuntime/真实网络/旧二进制/进程 kill 验收；新副本升级资格、完整迁移
协调、源/目标/Reader/旧 Version/负向 Pending owner 与继任锚点、对象 GC、split/merge、
常驻索引及恢复任务整机预算、历史有界、48 MiB 全量镜像限制、部署时钟资格、独立时间
历史和完整 TLA/TLC/Chaos 仍按 [第二阶段清单](RAW-TTL-PHASE2.md) 推进。
