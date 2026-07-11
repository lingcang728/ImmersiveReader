# PodcastTranscriber 全面风险审查 & 修复计划

三个分析子智能体对项目中三大核心文件进行了逐行审查，发现了约 50 个潜在问题。以下按严重度分级呈现，并给出修复方案。

> [!IMPORTANT]
> 本次审查主要聚焦你反馈的三大实际痛点：**Markdown 内容丢失/格式问题**、**并行处理异常**、**DeepSeek API 错误导致翻译失败**。

---

## 🔴 Critical — 会直接导致崩溃或数据丢失

### C1. `_run_postprocess_body` 取消路径返回值不一致 → 解包崩溃
- **文件**: [transcribe_podcasts.py](file:///c:/Users/15pro/Desktop/MyProject/PodcastTranscriber/scripts/transcribe_podcasts.py) L2957, L2964, L2992
- **问题**: 取消路径返回单独 `dict`，但调用方以 `result, failures = ...` 解包 → `ValueError`
- **影响**: 当前 `_job_cancelled_or_deleted()` 总是返回 False 所以不会触发，但一旦实现取消功能会立即崩溃
- **修复**: 返回值统一为 `(dict, [])`

### C2. `run_logger` 可能为 None 时直接调用 `.warning()`
- **文件**: [transcribe_podcasts.py](file:///c:/Users/15pro/Desktop/MyProject/PodcastTranscriber/scripts/transcribe_podcasts.py) L3344
- **问题**: CUDA 回退路径中 `run_logger.warning(failure)` 无 None 检查 → `AttributeError` 吞没真正错误
- **修复**: 添加 `if run_logger:` 守卫

### C3. `do_POST` 无 Content-Length 时会阻塞挂起
- **文件**: [run_with_gui.py](file:///c:/Users/15pro/Desktop/MyProject/PodcastTranscriber/scripts/run_with_gui.py) L623-624
- **问题**: `Content-Length == 0` 时 `self.rfile.read()` 无参数 → HTTP handler 线程永久阻塞
- **修复**: `length == 0` 时用 `b""` 替代

---

## 🟠 High — 高概率导致功能异常

### H1. `av.time_base` 属性名错误 → 进度探测永远失败
- **文件**: [transcribe_podcasts.py](file:///c:/Users/15pro/Desktop/MyProject/PodcastTranscriber/scripts/transcribe_podcasts.py) L1953
- **问题**: 应为 `av.TIME_BASE`（大写）→ `AttributeError` 被 except 捕获，PyAV 路径静默失效
- **修复**: `av.time_base` → `av.TIME_BASE`

### H2. `save_json` 回退路径非原子操作
- **文件**: [transcribe_podcasts.py](file:///c:/Users/15pro/Desktop/MyProject/PodcastTranscriber/scripts/transcribe_podcasts.py) L204
- **问题**: `tmp.rename(path)` 在 Windows 上如果目标已存在会失败（不像 `os.replace`）
- **修复**: 改为 `os.replace(str(tmp), str(path))`

### H3. PROCESS 全局变量竞态条件
- **文件**: [run_with_gui.py](file:///c:/Users/15pro/Desktop/MyProject/PodcastTranscriber/scripts/run_with_gui.py) L238-239, L266, L280, L305
- **问题**: `worker_is_running()` 和 `stop_worker()` 读写全局 `PROCESS` 不持锁 → 多线程竞态
- **修复**: 所有 `PROCESS` 访问放入 `PROCESS_LOCK`

### H4. `stop_worker()` 后 PROCESS 不重置为 None
- **文件**: [run_with_gui.py](file:///c:/Users/15pro/Desktop/MyProject/PodcastTranscriber/scripts/run_with_gui.py) L299-331
- **问题**: taskkill 后旧 Popen 对象残留 → 状态判断可能混乱
- **修复**: 在 `stop_worker()` 成功后 `PROCESS = None`

### H5. `render_final_markdown` 语言检测与 `build_turns` 不一致
- **文件**: [polish_interview_markdown.py](file:///c:/Users/15pro/Desktop/MyProject/PodcastTranscriber/scripts/polish_interview_markdown.py) L1778-1779 vs L1517
- **问题**: `build_turns` 用 `inferred_language()`，但 `render_final_markdown` 只看 `detected_language` → turns 按英文构建，markdown 按中文渲染
- **影响**: **直接导致输出内容格式错乱**（你反馈的 Markdown 内容丢失/格式问题可能与此相关）
- **修复**: `render_final_markdown` 使用 `inferred_language()` 保持一致

### H6. `ollama_generate` 完全没有重试机制
- **文件**: [polish_interview_markdown.py](file:///c:/Users/15pro/Desktop/MyProject/PodcastTranscriber/scripts/polish_interview_markdown.py) L1182-1208
- **问题**: DeepSeek 有 3 次重试 + 指数退避，但 Ollama 路径 0 次重试 → 任何网络抖动都导致失败
- **修复**: 添加 2-3 次重试 + URLError 捕获

### H7. `llm_config` dict 被就地修改 — 线程不安全
- **文件**: [polish_interview_markdown.py](file:///c:/Users/15pro/Desktop/MyProject/PodcastTranscriber/scripts/polish_interview_markdown.py) L1280, L1286, L1288
- **问题**: 多个线程共享同一 `llm_config` dict，无锁修改 `_consecutive_errors` → 竞态
- **修复**: 用 `threading.Lock` 保护或给每个文件独立拷贝

### H8. `_dynamic_throttle_deepseek_polish` 替换全局信号量导致竞争
- **文件**: [polish_interview_markdown.py](file:///c:/Users/15pro/Desktop/MyProject/PodcastTranscriber/scripts/polish_interview_markdown.py) L667-669
- **问题**: 直接替换信号量对象 → 旧信号量的 release 不影响新的 acquire → 泄漏
- **修复**: 不替换信号量对象，而是使用 `set_limit` 方法

### H9. retry 操作竞态和清理不充分
- **文件**: [run_with_gui.py](file:///c:/Users/15pro/Desktop/MyProject/PodcastTranscriber/scripts/run_with_gui.py) L534-547
- **问题**: `stop_worker()` 后 `sleep(1)` 不可靠 — taskkill 异步，进程可能未退出
- **修复**: 循环等待 `worker_is_running() == False`（带超时）

---

## 🟡 Medium — 可能导致意外行为

| # | 问题 | 文件 | 修复 |
|---|------|------|------|
| M1 | `save_text` 无原子写入保护（最终 Markdown 可能被截断） | polish L512-514 | 用 tmp + `os.replace` |
| M2 | `main()` 中单个文件异常导致全部中断 | polish L1963-1965 | 循环内 try/except |
| M3 | `record_deepseek_polish_usage` 中 JSON 损坏未处理 | polish L640 | load_json 失败时重置为空 dict |
| M4 | `clean_text` 中文逗号替换过激（破坏英文数字 `1,000`） | polish L804 | 只替换 CJK 字符之间的逗号 |
| M5 | `save_config()` 无并发写入保护 | run_with_gui L80-85 | 添加 `CONFIG_LOCK` |
| M6 | `quick_validate.py` 字符串匹配过于脆弱 | quick_validate L49 | 改用集合包含检查 |
| M7 | `quick_validate.py` 读取二进制文件可能报错 | quick_validate L40-44 | 过滤只读 `.py` 和 `.ps1` |
| M8 | `state["completed_at"]` 重复赋值 | transcribe L3046-3047 | 删除重复行 |
| M9 | `setup_file_logger` handler 竞态 + 文件句柄泄漏 | transcribe L2016-2027 | 检查已有 handlers |
| M10 | 翻译缓存不含 source_language | transcribe L1471-1480 | 缓存匹配加入 source_language |
| M11 | `pp_executor.shutdown(wait=False)` 后台线程清理不完整 | transcribe L3551 | 改为 `wait=True` |
| M12 | 前端 `escapeAttr` 不处理 `"` 字符 | HTML L940-942 | 增加 `"` 转义 |
| M13 | `AGA` → `AGI` 无词边界检查可能误替换 | polish L105 | 使用 `\bAGA\b` 正则 |
| M14 | `process_json` 先写文件再写状态 — 崩溃时状态不一致 | polish L1934, L1948 | 调整写入顺序 |

---

## 🟢 Low — 代码质量/维护性问题

| # | 问题 | 文件 |
|---|------|------|
| L1 | `COMMON_REPLACEMENTS` 链式替换脆弱（`姚顺语→姚顺宇→姚顺雨`） | polish L70-71 |
| L2 | 死代码 `should_continue_previous_turn` L881 | polish L881 |
| L3 | `contains_cjk` 与 `_is_chinese_dominant` 使用不同 CJK 范围 | polish L789/829 |
| L4 | `is_target` 检查线性扫描 → 可用 set 优化 | polish L1879 |
| L5 | `elapsed` 只计最后一次请求时间 | polish L758 |
| L6 | `Mita→Meta` 可能误替换其他词 | polish L125 |
| L7 | `acquire_run_lock` 可能无限递归 | transcribe L651 |
| L8 | 取消路径返回值缺 `file` 字段 → `write_run_summary` KeyError | transcribe L2826 |
| L9 | `manifest` 参数被遮蔽（传入后重新加载） | transcribe L2524/2690 |
| L10 | 前端轮询计时器可能累积 | HTML L513 |
| L11 | config.json ASR model 绝对路径 | config L18 |
| L12 | PowerShell $BatchSize 默认值与 config 不一致 | PS1 L101 |

---

## Proposed Changes

按文件分组进行修复。

### transcribe_podcasts.py（8 处修复）

#### [MODIFY] [transcribe_podcasts.py](file:///c:/Users/15pro/Desktop/MyProject/PodcastTranscriber/scripts/transcribe_podcasts.py)
- **C1**: L2957/2964/2992 — 返回值改为 `(dict, [])`
- **C2**: L3344 — 添加 `if run_logger:` 检查
- **H1**: L1953 — `av.time_base` → `av.TIME_BASE`
- **H2**: L204 — `tmp.rename(path)` → `os.replace(str(tmp), str(path))`
- **M8**: L3046-3047 — 删除重复的 `completed_at` 赋值
- **M11**: L3551 — `shutdown(wait=False)` → `shutdown(wait=True)`
- **L7**: L651 — 递归改循环并加最大重试
- **L8**: L2826 — 补全 cancelled 返回值的字段

### polish_interview_markdown.py（10 处修复）

#### [MODIFY] [polish_interview_markdown.py](file:///c:/Users/15pro/Desktop/MyProject/PodcastTranscriber/scripts/polish_interview_markdown.py)
- **H5**: L1778-1779 — 使用 `inferred_language()` 与 `build_turns` 保持一致
- **H6**: L1182-1208 — `ollama_generate` 添加重试逻辑
- **H7**: L1280/1286/1288 — `llm_config` 修改加线程锁
- **H8**: L667-669 — 信号量替换改为动态调整
- **M1**: L512-514 — `save_text` 改为原子写入
- **M2**: L1963-1965 — `main()` 循环内加 try/except
- **M3**: L640 — `load_json` 失败时重置
- **M4**: L804 — 中文逗号替换只限 CJK 之间
- **M13**: L105 — `AGA` 改用正则
- **L1**: L70-71 — 合并链式替换

### run_with_gui.py（5 处修复）

#### [MODIFY] [run_with_gui.py](file:///c:/Users/15pro/Desktop/MyProject/PodcastTranscriber/scripts/run_with_gui.py)
- **C3**: L623-624 — 空 Content-Length 保护
- **H3**: L238-305 — PROCESS 访问加锁
- **H4**: L299-331 — stop 后重置 PROCESS = None
- **H9**: L534-547 — retry 等待进程退出
- **M5**: L80-85 — save_config 加并发锁

### quick_validate.py（2 处修复）

#### [MODIFY] [quick_validate.py](file:///c:/Users/15pro/Desktop/MyProject/PodcastTranscriber/scripts/quick_validate.py)
- **M6**: L49 — 改用更宽松的集合包含检查
- **M7**: L40-44 — 过滤二进制文件

### podcast-transcriber-v2.html（1 处修复）

#### [MODIFY] [podcast-transcriber-v2.html](file:///c:/Users/15pro/Desktop/MyProject/PodcastTranscriber/podcast-transcriber-v2.html)
- **M12**: L940-942 — escapeAttr 增加 `"` 转义

---

## 不修复的项目

以下问题评估为低风险且修复代价较高，暂不修改：

- **翻译信号量缓存** (H5-transcribe): 当前单实例运行，风险可控
- **前端轮询计时器累积** (L10): 不影响功能
- **config.json 绝对路径** (L11): 用户本地配置，不影响其他人
- **config.json 与 config.example.json 键不一致**: 不影响运行（代码有 fallback 默认值）
- **API Key 明文**: config.json 已在 .gitignore 中，不会被提交

---

## Verification Plan

### Automated Tests
```powershell
# 语法检查
.\.venv\Scripts\python.exe -m py_compile scripts\transcribe_podcasts.py
.\.venv\Scripts\python.exe -m py_compile scripts\polish_interview_markdown.py
.\.venv\Scripts\python.exe -m py_compile scripts\run_with_gui.py

# 结构验证
.\.venv\Scripts\python.exe scripts\quick_validate.py

# Dry run
.\.venv\Scripts\python.exe scripts\transcribe_podcasts.py --dry-run

# 单元测试
.\.venv\Scripts\python.exe -m pytest tests\test_polish_interview_markdown.py -q
```

### Manual Verification
- 启动 GUI 确认正常
- 放入一个英文 + 一个中文音频文件，验证端到端流程
