import { invoke } from "@tauri-apps/api/core";
import { listen, type EventCallback, type UnlistenFn } from "@tauri-apps/api/event";

/** Rejected when an IPC call does not answer within its budget. */
export class IpcTimeoutError extends Error {
	readonly command: string;
	readonly timeoutMs: number;

	constructor(command: string, timeoutMs: number) {
		super(`IPC 调用超时：${command}（${timeoutMs}ms）`);
		this.name = "IpcTimeoutError";
		this.command = command;
		this.timeoutMs = timeoutMs;
	}
}

export type IpcErrorKind = "timeout" | "unavailable" | "backend";

/** 粗略区分错误来源：超时 / Tauri 桥不可用 / 后端返回的业务错误。 */
export function classifyIpcError(error: unknown): IpcErrorKind {
	if (error instanceof IpcTimeoutError) return "timeout";
	const message = error instanceof Error ? error.message : String(error);
	if (/__TAURI_INTERNALS__|tauri.*not defined|ipc.*unavailable/i.test(message)) {
		return "unavailable";
	}
	return "backend";
}

// Per-command response budgets; anything unlisted gets the default.
const COMMAND_TIMEOUT_MS: Record<string, number> = {
	// File IO can be slow on very large documents.
	read_markdown_file: 30_000,
	save_markdown_file: 30_000,
	get_file_mtime: 30_000,
	// Library scans / bulk moves walk the filesystem.
	scan_library: 60_000,
	import_markdown_folder: 600_000,
	remove_book: 60_000,
	delete_book: 60_000,
	restore_trash_item: 30_000,
	permanently_delete_trash_item: 30_000,
	// Sidecar session setup tears down/starts the reader server.
	open_reader_session: 30_000,
	close_reader_session: 30_000,
	open_task_result: 30_000,
	// Task control round-trips through the control DB and child processes.
	start_podcast_task: 30_000,
	restart_podcast_task: 30_000,
	control_podcast_task: 30_000,
	start_zhihu_task: 30_000,
	control_zhihu_task: 30_000,
	// Logout = remote best-effort request + browser teardown + profile delete.
	clear_zhihu_login: 30_000,
};

const DEFAULT_IPC_TIMEOUT_MS = 15_000;

/**
 * `invoke` with a response budget. A hung backend command rejects instead of
 * leaving callers (isLoading / actionBusy / panelLoading) stuck forever.
 * Note: the timeout only detaches the caller — the backend command keeps
 * running; commands that must never be abandoned should stay on raw invoke.
 */
export function invokeWithTimeout<T>(
	command: string,
	args?: Record<string, unknown>,
	ms = 15_000,
): Promise<T> {
	return new Promise<T>((resolve, reject) => {
		const timer = setTimeout(() => reject(new IpcTimeoutError(command, ms)), ms);
		invoke<T>(command, args).then(
			(value) => {
				clearTimeout(timer);
				resolve(value);
			},
			(error) => {
				clearTimeout(timer);
				reject(error instanceof Error ? error : new Error(String(error)));
			},
		);
	});
}

/** `invoke` using the per-command budget table above. */
export function invokeCommand<T>(
	command: string,
	args?: Record<string, unknown>,
): Promise<T> {
	return invokeWithTimeout<T>(
		command,
		args,
		COMMAND_TIMEOUT_MS[command] ?? DEFAULT_IPC_TIMEOUT_MS,
	);
}

/**
 * Single cleanup shape for `listen()`: returns an unsubscribe function that
 * is safe to call synchronously — if the listener has not resolved yet it is
 * unsubscribed the moment registration lands.
 */
export function listenManaged<T>(
	event: string,
	handler: EventCallback<T>,
): () => void {
	let unlisten: UnlistenFn | null = null;
	let disposed = false;
	listen<T>(event, handler).then((fn) => {
		if (disposed) {
			fn();
		} else {
			unlisten = fn;
		}
	});
	return () => {
		disposed = true;
		unlisten?.();
		unlisten = null;
	};
}
