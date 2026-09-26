// Pure ReaderLocator helpers — construction, normalization of untrusted
// input (locators arrive over IPC/from disk), and scroll-ratio math.
// DOM-dependent anchor capture/restore lives in ./dom.ts.

import type { ReaderAnchor, ReaderLocator } from "./types";

export function clamp01(value: number): number {
	if (!Number.isFinite(value)) return 0;
	return Math.min(1, Math.max(0, value));
}

/** Scroll position as a 0..1 ratio; degenerate (non-scrollable) → 0. */
export function ratioFromScroll(
	scrollTop: number,
	scrollHeight: number,
	clientHeight: number,
): number {
	const span = scrollHeight - clientHeight;
	if (!Number.isFinite(span) || span <= 0) return 0;
	return clamp01(scrollTop / span);
}

export function makeLocator(
	bookId: string,
	chapterId: string,
	anchor: ReaderAnchor,
	ratio: number,
	updated?: string,
): ReaderLocator {
	return {
		schemaVersion: 1,
		bookId,
		chapterId,
		anchor,
		ratio: clamp01(ratio),
		updated: updated ?? new Date().toISOString(),
	};
}

/** Locator pointing at the top of a chapter — used on fresh navigation. */
export function topOfChapterLocator(bookId: string, chapterId: string): ReaderLocator {
	return makeLocator(bookId, chapterId, { kind: "ratio" }, 0);
}

/** Validate an anchor of unknown provenance (disk/IPC). */
export function normalizeReaderAnchor(raw: unknown): ReaderAnchor | null {
	if (typeof raw !== "object" || raw === null) return null;
	const anchor = raw as Record<string, unknown>;
	switch (anchor.kind) {
		case "element":
			return typeof anchor.path === "string" && anchor.path
				? { kind: "element", path: anchor.path }
				: null;
		case "text":
			return typeof anchor.quote === "string" && anchor.quote
				? {
						kind: "text",
						quote: anchor.quote,
						offset:
							typeof anchor.offset === "number" && Number.isFinite(anchor.offset)
								? Math.max(0, anchor.offset)
								: 0,
					}
				: null;
		case "ratio":
			return { kind: "ratio" };
		default:
			return null;
	}
}

/**
 * Validate a locator of unknown provenance. `bookId`, when given, must match
 * — a locator saved for another book must never drive navigation here.
 */
export function normalizeReaderLocator(raw: unknown, bookId?: string): ReaderLocator | null {
	if (typeof raw !== "object" || raw === null) return null;
	const loc = raw as Record<string, unknown>;
	if (typeof loc.bookId !== "string" || !loc.bookId) return null;
	if (bookId && loc.bookId !== bookId) return null;
	if (typeof loc.chapterId !== "string" || !loc.chapterId) return null;
	const anchor = normalizeReaderAnchor(loc.anchor) ?? { kind: "ratio" as const };
	const ratio = typeof loc.ratio === "number" ? clamp01(loc.ratio) : 0;
	return {
		schemaVersion: 1,
		bookId: loc.bookId,
		chapterId: loc.chapterId,
		anchor,
		ratio,
		updated: typeof loc.updated === "string" ? loc.updated : new Date().toISOString(),
	};
}

/** True when two locators describe the same position — used to dedupe emits. */
export function sameLocator(a: ReaderLocator | null, b: ReaderLocator | null): boolean {
	if (a === b) return true;
	if (!a || !b) return false;
	if (
		a.bookId !== b.bookId ||
		a.chapterId !== b.chapterId ||
		a.anchor.kind !== b.anchor.kind ||
		Math.abs(a.ratio - b.ratio) > 0.005
	) {
		return false;
	}
	if (a.anchor.kind === "element" && b.anchor.kind === "element") {
		return a.anchor.path === b.anchor.path;
	}
	if (a.anchor.kind === "text" && b.anchor.kind === "text") {
		return a.anchor.quote === b.anchor.quote && a.anchor.offset === b.anchor.offset;
	}
	return true;
}
