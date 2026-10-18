import { describe, expect, it } from 'vitest';

import { MAX_CHAPTER_MATCHES, findOccurrences } from './search';

describe('findOccurrences', () => {
	it('finds every case-insensitive occurrence', () => {
		expect(findOccurrences('Alpha alpha ALPHA', 'alpha')).toEqual([0, 6, 12]);
	});

	it('returns every offset inside a run without loops', () => {
		expect(findOccurrences('aaaa', 'aa')).toEqual([0, 2]);
	});

	it('trims the query and treats blank queries as no match', () => {
		expect(findOccurrences('阅读 阅读', '  阅读 ')).toEqual([0, 3]);
		expect(findOccurrences('阅读', '   ')).toEqual([]);
		expect(findOccurrences('阅读', '')).toEqual([]);
	});

	it('handles mixed CJK / latin text', () => {
		expect(findOccurrences('沉浸阅读 ImmersiveReader 沉浸', '沉浸')).toEqual([0, 21]);
	});

	it('returns nothing when the query is absent', () => {
		expect(findOccurrences('hello world', 'missing')).toEqual([]);
	});

	it('guards against pathological long inputs', () => {
		const text = 'a'.repeat(30_000);
		const offsets = findOccurrences(text, 'a');
		expect(offsets.length).toBe(10_001);
		expect(MAX_CHAPTER_MATCHES).toBeLessThan(offsets.length);
	});
});
