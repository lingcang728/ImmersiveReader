// IPC wrappers for the EPUB reader surface. Commands are implemented in
// apps/desktop/src-tauri (see epub.rs); all calls go through the shared
// budgeted invoke helpers in ../ipc.

import { invokeCommand, invokeWithTimeout } from "../ipc";
import type {
	Bookmark,
	ReadableChapter,
	ReaderLocator,
	SearchBookResult,
	SearchHit,
} from "./types";
import { normalizeReaderLocator } from "./locator";

const locatorWrites = new Map<string, Promise<void>>();
const shadowKey = (bookId: string) => `ir-epub-progress:${bookId}`;

function parseLocator(raw: unknown, bookId: string): ReaderLocator | null {
	if (typeof raw === "string") {
		try { raw = JSON.parse(raw); } catch { return null; }
	}
    if (raw == null) return null;
    if (typeof raw !== "object" || (raw as ReaderLocator).schemaVersion !== 1 || !Number.isFinite((raw as ReaderLocator).ratio) || !Number.isFinite(Date.parse((raw as ReaderLocator).updated))) return null;
    return normalizeReaderLocator(raw, bookId);
}

function readShadow(bookId: string): ReaderLocator | null {
	try { return parseLocator(localStorage.getItem(shadowKey(bookId)), bookId); }
	catch { return null; }
}

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
	await locatorWrites.get(bookId)?.catch(() => {});
	const shadow = readShadow(bookId);
	try {
        const raw = await invokeCommand<unknown>("load_reader_locator", { bookId });
        const saved = parseLocator(raw, bookId);
        if (raw != null && !saved) throw new Error("阅读位置记录损坏，请重试或恢复备份");
		return shadow && (!saved || shadow.updated > saved.updated) ? shadow : saved;
	} catch (error) {
		if (shadow) return shadow;
		throw error;
	}
}

/**
 * Persist a locator. Fire-and-forget by callers — this is a hot path
 * (every ~2s while scrolling) so it must never be allowed to throw into
 * reader state. Returns a promise purely so tests can await it.
 */
export function saveReaderLocator(bookId: string, locator: ReaderLocator): Promise<void> {
	const valid = normalizeReaderLocator(locator, bookId);
	if (!valid) return Promise.reject(new Error("阅读位置与书籍不匹配"));
	const locatorJson = JSON.stringify(valid);
	try { localStorage.setItem(shadowKey(bookId), locatorJson); } catch { /* storage may be disabled */ }
	const write = (locatorWrites.get(bookId) ?? Promise.resolve())
		.catch(() => {})
		.then(() => invokeCommand<void>("save_reader_locator", { bookId, locatorJson }))
		.then(() => {
			try {
				if (localStorage.getItem(shadowKey(bookId)) === locatorJson) localStorage.removeItem(shadowKey(bookId));
			} catch { /* storage may be disabled */ }
		});
	locatorWrites.set(bookId, write);
	const release = () => { if (locatorWrites.get(bookId) === write) locatorWrites.delete(bookId); };
	void write.then(release, release);
	return write;
}

export async function flushReaderLocatorWrites(bookId: string): Promise<void> {
	await locatorWrites.get(bookId);
}

/** Decode the persisted native bookmark rows. */
export async function listBookmarks(bookId: string): Promise<Bookmark[]> {
	const raw = await invokeCommand<Array<{ bookmarkId: string; locatorJson: string; label: string; createdAt: string }>>("list_reader_bookmarks", { bookId });
	return raw.flatMap(row => {
		const locator = parseLocator(row.locatorJson, bookId);
		return locator ? [{ bookmarkId: row.bookmarkId, locator, label: row.label, createdAt: row.createdAt }] : [];
	});
}

/** Returns the new bookmark id. */
export async function addBookmark(
	bookId: string,
	locator: ReaderLocator,
	label: string,
): Promise<string> {
	const result = await invokeCommand<{ bookmarkId: string }>("add_reader_bookmark", {
		bookId,
		locatorJson: JSON.stringify(locator),
		label,
	});
	return result?.bookmarkId ?? "";
}

export function removeBookmark(bookId: string, bookmarkId: string): Promise<void> {
	return invokeCommand<void>("remove_reader_bookmark", { bookId, bookmarkId });
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
