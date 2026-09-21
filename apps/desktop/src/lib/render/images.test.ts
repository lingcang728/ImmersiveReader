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

	it('blocks remote images but keeps embedded images', () => {
		// P2-3: remote http(s) must not become a real request — placeholder pixel.
		expect(resolveMarkdownImageSrc('https://example.com/cover.png', 'C:\\docs\\skills.md', convert)).toBe(
			'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7'
		);
		expect(resolveMarkdownImageSrc('data:image/png;base64,abc', 'C:\\docs\\skills.md', convert)).toBe(
			'data:image/png;base64,abc'
		);
		expect(resolveMarkdownImageSrc('https://asset.localhost/book/a.png', 'C:\\docs\\skills.md', convert)).toBe(
			'https://asset.localhost/book/a.png'
		);
	});
});

describe('resolveMarkdownImageSources', () => {
	it('rewrites only local image src attributes in rendered HTML', () => {
		const html =
			'<p><img src="cover.png" alt="Cover"> <img src="https://example.com/remote.png" alt="Remote"></p>';

		const result = resolveMarkdownImageSources(html, 'C:\\Users\\reader\\docs\\skills.md', convert);

		expect(result).toContain('src="asset://C:\\Users\\reader\\docs\\cover.png"');
		expect(result).not.toContain('src="https://example.com/remote.png"');
		expect(result).toContain('src="data:image/gif;base64,R0lGODlhAQAB');
	});

	it('adds lazy loading and async decoding to rewritten images', () => {
		const result = resolveMarkdownImageSources(
			'<img src="cover.png">',
			'C:\\Users\\reader\\docs\\skills.md',
			convert
		);

		expect(result).toBe(
			'<img src="asset://C:\\Users\\reader\\docs\\cover.png" loading="lazy" decoding="async">'
		);
	});

	it('defers remote and embedded images that are not rewritten', () => {
		const result = resolveMarkdownImageSources(
			'<img src="https://example.com/remote.png"> <img src="data:image/png;base64,abc">',
			'C:\\docs\\skills.md',
			convert
		);

		expect(result).toBe(
			'<img src="data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7" loading="lazy" decoding="async"> ' +
				'<img src="data:image/png;base64,abc" loading="lazy" decoding="async">'
		);
	});

	it('keeps author-specified loading and decoding attributes', () => {
		const result = resolveMarkdownImageSources(
			'<img src="cover.png" loading="eager" decoding="sync">',
			'C:\\docs\\skills.md',
			convert
		);

		expect(result).toContain('loading="eager"');
		expect(result).toContain('decoding="sync"');
		expect(result).not.toContain('loading="lazy"');
		expect(result).not.toContain('decoding="async"');
	});

	it('inserts defer attributes before a self-closing slash', () => {
		const result = resolveMarkdownImageSources(
			'<img src="cover.png"/>',
			'C:\\docs\\skills.md',
			convert
		);

		expect(result).toBe(
			'<img src="asset://C:\\docs\\cover.png" loading="lazy" decoding="async"/>'
		);
	});
});
