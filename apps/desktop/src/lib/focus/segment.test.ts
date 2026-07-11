import { describe, expect, it } from 'vitest';
import { splitSentences, splitSentencesFallback } from './segment';

function texts(source: string, ranges: { start: number; end: number }[]) {
	return ranges.map((r) => source.slice(r.start, r.end));
}

describe('splitSentencesFallback', () => {
	it('splits Chinese sentences at 。！？', () => {
		const source = '第一句话。第二句话！第三句话？还有结尾没有标点的部分';
		const parts = texts(source, splitSentencesFallback(source));
		expect(parts).toEqual(['第一句话。', '第二句话！', '第三句话？', '还有结尾没有标点的部分']);
	});

	it('keeps closing quotes attached to the sentence', () => {
		const source = '他说：“今天很好。”然后离开了。';
		const parts = texts(source, splitSentencesFallback(source));
		expect(parts[0]).toBe('他说：“今天很好。”');
		expect(parts[1]).toBe('然后离开了。');
	});

	it('covers the full string without gaps', () => {
		const source = '短句。中间一段较长的内容，没有终止符,直到结束!尾巴';
		const ranges = splitSentencesFallback(source);
		expect(ranges[0].start).toBe(0);
		expect(ranges[ranges.length - 1].end).toBe(source.length);
		for (let i = 1; i < ranges.length; i++) {
			expect(ranges[i].start).toBe(ranges[i - 1].end);
		}
	});

	it('returns a single range for text without terminators', () => {
		const source = '没有任何句号的一整段文字';
		expect(splitSentencesFallback(source)).toEqual([{ start: 0, end: source.length }]);
	});
});

describe('splitSentences', () => {
	it('returns non-empty ordered ranges for mixed text', () => {
		const source = 'This is a sentence. 这是第二句。And a third one!';
		const ranges = splitSentences(source);
		expect(ranges.length).toBeGreaterThanOrEqual(2);
		expect(ranges[0].start).toBe(0);
		expect(ranges[ranges.length - 1].end).toBe(source.length);
		for (let i = 1; i < ranges.length; i++) {
			expect(ranges[i].start).toBeGreaterThanOrEqual(ranges[i - 1].end);
		}
	});

	it('skips whitespace-only pieces', () => {
		const ranges = splitSentences('   ');
		expect(ranges).toEqual([]);
	});
});
