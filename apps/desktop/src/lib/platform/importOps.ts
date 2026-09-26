import { invokeCommand } from "../ipc";

/**
 * Async backend import pipeline.
 *
 * The backend owns the whole import operation (`begin_import` returns an
 * operation id; progress is polled via `get_import_status`; `cancel_import`
 * requests cooperative cancellation). The frontend never stages `content://`
 * documents itself anymore — URIs are handed to the backend, which reads them
 * through the content provider, so a wedged staging call can no longer leave
 * the UI hanging.
 */

export type ImportKind = "directory" | "files" | "uris" | "archive";

export type ImportState =
	| "pending"
	| "running"
	| "completed"
	| "failed"
	| "cancelled";

export interface ImportIssue {
	/** Affected path/URI when the backend reports one. */
	path: string | null;
	message: string;
}

export interface ImportStatus {
	operationId: string;
	state: ImportState;
	phase: string;
	totalFiles: number;
	completedFiles: number;
	currentFile: string | null;
	issues: ImportIssue[];
	bookId: string | null;
	error: string | null;
}

// A `type` alias (not interface) so the object literal is assignable to
// `Record<string, unknown>` for invokeCommand.
export type BeginImportArgs = {
	kind: ImportKind;
	path?: string;
	paths?: string[];
	uris?: string[];
	title?: string;
};

type RawStatus = Record<string, unknown>;

function pick(raw: RawStatus, ...keys: string[]): unknown {
	for (const key of keys) {
		if (key in raw) return raw[key];
	}
	return undefined;
}

function pickString(raw: RawStatus, ...keys: string[]): string | null {
	const value = pick(raw, ...keys);
	return typeof value === "string" && value ? value : null;
}

function pickNumber(raw: RawStatus, ...keys: string[]): number {
	const value = pick(raw, ...keys);
	return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

function normalizeState(value: unknown): ImportState {
	if (typeof value !== "string") return "running";
	switch (value.toLowerCase()) {
		case "pending":
		case "queued":
			return "pending";
		case "completed":
		case "complete":
		case "done":
		case "succeeded":
		case "success":
			return "completed";
		case "failed":
		case "error":
			return "failed";
		case "cancelled":
		case "canceled":
			return "cancelled";
		default:
			return "running";
	}
}

function normalizeIssues(raw: unknown): ImportIssue[] {
	if (!Array.isArray(raw)) return [];
	const issues: ImportIssue[] = [];
	for (const item of raw) {
		if (typeof item === "string") {
			issues.push({ path: null, message: item });
			continue;
		}
		if (typeof item !== "object" || item === null) continue;
		const bag = item as Record<string, unknown>;
		const message = pickString(bag, "message", "error", "reason");
		if (!message) continue;
		issues.push({
			path: pickString(bag, "path", "uri", "file"),
			message,
		});
	}
	return issues;
}

/**
 * Normalize whatever the backend serializes into a stable `ImportStatus`.
 * Accepts snake_case and camelCase spellings; absent fields degrade to a
 * sane in-flight snapshot so progress UI can render partial payloads.
 */
export function normalizeImportStatus(
	raw: unknown,
	fallbackOperationId = "",
): ImportStatus {
	const bag: RawStatus =
		typeof raw === "object" && raw !== null ? (raw as RawStatus) : {};
	// `result` carries the flattened import outcome (manifest fields +
	// `issues`) once the op succeeded — dig there for bookId/issues too.
	const resultBag: RawStatus =
		typeof bag.result === "object" && bag.result !== null
			? (bag.result as RawStatus)
			: {};
	const issues = normalizeIssues(pick(bag, "issues", "warnings", "errors"));
	return {
		operationId:
			pickString(bag, "op_id", "opId", "operation_id", "operationId", "id") ??
			fallbackOperationId,
		state: normalizeState(pick(bag, "state", "status", "lifecycle_state")),
		phase: pickString(bag, "phase", "step") ?? "",
		totalFiles: pickNumber(bag, "total_files", "totalFiles", "files_total"),
		completedFiles: pickNumber(
			bag,
			"completed_files",
			"completedFiles",
			"done_files",
			"doneFiles",
			"files_done",
		),
		currentFile: pickString(bag, "current_file", "currentFile"),
		issues: issues.length
			? issues
			: normalizeIssues(pick(resultBag, "issues", "warnings")),
		bookId:
			pickString(bag, "book_id", "bookId") ??
			pickString(resultBag, "book_id", "bookId"),
		// The backend reports terminal detail in `message` (only ever set on
		// failure/cancel) — treat it as the error text.
		error: pickString(bag, "error", "error_message", "message"),
	};
}

export function isTerminalImportState(state: ImportState): boolean {
	return state === "completed" || state === "failed" || state === "cancelled";
}

const PHASE_LABELS: Record<string, string> = {
	pending: "等待开始",
	queued: "等待开始",
	running: "正在导入",
	staging: "正在读取文件",
	reading: "正在读取文件",
	importing: "正在导入",
	processing: "正在导入",
	indexing: "正在建立索引",
	finalizing: "正在收尾",
	completed: "导入完成",
	failed: "导入失败",
	cancelled: "已取消",
};

/** Short Chinese phase label for the progress card. */
export function importPhaseLabel(status: ImportStatus): string {
	const key = (status.phase || status.state).toLowerCase();
	const label = PHASE_LABELS[key];
	if (label) return label;
	// Unknown backend phases render verbatim when they look like a Chinese or
	// short latin tag instead of falling back to nothing.
	if (status.phase) return status.phase;
	return "正在导入";
}

/**
 * Kick off a backend import operation. `begin_import` itself is fast — the
 * heavy work continues behind the returned operation's `opId`. The backend
 * signature takes a single `request` object (Tauri maps JS camelCase arg
 * names to Rust snake_case, so the payload must live under `request`).
 */
export async function beginImport(
	args: BeginImportArgs,
): Promise<{ operationId: string }> {
	const raw = await invokeCommand<unknown>("begin_import", { request: args });
	const status = normalizeImportStatus(raw);
	if (!status.operationId) {
		throw new Error("begin_import 未返回 operationId");
	}
	return { operationId: status.operationId };
}

/**
 * Poll an op. The backend answers `Option<ImportOperation>` — `null` means
 * the id is unknown or was evicted, which is terminal from the UI's view.
 */
export function getImportStatus(operationId: string): Promise<ImportStatus> {
	// Rust param is `op_id`, so the JS arg must be `opId`.
	return invokeCommand<unknown>("get_import_status", { opId: operationId }).then(
		(raw) => {
			if (raw === null || raw === undefined) {
				return {
					operationId,
					state: "failed",
					phase: "",
					totalFiles: 0,
					completedFiles: 0,
					currentFile: null,
					issues: [],
					bookId: null,
					error: "导入操作不存在或已过期",
				};
			}
			return normalizeImportStatus(raw, operationId);
		},
	);
}

export function cancelImport(operationId: string): Promise<unknown> {
	return invokeCommand("cancel_import", { opId: operationId });
}

// ===== URI/file-name helpers (pure) =====

const MARKDOWN_EXTENSIONS = new Set(["md", "markdown", "txt"]);

/**
 * Best-effort extension of a file path, content:// URI or display name.
 * `content://` URIs often carry an opaque document id instead of a file name,
 * in which case this returns "" and callers should treat the content as
 * "unknown → backend import" (the backend sniffs the bytes).
 */
export function uriFileExtension(uri: string): string {
	const clean = uri.split(/[?#]/)[0];
	const tail = clean.split("/").pop() ?? "";
	let name = tail;
	try {
		name = decodeURIComponent(tail);
	} catch {
		// Leave the raw tail; a malformed escape is not fatal.
	}
	const dot = name.lastIndexOf(".");
	if (dot <= 0 || dot === name.length - 1) return "";
	const ext = name.slice(dot + 1).toLowerCase();
	return /^[a-z0-9]{1,10}$/.test(ext) ? ext : "";
}

/** True when the URI/path clearly names a markdown text file. */
export function uriLooksLikeMarkdown(uri: string): boolean {
	return MARKDOWN_EXTENSIONS.has(uriFileExtension(uri));
}

/** True when the URI/path clearly names an EPUB book. */
export function uriLooksLikeEpub(uri: string): boolean {
	return uriFileExtension(uri) === "epub";
}

/**
 * Derive a suggested anthology/book title from picked URIs. For a single pick
 * the file base name (minus extension) wins; multi-picks get a neutral
 * Chinese default the user can edit in the title dialog.
 */
export function suggestedImportTitle(uris: readonly string[]): string {
	if (uris.length === 1) {
		const clean = uris[0].split(/[?#]/)[0];
		const tail = clean.split("/").pop() ?? "";
		let name = tail;
		try {
			name = decodeURIComponent(tail);
		} catch {
			// keep raw tail
		}
		const dot = name.lastIndexOf(".");
		const base = dot > 0 ? name.slice(0, dot) : name;
		if (base.trim()) return base.trim();
	}
	return `导入合集 ${new Date().toISOString().slice(0, 10)}`;
}
