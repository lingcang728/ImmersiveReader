import { describe, expect, it } from 'vitest';
import type { ImportStatus } from './api';
import type { BookSummary } from './contracts';
import {
	IMPORT_POLL_INTERVAL_MS,
	importCurrentFileName,
	importFilesLabel,
	importPercent,
	importStatusView,
	importToast,
	isRunningImport,
	isTerminalImportState,
	keepsSheetOpenForIssues,
	newBooksAfter,
	pickImportedBook,
	visibleImportIssues,
} from './importState';

function status(overrides: Partial<ImportStatus> = {}): ImportStatus {
	return {
		operationId: 'op-1',
		state: 'running',
		phase: '',
		totalFiles: 0,
		completedFiles: 0,
		currentFile: null,
		issues: [],
		bookId: null,
		error: null,
		...overrides,
	};
}

function book(overrides: Partial<BookSummary> & { bookId: string }): BookSummary {
	return {
		title: overrides.bookId,
		source: 'manual',
		chapterCount: 1,
		readCount: 0,
		progress: 0,
		currentChapterTitle: null,
		updatedAt: '2026-07-01T00:00:00.000Z',
		lastReadAt: null,
		...overrides,
	};
}

describe('import progress mapping', () => {
	it('polls every 400ms', () => {
		expect(IMPORT_POLL_INTERVAL_MS).toBe(400);
	});

	it('reports determinate progress once the total is known', () => {
		expect(importPercent(status({ totalFiles: 10, completedFiles: 3 }))).toBe(30);
		expect(importPercent(status({ totalFiles: 10, completedFiles: 30 }))).toBe(100);
		expect(importPercent(status({ totalFiles: 0, completedFiles: 0 }))).toBeNull();
		expect(importPercent(status({ state: 'completed', totalFiles: 4, completedFiles: 4 }))).toBe(
			100,
		);
		expect(importPercent(null)).toBeNull();
	});

	it('labels file counts and the current file name', () => {
		expect(importFilesLabel(status({ totalFiles: 10, completedFiles: 3 }))).toBe('3 / 10');
		expect(importFilesLabel(status())).toBe('正在读取文件…');
		expect(importFilesLabel(status({ state: 'completed', totalFiles: 2 }))).toBe('共 2 个文件');
		expect(
			importCurrentFileName('content://com.android.providers/document/My%20Book.md?x=1'),
		).toBe('My Book.md');
		expect(importCurrentFileName('')).toBeNull();
		expect(importCurrentFileName(null)).toBeNull();
	});

	it('maps a running operation to a progress view', () => {
		const view = importStatusView(
			status({
				state: 'running',
				phase: 'importing',
				totalFiles: 4,
				completedFiles: 2,
				currentFile: 'content://doc/thinking.md',
			}),
		);
		expect(view.tone).toBe('running');
		expect(view.title).toBe('正在导入');
		expect(view.phaseLabel).toBe('正在导入');
		expect(view.percent).toBe(50);
		expect(view.filesLabel).toBe('2 / 4');
		expect(view.currentFileLabel).toBe('thinking.md');
		expect(view.terminal).toBe(false);
		expect(view.running).toBe(true);
	});

	it('keeps an unknown phase verbatim instead of blanking the label', () => {
		const view = importStatusView(status({ phase: 'building-index' }));
		expect(view.phaseLabel).toBe('building-index');
	});

	it('maps terminal states to tone, title and error text', () => {
		const completed = importStatusView(status({ state: 'completed', totalFiles: 3 }));
		expect(completed.tone).toBe('success');
		expect(completed.title).toBe('导入完成');
		expect(completed.percent).toBe(100);
		expect(completed.terminal).toBe(true);

		const failed = importStatusView(status({ state: 'failed', error: '内容 URI 读取失败' }));
		expect(failed.tone).toBe('error');
		expect(failed.errorText).toBe('内容 URI 读取失败');

		const failedNoMessage = importStatusView(status({ state: 'failed', error: null }));
		expect(failedNoMessage.errorText).toBe('导入未完成，请重试');

		const cancelled = importStatusView(status({ state: 'cancelled' }));
		expect(cancelled.tone).toBe('neutral');
		expect(cancelled.title).toBe('已取消导入');
	});

	it('returns an idle view before any operation starts', () => {
		const view = importStatusView(null);
		expect(view.tone).toBe('idle');
		expect(view.title).toBe('导入书籍');
		expect(view.percent).toBeNull();
		expect(view.running).toBe(false);
	});

	it('classifies terminal/running states', () => {
		expect(isTerminalImportState('completed')).toBe(true);
		expect(isTerminalImportState('failed')).toBe(true);
		expect(isTerminalImportState('cancelled')).toBe(true);
		expect(isTerminalImportState('running')).toBe(false);
		expect(isTerminalImportState('pending')).toBe(false);
		expect(isRunningImport(null)).toBe(false);
		expect(isRunningImport(status())).toBe(true);
		expect(isRunningImport(status({ state: 'completed' }))).toBe(false);
	});
});

describe('import completion handoff', () => {
	it('diffs newly shelved books against the previous snapshot', () => {
		const before = new Set(['old-a', 'old-b']);
		const after = [book({ bookId: 'old-a' }), book({ bookId: 'new-1' })];
		expect(newBooksAfter(before, after).map((entry) => entry.bookId)).toEqual(['new-1']);
	});

	it('prefers the operation book id when it is in the snapshot', () => {
		const imported = pickImportedBook(
			status({ state: 'completed', bookId: 'new-op' }),
			new Set(['old']),
			[book({ bookId: 'old' }), book({ bookId: 'new-op', title: '新书' })],
		);
		expect(imported).toEqual({ bookId: 'new-op', title: '新书' });
	});

	it('falls back to a single new book when the op id is missing', () => {
		const imported = pickImportedBook(status({ state: 'completed' }), new Set(['old']), [
			book({ bookId: 'old' }),
			book({ bookId: 'fresh', title: '唯一新书' }),
		]);
		expect(imported).toEqual({ bookId: 'fresh', title: '唯一新书' });
	});

	it('does not guess when several books appeared', () => {
		expect(
			pickImportedBook(status({ state: 'completed' }), new Set(), [
				book({ bookId: 'a' }),
				book({ bookId: 'b' }),
			]),
		).toBeNull();
	});

	it('returns null for non-completed operations', () => {
		expect(pickImportedBook(status({ state: 'running' }), new Set(), [])).toBeNull();
		expect(pickImportedBook(status({ state: 'failed' }), new Set(), [])).toBeNull();
	});

	it('keeps the sheet open only for completed imports with issues', () => {
		expect(keepsSheetOpenForIssues(status({ state: 'completed' }))).toBe(false);
		expect(
			keepsSheetOpenForIssues(
				status({ state: 'completed', issues: [{ path: 'a.md', message: '读取失败' }] }),
			),
		).toBe(true);
		expect(keepsSheetOpenForIssues(status({ state: 'running' }))).toBe(false);
		expect(keepsSheetOpenForIssues(null)).toBe(false);
	});
});

describe('import toast copy', () => {
	it('reports success with the imported title', () => {
		expect(importToast(status({ state: 'completed' }), { bookId: 'b', title: '深度工作' })).toEqual({
			text: '已导入《深度工作》',
			kind: 'success',
		});
	});

	it('mentions skipped files', () => {
		const withIssue = status({
			state: 'completed',
			issues: [{ path: 'a.epub', message: '损坏' }],
		});
		expect(importToast(withIssue, null)).toEqual({
			text: '导入完成，1 个文件未能导入',
			kind: 'info',
		});
		expect(importToast(withIssue, { bookId: 'b', title: '书' })).toEqual({
			text: '已导入《书》，1 个文件未能导入',
			kind: 'info',
		});
	});

	it('reports failures and cancellation', () => {
		expect(importToast(status({ state: 'failed', error: '磁盘已满' }), null)).toEqual({
			text: '磁盘已满',
			kind: 'error',
		});
		expect(importToast(status({ state: 'failed' }), null)).toEqual({
			text: '导入失败，请重试',
			kind: 'error',
		});
		expect(importToast(status({ state: 'cancelled' }), null)).toEqual({
			text: '已取消导入',
			kind: 'info',
		});
	});
});

describe('import issue list', () => {
	const issues = Array.from({ length: 8 }, (_, index) => ({
		path: `file-${index}.md`,
		message: `错误 ${index}`,
	}));

	it('caps the visible issue rows and reports the hidden remainder', () => {
		expect(visibleImportIssues(issues, 5)).toEqual({ shown: issues.slice(0, 5), hiddenCount: 3 });
		expect(visibleImportIssues([], 5)).toEqual({ shown: [], hiddenCount: 0 });
		expect(visibleImportIssues(issues, 0)).toEqual({ shown: [], hiddenCount: 8 });
	});
});
