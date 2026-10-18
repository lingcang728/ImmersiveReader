// Data access for the Android mobile surface.
//
// Every call goes through `$lib/ipc` (budgeted `invokeCommand`) so a wedged
// command rejects instead of hanging a phone UI forever. This module is the
// only place the mobile shell/readers should talk to Tauri; keeping it in one
// file makes the IPC surface auditable and easy to mock in the browser QA
// harness (window.__TAURI_INTERNALS__.invoke).

import { invokeCommand, invokeWithTimeout } from '$lib/ipc';
import { describeError } from '$lib/errors';
import { open } from '@tauri-apps/plugin-dialog';
import { openUrl } from '@tauri-apps/plugin-opener';
import type {
	BookDetail,
	BookReadingState,
	FileReadingState,
	MobileLibrarySnapshot,
	RawPlatformCapabilities,
	RecentFile,
	StorageUsage,
	CacheClearResult,
	TrashItem,
} from './contracts';
import {
	beginImport,
	cancelImport,
	getImportStatus,
	isTerminalImportState,
	suggestedImportTitle,
	uriLooksLikeEpub,
	uriLooksLikeMarkdown,
	type BeginImportArgs,
	type ImportStatus,
} from '$lib/platform/importOps';

export {
	beginImport,
	cancelImport,
	getImportStatus,
	isTerminalImportState,
	suggestedImportTitle,
	uriLooksLikeEpub,
	uriLooksLikeMarkdown,
};
export type { BeginImportArgs, ImportStatus };

// EPUB reader commands (single source of truth lives in $lib/epub/ipc).
export {
	getReadableChapter,
	getReaderLocator,
	saveReaderLocator,
	listBookmarks,
	addBookmark,
	removeBookmark,
	searchBook,
} from '$lib/epub/ipc';

/** Friendly error text for UI toasts/banners. */
export function errorText(error: unknown): string {
	return describeError(error);
}

// ----- Library ------------------------------------------------------------

/** Walk the configured library and return the shelf snapshot. */
export function scanLibrary(): Promise<MobileLibrarySnapshot> {
	return invokeCommand<MobileLibrarySnapshot>('scan_library');
}

/** Open a book: manifest + saved progress + publication descriptor. */
export function openBook(bookId: string): Promise<BookDetail> {
	return invokeCommand<BookDetail>('open_book', { bookId });
}

/** Absolute filesystem path of one Markdown chapter. */
export function getBookChapterPath(bookId: string, chapterId: string): Promise<string> {
	return invokeCommand<string>('get_book_chapter_path', { bookId, chapterId });
}

export interface ReadMarkdownResult {
	content: string;
	encoding: string;
}

/** Read one Markdown file with the backend's size/encoding guards. */
export function readMarkdown(path: string): Promise<ReadMarkdownResult> {
	return invokeCommand<ReadMarkdownResult>('read_markdown_file', { path });
}

/** Persist library-book progress (chapter + scroll ratio + read set). */
export function saveBookProgress(bookId: string, progress: BookReadingState): Promise<void> {
	return invokeCommand<void>('save_book_progress', { bookId, progress });
}

// ----- Single Markdown file ----------------------------------------------

export function loadFileReadingState(path: string): Promise<FileReadingState> {
	return invokeCommand<FileReadingState>('load_reading_state', { path });
}

export function saveFileReadingState(path: string, state: FileReadingState): Promise<void> {
	return invokeCommand<void>('save_reading_state', { path, state });
}

/** Recent single-file Markdown list (backend AppData store). */
export async function loadRecentFiles(): Promise<RecentFile[]> {
	// Rust serializes this struct as snake_case (no rename_all on
	// RecentFilesLoad); tolerate camelCase too so a future rename cannot
	// silently empty the mobile recent list.
	const raw = await invokeCommand<Record<string, unknown>>('load_recent_files');
	const json = typeof raw?.json === 'string' ? raw.json : '';
	try {
		const parsed: unknown = JSON.parse(json || '[]');
		if (!Array.isArray(parsed)) return [];
		const files: RecentFile[] = [];
		for (const item of parsed) {
			if (typeof item !== 'object' || item === null) continue;
			const bag = item as Record<string, unknown>;
			if (typeof bag.path !== 'string' || typeof bag.name !== 'string') continue;
			files.push({
				path: bag.path,
				name: bag.name,
				openedAt: typeof bag.openedAt === 'number' ? bag.openedAt : 0,
			});
		}
		return files.slice(0, 8);
	} catch {
		return [];
	}
}

export function saveRecentFiles(files: RecentFile[]): Promise<string> {
	return invokeCommand<string>('save_recent_files', {
		json: JSON.stringify(files.slice(0, 8)),
	});
}

// ----- Open-with / share intents -----------------------------------------

/** Drain the Android VIEW/SEND queue (cold start + every resume). */
export function takePendingOpenUris(): Promise<string[]> {
	return invokeCommand<string[]>('take_pending_open_uris');
}

// ----- Import -------------------------------------------------------------

/**
 * Open the Android document picker. Returns SAF `content://` URIs (the plugin
 * hands them back unchanged); an empty list means the user cancelled.
 */
export async function pickBooks(): Promise<string[]> {
	const selected = await open({
		multiple: true,
		title: '选择要导入的书籍',
		filters: [{ name: '书籍与文档', extensions: ['md', 'markdown', 'txt', 'epub'] }],
	});
	return (Array.isArray(selected) ? selected : selected ? [selected] : []).filter(
		(uri): uri is string => typeof uri === 'string' && uri.length > 0,
	);
}

/** Start a backend import for picked URIs. */
export function beginUriImport(uris: string[], title?: string): Promise<{ operationId: string }> {
	return beginImport({ kind: 'uris', uris, title });
}

// ----- Shelf maintenance --------------------------------------------------

export function removeBookToTrash(bookId: string): Promise<string> {
	return invokeCommand<string>('remove_book', { bookId });
}

export function deleteBookPermanently(bookId: string): Promise<string> {
	return invokeCommand<string>('delete_book', { bookId });
}

export function listTrash(): Promise<TrashItem[]> {
	return invokeCommand<TrashItem[]>('list_trash');
}

export function restoreTrashItem(trashId: string, expectedRevision: number): Promise<unknown> {
	return invokeCommand('restore_trash_item', { trashId, expectedRevision });
}

export function permanentlyDeleteTrashItem(
	trashId: string,
	expectedRevision: number,
): Promise<unknown> {
	return invokeCommand('permanently_delete_trash_item', { trashId, expectedRevision });
}

// ----- Settings / storage -------------------------------------------------

export interface AppSettingsView {
	schemaVersion: number;
	libraryRoot: string;
}

export function getAppSettings(): Promise<AppSettingsView> {
	return invokeCommand<AppSettingsView>('get_app_settings');
}

export function getStorageUsage(): Promise<StorageUsage> {
	return invokeCommand<StorageUsage>('get_storage_usage');
}

export function clearSafeCache(
	categories: string[] = ['general_temporary'],
): Promise<CacheClearResult> {
	return invokeWithTimeout<CacheClearResult>(
		'clear_safe_cache',
		{ categories, taskIds: null },
		120_000,
	);
}

/** App version (dynamic import keeps the Tauri app module out of the hot path). */
export async function getAppVersion(): Promise<string> {
	const { getVersion } = await import('@tauri-apps/api/app');
	return getVersion();
}

// ----- Platform -----------------------------------------------------------

export function getRawPlatformCapabilities(): Promise<RawPlatformCapabilities> {
	return invokeCommand<RawPlatformCapabilities>('get_platform_capabilities');
}

/** Volume-key capture (only meaningful while a reader is foregrounded). */
export function setVolumeKeyCapture(enabled: boolean): Promise<boolean> {
	return invokeCommand<boolean>('set_volume_key_capture', { enabled });
}

// ----- External links -----------------------------------------------------

const EXTERNAL_PROTOCOLS = /^https?:\/\//i;

/** Only explicit http(s) links ever leave the app. */
export async function openExternalUrl(url: string): Promise<void> {
	const trimmed = url.trim();
	if (!EXTERNAL_PROTOCOLS.test(trimmed)) {
		throw new Error('只允许打开 http/https 链接');
	}
	await openUrl(trimmed);
}
