import { describe, expect, it } from "vitest";
import {
	createChapterNavigationKeyLatch,
	readingScrollIntentForKey,
	resolveChapterBoundaryScroll,
	resolveFocusStep,
	resolveReadingScroll
} from "./navigation";

describe("reading keyboard navigation", () => {
	it("maps arrows and space to native-style line and page scrolling", () => {
		expect(readingScrollIntentForKey("ArrowUp")).toEqual({
			direction: -1,
			kind: "line"
		});
		expect(readingScrollIntentForKey("ArrowDown")).toEqual({
			direction: 1,
			kind: "line"
		});
		expect(readingScrollIntentForKey("ArrowLeft")).toEqual({
			direction: -1,
			kind: "page"
		});
		expect(readingScrollIntentForKey("ArrowRight")).toEqual({
			direction: 1,
			kind: "page"
		});
		expect(readingScrollIntentForKey(" ")).toEqual({
			direction: 1,
			kind: "page"
		});
		expect(readingScrollIntentForKey(" ", true)).toEqual({
			direction: -1,
			kind: "page"
		});
	});

	it("scrolls inside the current chapter before crossing a boundary", () => {
		expect(
			resolveReadingScroll(
				{ direction: 1, kind: "line" },
				{ scrollTop: 100, scrollHeight: 1000, clientHeight: 400 }
			)
		).toEqual({ type: "scroll", top: 156 });

		expect(
			resolveReadingScroll(
				{ direction: -1, kind: "page" },
				{ scrollTop: 500, scrollHeight: 1200, clientHeight: 400 }
			)
		).toEqual({ type: "scroll", top: 172 });
	});

	it("turns forward and backward input at the edges into chapter navigation", () => {
		expect(
			resolveReadingScroll(
				{ direction: 1, kind: "page" },
				{ scrollTop: 600, scrollHeight: 1000, clientHeight: 400 }
			)
		).toEqual({ type: "chapter", direction: 1, offsetPx: 328 });

		expect(
			resolveReadingScroll(
				{ direction: -1, kind: "line" },
				{ scrollTop: 0, scrollHeight: 1000, clientHeight: 400 }
			)
		).toEqual({ type: "chapter", direction: -1, offsetPx: 56 });
	});

	it("carries only the unconsumed part of an input across the chapter seam", () => {
		expect(
			resolveReadingScroll(
				{ direction: -1, kind: "line" },
				{ scrollTop: 20, scrollHeight: 1000, clientHeight: 400 }
			)
		).toEqual({ type: "chapter", direction: -1, offsetPx: 36 });

		expect(
			resolveReadingScroll(
				{ direction: 1, kind: "line" },
				{ scrollTop: 570, scrollHeight: 1000, clientHeight: 400 }
			)
		).toEqual({ type: "chapter", direction: 1, offsetPx: 26 });
	});

	it("places boundary input inside the adjacent chapter like continuous scrolling", () => {
		expect(resolveChapterBoundaryScroll(-1, 1200, 56)).toBe(1144);
		expect(resolveChapterBoundaryScroll(1, 1200, 56)).toBe(56);
		expect(resolveChapterBoundaryScroll(-1, 40, 100)).toBe(0);
		expect(resolveChapterBoundaryScroll(1, 40, 100)).toBe(40);
	});

	it("allows only one chapter transition per physical key press", () => {
		const latch = createChapterNavigationKeyLatch();

		expect(latch.tryLatch("ArrowUp")).toBe(true);
		expect(latch.isLatched("ArrowUp")).toBe(true);
		expect(latch.tryLatch("ArrowUp")).toBe(false);
		expect(latch.tryLatch("ArrowDown")).toBe(true);

		latch.release("ArrowUp");
		expect(latch.tryLatch("ArrowUp")).toBe(true);

		latch.reset();
		expect(latch.isLatched("ArrowUp")).toBe(false);
		expect(latch.isLatched("ArrowDown")).toBe(false);
	});

	it("resolves focus arrows to exactly one unit or one chapter boundary", () => {
		expect(resolveFocusStep(4, 10, -1)).toEqual({
			type: "focus",
			index: 3
		});
		expect(resolveFocusStep(4, 10, 1)).toEqual({
			type: "focus",
			index: 5
		});
		expect(resolveFocusStep(0, 10, -1)).toEqual({
			type: "chapter",
			direction: -1
		});
		expect(resolveFocusStep(9, 10, 1)).toEqual({
			type: "chapter",
			direction: 1
		});
	});
});
