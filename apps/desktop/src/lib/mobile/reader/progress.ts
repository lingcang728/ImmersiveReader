// Pure progress / boundary helpers for the mobile reader.
//
// All functions take plain numbers/state so they can be unit tested without a
// DOM. The scroll-ratio math itself is shared with the EPUB surface via
// `$lib/epub/locator` — a second ratio implementation would drift.

import type { BookChapter, BookReadingState } from '$lib/mobile/contracts';
import { calculateBookProgress, nextReadingState, resolveChapterIndex } from '$lib/library/books';
import { clamp01, ratioFromScroll } from '$lib/epub/locator';
import type { ReaderMeta, ScrollRestore } from './types';

/** A page tap scrolls this fraction of the viewport (docs: 82%). */
export const PAGE_SCROLL_VIEWPORT_RATIO = 0.82;
/** Start preloading the next chapter once past this point (desktop uses 0.84). */
export const NEXT_CHAPTER_PRELOAD_RATIO = 0.84;
/** Show the "next chapter" seam prompt from this point on. */
export const CHAPTER_SEAM_RATIO = 0.92;
/** A chapter counts as read at the very end… */
export const CHAPTER_COMPLETE_RATIO = 0.999;
/** …or at 95% of the last chapter (matches desktop semantics). */
export const LAST_CHAPTER_COMPLETE_RATIO = 0.95;

/** 0..1 scroll position inside a scroller (shared with the EPUB kernel). */
export function chapterRatioFromScroll(
	scrollTop: number,
	scrollHeight: number,
	clientHeight: number,
): number {
	return ratioFromScroll(scrollTop, scrollHeight, clientHeight);
}

/** Desktop-compatible "this chapter is finished" rule. */
export function isChapterComplete(ratio: number, isLastChapter: boolean): boolean {
	const value = clamp01(ratio);
	if (value >= CHAPTER_COMPLETE_RATIO) return true;
	return isLastChapter && value >= LAST_CHAPTER_COMPLETE_RATIO;
}

/** Max scrollTop for a scroller (never negative). */
export function maxScrollTop(scrollHeight: number, clientHeight: number): number {
	const span = scrollHeight - clientHeight;
	return Number.isFinite(span) && span > 0 ? span : 0;
}

/**
 * Scroll target for one page tap. `direction` 1 = forward, -1 = back.
 * The delta is 82% of the viewport, clamped to the scrollable range.
 */
export function pageScrollTarget(
	scrollTop: number,
	viewportHeight: number,
	scrollHeight: number,
	clientHeight: number,
	direction: -1 | 1,
): number {
	const max = maxScrollTop(scrollHeight, clientHeight);
	const delta = Math.max(0, viewportHeight) * PAGE_SCROLL_VIEWPORT_RATIO * direction;
	const target = scrollTop + delta;
	if (!Number.isFinite(target)) return scrollTop;
	return Math.min(max, Math.max(0, target));
}

/**
 * Where to land after a chapter switch: entering a chapter from the previous
 * one stops at the bottom so the seam reads continuously; entering from the
 * next one stops at the top.
 */
export function chapterBoundaryTarget(direction: -1 | 1, scrollHeight: number, clientHeight: number): number {
	return direction < 0 ? maxScrollTop(scrollHeight, clientHeight) : 0;
}

/**
 * Live whole-book ratio. The saved `read` set plus the in-chapter position —
 * identical to what `save_book_progress` will persist, so the progress bar
 * cannot disagree with the stored value.
 */
export function bookOverallProgress(
	chapters: readonly BookChapter[],
	state: BookReadingState,
	chapterId: string,
	ratio: number,
): number {
	if (chapters.length === 0) return 0;
	const live = nextReadingState(state, chapterId, ratio, false);
	return calculateBookProgress(chapters, live);
}

/** Single-file progress is just the scroll ratio. */
export function fileOverallProgress(ratio: number): number {
	return clamp01(ratio);
}

/** 0-based chapter index for a requested id, else saved-progress resume. */
export function resolveStartChapterIndex(
	chapters: readonly BookChapter[],
	state: BookReadingState,
	requestedChapterId?: string,
): number {
	if (requestedChapterId) {
		const requested = chapters.findIndex((chapter) => chapter.id === requestedChapterId);
		if (requested >= 0) return requested;
	}
	return resolveChapterIndex(chapters, state.current, state.read);
}

/** Clamp a chapter index into `[0, count)`; -1 when there are no chapters. */
export function clampChapterIndex(index: number, count: number): number {
	if (count <= 0 || !Number.isFinite(index)) return -1;
	return Math.min(count - 1, Math.max(0, Math.trunc(index)));
}

/** True when the reader should start preloading `index + 1`. */
export function shouldPreloadNext(ratio: number, canNext: boolean): boolean {
	return canNext && clamp01(ratio) >= NEXT_CHAPTER_PRELOAD_RATIO;
}

/** True when the chapter seam prompt should be visible. */
export function shouldPromptNext(ratio: number, canNext: boolean): boolean {
	return canNext && clamp01(ratio) >= CHAPTER_SEAM_RATIO;
}

/** 1-based chapter ordinal for display; never below 1. */
export function displayChapterNumber(index: number): number {
	return Math.max(1, Math.trunc(index) + 1);
}

/** Integer percentage 0..100 for labels and progress fill widths. */
export function percentOf(ratio: number): number {
	return Math.round(clamp01(ratio) * 100);
}

/** Build the chrome meta payload from live values. */
export function buildReaderMeta(input: {
	title: string;
	chapterId: string;
	chapterTitle: string;
	chapterIndex: number;
	chapterCount: number;
	chapterRatio: number;
	overall: number;
	canPrev: boolean;
	canNext: boolean;
}): ReaderMeta {
	const count = Math.max(0, Math.trunc(input.chapterCount));
	return {
		title: input.title,
		chapterId: input.chapterId,
		chapterTitle: input.chapterTitle,
		chapterIndex: clampChapterIndex(input.chapterIndex, Math.max(1, count)),
		chapterCount: Math.max(1, count),
		chapterRatio: clamp01(input.chapterRatio),
		overall: clamp01(input.overall),
		canPrev: input.canPrev,
		canNext: input.canNext,
	};
}

/** Clamp a restored book position read from disk. */
export function resolveBookRestoreRatio(position: unknown): number {
	return typeof position === 'number' && Number.isFinite(position) ? clamp01(position) : 0;
}

/**
 * Pick the scroll restore strategy for a single Markdown file. A saved
 * `progress` ratio wins over a raw pixel offset because it survives font and
 * viewport changes; the absolute offset is only the legacy fallback.
 */
export function resolveFileRestore(state: {
	scroll_position?: unknown;
	progress?: unknown;
}): ScrollRestore | null {
	const progress = typeof state.progress === 'number' && Number.isFinite(state.progress) ? state.progress : null;
	if (progress !== null && progress > 0) return { kind: 'ratio', value: clamp01(progress) };
	const absolute = typeof state.scroll_position === 'number' && Number.isFinite(state.scroll_position) ? state.scroll_position : null;
	if (absolute !== null && absolute > 0) return { kind: 'absolute', value: absolute };
	return null;
}

/** Scroll offset for a restore strategy. */
export function restoreScrollTop(
	restore: ScrollRestore | null,
	scrollHeight: number,
	clientHeight: number,
): number | null {
	if (!restore) return null;
	const max = maxScrollTop(scrollHeight, clientHeight);
	if (restore.kind === 'ratio') return Math.round(clamp01(restore.value) * max);
	return Math.min(max, Math.max(0, Math.round(restore.value)));
}
