# 受管运行时获取策略（Runtime Acquisition）

## 背景与发布策略

沉浸阅读的受管运行时（Node、Chromium/Edge、Python、FFmpeg、Whisper 模型以及 zhihu-packer / podcast-transcriber / contracts 应用代码）合计约 6.9 GB，**不随 NSIS 安装包分发**。每个 GitHub Release 携带两类资产：

| 资产 | 内容 |
| --- | --- |
| `ImmersiveReader_<版本>_x64-setup.exe`（+ `latest.json`） | 应用本体安装包与 updater 元数据，只含应用 |
| `runtime-bundle.zip.001` … `.00N` | 受管运行时整包 `runtime-bundle.zip` 的二进制分卷（每卷 < 2 GB）；分卷名、字节数与 SHA-256 固定记录在 `docs/release/<版本>/runtime-parts.json` |

这样取舍的原因：GitHub Release 单资产上限 2 GB、安装包体积与更新带宽受限，且应用更新（updater 只替换应用本体）不应强迫用户重复下载数 GB 运行时。

## 放置位置

安装后应用主程序为 `<安装目录>\immersive-reader.exe`（NSIS 默认如 `C:\Program Files\ImmersiveReader\`）。运行时的期望位置是 **`runtime` 与 exe 同级**，即 `<安装目录>\runtime\`；校验锚点是 `<安装目录>\runtime\manifest.json` 存在。目录不存在时手动创建即可。

## 获取与安装步骤

1. 安装 `ImmersiveReader_<版本>_x64-setup.exe`，记下安装目录。
2. 在**同版本** Release 页面下载全部 `runtime-bundle.zip.*` 分卷。
3. 按编号顺序合并分卷（分卷数量以该版本 `runtime-parts.json` 为准）：

   ```powershell
   cmd /c copy /b runtime-bundle.zip.001+runtime-bundle.zip.002+runtime-bundle.zip.003+runtime-bundle.zip.004 runtime-bundle.zip
   ```

4. 可选校验：逐分卷 `Get-FileHash -Algorithm SHA256` 与 `runtime-parts.json` 对照；合并后对 `runtime-bundle.zip` 校验 `bundle.sha256`。
5. 解压，使 `runtime\` 落到 exe 同级：

   ```powershell
   Expand-Archive -LiteralPath .\runtime-bundle.zip -DestinationPath "<安装目录>" -Force
   ```

   - 若 zip 顶层已含 `runtime\`，解压到 `<安装目录>`；
   - 若顶层直接是 `zhihu\`、`podcast\`、`packages\`、`manifest.json`，解压到 `<安装目录>\runtime`（不存在先创建）。

6. 重启应用。

## 校验

- 开发/维护机：`powershell -File scripts\verify-runtime.ps1 -RuntimeRoot "<安装目录>\runtime"`——按 `runtime\manifest.json`（schemaVersion 2）逐文件校验存在性、字节数与 SHA-256，与发布链 `prepare-runtime.ps1` 写出的清单一致。
- 发布维护机还可以用 `scripts\verify-runtime-bundle.ps1` 对已下载的分卷做端到端校验：逐分卷比对 `runtime-parts.json` 的名字/大小/SHA-256、拼合后校验整包哈希、解压并复跑 manifest 校验，最后对 zhihu / podcast / contracts / reader 模板做源码同源性比对。
- 终端用户无需脚本：运行时缺失/不完整时，应用内工具状态与错误信息会直接给出期望目录与本说明；工具状态变为 running 即说明就位。

## 高级：自定义位置

环境变量 `IMMERSIVE_RUNTIME_ROOT` 可指向任意已有 runtime 目录（开发/便携场景），优先级高于 `<安装目录>\runtime`。生产安装不应依赖该变量。

## 已知取舍

- 应用内自动下载/拼合/解压（bootstrapper）本期未实现——先以明确文档 + 错误文案指路收敛问题；后续可在设置面板补「下载运行时」入口。
- 安装包不预置 `runtime\` 占位目录：目录由解压动作产生，缺失时错误文案已给出期望路径。
- 版本匹配要求：runtime 分卷与应用应取同一 tag；跨版本混用时以 manifest 校验和应用内健康检查结果兜底。
