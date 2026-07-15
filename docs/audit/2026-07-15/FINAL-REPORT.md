# ImmersiveReader 代码审计计划最终报告

日期：2026-07-15（Asia/Shanghai）  
仓库：`ImmersiveReader`
最终分支：`main`  
最终提交：`930e525`

## 结论

计划中的 Phase 0、Wave 1–8 均已实施，并以逻辑切片提交到 `main` 后推送到 `origin/main`。最终工作树干净；未提交 Library、浏览器 profile、数据库、模型、输入输出、缓存、日志或凭据。

## 实施覆盖

- Phase 0：建立合成数据基线、故障矩阵、Focus Mode 视觉/锚点记录和性能决策记录；基准脚本位于 `scripts/audit_benchmark.py`，材料位于本目录。
- Wave 1：发布事务阶段化回滚、知乎/播客身份稳定性、同 stem 保护、SQLite 初始化重试及对应测试。
- Wave 2：控制命令 CAS 先于外部副作用、原子 claim/lease/reconciliation、持久化 progress sequence、取消并丢弃意图及 Trash phase journal。
- Wave 3：知乎图片 HTTPS/CDN allowlist、DNS/IP/逐跳 redirect 校验、流式大小/MIME/magic 限制、临时文件原子提升；managed runtime manifest v2、安装前后 verifier、发布 ADR 和 release gate。
- Wave 4：版本化非均匀 ChunkPlan、源坐标恢复、音频 cache fingerprint、CUDA→CPU fallback、sidecar 字幕预检和懒加载。
- Wave 5：chapter ID、watcher generation/single-flight、监听器和 drag teardown；Focus Mode 仅增加回归测试，保留既有视觉与 viewport-anchor 行为。
- Wave 6：移除受保护根内容 hash 扫描、manifest-only 书架检索、Reader 流式响应和 session 上限/TTL；Zhihu delta index 与 Podcast terminal state 保留已有实现，未在没有生产规模证据时引入 speculative 全量迁移。
- Wave 7：Schema/TypeScript/Rust/settings v3 parity、unknown-field/date/count policy、fresh-build 和 Git exit-code gates；quick validation 改为行为验证。
- Wave 8：删除两个无消费者动画、保留有证据的 compatibility exports/postprocessor helpers、补齐 contracts package main，并完成桌面生产安装和 QA。

## 最终验证证据

执行：

```powershell
powershell.exe -ExecutionPolicy Bypass -File .\scripts\verify.ps1
```

结果：

- contract parity：5 个 valid、5 个 invalid 全部符合预期。
- Desktop：68 tests；Svelte check 0 errors / 0 warnings。
- Rust：113 tests 通过，check/clippy 通过。
- Zhihu：fresh reader compile、49 tests、TypeScript build、reader compile 全部通过。
- Podcast：Ruff、46 tests、`quick_validate` 全部通过。

书架浏览器验收使用全局 Playwright 和合成 Tauri mock：

```powershell
& 'G:\python\python.exe' .\scripts\qa\verify_bookshelf.py
```

已验证 3 本书、1,469 章、6 个 viewport、字体 100%/150%，以及 ready/loading/empty/error 状态；无页面错误、无水平溢出、详情/设置弹窗均在安全区内。QA 输出只写入被忽略的 `artifacts/qa`。

## 生产安装与 smoke

执行：

```powershell
npm.cmd --prefix .\apps\desktop run ship:local
```

最终安装结果：

- 安装器 SHA-256：`1B07AEA20E0B552975B987A22F8DE84E72A5420A30568E6A7CDDEFE6E39370D0`
- 生产 EXE：`C:\Users\15pro\Desktop\MyProject\ImmersiveReader\immersive-reader.exe`
- EXE 时间戳：`2026-07-15 16:08:52`
- 产品版本：`1.1.0`
- EXE SHA-256：`51DB3D69020FFCB33D1E89D764EA7B9AA6530C7ACAE6E2F62F58BBEC26ECCC3F`
- managed runtime：4,152 项 critical entries 校验通过。

最新安装版使用 `IMMERSIVE_QA_RUN_ID=ship-smoke-20260715-2` 打开合成 Markdown，持续运行 8 秒后正常关闭；合成书库位于：
`C:\Users\15pro\Documents\Codex\ImmersiveReader-QA\ship-smoke-20260715-2`。

Markdown handler 和 Default Apps capabilities 已注册。Windows 的受保护 `UserChoice` 仍为 `md`，需要用户在 Windows Default Apps UI 中确认，未伪造 hash。

## 提交与推送

本轮逻辑切片均在 `main` 上提交并推送；最终收口提交为：

`930e525 fix(desktop): keep bookshelf details within safe area`

该提交同时修复了原生 `<dialog open>` 的默认绝对定位导致的安全区越界，并把 QA 断言改为验证“折叠时不可见、展开时可见”，避免将 DOM 中的折叠内容误判为产品上的 always-visible 信息。
