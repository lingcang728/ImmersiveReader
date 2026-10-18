// Touch-gesture arbitration for the mobile reader.
//
// The decision itself is a pure function (`resolveGesture`) so it can be
// unit-tested; only `gestureExcluded` touches the DOM and is exercised
// on-device. Rules (docs §4):
//   - tap in the middle → toggle chrome
//   - tap near the left/right edges → page back/forward
//   - horizontal swipe (> 60px, mostly horizontal) → previous/next chapter
//   - long-press (text selection), multi-touch, cancelled touches and
//     interactive/media content never trigger navigation
//   - swipeExcluded() from the EPUB kernel is reused for tables / code /
//     images / horizontally scrollable containers — no second exclusion list.

import { swipeExcluded } from '$lib/epub/dom';

export type TapZone = 'prev' | 'center' | 'next';

/** Left/right edge bands that page instead of toggling chrome. */
export const EDGE_TAP_RATIO = 0.22;
export const TAP_MAX_DISTANCE = 12;
export const TAP_MAX_DURATION_MS = 500;
export const SWIPE_MIN_DISTANCE = 60;
export const SWIPE_MAX_CROSS_DISTANCE = 45;
export const SWIPE_MAX_DURATION_MS = 500;
export const LONG_PRESS_MS = 520;

/** Which third of the viewport a tap landed in. */
export function classifyTapZone(clientX: number, viewportWidth: number, edgeRatio = EDGE_TAP_RATIO): TapZone {
	if (!Number.isFinite(clientX) || !Number.isFinite(viewportWidth) || viewportWidth <= 0) {
		return 'center';
	}
	const edge = Math.min(0.45, Math.max(0.05, edgeRatio)) * viewportWidth;
	if (clientX <= edge) return 'prev';
	if (clientX >= viewportWidth - edge) return 'next';
	return 'center';
}

export interface TouchSample {
	x: number;
	y: number;
	time: number;
}

export interface GestureInput {
	start: TouchSample;
	end: TouchSample;
	viewportWidth: number;
	/** A second finger touched down at any point. */
	multiTouch: boolean;
	/** The touch stream was cancelled (scroll started, system gesture…). */
	cancelled: boolean;
	/** The touch started on an excluded element (code/table/image/link/…). */
	excluded: boolean;
	/** Long-press fired / a text selection is active. */
	longPressed?: boolean;
	/** Any non-collapsed selection at release time. */
	selectionActive?: boolean;
	/** The scrolling container scrolled during the gesture (native fling). */
	scrolled?: boolean;
	edgeRatio?: number;
}

export type GestureVerdict =
	| { type: 'none'; reason: string }
	| { type: 'tap'; zone: TapZone }
	| { type: 'swipe'; direction: -1 | 1 };

function distance(a: TouchSample, b: TouchSample): { dx: number; absX: number; absY: number } {
	const dx = b.x - a.x;
	const dy = b.y - a.y;
	return { dx, absX: Math.abs(dx), absY: Math.abs(dy) };
}

/**
 * Decide what one finger gesture means. Pure — the caller feeds a snapshot of
 * flags it tracked during touchstart/touchmove/touchend.
 */
export function resolveGesture(input: GestureInput): GestureVerdict {
	if (input.multiTouch) return { type: 'none', reason: 'multi-touch' };
	if (input.cancelled) return { type: 'none', reason: 'cancelled' };
	if (input.longPressed) return { type: 'none', reason: 'long-press' };
	if (input.selectionActive) return { type: 'none', reason: 'selection' };
	if (input.excluded) return { type: 'none', reason: 'excluded-content' };

	const { dx, absX, absY } = distance(input.start, input.end);
	const duration = Math.max(0, input.end.time - input.start.time);

	// A native scroll/fling owns the gesture: never also page or navigate.
	if (input.scrolled) return { type: 'none', reason: 'scrolled' };

	if (
		absX >= SWIPE_MIN_DISTANCE &&
		absY <= SWIPE_MAX_CROSS_DISTANCE &&
		absX > absY * 1.4 &&
		duration <= SWIPE_MAX_DURATION_MS
	) {
		// Swipe left (dx < 0) = forward, like turning a page.
		return { type: 'swipe', direction: dx < 0 ? 1 : -1 };
	}

	if (absX <= TAP_MAX_DISTANCE && absY <= TAP_MAX_DISTANCE && duration <= TAP_MAX_DURATION_MS) {
		return { type: 'tap', zone: classifyTapZone(input.end.x, input.viewportWidth, input.edgeRatio) };
	}

	return { type: 'none', reason: 'ambiguous' };
}

/** Whether a tap on `clientX` would land on chrome-toggling center or a page edge. */
export function isEdgeTap(clientX: number, viewportWidth: number, edgeRatio = EDGE_TAP_RATIO): boolean {
	return classifyTapZone(clientX, viewportWidth, edgeRatio) !== 'center';
}

const LINK_SELECTOR = 'a[href], area[href]';
const INTERACTIVE_SELECTOR = 'button, input, textarea, select, summary, [role="button"], [data-no-gesture]';

/**
 * DOM predicate: true when the gesture started on content that owns the
 * interaction (text selection, media, code, tables, horizontal scrollers,
 * links, form controls). Reuses the EPUB kernel's `swipeExcluded` list.
 */
export function gestureExcluded(target: EventTarget | null, boundary: HTMLElement | null): boolean {
	if (!(target instanceof Element)) return false;
	if (!boundary || !boundary.contains(target)) return false;
	if (target.closest(LINK_SELECTOR)) return true;
	if (target.closest(INTERACTIVE_SELECTOR)) return true;
	if (target instanceof HTMLElement && target.isContentEditable) return true;
	return swipeExcluded(target, boundary);
}

/** True when the document currently has a non-collapsed text selection. */
export function hasActiveSelection(): boolean {
	if (typeof window === 'undefined' || typeof window.getSelection !== 'function') return false;
	const selection = window.getSelection();
	return selection !== null && !selection.isCollapsed && (selection.rangeCount ?? 0) > 0;
}
