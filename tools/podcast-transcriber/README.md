# Podcast 转写引擎

这是 ImmersiveReader 的受管 Python 播客引擎。桌面端负责选择文件、预算预检、任务控制和结果发布；本目录只负责单任务转写、翻译、润色、恢复状态和 sidecar 协议。

## 能力

- 本地 faster-whisper CUDA ASR，中文直接转写，英文可生成中英双语 Markdown。
- 翻译/润色支持受管 DeepSeek API 或本地 Ollama；凭据由桌面端从 Windows Credential Manager 注入内存环境，不写入 TaskSpec、JSON、SQLite 或日志。
- 输入、输出、切片和日志必须位于受管 Data/Cache 根；不要把真实音频、模型、配置或输出放入 Git。
- 支持可恢复任务、输入 SHA-256、预算上限、短样本验证和 sidecar READY/health 协议。

## 本地检查

从仓库根目录运行：

```powershell
G:\python\python.exe -m pytest .\tools\podcast-transcriber\tests -q
G:\python\python.exe .\tools\podcast-transcriber\scripts\quick_validate.py
```

跨包改动使用：

```powershell
.\scripts\verify.ps1
```

真实长音频必须先完成只读预检，并单独确认 API 预算；不要在命令行、文档或报告中打印 API Key。
