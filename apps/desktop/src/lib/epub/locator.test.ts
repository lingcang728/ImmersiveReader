import { describe, expect, it } from "vitest";
import {
	clamp01,
	makeLocator,
	normalizeReaderAnchor,
	normalizeReaderLocator,
	ratioFromScroll,
	sameLocator,
	topOfChapterLocator,
} from "./locator";

describe("clamp01 / ratioFromScroll", () => {
	it("clamps into 0..1", () => {
		expect(clamp01(-1)).toBe(0);
		expect(clamp01(2)).toBe(1);
		expect(clamp01(0.5)).toBe(0.5);
		expect(clamp01(Number.NaN)).toBe(0);
	});

	it("computes scroll ratio", () => {
		expect(ratioFromScroll(50, 1000, 500)).toBe(0.1);
		expect(ratioFromScroll(500, 1000, 500)).toBe(1);
	});

	it("degenerate (non-scrollable) cases return 0", () => {
		expect(ratioFromScroll(0, 500, 500)).toBe(0);
		expect(ratioFromScroll(10, 400, 500)).toBe(0);
	});
});

describe("makeLocator / topOfChapterLocator", () => {
	it("stamps schemaVersion, ratio clamp, RFC-3339 timestamp", () => {
		const loc = makeLocator("b1", "ch1", { kind: "ratio" }, 1.7);
		expect(loc.schemaVersion).toBe(1);
		expect(loc.bookId).toBe("b1");
		expect(loc.chapterId).toBe("ch1");
		expect(loc.ratio).toBe(1);
		expect(() => new Date(loc.updated).toISOString()).not.toThrow();
	});

	it("top locator points at ratio 0", () => {
		const loc = topOfChapterLocator("b", "c");
		expect(loc.anchor).toEqual({ kind: "ratio" });
		expect(loc.ratio).toBe(0);
	});
});

describe("normalizeReaderAnchor", () => {
	it("accepts the three anchor kinds", () => {
		expect(normalizeReaderAnchor({ kind: "element", path: "#a" })).toEqual({
			kind: "element",
			path: "#a",
		});
		expect(normalizeReaderAnchor({ kind: "text", quote: "q", offset: 2 })).toEqual({
			kind: "text",
			quote: "q",
			offset: 2,
		});
		expect(normalizeReaderAnchor({ kind: "ratio" })).toEqual({ kind: "ratio" });
	});

	it("rejects malformed anchors", () => {
		expect(normalizeReaderAnchor(null)).toBeNull();
		expect(normalizeReaderAnchor("x")).toBeNull();
		expect(normalizeReaderAnchor({ kind: "element" })).toBeNull();
		expect(normalizeReaderAnchor({ kind: "element", path: "" })).toBeNull();
		expect(normalizeReaderAnchor({ kind: "text", quote: 1 })).toBeNull();
		expect(normalizeReaderAnchor({ kind: "nope" })).toBeNull();
	});

	it("defaults a missing text offset to 0 and clamps negatives", () => {
		expect(normalizeReaderAnchor({ kind: "text", quote: "q" })).toEqual({
			kind: "text",
			quote: "q",
			offset: 0,
		});
		expect(normalizeReaderAnchor({ kind: "text", quote: "q", offset: -5 })).toEqual({
			kind: "text",
			quote: "q",
			offset: 0,
		});
	});
});

describe("normalizeReaderLocator", () => {
	const raw = {
		bookId: "b1",
		chapterId: "ch2",
		anchor: { kind: "element", path: "#sec1" },
		ratio: 0.4,
		updated: "2026-01-01T00:00:00.000Z",
	};

	it("accepts a well-formed locator", () => {
		expect(normalizeReaderLocator(raw, "b1")).toMatchObject({
			schemaVersion: 1,
			bookId: "b1",
			chapterId: "ch2",
			anchor: { kind: "element", path: "#sec1" },
			ratio: 0.4,
		});
	});

	it("rejects another book's locator", () => {
		expect(normalizeReaderLocator(raw, "other-book")).toBeNull();
	});

	it("rejects malformed locators", () => {
		expect(normalizeReaderLocator(null)).toBeNull();
		expect(normalizeReaderLocator({ bookId: "b1" })).toBeNull();
		expect(normalizeReaderLocator({ ...raw, chapterId: "" })).toBeNull();
	});

	it("falls back to a ratio anchor when the anchor is unreadable", () => {
		const loc = normalizeReaderLocator({ ...raw, anchor: { kind: "bogus" } }, "b1");
		expect(loc?.anchor).toEqual({ kind: "ratio" });
	});

	it("clamps a stale ratio", () => {
		expect(normalizeReaderLocator({ ...raw, ratio: 9 }, "b1")?.ratio).toBe(1);
	});
});

describe("sameLocator", () => {
	const a = makeLocator("b", "c", { kind: "element", path: "#x" }, 0.5);

	it("detects equality and drift", () => {
		expect(sameLocator(a, { ...a })).toBe(true);
		expect(sameLocator(a, null)).toBe(false);
		expect(sameLocator(a, { ...a, chapterId: "other" })).toBe(false);
		expect(sameLocator(a, { ...a, ratio: 0.9 })).toBe(false);
		expect(
			sameLocator(a, makeLocator("b", "c", { kind: "element", path: "#y" }, 0.5)),
		).toBe(false);
	});

	it("treats tiny ratio jitter as equal", () => {
		expect(sameLocator(a, { ...a, ratio: 0.504 })).toBe(true);
	});
});
