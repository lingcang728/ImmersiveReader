export type ReadingScrollKind = "line" | "page";

export interface ReadingScrollIntent {
	direction: -1 | 1;
	kind: ReadingScrollKind;
}

export type ReadingScrollResolution =
	| { type: "scroll"; top: number }
	| { type: "chapter"; direction: -1 | 1 };

const LINE_SCROLL_PX = 56;
const PAGE_SCROLL_RATIO = 0.82;
const EDGE_EPSILON_PX = 1;

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
	const atBoundary =
		intent.direction > 0
			? maxScrollTop - currentScrollTop <= EDGE_EPSILON_PX
			: currentScrollTop <= EDGE_EPSILON_PX;

	if (atBoundary) {
		return { type: "chapter", direction: intent.direction };
	}

	const distance =
		intent.kind === "line"
			? LINE_SCROLL_PX
			: Math.max(LINE_SCROLL_PX, viewport.clientHeight * PAGE_SCROLL_RATIO);
	return {
		type: "scroll",
		top: Math.max(
			0,
			Math.min(maxScrollTop, currentScrollTop + intent.direction * distance)
		)
	};
}
