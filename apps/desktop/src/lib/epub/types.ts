// Shared EPUB contract types — mirrors apps/desktop/src-tauri/src/epub.rs
// and packages/contracts ReaderLocator schema. Keep in lockstep.

export interface EpubNavItem {
	title: string;
	chapterId: string;
	children?: EpubNavItem[];
}

export interface Publication {
	schemaVersion: number;
	format: "epub";
	epubVersion: string;
	title: string;
	creator?: string;
	language?: string;
	cover?: string;
	nav: EpubNavItem[];
	spine: string[];
	resources: Record<string, string>;
	unsupported: string[];
}

export type ReaderAnchor =
	| { kind: "element"; path: string }
	| { kind: "text"; quote: string; offset: number }
	| { kind: "ratio" };

export interface ReaderLocator {
	schemaVersion: 1;
	bookId: string;
	chapterId: string;
	anchor: ReaderAnchor;
	/** Scroll position inside the chapter, 0..1 — fallback when anchor misses. */
	ratio: number;
	updated: string; // RFC-3339
}

export interface ReadableChapter {
	format: "xhtml" | "markdown";
	chapterId: string;
	title: string;
	content: string;
	resourceDir: string;
	prevChapterId?: string;
	nextChapterId?: string;
}

export interface Bookmark {
	bookmarkId: string;
	locator: ReaderLocator;
	label: string;
	createdAt: string;
}

export interface SearchHit {
	chapterId: string;
	title: string;
	snippet: string;
	locator?: ReaderLocator;
}

// ---- IPC envelope shapes (epub reader surface additions) ----

/** `add_bookmark` response. */
export interface AddBookmarkResult {
	bookmarkId: string;
}

/** `search_book` response. */
export interface SearchBookResult {
	hits: SearchHit[];
}

/** `list_bookmarks` response — the backend may also return a bare array. */
export interface ListBookmarksResult {
	bookmarks: Bookmark[];
}
