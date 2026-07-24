export type ReadingScrollKind = "line" | "page";

export interface ReadingScrollIntent {
	direction: -1 | 1;
	kind: ReadingScrollKind;
}

export type ReadingScrollResolution =
	| { type: "scroll"; top: number }
	| { type: "chapter"; direction: -1 | 1; offsetPx: number };

export type FocusStepResolution =
	| { type: "focus"; index: number }
	| { type: "chapter"; direction: -1 | 1 };

export interface ChapterNavigationKeyLatch {
	isLatched(key: string): boolean;
	tryLatch(key: string): boolean;
	release(key: string): void;
	reset(): void;
}

const LINE_SCROLL_PX = 56;
const PAGE_SCROLL_RATIO = 0.82;

export function createChapterNavigationKeyLatch(): ChapterNavigationKeyLatch {
	const latchedKeys = new Set<string>();
	return {
		isLatched(key) {
			return latchedKeys.has(key);
		},
		tryLatch(key) {
			if (latchedKeys.has(key)) return false;
			latchedKeys.add(key);
			return true;
		},
		release(key) {
			latchedKeys.delete(key);
		},
		reset() {
			latchedKeys.clear();
		}
	};
}

export function readingScrollIntentForKey(
	key: string,
	shiftKey = false
): ReadingScrollIntent | null {
	switch (key) {
		case "ArrowUp":
			return { direction: -1, kind: "line" };
		case "ArrowDown":
			return { direction: 1, kind: "line" };
		case "ArrowLeft":
		case "PageUp":
			return { direction: -1, kind: "page" };
		case "ArrowRight":
		case "PageDown":
			return { direction: 1, kind: "page" };
		case " ":
		case "Spacebar":
			return { direction: shiftKey ? -1 : 1, kind: "page" };
		default:
			return null;
	}
}

export function resolveReadingScroll(
	intent: ReadingScrollIntent,
	viewport: {
		scrollTop: number;
		scrollHeight: number;
		clientHeight: number;
	}
): ReadingScrollResolution {
	const maxScrollTop = Math.max(0, viewport.scrollHeight - viewport.clientHeight);
	const currentScrollTop = Math.max(0, Math.min(maxScrollTop, viewport.scrollTop));
	const distance =
		intent.kind === "line"
			? LINE_SCROLL_PX
			: Math.max(LINE_SCROLL_PX, viewport.clientHeight * PAGE_SCROLL_RATIO);
	const requestedTop = currentScrollTop + intent.direction * distance;

	// Carry the unused part of the key gesture across the chapter seam. This
	// makes separate Markdown files behave like one continuous document:
	// ArrowUp near the start lands inside the previous chapter instead of
	// stopping at its absolute bottom (or, worse, jumping to its top).
	if (requestedTop < 0) {
		return {
			type: "chapter",
			direction: -1,
			offsetPx: Math.abs(requestedTop)
		};
	}
	if (requestedTop > maxScrollTop) {
		return {
			type: "chapter",
			direction: 1,
			offsetPx: requestedTop - maxScrollTop
		};
	}

	return {
		type: "scroll",
		top: requestedTop
	};
}

export function resolveChapterBoundaryScroll(
	direction: -1 | 1,
	maxScrollTop: number,
	offsetPx: number
): number {
	const safeMax = Math.max(0, Number.isFinite(maxScrollTop) ? maxScrollTop : 0);
	const safeOffset = Math.max(0, Number.isFinite(offsetPx) ? offsetPx : 0);
	return direction > 0
		? Math.min(safeMax, safeOffset)
		: Math.max(0, safeMax - safeOffset);
}

export function resolveFocusStep(
	currentIndex: number,
	unitCount: number,
	direction: -1 | 1
): FocusStepResolution {
	const safeUnitCount = Number.isFinite(unitCount)
		? Math.max(0, Math.trunc(unitCount))
		: 0;
	if (safeUnitCount === 0) {
		return { type: "chapter", direction };
	}

	const normalizedCurrentIndex = Number.isFinite(currentIndex)
		? Math.trunc(currentIndex)
		: 0;
	const safeCurrentIndex = Math.max(
		0,
		Math.min(safeUnitCount - 1, normalizedCurrentIndex)
	);
	const requestedIndex = safeCurrentIndex + direction;
	if (requestedIndex < 0 || requestedIndex >= safeUnitCount) {
		return { type: "chapter", direction };
	}
	return { type: "focus", index: requestedIndex };
}
