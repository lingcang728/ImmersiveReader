// Shared types for the Android mobile reader surface.
//
// Everything here is plain data (no Svelte/DOM) so the pure helpers and the
// Vitest suites can import it freely. ReaderScreen owns the orchestration;
// MarkdownReader / EpubReaderMobile only report progress through these shapes.

import type { BookChapter } from '$lib/mobile/contracts';

/** One chapter row as the TOC sheet needs it (book chapters or EPUB nav). */
export interface TocEntryView {
	id: string;
	label: string;
	/** 1-based indent level (EPUB nested nav); 1 for flat chapter lists. */
	level: number;
}

/** Bottom-sheet identity. Back always closes the sheet before leaving. */
export type ReaderPanel = '' | 'toc' | 'search' | 'bookmarks' | 'settings';

/** Which reader implementation owns the current target. */
export type ReaderMode = 'loading' | 'markdown-book' | 'markdown-file' | 'epub' | 'error';

/**
 * Live reading position reported by either reader implementation. The shell
 * chrome (title, progress bar, chapter buttons) renders entirely from this —
 * no reader-switching conditionals in the chrome itself.
 */
export interface ReaderMeta {
	chapterId: string;
	chapterTitle: string;
	/** 0-based index of the chapter inside the current book/EPUB spine. */
	chapterIndex: number;
	chapterCount: number;
	/** 0..1 scroll position inside the current chapter. */
	chapterRatio: number;
	/** 0..1 position across the whole book / file. */
	overall: number;
	/** Whole-book title (EPUB publication title / manifest title). */
	title: string;
	canPrev: boolean;
	canNext: boolean;
}

export function emptyMeta(title = ''): ReaderMeta {
	return {
		chapterId: '',
		chapterTitle: '',
		chapterIndex: 0,
		chapterCount: 1,
		chapterRatio: 0,
		overall: 0,
		title,
		canPrev: false,
		canNext: false,
	};
}

/** Book-mode chapter list + resume information resolved before mounting. */
export interface MarkdownBookSession {
	bookId: string;
	title: string;
	chapters: BookChapter[];
	initialChapterId: string;
	initialChapterRatio: number;
}

/** A single-chapter search result set (Markdown in-chapter search). */
export interface ChapterSearchResult {
	query: string;
	count: number;
	/** 0-based index of the currently highlighted match; -1 when none. */
	index: number;
}

/** One row in the search sheet (EPUB full-book search hits use snippets). */
export interface SearchResultView {
	id: string;
	title: string;
	/** Pre-sanitized snippet HTML (only <b> marks survive sanitization). */
	snippetHtml?: string;
	/** 1-based display position shown in the sheet. */
	ordinal: number;
}

/** Reader scroll target used when restoring after a chapter change. */
export interface ScrollRestore {
	/** 'ratio' is preferred (font/reflow safe); absolute is the file fallback. */
	kind: 'ratio' | 'absolute';
	value: number;
}

/**
 * Metadata reported by the EPUB kernel while embedded. Mirrors the fields the
 * ReaderScreen chrome needs; the whole-book title comes from the book detail.
 */
export interface EpubMeta {
	chapterId: string;
	chapterTitle: string;
	chapterIndex: number;
	chapterCount: number;
	chapterRatio: number;
	overall: number;
	canPrev: boolean;
	canNext: boolean;
}
