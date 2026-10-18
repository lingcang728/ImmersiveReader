import { describe, expect, it } from 'vitest';
import type { BookSummary } from './contracts';
import {
	SHELF_BATCH_SIZE,
	bookFormat,
	bookInitial,
	buildShelfView,
	clampRatio,
	filterBooksByTitle,
	formatBookMeta,
	formatBytes,
	formatChapterCount,
	formatLabel,
	formatProgress,
	formatRelativeTime,
	initialBatchCount,
	nextBatchCount,
	progressPercent,
	progressRingDash,
	selectContinueBook,
	sortShelfBooks,
} from './shelf';

function book(overrides: Partial<BookSummary> & { bookId: string }): BookSummary {
	return {
		title: overrides.bookId,
		source: 'manual',
		chapterCount: 3,
		readCount: 0,
		progress: 0,
		currentChapterTitle: null,
		updatedAt: '2026-07-01T00:00:00.000Z',
		lastReadAt: null,
		...overrides,
	};
}

// Fixed local noon so date formatting is timezone-stable.
const NOW = new Date(2026, 6, 18, 12, 0, 0).getTime();
const iso = (ms: number) => new Date(ms).toISOString();

describe('shelf book formatting', () => {
	it('detects EPUB books from the backend book-id prefix', () => {
		expect(bookFormat({ bookId: 'epub:1a2b3c' })).toBe('epub');
		expect(bookFormat({ bookId: 'epub-xiao-wang-zi' })).toBe('epub');
		expect(bookFormat({ bookId: ' Epub:ABC ' })).toBe('epub');
		expect(bookFormat({ bookId: 'zhihu-article-1' })).toBe('markdown');
		expect(formatLabel('epub')).toBe('EPUB');
		expect(formatLabel('markdown')).toBe('Markdown');
	});

	it('falls back to the first title character for the cover tile', () => {
		expect(bookInitial('沉浸阅读')).toBe('沉');
		expect(bookInitial('  Deep Work ')).toBe('D');
		expect(bookInitial('   ')).toBe('书');
	});

	it('formats progress into clamped integer percentages', () => {
		expect(clampRatio(Number.NaN)).toBe(0);
		expect(clampRatio(-1)).toBe(0);
		expect(clampRatio(2)).toBe(1);
		expect(progressPercent(0.424)).toBe(42);
		expect(progressPercent(-0.5)).toBe(0);
		expect(progressPercent(1.5)).toBe(100);
		expect(formatProgress(0.5)).toBe('50%');
	});

	it('computes circular progress dash offsets', () => {
		const empty = progressRingDash(0, 10);
		expect(empty.circumference).toBeCloseTo(2 * Math.PI * 10);
		expect(empty.offset).toBeCloseTo(empty.circumference);
		const half = progressRingDash(0.5, 10);
		expect(half.offset).toBeCloseTo(half.circumference / 2);
		expect(progressRingDash(1, 10).offset).toBeCloseTo(0);
	});

	it('formats chapter counts defensively', () => {
		expect(formatChapterCount(12)).toBe('12 章');
		expect(formatChapterCount(0)).toBe('0 章');
		expect(formatChapterCount(Number.NaN)).toBe('0 章');
	});

	it('formats bytes with one decimal below 100', () => {
		expect(formatBytes(0)).toBe('0 B');
		expect(formatBytes(512)).toBe('512 B');
		expect(formatBytes(1536)).toBe('1.5 KB');
		expect(formatBytes(12.4 * 1024 * 1024)).toBe('12.4 MB');
		expect(formatBytes(101 * 1024 * 1024)).toBe('101 MB');
		expect(formatBytes(1.5 * 1024 * 1024 * 1024)).toBe('1.5 GB');
		expect(formatBytes(Number.NaN)).toBe('0 B');
	});
});

describe('shelf relative time', () => {
	it('labels recent timestamps in Chinese', () => {
		expect(formatRelativeTime(iso(NOW - 10_000), NOW)).toBe('刚刚');
		expect(formatRelativeTime(iso(NOW - 5 * 60_000), NOW)).toBe('5 分钟前');
		expect(formatRelativeTime(iso(NOW - 2 * 3_600_000), NOW)).toBe('2 小时前');
		expect(formatRelativeTime(iso(NOW - 30 * 3_600_000), NOW)).toBe('昨天');
		expect(formatRelativeTime(iso(NOW - 3 * 86_400_000), NOW)).toBe('3 天前');
	});

	it('falls back to a local date for older timestamps', () => {
		const old = new Date(2026, 0, 2, 9, 30).getTime();
		expect(formatRelativeTime(iso(old), NOW)).toBe('2026-01-02');
	});

	it('returns an empty label for missing/invalid timestamps', () => {
		expect(formatRelativeTime(null, NOW)).toBe('');
		expect(formatRelativeTime(undefined, NOW)).toBe('');
		expect(formatRelativeTime('not-a-date', NOW)).toBe('');
	});

	it('composes the book metadata line', () => {
		const meta = formatBookMeta(
			book({
				bookId: 'epub:abc',
				chapterCount: 12,
				lastReadAt: iso(NOW - 5 * 60_000),
			}),
			NOW,
		);
		expect(meta).toBe('EPUB · 12 章 · 5 分钟前');
		expect(formatBookMeta(book({ bookId: 'md-1', chapterCount: 1 }), NOW)).toBe(
			'Markdown · 1 章',
		);
	});
});

describe('shelf ordering and grouping', () => {
	it('sorts by last-read then update time, unread books last', () => {
		const recent = book({ bookId: 'recent', lastReadAt: iso(NOW - 60_000) });
		const older = book({ bookId: 'older', lastReadAt: iso(NOW - 86_400_000) });
		const unreadNewer = book({
			bookId: 'unread-b',
			updatedAt: iso(NOW - 1_000),
			title: '乙',
		});
		const unreadOlder = book({
			bookId: 'unread-a',
			updatedAt: iso(NOW - 10_000),
			title: '甲',
		});

		expect(sortShelfBooks([unreadOlder, older, recent, unreadNewer]).map((b) => b.bookId)).toEqual(
			['recent', 'older', 'unread-b', 'unread-a'],
		);
	});

	it('selects the most recently read book as the continue hero', () => {
		const sorted = sortShelfBooks([
			book({ bookId: 'a', lastReadAt: iso(NOW - 7_200_000) }),
			book({ bookId: 'b', lastReadAt: iso(NOW - 60_000) }),
			book({ bookId: 'c' }),
		]);
		expect(selectContinueBook(sorted)?.bookId).toBe('b');
		expect(selectContinueBook([book({ bookId: 'c' })])).toBeNull();
	});

	it('splits the shelf into hero + remaining list', () => {
		const view = buildShelfView([
			book({ bookId: 'a', lastReadAt: iso(NOW - 60_000) }),
			book({ bookId: 'b', lastReadAt: iso(NOW - 7_200_000) }),
			book({ bookId: 'c' }),
		]);
		expect(view.hero?.bookId).toBe('a');
		expect(view.primary.map((entry) => entry.bookId)).toEqual(['b', 'c']);
	});

	it('keeps every book in the list when nothing has been read', () => {
		const view = buildShelfView([book({ bookId: 'a' }), book({ bookId: 'b' })]);
		expect(view.hero).toBeNull();
		expect(view.primary).toHaveLength(2);
	});
});

describe('shelf search and batching', () => {
	const books = [
		book({ bookId: 'a', title: '深度工作' }),
		book({ bookId: 'b', title: 'Deep Work 英文版' }),
		book({ bookId: 'c', title: '沉浸阅读指南' }),
	];

	it('filters by title, case-insensitively and trimmed', () => {
		expect(filterBooksByTitle(books, '工作').map((b) => b.bookId)).toEqual(['a']);
		expect(filterBooksByTitle(books, '  deep ').map((b) => b.bookId)).toEqual(['b']);
		expect(filterBooksByTitle(books, '')).toHaveLength(3);
		expect(filterBooksByTitle(books, '不存在的书')).toHaveLength(0);
	});

	it('caps the first render and grows in fixed batches', () => {
		expect(SHELF_BATCH_SIZE).toBe(30);
		expect(initialBatchCount(100)).toBe(30);
		expect(initialBatchCount(12)).toBe(12);
		expect(nextBatchCount(30, 100)).toBe(60);
		expect(nextBatchCount(90, 100)).toBe(100);
		expect(nextBatchCount(Number.NaN, Number.NaN)).toBe(0);
	});
});
