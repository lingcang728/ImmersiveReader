import { describe, expect, it } from "vitest";
import {
	readingScrollIntentForKey,
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
		).toEqual({ type: "chapter", direction: 1 });

		expect(
			resolveReadingScroll(
				{ direction: -1, kind: "line" },
				{ scrollTop: 0, scrollHeight: 1000, clientHeight: 400 }
			)
		).toEqual({ type: "chapter", direction: -1 });
	});
});
