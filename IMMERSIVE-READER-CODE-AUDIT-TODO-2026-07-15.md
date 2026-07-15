# ImmersiveReader 代码审计执行清单（2026-07-15）

来源：`C:\Users\15pro\Desktop\ImmersiveReader-Code-Audit-Plan-2026-07-15.md`

状态约定：`[ ]` 未完成，`[~]` 已核验但需修复/补证据，`[x]` 已实现并通过对应验证，`[-]` 经当前代码反证后不适用或延期（必须写明原因）。每个 `[x]` 都要在“证据”列留下命令、测试或提交引用。

## Wave 0：基线与测试保护

| 状态 | 待办 | 证据 |
|---|---|---|
| [ ] | 记录 commit、Windows、CPU/内存/磁盘、Rust/Node/Python、构建模式 |  |
| [ ] | 建立 QA 隔离目录与合成 Library/Podcast/Zhihu/Reader 数据 |  |
| [ ] | 建立 Rust Publish phase failpoint matrix |  |
| [ ] | 建立 Zhihu publish/rollback failpoint matrix |  |
| [ ] | 建立 SQLite claim、event、progress 故障注入用例 |  |
| [ ] | 建立 worker/sidecar 进程生命周期故障注入用例 |  |
| [ ] | 建立非均匀 Podcast chunk、reuse、resume 失败回归用例 |  |
| [ ] | 建立 watcher deferred IPC 与 listener 生命周期回归用例 |  |
| [ ] | 建立 Focus Mode characterization suite，锁定 anchor/键盘/跳转行为 |  |
| [ ] | 记录 `scripts\verify.ps1` 基线报告与性能 before 数据 |  |

## Wave 1：发布事务与稳定磁盘身份（P0）

| 状态 | 待办 | 证据 |
|---|---|---|
| [x] | Rust Prepared 阶段 validation 失败不得移动/隐藏旧 final | `cargo test ... publish::tests`：`prepared_validation_failure_preserves_last_successful_book` |
| [~] | Rust OldMoved/NewMoved/Committed 失败矩阵恢复旧版本且幂等 | 已覆盖 OldMoved/NewMoved/Committed recovery；完整 failpoint matrix 待补 |
| [~] | Rust 同书 publish claim 原子化，并覆盖重启 stale owner | 已加入 root+book keyed mutex 与 durable journal；并发 barrier/restart 用例待补 |
| [x] | Rust 不同 identity 的 final path 碰撞显式拒绝 | `rejects_a_final_path_owned_by_another_book` |
| [x] | Zhihu old rename 失败时绝不删除最后有效 final | `npm run build` + `failed old-final preparation preserves...` |
| [x] | Zhihu new rename/validation 失败时隔离 incoming 并恢复旧 final | `publish.ts` 以 `oldMoved/newMoved` 控制 rollback，publish tests 通过 |
| [ ] | Zhihu recovery 重放两次幂等 |  |
| [x] | Zhihu 作者目录保留稳定 authorId，覆盖清洗/大小写/截断碰撞 | `authorDirectoryName`、extractor、scheduler 均保留 `_authorId` |
| [x] | Podcast final path 使用稳定 sourceId，避免同 stem 互相替换 | `播客/{stem}-{sourceId[0..12]}`，现有 publish test 已更新并通过 |
| [~] | Podcast 多输出通过相对路径、case-fold、chapter/file 预检 | 已加入 case-fold destination collision guard；专门 multi-output test 待补 |

## Wave 2：控制面、幂等、取消与恢复（P0/P1）

| 状态 | 待办 | 证据 |
|---|---|---|
| [ ] | 所有 expected revision/CAS 先 reservation，stale 时外部调用为 0 |  |
| [x] | command claim 使用原子 INSERT/冲突重读 fingerprint | `control::tests::concurrent_command_claims_are_deterministic`，15 个 control tests 全绿 |
| [ ] | pending claim 持久化 owner、lease、state、reconciliation 信息 |  |
| [ ] | 重启后 pending claim 有领域对账，不盲目重做破坏性操作 |  |
| [~] | input-copy progress 仅在 DB commit 后推进 sequence/revision 并广播 | 已改为 commit 后推进并禁止广播未落库终态；单次 DB 故障注入用例待补 |
| [ ] | cancel-and-discard 先固定 task 集合再终止并清理 cache |  |
| [ ] | supervisor 与 cancel 两种事件顺序都能收敛到正确 terminal state |  |
| [ ] | Trash move/restore/delete 建 phase journal 并处理冲突/orphan |  |

## Wave 3：SSRF 防护与完整发布物（P0）

| 状态 | 待办 | 证据 |
|---|---|---|
| [ ] | Zhihu 图片下载实施 HTTPS CDN allowlist |  |
| [ ] | 每个 redirect hop 重验 host、DNS 与最终连接 IP |  |
| [ ] | 图片下载改为流式 temp 文件，限制单图/item/task bytes、time、count |  |
| [ ] | Content-Type 与 magic signature 双校验，成功后原子 rename |  |
| [ ] | 补 loopback/private/link-local/IPv6/redirect/DNS rebinding/超大响应测试 |  |
| [ ] | release 与 local ship 共用 runtime provision/verify |  |
| [ ] | clean machine 无 `IMMERSIVE_RUNTIME_ROOT` 可启动 Zhihu 与 Podcast |  |
| [ ] | runtime critical manifest 任一缺失/篡改都阻断发布 |  |

## Wave 4：Podcast 输出正确性与容错（P0/P1）

| 状态 | 待办 | 证据 |
|---|---|---|
| [ ] | 新建 versioned ChunkPlan，记录每块 source start/end |  |
| [ ] | fresh/reuse/resume/force 共用实际 chunk offset |  |
| [ ] | 非均匀切点输出时间戳不漂移，覆盖 recovery metadata 混用 |  |
| [ ] | CUDA inference 明确 GPU 错误最多 fallback 一次到 CPU |  |
| [ ] | CPU 二次失败保留 GPU/CPU 两层上下文，非 GPU 错误不重试 |  |
| [ ] | normalized audio cache 记录 fingerprint、参数、版本并安全重建 legacy cache |  |
| [ ] | subtitle-only/skip/recovery 在模型加载前预分类 |  |
| [ ] | trailing silence 与短语言片段边界回归通过 |  |

## Wave 5：前端竞态、生命周期与 Focus Mode（P1/P2）

| 状态 | 待办 | 证据 |
|---|---|---|
| [ ] | current chapter 全部按 chapter ID 匹配，覆盖 50+、重复标题 |  |
| [ ] | 外部 watcher 使用 generation/token，旧 A 响应不能覆盖 B |  |
| [ ] | watcher tick 不重叠，保留 progress flush/edit guard/anchor restore |  |
| [ ] | TaskRow visibility listener 对称清理且 mount/unmount 回 baseline |  |
| [ ] | PodcastWorkflow drag listener destroy-before-resolve exactly-once unlisten |  |
| [ ] | 普通 cancel 只有一个 confirmation owner，避免重复副作用 |  |
| [ ] | Focus Mode anchor/ratio、字体重排、长 block、keyup/blur、跳转行为保护 |  |

## Wave 6：有基线支持的性能优化（P1/P2）

| 状态 | 待办 | 证据 |
|---|---|---|
| [ ] | Cache cleanup 移除保护根双重全内容 hash并保留 containment/reparse 防护 |  |
| [ ] | Zhihu index 按 delta 持久化，单事务 prepared statement，保留排序语义 |  |
| [ ] | Library 先做 manifest-only lookup，再由基线决定是否建 index |  |
| [ ] | Reader file-backed streaming、bounded worker、session cap/TTL |  |
| [ ] | Podcast worker 只有在 churn 超预算时 coalesce 非终端事件 |  |
| [ ] | Podcast state 只有在实测超线性时拆分/去重存储 |  |
| [ ] | Search/storage/task sorting 仅在超预算时处理，不凭假设优化 |  |

## Wave 7：Contracts、构建与验证门禁（P1）

| 状态 | 待办 | 证据 |
|---|---|---|
| [ ] | 明确 unknown fields、duplicate、date/date-time、empty chapters、current policy |  |
| [ ] | 建立 valid/invalid/legacy shared parity corpus |  |
| [ ] | Schema、TypeScript、Rust 对 corpus 100% 一致 |  |
| [ ] | `metadataStatus` Rust/TS round-trip |  |
| [ ] | Settings v3 与 legacy v1/v2 分离命名和验证 |  |
| [ ] | `verify.ps1` 纳入 parity、clean dist rebuild、git exit-code gate |  |
| [ ] | quick validation 改为行为测试，不用 token-only 证明 |  |

## Wave 8：低风险死代码与冗余收敛（P3）

| 状态 | 待办 | 证据 |
|---|---|---|
| [ ] | 动态入口清单审查完成，保留不确定入口 |  |
| [ ] | unused CSS keyframes 单独核验并单独提交 |  |
| [ ] | Markdown compatibility exports 先迁测试到真实入口再处理 |  |
| [ ] | Podcast postprocessor helper/参数核对 CLI、测试、历史兼容 |  |
| [ ] | Zhihu package `main` 与生产入口决策并验证 |  |
| [ ] | 手工 reader 工具查 Git 历史后决定保留/文档化/删除 |  |

## 交付门禁

| 状态 | 待办 | 证据 |
|---|---|---|
| [ ] | 每个逻辑修复与本清单证据同一提交 |  |
| [ ] | 跨包/shared-contract wave 通过 `scripts\verify.ps1` |  |
| [ ] | 桌面/Rust/UI 变更通过 `npm.cmd --prefix .\apps\desktop run ship:local` |  |
| [ ] | 记录生产 EXE 时间戳、SHA-256、QA 隔离根与 clean install smoke |  |
| [ ] | 清理临时测试产物，确认工作树与提交内容不含受保护数据 |  |
