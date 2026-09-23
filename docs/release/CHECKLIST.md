# 发布检查清单（Release Checklist）

适用于每个 `v<x.y.z>` tag 的正式发布。release.yml 会在 tag 推送后自动校验第 4、5、7 步的机械部分，但生成资产与人工 QA 仍是发布者职责。

## 0. 版本一致性

- [ ] `apps/desktop/package.json`、`apps/desktop/src-tauri/tauri.conf.json`、`apps/desktop/src-tauri/Cargo.toml` 三处版本号一致，`Cargo.lock` 已同步。
- [ ] `docs/release/<版本>/RELEASE_NOTES.md` 已创建；`README.md` 的「当前发布版本」与链接已更新。
- [ ] `scripts\verify.ps1` 全绿（report-only 项无遗留或已记录豁免）。

## 1. 本地验证

- [ ] 各包测试通过：desktop `npm test`/`run check`、contracts `npm test`/`run build`/`run typecheck`、zhihu-packer `run build`/`run compile-reader`/`npm test`、podcast `ruff check`/`pytest -q`/`quick_validate.py`。
- [ ] `cargo fmt --check`、`cargo clippy --locked`、`cargo test --locked` 通过。
- [ ] PowerShell 脚本测试（`scripts\tests\`）通过。
- [ ] 生产安装走完整流程：`npm.cmd --prefix .\apps\desktop run ship:local`，记录生产 EXE 时间戳与 SHA-256。

## 2. QA 报告

- [ ] `docs/release/<版本>/QA_REPORT.md` 存在，覆盖：安装/卸载/覆盖升级、运行时获取、updater 检查→下载→验签→安装→重启链路、知乎与播客工具冒烟。

## 3. 运行时整包

- [ ] 运行 `scripts\pack-runtime-bundle.ps1`（自动先跑 `verify-runtime.ps1` 校验 manifest），产物落在 `output\runtime\runtime-bundle.zip.*`，清单写入 `docs\release\<版本>\runtime-parts.json`。
- [ ] 用 `scripts\verify-runtime-bundle.ps1 -PartsDirectory output\runtime` 对分卷做端到端校验：分卷名/大小/SHA-256、整包哈希、解压后 manifest、应用代码与当前 checkout 同源性比对。
- [ ] `runtime-parts.json` 已随发布提交（release.yml 在 tag 上读它核对 Release 资产）。

## 4. GitHub Release 资产

- [ ] 先创建**草稿** Release（tag `v<版本>`），上传 `runtime-bundle.zip.001…N` 全部分卷。
- [ ] `apps\desktop\scripts\collect-release.ps1` 产出 `output\desktop\` 下的 `ImmersiveReader_<版本>_x64-setup.exe`、`….sig` 与 `latest.json`（notes 兜底取 `RELEASE_NOTES.md` 首段，可用 `IMMERSIVE_READER_RELEASE_NOTES` 覆盖）。
- [ ] 上传 setup.exe、`….sig`、`latest.json` 到草稿 Release，随后推送 tag 触发 release.yml 复核并发布。

## 5. 发布后验证

- [ ] 干净机器/账户安装：解压 runtime 分卷至 `<安装目录>\runtime`（见 `docs\runtime-acquisition.md`），应用内工具状态变为 running。
- [ ] 覆盖升级旧版本：书库、进度、凭据、运行时均保留。
- [ ] updater 端对端：旧版本检查更新能拿到本版 `latest.json` 并完成验签安装。
