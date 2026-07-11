# MMbook 📚
---
**MMbook** 是一款为个人开发者量身定制的 Markdown 阅读器，旨在提供最纯粹、最沉浸的阅读体验。它不仅尊重你的内容，更专注于通过精致的视觉设计和独特的“专注模式”来提升阅读质量。

![MMbook Preview](https://img.shields.io/badge/Status-Beta-orange)
![Tauri](https://img.shields.io/badge/Powered%20by-Tauri-blue)
![Svelte](https://img.shields.io/badge/Built%20with-Svelte-ff3e00)

## ✨ 核心特性

- 🎨 **多款精美主题**：内置“素纸”、“墨石”、“暮光”等多种极简风格主题，支持深浅色模式切换，每一处色彩都经过精心调配。
- 👁️ **沉浸式专注模式**：独创的“聚光灯”阅读体验。智能分段——相邻短句合并聚焦、超长段落按句推进，周围内容自然淡化并辅以优雅的模糊效果。
- 🔠 **排版自主权**：字号缩放（`Ctrl + 滚轮`）、行距、栏宽、黑体/宋体切换，全部即时生效并记住偏好。
- 🔍 **高效全局搜索**：macOS 风格悬浮搜索框，实时高亮定位，右缘刻度一览全部命中位置。
- 📑 **命令面板式目录**：`Ctrl + T` 呼出居中目录，键入即过滤标题，回车直达，自动标记当前章节。
- ✍️ **轻量级即时编辑**：双击任意段落进入编辑，`Enter` 保存、`Esc` 取消，精确回写源码行，绝不弄乱文件其他部分。
- 🧭 **阅读位置感**：滚动时底部浮现“当前章节 · 进度% · 约剩分钟”，静止后自动隐去；重开文件回到上次位置，欢迎页一键续读。
- 🧮 **完整显示层**：KaTeX 数学公式、Mermaid 图表、脚注悬浮预览、YAML 元信息卡、图片点击放大（暗色主题自动压暗）、代码块一键复制、宽表格横向滚动。
- 🚀 **极致性能**：基于 Rust & Tauri 构建，渲染在 Web Worker 中进行，超长文档与专注模式依然流畅。
- 🖋️ **代码高亮**：集成 Shiki 高亮引擎，支持数十种主流编程语言的精准着色。

## 🛠️ 技术栈

- **Frontend**: Svelte 5 + TypeScript + Vite
- **Backend**: Rust (Tauri 2.0)
- **Engine**: Unified / Remark / Rehype (Markdown 解析)
- **Highlighter**: Shiki (代码高亮)
- **Styling**: Vanilla CSS (极简主义设计)


## ⌨️ 快捷键

为了提升操作效率，MMbook 提供了一系列快捷键：

| 功能 | Windows / Linux | macOS |
| :--- | :--- | :--- |
| 打开文件 | `Ctrl + O` | `⌘ + O` |
| 开启/关闭搜索 | `Ctrl + F` | `⌘ + F` |
| 开启/关闭目录 | `Ctrl + T` | `⌘ + T` |
| 开启/关闭专注模式 | `F11` / `Ctrl + Shift + F` | `F11` / `⌘ + Shift + F` |
| 字号放大 / 缩小 / 复位 | `Ctrl + =` / `-` / `0`（或 `Ctrl + 滚轮`） | `⌘ + =` / `-` / `0` |
| 打开设置（主题 + 排版） | `Ctrl + ,` | `⌘ + ,` |
| 编辑段落 | 双击段落；`Enter` 保存 · `Shift + Enter` 换行 · `Esc` 取消 | 同左 |
| 逐层关闭弹窗/编辑/模式 | `Esc` | `Esc` |
| 专注模式逐段移动 | `↑` / `↓`（长按连续滚动） | `↑` / `↓` |

## 📦 快速开发

### 前置要求
- [Node.js](https://nodejs.org/) (推荐 v18+)
- [Rust](https://www.rust-lang.org/) 环境

### 安装与运行
1. 克隆仓库：
   ```bash
   git clone https://github.com/lingcang728/MMbook.git
   cd MMbook
   ```

2. 安装依赖：
   ```bash
   npm install
   ```
3. 启动开发服务器：
   ```bash
   npm run tauri dev
   ```

## 📜 开源协议

本项目采用 [MIT](LICENSE) 协议开源。

---

*由 **凌苍** 精心打造*

## Recommended IDE Setup

[VS Code](https://code.visualstudio.com/) + [Svelte](https://marketplace.visualstudio.com/items?itemName=svelte.svelte-vscode) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer).
