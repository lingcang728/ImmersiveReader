import { describe, expect, it } from 'vitest';

import {
	BLOCKED_REMOTE_IMAGE_SRC,
	resolveMarkdownImageSources,
	resolveMarkdownImageSrc
} from './images';

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
		// P2-3: remote http(s) must not become a real request — the src is
		// swapped for an inline SVG placeholder data URI instead.
		expect(resolveMarkdownImageSrc('https://example.com/cover.png', 'C:\\docs\\skills.md', convert)).toBe(
			BLOCKED_REMOTE_IMAGE_SRC
		);
		expect(BLOCKED_REMOTE_IMAGE_SRC.startsWith('data:image/svg+xml')).toBe(true);
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
		// The live src must be the placeholder; the original URL survives only
		// in the data-ir-remote-src diagnostics attribute.
		expect(result).not.toMatch(/\ssrc="https:\/\/example\.com\/remote\.png"/);
		expect(result).toContain(`src="${BLOCKED_REMOTE_IMAGE_SRC}"`);
		expect(result).toContain('remote-blocked');
		expect(result).toContain('data-ir-remote-src="https://example.com/remote.png"');
	});

	it('merges remote-blocked into an existing class list', () => {
		const result = resolveMarkdownImageSources(
			'<img src="https://example.com/remote.png" class="hero  banner">',
			'C:\\docs\\skills.md',
			convert
		);

		expect(result).toContain('class="hero  banner remote-blocked"');
		expect(result).toContain('data-ir-remote-src="https://example.com/remote.png"');
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

		expect(result).toContain('class="remote-blocked"');
		expect(result).toContain('loading="lazy"');
		expect(result).toContain('<img src="data:image/png;base64,abc" loading="lazy" decoding="async">');
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
