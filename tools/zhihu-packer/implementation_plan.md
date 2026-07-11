# 知乎回答/文章归档工具 (Zhihu Packer) 实施计划

## 核心架构设计保留点
1. **Node.js + TypeScript + Playwright** 作为技术栈。
2. **CLI 和 Web 控制台** 共享 Scraper Core。
3. **Playwright Persistent Context** 保存登录态。强制使用项目内独立目录（如 `.browser-profile/` 并加入 `.gitignore`），严禁使用用户日常 Chrome Profile，以避免锁冲突、日常环境污染与账号隐私泄露。
4. **SQLite** 负责任务和状态持久化。
5. **索引阶段** 和 **正文抓取阶段** 彻底分离。
6. **单线程低速抓取**，加入 2 到 5 秒随机延迟。
7. **Web 控制台** 提供完备的进度、任务表、日志及暂停/重试功能。
8. 输出为 **纯文字 Markdown**，按作者建文件夹存入 Obsidian。

---

## 数据库主键与任务状态设计 (Q&A #1)

为了解决同一回答在不同任务、不同 Top N 配置下状态混淆的问题，我们采用 **方案 A**（推荐方案）：
- `items` 表仅存储内容本体数据，主键为 `answer_id` 或 `article_id`（加前缀 `answer:` 或 `article:`）。
- `task_items` 表存储任务内的抓取状态关联，外键指向 `tasks.id` 和 `items.id`。
**理由**：此方案符合关系型数据库设计范式，完美解耦“内容属性”与“抓取任务状态”。即使针对不同答主的多次任务抓取到了同一篇内容，也可以只保留一份实体数据，同时在多个任务中独立记录其状态。

---

## MVP 阶段划分 (细粒度)

### P0 (核心单篇抓取与基础验证)
- **P0.1**：实现基础 `doctor` 自检（检查 Node >= 22.13、Playwright 安装、Persistent Context 启动、输出目录可写性、知乎连通性）。
- **P0.2**：实现 `login` 命令，显式开启 Playwright 有头浏览器并保存登录态。
- **P0.3**：实现 `save-answer` 单篇回答抓取。
- **P0.4**：实现精准的 `AnswerExtractor` 与严格的 Markdown 输出质量验证（剔除图片、公式保留基础文本或占位、卡片转文本、匿名作者处理等）。
- **P0.5**：完善输出目录规范、严格的文件名安全处理 (`sanitizeFilename`) 与包含新 ID 的 Frontmatter 写入。
- **P0.6**：实现 `save-article` 与对应的 `ArticleExtractor`。

### P1 (主页批量与断点续爬)
- 作者主页回答列表索引抓取 (Answer Indexer)。
- 作者主页文章列表索引抓取。
- 基于 SQLite `tasks` 和 `task_items` 的任务调度与状态持久化。
- 基于数据库的断点续爬与重跑机制。
- 任务队列严格按照 `created_time` (发布时间) 或 `voteup_count` 排序。

### P2 (进阶调度与 Web 控制台)
- Top N 限制抓取。
- 基于 SSE 的 Web 控制台推送进度表、日志和状态变化。
- 基于明确 `failure_code` 的失败分类与指数退避重试（最高重试 3 次）。
- 生成作者主目录下的导航索引 `index.md`。
- 知乎公式完美还原为 LaTeX (`$$...$$`) 优化。

### P3 (完善体验)
- `dry-run` 预览命令。
- Web 控制台“打开输出目录”和“打包下载”。
- 更漂亮美观的 UI 数据大屏展示。

---

## 数据模型设计 (SQLite)

推荐依赖 Node.js **>= 22.13**（此版本起 `node:sqlite` 无需 experimental flag，但官方仍标注为 Release Candidate）。`doctor` 检测低版本时直接提示升级。为避免未来可能替换数据库引擎，所有数据库操作必须集中封装在 `src/db.ts` 中。为保持极简架构，暂不提供其他 DB fallback。

```sql
CREATE TABLE IF NOT EXISTS tasks (
  id TEXT PRIMARY KEY,
  input_url TEXT,
  author_id TEXT,
  author_name TEXT,
  item_types TEXT,          -- answers/articles/all
  output_dir TEXT,
  sort_by TEXT,             -- time/vote
  top_n INTEGER,
  status TEXT,              -- pending/running/paused/success/failed
  total_count INTEGER,
  success_count INTEGER,
  failed_count INTEGER,
  created_at INTEGER,
  updated_at INTEGER
);

-- 内容本体表 (与任务解耦)
CREATE TABLE IF NOT EXISTS items (
  id TEXT PRIMARY KEY,      -- 格式：answer:12345 或 article:67890
  item_type TEXT,           -- answer/article
  author_id TEXT,
  author_name TEXT,

  title TEXT,               -- 对文章：文章标题；对回答：问题标题
  answer_id TEXT,           -- 仅回答有
  question_id TEXT,         -- 仅回答有
  article_id TEXT,          -- 仅文章有

  url TEXT,                 -- 标准归一化后的链接
  question_url TEXT,        -- 仅回答有
  created_time INTEGER,     -- 必须永远代表首次发布时间
  updated_time INTEGER,     -- 必须永远代表最后更新时间
  voteup_count INTEGER,
  comment_count INTEGER
);

-- 任务状态关系表
CREATE TABLE IF NOT EXISTS task_items (
  task_id TEXT,
  item_id TEXT,             -- 关联 items.id
  status TEXT DEFAULT 'pending', -- pending/running/success/failed
  output_path TEXT,
  failure_code TEXT,        -- 限定范围内的错误分类码
  error_message TEXT,       -- 详细的异常堆栈或原因
  created_at INTEGER,
  updated_at INTEGER,
  PRIMARY KEY (task_id, item_id)
);
```

**失败代码约束 (`failure_code`)**：
仅能为以下枚举值之一：`LOGIN_REQUIRED` / `CAPTCHA_REQUIRED` / `RATE_LIMITED` / `API_FAILED` / `DOM_NOT_FOUND` / `CONTENT_EMPTY` / `PERMISSION_DENIED` / `CONTENT_BLOCKED` / `NETWORK_ERROR` / `FILE_MISSING` / `UNKNOWN`。

---

## 核心抓取与提取逻辑

### 1. 链接归一化 (NormalizeUrl)
- 提取并去除无关 Query Params。标准形态：`https://www.zhihu.com/question/{question_id}/answer/{answer_id}`。

### 2. 回答索引器 (Answer Indexer) 与滚动停止条件
**职责**：获取用户回答列表。优先调 API，失效后降级 DOM 滚动。
**滚动兜底停止条件**（满足其一即判定到底）：
1. 连续 N (如 5) 次页面向底滚动操作后，DOM 中没有提取到任何新增的 item。
2. 页面中明确出现了“没有更多内容”等占位提示节点。
3. 提取总数量达到用户设定的 `top_n` 或系统的最大安全扫描上限（防死循环）。
4. 解析出节点但对比队列去重后新增数量为 0，并持续多次。

### 3. 正文提取器分离 (AnswerExtractor 鲁棒性升级)
**AnswerExtractor (回答专用)**
- **第一优先级**：拦截浏览器上下文 API 请求 `answer.content`。
- **第二优先级 (DOM 选择器降级链)**：坚决不写死，依次尝试：
  1. `div.AnswerCard[data-answer-id="xxx"]` 容器。
  2. 匹配页面外链或动态属性的 `answer_id` 定位节点。
  3. 兜底提取页面内的 Hydration JSON (如 `<script id="js-initialData">`) 解析对应实体。
- **第三优先级**：所有尝试失败，标记为 `DOM_NOT_FOUND`，**坚决不生成无意义或污染的 Markdown**。

---

## 输出与 Markdown 设计

### 1. 严格的文件名安全处理 (`sanitizeFilename`)
保存文件前，执行清理：
1. 移除 Windows 非法字符 (`\ / : * ? " < > |`) 及收尾空格。
2. 限制文件名主干最大长度（如 100 字符），防止路径超出 Windows 260 字符限制。
3. 重名处理：强制保留 `_回答ID` 或 `_文章ID` 作为后缀保障唯一性。例如：`2024-05-12-问题标题_123456.md`。

### 2. Markdown Frontmatter 扩充设计
必须增加 `answer_id` 和 `question_id` 以便检索与重建索引。
**回答 (zhihu_answer)**
```markdown
---
type: "zhihu_answer"
answer_id: "12345"
question_id: "67890"
question: "问题标题"
author: "作者名"
date: "YYYY-MM-DD"
url: "回答链接"
question_url: "问题链接"
voteup_count: 1234
source: "zhihu"
---
```

---

## 边界情况与个人偏好设计 (基于 Q&A)
编码时严格遵循以下设定：
1. **纯净输出**：Frontmatter 无额外 Tags。剔除所有图片、评论。引用的知乎卡片转为 Markdown 纯文本链接。
2. **公式处理**：P0 阶段仅尽力保留公式基础文本或占位符，不强求渲染。完美的 LaTeX 转换优化放在 P2/P3 阶段。
3. **状态与更新机制**：
   - 增量抓取比对 `updated_time`，有更新则重新覆盖本地文件。
   - 默认信赖 SQLite 状态，但如果 `task_items.status = success` 且对应的 `output_path` 文件丢失，将标记 `failure_code = FILE_MISSING`，并在 Web 控制台提示用户选择跳过还是重新导出。
   - 启动时自动将遗留的 `running` 重置为 `pending`。
4. **特殊内容处理**：
   - 被屏蔽问题直接跳过，分类为 `CONTENT_BLOCKED` 或 `PERMISSION_DENIED`。
   - 匿名回答的 author 字段统一为 `"匿名用户"`。
   - 相互引用保持原始网络外链。
5. **稳定性与体验**：
   - **重试策略**：指数退避（2s、4s、8s），最高 3 次重试。
   - **日志**：每天持久化本地 `.log` 文件。
   - **终端**：单行进度条输出，拒绝刷屏。
   - **账号**：专注独立目录的单账号 Persistent Context。

## 用户审批
请查看以上最终修改后的计划。如果您确认无误，我将开始 P0 代码编写。
