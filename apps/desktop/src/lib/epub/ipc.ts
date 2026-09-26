// IPC wrappers for the EPUB reader surface. Commands are implemented in
// apps/desktop/src-tauri (see epub.rs); all calls go through the shared
// budgeted invoke helpers in ../ipc.

import { invokeCommand, invokeWithTimeout } from "../ipc";
import type {
	AddBookmarkResult,
	Bookmark,
	ListBookmarksResult,
	ReadableChapter,
	ReaderLocator,
	SearchBookResult,
	SearchHit,
} from "./types";
import { normalizeReaderLocator } from "./locator";

export function getReadableChapter(
	bookId: string,
	chapterId: string,
): Promise<ReadableChapter> {
	return invokeCommand<ReadableChapter>("get_readable_chapter", { bookId, chapterId });
}

/**
 * Saved position for this book, or null on first open. Tolerates either a
 * bare `ReaderLocator` or a `{locator: ...}` envelope.
 */
export async function getReaderLocator(bookId: string): Promise<ReaderLocator | null> {
	const raw = await invokeCommand<unknown>("get_reader_locator", { bookId });
	if (raw === null || raw === undefined) return null;
	const candidate =
		typeof raw === "object" && "locator" in (raw as Record<string, unknown>)
			? (raw as Record<string, unknown>).locator
			: raw;
	return normalizeReaderLocator(candidate, bookId);
}

/**
 * Persist a locator. Fire-and-forget by callers — this is a hot path
 * (every ~2s while scrolling) so it must never be allowed to throw into
 * reader state. Returns a promise purely so tests can await it.
 */
export function saveReaderLocator(bookId: string, locator: ReaderLocator): Promise<void> {
	return invokeCommand<void>("save_reader_locator", { bookId, locator });
}

/** `list_bookmarks` may answer `{bookmarks:[...]}` or a bare array — accept both. */
export async function listBookmarks(bookId: string): Promise<Bookmark[]> {
	const raw = await invokeCommand<ListBookmarksResult | Bookmark[]>("list_bookmarks", {
		bookId,
	});
	if (Array.isArray(raw)) return raw;
	return Array.isArray(raw?.bookmarks) ? raw.bookmarks : [];
}

/** Returns the new bookmark id. */
export async function addBookmark(
	bookId: string,
	locator: ReaderLocator,
	label: string,
): Promise<string> {
	const result = await invokeCommand<AddBookmarkResult>("add_bookmark", {
		bookId,
		locator,
		label,
	});
	return result?.bookmarkId ?? "";
}

export function removeBookmark(bookId: string, bookmarkId: string): Promise<void> {
	return invokeCommand<void>("remove_bookmark", { bookId, bookmarkId });
}

/** Full-book search. Bigger budget — first query may build the FTS index. */
export async function searchBook(
	bookId: string,
	query: string,
	limit = 50,
): Promise<SearchHit[]> {
	const raw = await invokeWithTimeout<SearchBookResult | SearchHit[]>(
		"search_book",
		{ bookId, query, limit },
		60_000,
	);
	if (Array.isArray(raw)) return raw;
	return Array.isArray(raw?.hits) ? raw.hits : [];
}
