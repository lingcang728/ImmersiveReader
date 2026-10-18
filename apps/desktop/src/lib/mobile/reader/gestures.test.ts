import { describe, expect, it } from 'vitest';

import {
	EDGE_TAP_RATIO,
	classifyTapZone,
	isEdgeTap,
	resolveGesture,
	type GestureInput,
	type TouchSample,
} from './gestures';

const VIEWPORT = 400;
const start = (x = 200, y = 300, time = 0): TouchSample => ({ x, y, time });

function input(overrides: Partial<GestureInput> = {}): GestureInput {
	return {
		start: start(),
		end: start(200, 300, 80),
		viewportWidth: VIEWPORT,
		multiTouch: false,
		cancelled: false,
		excluded: false,
		...overrides,
	};
}

describe('classifyTapZone', () => {
	it('maps the left/right bands to page turns and the middle to chrome', () => {
		expect(classifyTapZone(0, VIEWPORT)).toBe('prev');
		expect(classifyTapZone(VIEWPORT * EDGE_TAP_RATIO, VIEWPORT)).toBe('prev');
		expect(classifyTapZone(VIEWPORT / 2, VIEWPORT)).toBe('center');
		expect(classifyTapZone(VIEWPORT - VIEWPORT * EDGE_TAP_RATIO, VIEWPORT)).toBe('next');
		expect(classifyTapZone(VIEWPORT, VIEWPORT)).toBe('next');
	});

	it('degrades to center for unusable geometry', () => {
		expect(classifyTapZone(Number.NaN, VIEWPORT)).toBe('center');
		expect(classifyTapZone(10, 0)).toBe('center');
	});

	it('isEdgeTap mirrors the zone decision', () => {
		expect(isEdgeTap(10, VIEWPORT)).toBe(true);
		expect(isEdgeTap(VIEWPORT / 2, VIEWPORT)).toBe(false);
	});
});

describe('resolveGesture', () => {
	it('recognizes a center tap', () => {
		expect(resolveGesture(input())).toEqual({ type: 'tap', zone: 'center' });
	});

	it('recognizes edge taps as paging gestures', () => {
		expect(
			resolveGesture(input({ start: start(10, 300, 0), end: start(12, 302, 60) })),
		).toEqual({ type: 'tap', zone: 'prev' });
		expect(
			resolveGesture(input({ start: start(390, 300, 0), end: start(392, 302, 60) })),
		).toEqual({ type: 'tap', zone: 'next' });
	});

	it('turns a left swipe into a forward chapter change', () => {
		expect(
			resolveGesture(input({ start: start(300, 300, 0), end: start(200, 310, 220) })),
		).toEqual({ type: 'swipe', direction: 1 });
	});

	it('turns a right swipe into a backward chapter change', () => {
		expect(
			resolveGesture(input({ start: start(100, 300, 0), end: start(200, 310, 220) })),
		).toEqual({ type: 'swipe', direction: -1 });
	});

	it('rejects swipes that are too short or too slow', () => {
		expect(
			resolveGesture(input({ start: start(300, 300, 0), end: start(250, 300, 200) })),
		).toEqual({ type: 'none', reason: 'ambiguous' });
		expect(
			resolveGesture(input({ start: start(300, 300, 0), end: start(150, 300, 900) })),
		).toEqual({ type: 'none', reason: 'ambiguous' });
	});

	it('rejects diagonal drags', () => {
		expect(
			resolveGesture(input({ start: start(300, 300, 0), end: start(220, 220, 200) })),
		).toEqual({ type: 'none', reason: 'ambiguous' });
	});

	it('ignores long-press, selection, multi-touch and cancelled touches', () => {
		expect(resolveGesture(input({ longPressed: true }))).toEqual({
			type: 'none',
			reason: 'long-press',
		});
		expect(resolveGesture(input({ selectionActive: true }))).toEqual({
			type: 'none',
			reason: 'selection',
		});
		expect(resolveGesture(input({ multiTouch: true }))).toEqual({
			type: 'none',
			reason: 'multi-touch',
		});
		expect(resolveGesture(input({ cancelled: true }))).toEqual({
			type: 'none',
			reason: 'cancelled',
		});
	});

	it('ignores gestures on excluded content and native scrolls', () => {
		expect(resolveGesture(input({ excluded: true }))).toEqual({
			type: 'none',
			reason: 'excluded-content',
		});
		expect(
			resolveGesture(
				input({ start: start(300, 300, 0), end: start(200, 300, 200), scrolled: true }),
			),
		).toEqual({ type: 'none', reason: 'scrolled' });
	});

	it('treats a slow drag as no gesture rather than a page tap', () => {
		expect(
			resolveGesture(input({ start: start(200, 300, 0), end: start(204, 304, 800) })),
		).toEqual({ type: 'none', reason: 'ambiguous' });
	});
});
