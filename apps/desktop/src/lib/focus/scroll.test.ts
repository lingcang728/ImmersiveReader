import { describe, expect, it } from 'vitest';
import { getFocusScrollTarget } from './scroll';

const base = {
	viewportHeight: 800,
	currentScrollTop: 1000,
	anchorOffset: 400,
	longBlockRatio: 0.78,
	longBlockEdgeBuffer: 96,
};

describe('getFocusScrollTarget', () => {
	it('centers regular blocks on the focus anchor', () => {
		expect(
			getFocusScrollTarget({
				...base,
				blockTop: 1800,
				blockBottom: 2000,
			}),
		).toBe(1500);
	});

	it('keeps native scrolling when the focus anchor is already inside a long block', () => {
		expect(
			getFocusScrollTarget({
				...base,
				blockTop: 1200,
				blockBottom: 2400,
			}),
		).toBeNull();
	});

	it('aligns the start of a long block near the readable top edge when entering from above', () => {
		expect(
			getFocusScrollTarget({
				...base,
				blockTop: 1800,
				blockBottom: 3000,
			}),
		).toBe(1704);
	});

	it('aligns the end of a long block near the readable bottom edge when returning from below', () => {
		expect(
			getFocusScrollTarget({
				...base,
				blockTop: 0,
				blockBottom: 900,
			}),
		).toBe(196);
	});
});
