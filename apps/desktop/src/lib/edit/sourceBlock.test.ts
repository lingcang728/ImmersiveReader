import { describe, expect, it } from 'vitest';

import {
	getMarkdownSourceBlock,
	normalizeMarkdownEditText,
	replaceMarkdownSourceBlock
} from './sourceBlock';

describe('markdown source block editing', () => {
	it('extracts a one-based source block range', () => {
		const source = ['# Title', '', 'A **bold** paragraph.', '', '- item'].join('\n');

		expect(getMarkdownSourceBlock(source, 3, 3)).toBe('A **bold** paragraph.');
		expect(getMarkdownSourceBlock(source, 5, 5)).toBe('- item');
	});

	it('replaces a source block without touching surrounding content', () => {
		const source = ['before', '', 'A [link](https://example.com).', '', 'after'].join('\n');
		const updated = replaceMarkdownSourceBlock(source, 3, 3, 'A [renamed link](https://example.com).');

		expect(updated).toBe(['before', '', 'A [renamed link](https://example.com).', '', 'after'].join('\n'));
	});

	it('supports multiline replacements for complex blocks', () => {
		const source = ['> old', '> quote', '', 'next'].join('\n');
		const updated = replaceMarkdownSourceBlock(source, 1, 2, '> new\n> quote');

		expect(updated).toBe(['> new', '> quote', '', 'next'].join('\n'));
	});

	it('rejects invalid ranges', () => {
		expect(getMarkdownSourceBlock('one', 0, 1)).toBeNull();
		expect(replaceMarkdownSourceBlock('one', 2, 2, 'two')).toBeNull();
	});

	it('normalizes browser edit newlines before saving', () => {
		expect(normalizeMarkdownEditText('a\r\nb\rc')).toBe('a\nb\nc');
	});
});
