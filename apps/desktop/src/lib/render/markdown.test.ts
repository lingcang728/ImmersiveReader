import { describe, expect, it } from 'vitest';

import {
	addHeadingIds,
	extractToc,
	renderMarkdown,
	renderMarkdownDocument,
	stripYamlFrontMatterForRender
} from './markdown';

describe('renderMarkdown', () => {
	it('sanitizes unsafe HTML payloads', async () => {
		const source = [
			'# Title',
			'<script>alert("pwn")</script>',
			'<img src="x" onerror="alert(1)" />'
		].join('\n');

		const html = await renderMarkdown(source);

		expect(html.toLowerCase()).not.toContain('<script');
		expect(html.toLowerCase()).not.toContain('onerror=');
	});

	it('renders footnotes whose links match their target ids', async () => {
		const html = await renderMarkdown('hello[^1]\n\n[^1]: the note text');

		expect(html).toContain('href="#user-content-fn-1"');
		expect(html).toContain('id="user-content-fn-1"');
	});

	it('renders math via KaTeX when the document contains $…$', async () => {
		const html = await renderMarkdown('计算 $x^2 + y^2$ 的值');

		expect(html).toContain('katex');
	});

	it('leaves escaped dollar signs alone', async () => {
		const html = await renderMarkdown('价格是 \\$5 一斤');

		expect(html).not.toContain('katex');
		expect(html).toContain('$5');
	});

	it('keeps source position data attributes for editable blocks', async () => {
		const html = await renderMarkdown('first paragraph\n\nsecond paragraph');

		expect(html).toContain('data-source-start="1"');
		expect(html).toContain('data-source-end="1"');
		expect(html).toContain('data-source-start="3"');
		expect(html).toContain('data-source-end="3"');
	});

	it('still highlights fenced code blocks', async () => {
		const html = await renderMarkdown('```javascript\nconst x = 1;\n```');

		expect(html).toContain('class="shiki');
		expect(html).toContain('const');
	});

	it('preserves source position data on highlighted code blocks', async () => {
		const html = await renderMarkdown('intro\n\n```javascript\nconst x = 1;\n```');

		expect(html).toContain('class="shiki');
		expect(html).toContain('data-source-start="3"');
		expect(html).toContain('data-source-end="5"');
	});

	it('hides leading YAML front matter without shifting source line positions', async () => {
		const source = [
			'---',
			'title: Skills',
			'tags:',
			'  - codex',
			'---',
			'# Heading'
		].join('\n');

		const result = await renderMarkdownDocument(source);

		expect(result.html).not.toContain('title: Skills');
		expect(result.html).toContain('id="heading"');
		expect(result.html).toContain('data-source-start="6"');
		expect(result.html).toContain('data-source-end="6"');
		expect(result.toc).toEqual([{ level: 1, text: 'Heading', id: 'heading' }]);
	});

	it('leaves an opening thematic break alone when there is no YAML close marker', () => {
		const source = '---\n# Heading';

		expect(stripYamlFrontMatterForRender(source)).toBe(source);
	});

	it('decodes escaped heading text for generated ids and TOC labels', async () => {
		const html = addHeadingIds(await renderMarkdown('# Tom & Jerry'));
		const toc = extractToc(html);

		expect(html).toContain('id="tom-jerry"');
		expect(toc).toEqual([{ level: 1, text: 'Tom & Jerry', id: 'tom-jerry' }]);
	});

	it('generates unique ids for duplicate headings', async () => {
		const html = addHeadingIds(await renderMarkdown('## Duplicate\n\n## Duplicate\n\n## !!!'));
		const toc = extractToc(html);

		expect(toc).toEqual([
			{ level: 2, text: 'Duplicate', id: 'duplicate' },
			{ level: 2, text: 'Duplicate', id: 'duplicate-1' },
			{ level: 2, text: '!!!', id: 'unnamed' }
		]);
	});

	it('returns HTML and TOC from the single render pass', async () => {
		const result = await renderMarkdownDocument('# Title\n\n## 子标题');

		expect(result.html).toContain('id="title"');
		expect(result.html).toContain('id="子标题"');
		expect(result.toc).toEqual([
			{ level: 1, text: 'Title', id: 'title' },
			{ level: 2, text: '子标题', id: '子标题' }
		]);
	});
});
