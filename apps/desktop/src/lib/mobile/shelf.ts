// Pure shelf helpers for the Android mobile surface.
//
// Kept free of Svelte/DOM/Tauri imports so the list ordering, grouping,
// formatting and batching rules can be unit-tested with plain vitest. The
// components only render what these functions return.

import type { BookSummary } from './contracts';

/** Every EPUB imported by the backend is shelved with an `epub:` book id. */
const EPUB_BOOK_ID_PREFIX = 'epub:';
/** QA/older fixtures use `epub-…`; the dash form is treated the same way. */
const EPUB_BOOK_ID_DASH_PREFIX = 'epub-';

/** How many books the list renders before the first scroll/batch extension. */
export const SHELF_BATCH_SIZE = 30;

export type BookFormat = 'epub' | 'markdown';

export function bookFormat(book: Pick<BookSummary, 'bookId'>): BookFormat {
	const id = book.bookId.trim().toLowerCase();
	return id.startsWith(EPUB_BOOK_ID_PREFIX) || id.startsWith(EPUB_BOOK_ID_DASH_PREFIX)
		? 'epub'
		: 'markdown';
}

export function formatLabel(format: BookFormat): string {
	return format === 'epub' ? 'EPUB' : 'Markdown';
}

/** First non-space character of a title, used by the cover fallback tile. */
export function bookInitial(title: string): string {
	const trimmed = title.trim();
	if (!trimmed) return '书';
	return Array.from(trimmed)[0] ?? '书';
}

function timestamp(value: string | null | undefined): number {
	if (!value) return 0;
	const parsed = Date.parse(value);
	return Number.isFinite(parsed) ? parsed : 0;
}

/** The backend already sorts, but a fresh mobile scan must stay deterministic. */
export function sortShelfBooks(books: readonly BookSummary[]): BookSummary[] {
	return [...books].sort((left, right) => {
		const byLastRead = timestamp(right.lastReadAt) - timestamp(left.lastReadAt);
		if (byLastRead !== 0) return byLastRead;
		const byUpdated = timestamp(right.updatedAt) - timestamp(left.updatedAt);
		if (byUpdated !== 0) return byUpdated;
		const byTitle = left.title.localeCompare(right.title, 'zh-Hans-CN', {
			numeric: true,
			sensitivity: 'base',
		});
		if (byTitle !== 0) return byTitle;
		return left.bookId.localeCompare(right.bookId);
	});
}

/** Most recently read book, or null when nothing has been opened yet. */
export function selectContinueBook(sortedBooks: readonly BookSummary[]): BookSummary | null {
	return sortedBooks.find((book) => timestamp(book.lastReadAt) > 0) ?? null;
}

export interface ShelfView {
	/** "继续阅读" hero; null when the library has no reading history. */
	hero: BookSummary | null;
	/** Every other book, already ordered for the list. */
	primary: BookSummary[];
}

/**
 * Split the shelf into the continue-reading hero and the remaining list.
 * The hero is excluded from the list (the design shows it once).
 */
export function buildShelfView(books: readonly BookSummary[]): ShelfView {
	const sorted = sortShelfBooks(books);
	const hero = selectContinueBook(sorted);
	return {
		hero,
		primary: hero ? sorted.filter((book) => book.bookId !== hero.bookId) : sorted,
	};
}

/** Case/trailing-space insensitive title search. Empty query returns all. */
export function filterBooksByTitle(
	books: readonly BookSummary[],
	query: string,
): BookSummary[] {
	const needle = query.trim().toLowerCase();
	if (!needle) return [...books];
	return books.filter((book) => book.title.toLowerCase().includes(needle));
}

/** Clamp anything the backend might report to a 0..1 ratio. */
export function clampRatio(value: number): number {
	if (!Number.isFinite(value)) return 0;
	return Math.min(1, Math.max(0, value));
}

export function progressPercent(progress: number): number {
	return Math.round(clampRatio(progress) * 100);
}

export function formatProgress(progress: number): string {
	return `${progressPercent(progress)}%`;
}

/** SVG stroke math for the circular progress rings (list + hero). */
export function progressRingDash(
	progress: number,
	radius = 9,
): { circumference: number; offset: number } {
	const circumference = 2 * Math.PI * radius;
	return { circumference, offset: circumference * (1 - clampRatio(progress)) };
}

const MINUTE_MS = 60_000;
const HOUR_MS = 60 * MINUTE_MS;
const DAY_MS = 24 * HOUR_MS;

function pad2(value: number): string {
	return String(value).padStart(2, '0');
}

function formatShortDate(timestampMs: number): string {
	const date = new Date(timestampMs);
	return `${date.getFullYear()}-${pad2(date.getMonth() + 1)}-${pad2(date.getDate())}`;
}

/**
 * Compact Chinese relative time for the book metadata line:
 * 刚刚 / N 分钟前 / N 小时前 / 昨天 / N 天前 / YYYY-MM-DD.
 * Returns '' for missing/invalid timestamps.
 */
export function formatRelativeTime(
	iso: string | null | undefined,
	now: number = Date.now(),
): string {
	const value = timestamp(iso);
	if (value <= 0) return '';
	const delta = now - value;
	if (delta < MINUTE_MS) return '刚刚';
	if (delta < HOUR_MS) return `${Math.floor(delta / MINUTE_MS)} 分钟前`;
	if (delta < DAY_MS) return `${Math.floor(delta / HOUR_MS)} 小时前`;
	if (delta < 2 * DAY_MS) return '昨天';
	if (delta < 7 * DAY_MS) return `${Math.floor(delta / DAY_MS)} 天前`;
	return formatShortDate(value);
}

/** "EPUB · 12 章 · 昨天" — the book row's secondary line. */
export function formatBookMeta(book: BookSummary, now: number = Date.now()): string {
	const parts = [formatLabel(bookFormat(book)), `${book.chapterCount} 章`];
	const read = formatRelativeTime(book.lastReadAt, now);
	if (read) parts.push(read);
	return parts.join(' · ');
}

export function formatChapterCount(count: number): string {
	const safe = Number.isFinite(count) ? Math.max(0, Math.trunc(count)) : 0;
	return `${safe} 章`;
}

const BYTE_UNITS = ['B', 'KB', 'MB', 'GB', 'TB'] as const;

/** Human bytes for the storage section: 0 B / 12.4 MB / 1.2 GB. */
export function formatBytes(bytes: number): string {
	if (!Number.isFinite(bytes) || bytes <= 0) return '0 B';
	let value = bytes;
	let unit = 0;
	while (value >= 1024 && unit < BYTE_UNITS.length - 1) {
		value /= 1024;
		unit += 1;
	}
	const digits = unit === 0 || value >= 100 ? 0 : 1;
	return `${value.toFixed(digits)} ${BYTE_UNITS[unit]}`;
}

/** Next visible-book count for the incremental list render. */
export function nextBatchCount(visible: number, total: number): number {
	const safeVisible = Number.isFinite(visible) ? Math.max(0, Math.trunc(visible)) : 0;
	const safeTotal = Number.isFinite(total) ? Math.max(0, Math.trunc(total)) : 0;
	return Math.min(safeTotal, safeVisible + SHELF_BATCH_SIZE);
}

export function initialBatchCount(total: number): number {
	const safeTotal = Number.isFinite(total) ? Math.max(0, Math.trunc(total)) : 0;
	return Math.min(safeTotal, SHELF_BATCH_SIZE);
}
