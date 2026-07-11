import { describe, expect, it } from 'vitest';

import { resolveMarkdownImageSources, resolveMarkdownImageSrc } from './images';

const convert = (path: string) => `asset://${path}`;

describe('resolveMarkdownImageSrc', () => {
	it('resolves relative image paths against the markdown file directory', () => {
		const result = resolveMarkdownImageSrc(
			'images/cover%20one.png',
			'C:\\Users\\reader\\docs\\skills.md',
			convert
		);

		expect(result).toBe('asset://C:\\Users\\reader\\docs\\images\\cover one.png');
	});

	it('normalizes parent directory segments', () => {
		const result = resolveMarkdownImageSrc(
			'../assets/cover.png',
			'C:\\Users\\reader\\docs\\chapter\\skills.md',
			convert
		);

		expect(result).toBe('asset://C:\\Users\\reader\\docs\\assets\\cover.png');
	});

	it('does not rewrite remote or embedded images', () => {
		expect(resolveMarkdownImageSrc('https://example.com/cover.png', 'C:\\docs\\skills.md', convert)).toBe(
			'https://example.com/cover.png'
		);
		expect(resolveMarkdownImageSrc('data:image/png;base64,abc', 'C:\\docs\\skills.md', convert)).toBe(
			'data:image/png;base64,abc'
		);
	});
});

describe('resolveMarkdownImageSources', () => {
	it('rewrites only local image src attributes in rendered HTML', () => {
		const html =
			'<p><img src="cover.png" alt="Cover"> <img src="https://example.com/remote.png" alt="Remote"></p>';

		const result = resolveMarkdownImageSources(html, 'C:\\Users\\reader\\docs\\skills.md', convert);

		expect(result).toContain('src="asset://C:\\Users\\reader\\docs\\cover.png"');
		expect(result).toContain('src="https://example.com/remote.png"');
	});
});
