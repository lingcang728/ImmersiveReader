import { describe, expect, it } from 'vitest';
import {
	calculateBookProgress,
	chapterTocItems,
	resolveChapterIndex,
	type BookChapter,
} from './books';

const chapters: BookChapter[] = [
	{ id: 'a', path: '001.md', title: '第一章', voteCount: 0, wordCount: 10 },
	{ id: 'b', path: '002.md', title: '第二章', voteCount: 0, wordCount: 20 },
	{ id: 'c', path: '003.md', title: '第三章', voteCount: 0, wordCount: 30 },
];

describe('book reading helpers', () => {
	it('uses manifest order for the chapter table of contents', () => {
		expect(chapterTocItems(chapters)).toEqual([
			{ id: 'a', text: '第一章', level: 1 },
			{ id: 'b', text: '第二章', level: 1 },
			{ id: 'c', text: '第三章', level: 1 },
		]);
	});

	it('combines completed chapters with the current unread chapter position', () => {
		expect(calculateBookProgress(chapters, {
			schemaVersion: 1,
			current: 'b',
			position: 0.5,
			read: ['a'],
			updated: '2026-07-10T00:00:00.000Z',
		})).toBe(0.5);
	});

	it('does not count the current chapter twice when it is already read', () => {
		expect(calculateBookProgress(chapters, {
			schemaVersion: 1,
			current: 'b',
			position: 0.75,
			read: ['a', 'b'],
			updated: '2026-07-10T00:00:00.000Z',
		})).toBeCloseTo(2 / 3);
	});

	it('falls back to the first unread chapter, then the first chapter', () => {
		expect(resolveChapterIndex(chapters, 'missing', ['a'])).toBe(1);
		expect(resolveChapterIndex(chapters, 'missing', ['a', 'b', 'c'])).toBe(0);
		expect(resolveChapterIndex([], 'missing', [])).toBe(-1);
	});
});
