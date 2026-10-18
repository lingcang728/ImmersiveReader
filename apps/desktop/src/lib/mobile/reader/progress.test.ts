import { describe, expect, it } from 'vitest';

import {
	CHAPTER_COMPLETE_RATIO,
	PAGE_SCROLL_VIEWPORT_RATIO,
	bookOverallProgress,
	buildReaderMeta,
	chapterBoundaryTarget,
	clampChapterIndex,
	displayChapterNumber,
	fileOverallProgress,
	isChapterComplete,
	maxScrollTop,
	pageScrollTarget,
	percentOf,
	resolveBookRestoreRatio,
	resolveFileRestore,
	resolveStartChapterIndex,
	restoreScrollTop,
	shouldPreloadNext,
	shouldPromptNext,
} from './progress';
import type { BookChapter, BookReadingState } from '$lib/mobile/contracts';

const chapters: BookChapter[] = [
	{ id: 'c1', path: 'c1.md', title: '一', voteCount: 0, wordCount: 100 },
	{ id: 'c2', path: 'c2.md', title: '二', voteCount: 0, wordCount: 100 },
	{ id: 'c3', path: 'c3.md', title: '三', voteCount: 0, wordCount: 100 },
];

function state(partial: Partial<BookReadingState> = {}): BookReadingState {
	return {
		schemaVersion: 1,
		current: 'c1',
		position: 0,
		read: [],
		updated: '2026-01-01T00:00:00.000Z',
		...partial,
	};
}

describe('pageScrollTarget', () => {
	it('moves forward by 82% of the viewport', () => {
		const target = pageScrollTarget(0, 1000, 5000, 1000, 1);
		expect(target).toBe(1000 * PAGE_SCROLL_VIEWPORT_RATIO);
	});

	it('moves backward and clamps at zero', () => {
		expect(pageScrollTarget(300, 1000, 5000, 1000, -1)).toBe(0);
		expect(pageScrollTarget(1500, 1000, 5000, 1000, -1)).toBe(680);
	});

	it('clamps at the maximum scroll offset', () => {
		expect(pageScrollTarget(3900, 1000, 5000, 1000, 1)).toBe(4000);
	});

	it('returns the current offset when the content does not scroll', () => {
		expect(pageScrollTarget(0, 1000, 800, 1000, 1)).toBe(0);
	});

	it('ignores a NaN viewport height', () => {
		expect(pageScrollTarget(100, Number.NaN, 5000, 1000, 1)).toBe(100);
	});
});

describe('maxScrollTop / chapterBoundaryTarget', () => {
	it('is bottom for a backward chapter entry, top for forward', () => {
		expect(maxScrollTop(3000, 1000)).toBe(2000);
		expect(chapterBoundaryTarget(-1, 3000, 1000)).toBe(2000);
		expect(chapterBoundaryTarget(1, 3000, 1000)).toBe(0);
	});

	it('never goes negative', () => {
		expect(maxScrollTop(500, 1000)).toBe(0);
	});
});

describe('chapter completion', () => {
	it('requires the very end for a middle chapter', () => {
		expect(isChapterComplete(0.98, false)).toBe(false);
		expect(isChapterComplete(CHAPTER_COMPLETE_RATIO, false)).toBe(true);
	});

	it('accepts 95% for the last chapter', () => {
		expect(isChapterComplete(0.94, true)).toBe(false);
		expect(isChapterComplete(0.95, true)).toBe(true);
	});

	it('treats out-of-range ratios safely', () => {
		expect(isChapterComplete(-1, false)).toBe(false);
		expect(isChapterComplete(2, false)).toBe(true);
		expect(isChapterComplete(Number.NaN, false)).toBe(false);
	});
});

describe('bookOverallProgress', () => {
	it('counts read chapters plus the live position of the current one', () => {
		const progress = bookOverallProgress(chapters, state({ read: ['c1'] }), 'c2', 0.5);
		expect(progress).toBeCloseTo((1 + 0.5) / 3, 6);
	});

	it('does not double count a chapter that is already read', () => {
		const progress = bookOverallProgress(chapters, state({ read: ['c1', 'c2'] }), 'c2', 0.9);
		expect(progress).toBeCloseTo(2 / 3, 6);
	});

	it('is zero for an empty book', () => {
		expect(bookOverallProgress([], state(), 'c1', 0.5)).toBe(0);
	});
});

describe('fileOverallProgress / percentOf', () => {
	it('clamps and rounds percentages', () => {
		expect(fileOverallProgress(0.734)).toBeCloseTo(0.734, 6);
		expect(percentOf(0.734)).toBe(73);
		expect(percentOf(-1)).toBe(0);
		expect(percentOf(2)).toBe(100);
		expect(percentOf(Number.NaN)).toBe(0);
	});
});

describe('resolveStartChapterIndex', () => {
	it('honours an explicit chapter id', () => {
		expect(resolveStartChapterIndex(chapters, state(), 'c2')).toBe(1);
	});

	it('falls back to the saved current chapter', () => {
		expect(resolveStartChapterIndex(chapters, state({ current: 'c3' }))).toBe(2);
	});

	it('falls back to the first unread chapter', () => {
		expect(resolveStartChapterIndex(chapters, state({ current: 'gone', read: ['c1'] }))).toBe(1);
	});

	it('ignores an unknown explicit id', () => {
		expect(resolveStartChapterIndex(chapters, state({ current: 'c2' }), 'missing')).toBe(1);
	});

	it('returns -1 for an empty book', () => {
		expect(resolveStartChapterIndex([], state())).toBe(-1);
	});
});

describe('clampChapterIndex / displayChapterNumber', () => {
	it('clamps to the chapter range', () => {
		expect(clampChapterIndex(-1, 3)).toBe(0);
		expect(clampChapterIndex(5, 3)).toBe(2);
		expect(clampChapterIndex(1, 0)).toBe(-1);
		expect(clampChapterIndex(Number.NaN, 3)).toBe(-1);
	});

	it('shows 1-based ordinals', () => {
		expect(displayChapterNumber(0)).toBe(1);
		expect(displayChapterNumber(4)).toBe(5);
		expect(displayChapterNumber(-2)).toBe(1);
	});
});

describe('preload / seam thresholds', () => {
	it('preloads only when a next chapter exists and past 84%', () => {
		expect(shouldPreloadNext(0.5, true)).toBe(false);
		expect(shouldPreloadNext(0.84, true)).toBe(true);
		expect(shouldPreloadNext(0.99, false)).toBe(false);
	});

	it('prompts at 92% only when a next chapter exists', () => {
		expect(shouldPromptNext(0.91, true)).toBe(false);
		expect(shouldPromptNext(0.92, true)).toBe(true);
		expect(shouldPromptNext(0.99, false)).toBe(false);
	});
});

describe('buildReaderMeta', () => {
	it('clamps ranges and normalizes the chapter count', () => {
		const meta = buildReaderMeta({
			title: '书',
			chapterId: 'c2',
			chapterTitle: '二',
			chapterIndex: 1,
			chapterCount: 3,
			chapterRatio: 1.4,
			overall: -0.2,
			canPrev: true,
			canNext: true,
		});
		expect(meta.chapterRatio).toBe(1);
		expect(meta.overall).toBe(0);
		expect(meta.chapterIndex).toBe(1);
	});

	it('keeps a usable chapter count when the book reports none', () => {
		const meta = buildReaderMeta({
			title: '书',
			chapterId: '',
			chapterTitle: '',
			chapterIndex: 0,
			chapterCount: 0,
			chapterRatio: 0,
			overall: 0,
			canPrev: false,
			canNext: false,
		});
		expect(meta.chapterCount).toBe(1);
		expect(meta.chapterIndex).toBe(0);
	});
});

describe('restore strategies', () => {
	it('prefers a ratio over an absolute pixel offset', () => {
		expect(resolveFileRestore({ scroll_position: 900, progress: 0.4 })).toEqual({
			kind: 'ratio',
			value: 0.4,
		});
	});

	it('falls back to the absolute offset for legacy state', () => {
		expect(resolveFileRestore({ scroll_position: 900 })).toEqual({ kind: 'absolute', value: 900 });
	});

	it('returns null when nothing usable was saved', () => {
		expect(resolveFileRestore({})).toBeNull();
		expect(resolveFileRestore({ progress: 0, scroll_position: 0 })).toBeNull();
		expect(resolveFileRestore({ progress: Number.NaN, scroll_position: Number.NaN })).toBeNull();
	});

	it('converts a restore strategy back into a scroll offset', () => {
		expect(restoreScrollTop({ kind: 'ratio', value: 0.25 }, 5000, 1000)).toBe(1000);
		expect(restoreScrollTop({ kind: 'absolute', value: 123.4 }, 5000, 1000)).toBe(123);
		expect(restoreScrollTop({ kind: 'absolute', value: 99999 }, 5000, 1000)).toBe(4000);
		expect(restoreScrollTop(null, 5000, 1000)).toBeNull();
	});

	it('validates a saved book ratio', () => {
		expect(resolveBookRestoreRatio(0.6)).toBe(0.6);
		expect(resolveBookRestoreRatio(undefined)).toBe(0);
		expect(resolveBookRestoreRatio('x')).toBe(0);
	});
});
