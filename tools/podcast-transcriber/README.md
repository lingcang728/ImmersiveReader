<div align="center">
  <img src="assets/icon.png" width="150" alt="PodcastTranscriber Logo">
  <h1>🎙️ PodcastTranscriber</h1>
  <p><strong>一键将你的播客音频转换为精美的中英双语 Markdown 笔记！</strong></p>
</div>

## ✨ 这是什么？

**PodcastTranscriber** 是一个强大且极简的自动化工具。它的目标只有一个：把枯燥的 `.mp3`、`.m4a`、`.wav` 音频，甚至 `.mp4` 等**视频文件**，魔法般地转换成排版精美、高度可读的 Markdown 文本！

- 🎯 **开箱即用**：只需把音频/视频扔进 `input/` 文件夹，双击运行，剩下的交给我们。
- 🎬 **视频也能转**：直接丢一个视频进去，工具会用本机 FFmpeg 自动抽出音轨再转写，无需你手动转格式。
- ⚡ **强大的引擎**：底层基于 `faster-whisper`，转写精准且迅速。
- 🌍 **智能双语**：对于英文播客，自动调用本地大模型（Ollama）或云端 API（DeepSeek），为你生成“中文在前，英文在后”的对照笔记。
- 👥 **角色分离**：聪明的后处理逻辑，自动为你梳理“采访者 / 受访者”的对话流，阅读体验拉满。
- 🎨 **现代化 UI**：不仅能在命令行跑，还自带带有玻璃拟物化风格（Glassmorphism）的 Web UI，进度状态一目了然！

---

## 🚀 快速开始

想要在你的电脑上跑起来？跟着这几个简单的步骤做：

### 第一步：克隆/下载项目
将这个项目下载到你的本地电脑。你可以直接下载 ZIP 包并解压，或者使用 Git：
```bash
git clone https://github.com/lingcang728/PodcastTranscriber.git
cd PodcastTranscriber
```

### 第二步：一键初始化
确保你已经安装了 Python 和 FFmpeg，然后双击或在 PowerShell 中运行：
```powershell
.\scripts\setup.ps1
```
它会创建本地虚拟环境 `.venv/`、安装依赖，并在首次运行时把 `config.example.json` 复制成你的本地 `config.json`。`config.json` 用来保存个人设置和 API Key，不会被提交到 Git。

### 第三步：开始转写！
1. 把你想转写的播客音频（`.mp3`/`.m4a`/`.wav`）或视频（`.mp4`/`.mkv`/`.mov`/`.webm` 等）扔进 `input/` 文件夹。
2. 双击运行 `.\scripts\run_podcast_transcriber.ps1`，或者在终端中执行它。
3. 坐和放宽，看着进度条走完。转写完成后，我们会自动为你打开 `output/` 文件夹，你的精美 Markdown 就在那里！

> 🎬 **视频与字幕**：丢进视频会自动抽音轨再转写，规范不变（英文→中英双语，中文→直接转写，两者都润色）。
> 如果你在视频旁放一个**同名外挂字幕**（如 `talk.mp4` + `talk.srt`，也支持 `.ass`/`.vtt`），工具会**直接用该字幕当转写文本、跳过语音识别**——请放“内容原语言”的字幕（中文译文仍由工具自己翻译+润色生成）。烧录进画面的硬字幕无法提取，会照常走音频转写。

---

## ⚙️ 进阶玩法：搭配 DeepSeek

本地模型跑得太慢？没关系，我们无缝支持 DeepSeek 强大的 API 翻译与润色！

1. 启动 GUI 界面（运行 `.\scripts\run_with_gui.py`）。
2. 点击右上角的 ⚙️（设置图标）。
3. 选择 `DeepSeek` 作为你的翻译后端，并填入你的 API Key。
4. 享受云端大模型带来的极致速度与高质量翻译！

---

## ⚡ Large V3 Turbo 极速档

默认配置使用 `faster-whisper` 的 `large-v3-turbo`。当前依赖 `faster-whisper==1.2.1` 已支持这个模型，不需要删除或升级 `requirements.txt`。

首次使用前可以先手动下载模型缓存：

```powershell
.\.venv\Scripts\python.exe -c "from huggingface_hub import snapshot_download; snapshot_download('mobiuslabsgmbh/faster-whisper-large-v3-turbo', cache_dir='models'); print('large-v3-turbo download OK')"
```

下载完成后，再验证它能被 `faster-whisper` 加载：

```powershell
.\.venv\Scripts\python.exe -c "from faster_whisper import WhisperModel; WhisperModel('large-v3-turbo', device='cuda', compute_type='int8_float16', download_root='models')"
```

旧的 `models--Systran--faster-whisper-large-v3` 缓存可以保留；如果 Turbo 在个别音频上质量不满意，把 `config.json` 里的 `asr.model` 改回 `large-v3` 即可回滚。

---

## 📂 目录说明

保持桌面整洁是好习惯，本项目目录同样如此：

- 📥 `input/`：这里是起点。把音频或视频丢这里（视频会自动抽音轨；可选放同名外挂字幕）。
- 📤 `output/`：这里是终点。只放最终的纯净 Markdown 成品。
- 🛠️ `work/`：我们干活时的草稿纸（中间产物、切片、日志）。如果不需要了，随时可以删掉。
- 📦 `models/`：大模型的家（Whisper 模型缓存）。

---

## 🛠️ 遇到问题？(FAQ)

**Q: 报错说找不到 FFmpeg？**
> A: 确保你安装了 FFmpeg 并且把它加到了系统的环境变量 `PATH` 中。如果你不知道怎么做，再跑一次 `.\scripts\setup.ps1` 试试。

**Q: 英文音频没有翻译？**
> A: 检查一下你是否启动了 Ollama（如果你使用的是本地翻译）。或者如果你用的是 DeepSeek，去设置面板检查一下 API Key 是否正确填好。

**Q: 想要更深度的定制？**
> A: 打开 `config.json` 看看吧！那里有并行处理数量、翻译段落大小等高阶设置。

**Q: 为什么仓库里没有 `config.json`？**
> A: `config.json` 是本地配置文件，可能包含 API Key。开源仓库只提供 `config.example.json`，运行 `.\scripts\setup.ps1` 会自动生成本地副本。

---

<div align="center">
  <p>💡 <i>"技术改变播客，阅读点亮思想"</i></p>
  <p>Made with ❤️ by Lingcang</p>
</div>
