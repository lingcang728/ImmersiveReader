/**
 * Map raw backend/IPC error strings to user-facing Chinese copy.
 *
 * Rust commands surface stable UPPER_SNAKE codes (and a few English sentences
 * from shared validation paths). Toast/notice UI must never print those raw
 * to the user — they get a short Chinese sentence here, while the raw string
 * stays available via rawErrorMessage() for console/diagnostic use.
 */

export function rawErrorMessage(error: unknown): string {
	if (error instanceof Error) return error.message || String(error);
	return String(error);
}

const ERROR_COPY: [pattern: RegExp, message: string][] = [
	// Markdown file access
	[/MARKDOWN_PATH_EMPTY/, "文件路径为空"],
	[/MARKDOWN_PATH_TYPE_NOT_ALLOWED/, "不支持的文件类型"],
	[/MARKDOWN_PATH_NOT_ABSOLUTE|MARKDOWN_PATH_NOT_ALLOWED|PATH_OUTSIDE_MANAGED_ROOT/, "该路径不在允许的范围内"],
	[/MARKDOWN_PATH_NOT_FILE/, "目标不是 Markdown 文件"],
	[/MARKDOWN_FILE_TOO_LARGE|FILE_TOO_LARGE/, "文件太大，无法打开"],
	[/Failed to decode UTF-16/, "文件编码无法识别"],
	[/Failed to encode as GB18030/, "无法按 GB18030 编码保存"],
	[/STATE_FILE_TOO_LARGE/, "状态文件过大"],
	// Library
	[/Book not found/, "书目不存在或已被移出书架，请刷新"],
	[/Book resolves outside the library root|Chapter resolves outside/, "书库数据位置异常"],
	[/Refusing to operate on the library root/, "不能对书库根目录执行此操作"],
	[/Book provenance does not match/, "书目来源信息不一致"],
	[/Not a file/, "目标不是文件"],
	[/LIBRARY_DEPTH_LIMIT/, "文件夹层级过深"],
	// Importer
	[/Import source must be a folder/, "请选择文件夹"],
	[/no Markdown files/, "所选文件夹中没有 Markdown 文件"],
	[/no readable Markdown files/, "所选文件夹中没有可读取的 Markdown 文件"],
	[/Import finalize failed/, "导入收尾失败，请重试"],
	// Tasks / control plane
	[/TASK_NOT_FOUND/, "任务不存在或已结束"],
	[/TASK_NOT_QUEUED/, "任务不在等待队列中"],
	[/TASK_NOT_RETRYABLE/, "该任务当前不能重试"],
	[/TASK_KIND_CONFLICT/, "任务类型冲突"],
	[/REVISION_CONFLICT|EVENT_SEQUENCE_CONFLICT/, "任务状态已变化，请刷新后重试"],
	[/IDEMPOTENCY_KEY_REUSED/, "重复请求已忽略"],
	[/INVALID_TASK_EVENT/, "任务事件数据异常"],
	[/INVALID_ENGINE_INSTANCE|ENGINE_UNAVAILABLE|RUNTIME_UNAVAILABLE/, "运行环境未就绪，请重启应用"],
	[/ENGINE_BUSY/, "引擎正忙，请稍后重试"],
	[/UNKNOWN_ENGINE/, "未知的任务引擎"],
	[/ZHIHU_ENGINE_UNRESPONSIVE|ZHIHU_ENGINE_UNSUPPORTED/, "知乎组件异常，请重启应用"],
	[/ZHIHU_REMOTE_TASK_GONE/, "知乎任务已在远端消失，请刷新"],
	[/ZHIHU_LOGIN_CLEAR_FAILED|ZHIHU_LOGIN_START_FAILED|ZHIHU_LOGIN_STATUS/, "知乎登录操作失败，请重试"],
	[/INVALID_ZHIHU_PEOPLE_ID/, "知乎主页标识无效"],
	[/INVALID_ZHIHU_TOP_N/, "知乎数量参数无效"],
	[/INPUT_CHANGED|INPUT_MISSING/, "输入文件已变更或丢失"],
	[/TASK_CONTRACT_MISSING/, "任务信息已丢失"],
	[/TASK_RESULT_NOT_READY/, "结果尚未生成，请稍后查看"],
	[/TASK_DISCARD_PENDING/, "任务正在清理中"],
	[/BUDGET_CONFIRMATION_REQUIRED/, "需要确认预算上限"],
	[/PROMPT_BUDGET_EXCEEDED/, "超出提示词预算"],
	[/PUBLISH_FAILED/, "发布失败"],
	[/PUBLISH_RECOVERY_REQUIRED/, "发布需要恢复，请重试"],
	[/UPSTREAM_UNAUTHORIZED|SECRET_MISSING/, "缺少或无效的 API Key"],
	[/RATE_LIMITED/, "请求过于频繁，请稍后重试"],
	[/UPSTREAM_TIMEOUT|LOCAL_TIMEOUT|PROBE_TIMEOUT/, "请求超时，请检查网络"],
	[/UPSTREAM_UNAVAILABLE|LOCAL_NETWORK/, "网络不可用"],
	[/TRANSCRIPTION_FAILED/, "转写失败"],
	[/MODEL_LOAD_FAILED|MODEL_INCOMPATIBLE/, "模型不可用"],
	[/PIPELINE_INCOMPATIBLE|CONFIG_INCOMPATIBLE/, "任务配置与当前版本不兼容"],
	[/LOCAL_IO/, "本地文件读写失败"],
	[/INVALID_TASK_SPEC|INVALID_TASK_CONTROL|INVALID_ARGUMENT|INVALID_REQUEST_ID/, "请求参数无效"],
	[/WORKER_NOT_RUNNING|INVALID_WORKER_STREAM/, "后台进程异常"],
	// Trash
	[/NOT_FOUND/, "回收站中未找到该条目"],
	[/INVALID_TRASH_JOURNAL|INVALID_TRASH_ENTRY/, "回收站记录损坏"],
	[/CONFLICT/, "操作冲突，请刷新后重试"],
	// Reader server / flow
	[/READER_SESSION_LIMIT/, "连读会话数已达上限"],
	[/Reader server did not bind|尚未编译/, "连读组件未就绪"],
	// Settings / credentials
	[/Windows Credential Manager is unavailable|Credential/, "系统凭据管理不可用"],
	[/settings\.json changed on disk/, "设置文件被外部修改，请重试"],
	[/Unsupported settings schema version/, "设置文件版本不兼容"],
	[/Library root must be absolute/, "书库路径无效"],
	[/Unsupported manifest schema version|Manifest |Unsupported book source/, "书目数据不兼容"],
	[/Migration run was not started/, "迁移尚未开始"],
	[/Disk space lookup is unsupported/, "无法查询磁盘空间"],
	// Podcast cache
	[/Invalid Podcast recovery metadata|does not match the task/, "播客任务恢复信息无效"],
	[/Podcast task id must contain/, "任务标识无效"],
	// Misc process errors
	[/PROCESS_THREADS_NOT_FOUND|PRIMARY_THREAD_NOT_FOUND|SetInformationJobObject failed|UNEXPECTED_SUSPEND_COUNT/, "进程管理异常"],
	[/TLS_CRYPTO_PROVIDER_UNAVAILABLE/, "安全组件不可用"],
	[/Only the configured/, "不允许启动该工具"],
	// Mobile / platform limits
	[/Android storage roots|Android app (data|cache|local data) directory/, "存储目录初始化失败，请重启应用"],
	[/Folder picker is not implemented on mobile/, "暂不支持选择文件夹，请改用文件选择"],
	[/FILE_NAME_UNAVAILABLE/, "无法读取文件名"],
	[/failed to open file/, "无法打开所选文件"],
];

/** Returns true when the message is already user-facing Chinese copy. */
function looksLocalized(message: string): boolean {
	return /[一-鿿]/.test(message);
}

/**
 * User-facing description of an IPC/backend failure. Never returns raw
 * UPPER_SNAKE codes or English sentences to the UI; callers that need the
 * detail should log rawErrorMessage() to the console.
 */
export function describeError(error: unknown): string {
	const raw = rawErrorMessage(error).trim();
	if (!raw) return "操作未完成";
	if (looksLocalized(raw)) return raw;
	for (const [pattern, message] of ERROR_COPY) {
		if (pattern.test(raw)) return message;
	}
	// Unrecognized technical string — keep the UI clean; diagnostics stay in
	// the console via the caller's console.error.
	return "操作未完成，请稍后重试";
}

/** Log the raw error and return the localized description in one step. */
export function reportError(context: string, error: unknown): string {
	console.error(`[${context}]`, error);
	return describeError(error);
}
