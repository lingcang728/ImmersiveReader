// Pure import-state mapping for the Android mobile surface.
//
// `api.ts` hands the shell a normalized `ImportStatus`; the import sheet and
// the shell both derive their labels/progress from these functions so the
// polling UI stays deterministic and unit-testable (no DOM/Svelte imports).

import type { ImportStatus } from './api';
import type { BookSummary } from './contracts';

/** Poll cadence for `get_import_status` while an import is running. */
export const IMPORT_POLL_INTERVAL_MS = 400;

export type ImportIssue = ImportStatus['issues'][number];

export type ImportTone = 'idle' | 'running' | 'success' | 'error' | 'neutral';

export interface ImportView {
	tone: ImportTone;
	title: string;
	phaseLabel: string;
	/** 0..100, or null while the backend has not reported a file total. */
	percent: number | null;
	filesLabel: string;
	currentFileLabel: string | null;
	issues: ImportIssue[];
	errorText: string;
	running: boolean;
	terminal: boolean;
}

export interface ImportedBook {
	bookId: string;
	title: string;
}

const PHASE_LABELS: Record<string, string> = {
	pending: '等待开始',
	queued: '等待开始',
	running: '正在导入',
	staging: '正在读取文件',
	reading: '正在读取文件',
	importing: '正在导入',
	processing: '正在导入',
	indexing: '正在建立索引',
	finalizing: '正在收尾',
	completed: '导入完成',
	failed: '导入失败',
	cancelled: '已取消',
};

function phaseLabel(status: ImportStatus): string {
	const key = (status.phase || status.state).trim().toLowerCase();
	const known = PHASE_LABELS[key];
	if (known) return known;
	if (status.phase.trim()) return status.phase.trim();
	return '正在导入';
}

export function isRunningImport(status: ImportStatus | null): boolean {
	return status !== null && !isTerminalImportState(status.state);
}

export function isTerminalImportState(state: ImportStatus['state']): boolean {
	return state === 'completed' || state === 'failed' || state === 'cancelled';
}

/**
 * Integer progress for the sheet's bar. `null` means "indeterminate": the
 * backend has not yet reported how many files the operation will touch.
 */
export function importPercent(status: ImportStatus | null): number | null {
	if (!status) return null;
	if (status.state === 'completed') return 100;
	if (!Number.isFinite(status.totalFiles) || status.totalFiles <= 0) return null;
	const done = Math.min(Math.max(0, status.completedFiles), status.totalFiles);
	return Math.round((done / status.totalFiles) * 100);
}

/** Display name of the URI/path currently being imported. */
export function importCurrentFileName(currentFile: string | null): string | null {
	if (!currentFile) return null;
	const clean = currentFile.split(/[?#]/)[0];
	const tail = clean.split('/').pop() ?? '';
	if (!tail) return null;
	try {
		const decoded = decodeURIComponent(tail);
		return decoded || null;
	} catch {
		return tail || null;
	}
}

export function importFilesLabel(status: ImportStatus | null): string {
	if (!status) return '';
	if (status.state === 'completed') {
		return status.totalFiles > 0 ? `共 ${status.totalFiles} 个文件` : '已完成';
	}
	if (status.totalFiles > 0) {
		const done = Math.min(Math.max(0, status.completedFiles), status.totalFiles);
		return `${done} / ${status.totalFiles}`;
	}
	return '正在读取文件…';
}

/** Full view model for the import sheet (and the shell's intent import). */
export function importStatusView(status: ImportStatus | null): ImportView {
	const issues = status?.issues ?? [];
	if (!status) {
		return {
			tone: 'idle',
			title: '导入书籍',
			phaseLabel: '',
			percent: null,
			filesLabel: '',
			currentFileLabel: null,
			issues: [],
			errorText: '',
			running: false,
			terminal: false,
		};
	}
	const currentFileLabel = importCurrentFileName(status.currentFile);
	switch (status.state) {
		case 'completed':
			return {
				tone: 'success',
				title: '导入完成',
				phaseLabel: '导入完成',
				percent: 100,
				filesLabel: importFilesLabel(status),
				currentFileLabel: null,
				issues,
				errorText: '',
				running: false,
				terminal: true,
			};
		case 'failed':
			return {
				tone: 'error',
				title: '导入失败',
				phaseLabel: '导入失败',
				percent: importPercent(status),
				filesLabel: importFilesLabel(status),
				currentFileLabel: null,
				issues,
				errorText: status.error?.trim() || '导入未完成，请重试',
				running: false,
				terminal: true,
			};
		case 'cancelled':
			return {
				tone: 'neutral',
				title: '已取消导入',
				phaseLabel: '已取消',
				percent: importPercent(status),
				filesLabel: importFilesLabel(status),
				currentFileLabel: null,
				issues,
				errorText: '',
				running: false,
				terminal: true,
			};
		default:
			return {
				tone: 'running',
				title: '正在导入',
				phaseLabel: phaseLabel(status),
				percent: importPercent(status),
				filesLabel: importFilesLabel(status),
				currentFileLabel,
				issues,
				errorText: '',
				running: true,
				terminal: false,
			};
	}
}

/** Books that appeared since `beforeIds` was captured. */
export function newBooksAfter(
	beforeIds: ReadonlySet<string>,
	books: readonly BookSummary[],
): BookSummary[] {
	return books.filter((book) => !beforeIds.has(book.bookId));
}

/**
 * The one book an import just shelved, when the result is unambiguous.
 *
 * Prefers the operation's own `bookId`; falls back to a single new book in
 * the refreshed snapshot. Multiple new books mean the caller cannot guess
 * which one to open, so it returns null and the shelf shows them instead.
 */
export function pickImportedBook(
	status: ImportStatus,
	beforeIds: ReadonlySet<string>,
	books: readonly BookSummary[],
): ImportedBook | null {
	if (status.state !== 'completed') return null;
	if (status.bookId) {
		const match = books.find((book) => book.bookId === status.bookId);
		if (match) return { bookId: match.bookId, title: match.title };
		const fresh = newBooksAfter(beforeIds, books);
		const fallback = fresh.find((book) => book.bookId === status.bookId);
		if (fallback) return { bookId: fallback.bookId, title: fallback.title };
		if (fresh.length === 0) return { bookId: status.bookId, title: '导入的书' };
		return null;
	}
	const fresh = newBooksAfter(beforeIds, books);
	if (fresh.length === 1) return { bookId: fresh[0].bookId, title: fresh[0].title };
	return null;
}

export interface ImportToast {
	text: string;
	kind: 'info' | 'success' | 'error';
}

/** Toast copy shown once an import reaches a terminal state. */
export function importToast(status: ImportStatus, imported: ImportedBook | null): ImportToast {
	if (status.state === 'cancelled') return { text: '已取消导入', kind: 'info' };
	if (status.state === 'failed') {
		return { text: status.error?.trim() || '导入失败，请重试', kind: 'error' };
	}
	if (status.state !== 'completed') return { text: '导入进行中', kind: 'info' };
	if (status.issues.length > 0 && imported === null) {
		return { text: `导入完成，${status.issues.length} 个文件未能导入`, kind: 'info' };
	}
	if (status.issues.length > 0 && imported) {
		return { text: `已导入《${imported.title}》，${status.issues.length} 个文件未能导入`, kind: 'info' };
	}
	if (imported) return { text: `已导入《${imported.title}》`, kind: 'success' };
	return { text: '导入完成', kind: 'success' };
}

/** First N issues for the sheet; the rest are summarized as a count. */
export function visibleImportIssues(
	issues: readonly ImportIssue[],
	limit = 5,
): { shown: ImportIssue[]; hiddenCount: number } {
	const safeLimit = Number.isFinite(limit) ? Math.max(0, Math.trunc(limit)) : 0;
	return {
		shown: issues.slice(0, safeLimit),
		hiddenCount: Math.max(0, issues.length - safeLimit),
	};
}

/** Completed imports with issues keep the sheet open so failures are visible. */
export function keepsSheetOpenForIssues(status: ImportStatus | null): boolean {
	return status !== null && status.state === 'completed' && status.issues.length > 0;
}
