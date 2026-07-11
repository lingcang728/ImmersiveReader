import { unified } from 'unified';
import remarkParse from 'remark-parse';
import remarkGfm from 'remark-gfm';
import remarkRehype from 'remark-rehype';
import rehypeRaw from 'rehype-raw';
import rehypeSanitize, { defaultSchema } from 'rehype-sanitize';
import rehypeStringify from 'rehype-stringify';
import type { HighlighterCore, LanguageRegistration } from 'shiki/core';

const sanitizeSchema = {
	...defaultSchema,
	// remark-rehype already prefixes footnote ids with "user-content"; letting
	// sanitize clobber them again would break footnote links (href keeps the
	// single prefix while the id gets a double one).
	clobberPrefix: '',
	attributes: {
		...defaultSchema.attributes,
		code: [
			...((defaultSchema.attributes?.code as any[]) ?? []),
			['className', /^language-[\w+-]+$/]
		],
		// remark-math output; rehype-katex picks these up after sanitizing.
		span: [
			...((defaultSchema.attributes?.span as any[]) ?? []),
			['className', 'math', 'math-inline', 'math-display']
		],
		div: [
			...((defaultSchema.attributes?.div as any[]) ?? []),
			['className', 'math', 'math-inline', 'math-display']
		]
	}
};

type LanguageLoader = () => Promise<{ default: LanguageRegistration[] }>;

let highlighterPromise: Promise<HighlighterCore> | null = null;
const loadedLanguages = new Set<string>();

const languageLoaders: Record<string, LanguageLoader> = {
	bash: () => import('@shikijs/langs/bash'),
	c: () => import('@shikijs/langs/c'),
	cpp: () => import('@shikijs/langs/cpp'),
	css: () => import('@shikijs/langs/css'),
	go: () => import('@shikijs/langs/go'),
	html: () => import('@shikijs/langs/html'),
	java: () => import('@shikijs/langs/java'),
	javascript: () => import('@shikijs/langs/javascript'),
	json: () => import('@shikijs/langs/json'),
	markdown: () => import('@shikijs/langs/markdown'),
	python: () => import('@shikijs/langs/python'),
	rust: () => import('@shikijs/langs/rust'),
	sql: () => import('@shikijs/langs/sql'),
	typescript: () => import('@shikijs/langs/typescript'),
	yaml: () => import('@shikijs/langs/yaml')
};

const languageAliases: Record<string, string> = {
	cjs: 'javascript',
	'c++': 'cpp',
	js: 'javascript',
	mjs: 'javascript',
	md: 'markdown',
	mts: 'typescript',
	py: 'python',
	rs: 'rust',
	sh: 'bash',
	shell: 'bash',
	shellscript: 'bash',
	ts: 'typescript',
	txt: 'plaintext',
	text: 'plaintext',
	plain: 'plaintext',
	yml: 'yaml'
};

function normalizeLanguage(lang: string): string {
	const normalized = lang.trim().toLowerCase();
	return languageAliases[normalized] ?? normalized;
}

async function getHighlighter(): Promise<HighlighterCore> {
	if (!highlighterPromise) {
		highlighterPromise = Promise.all([
			import('shiki/core'),
			import('shiki/engine/javascript'),
			import('@shikijs/themes/github-light'),
			import('@shikijs/themes/github-dark')
		]).then(([core, engine, githubLight, githubDark]) =>
			core.createHighlighterCore({
				engine: engine.createJavaScriptRegexEngine(),
				themes: [githubLight.default, githubDark.default],
				langs: [],
				langAlias: {
					cjs: 'javascript',
					js: 'javascript',
					mjs: 'javascript',
					md: 'markdown',
					mts: 'typescript',
					py: 'python',
					rs: 'rust',
					sh: 'bash',
					shell: 'bash',
					shellscript: 'bash',
					ts: 'typescript',
					yml: 'yaml'
				}
			})
		);
	}
	return highlighterPromise;
}

async function ensureLanguages(langs: string[]) {
	const highlighter = await getHighlighter();
	const registrations: LanguageRegistration[][] = [];

	for (const rawLang of langs) {
		const lang = normalizeLanguage(rawLang);
		if (lang === 'plaintext' || loadedLanguages.has(lang)) continue;

		const loader = languageLoaders[lang];
		if (!loader) continue;
		const module = await loader();
		registrations.push(module.default);
		loadedLanguages.add(lang);
	}

	if (registrations.length > 0) {
		await highlighter.loadLanguage(...registrations);
	}

	return highlighter;
}

function escapeHtml(str: string): string {
	return str
		.replace(/&/g, '&amp;')
		.replace(/</g, '&lt;')
		.replace(/>/g, '&gt;')
		.replace(/"/g, '&quot;')
		.replace(/'/g, '&#39;');
}

function blankLineContent(line: string): string {
	return line.replace(/[^\r\n]/g, '');
}

function lineContent(line: string): string {
	return line.replace(/\r\n$|\n$|\r$/g, '');
}

export function stripYamlFrontMatterForRender(source: string): string {
	const lines = source.match(/.*(?:\r\n|\n|\r|$)/g) ?? [];
	if (lines.length > 0 && lines[lines.length - 1] === '') lines.pop();
	if (lines.length < 2) return source;

	const firstRawLine = lines[0];
	if (firstRawLine === undefined) return source;

	const firstLine = lineContent(firstRawLine).replace(/^\ufeff/, '');
	if (!/^---[ \t]*$/.test(firstLine)) return source;

	let closingLineIndex = -1;
	for (let i = 1; i < lines.length; i += 1) {
		const line = lines[i];
		if (line !== undefined && /^(---|\.\.\.)[ \t]*$/.test(lineContent(line))) {
			closingLineIndex = i;
			break;
		}
	}

	if (closingLineIndex === -1) return source;

	return lines
		.map((line, index) => (index <= closingLineIndex ? blankLineContent(line) : line))
		.join('');
}

export interface TocItem {
	level: number;
	text: string;
	id: string;
}

export interface FrontMatterEntry {
	key: string;
	value: string;
}

export interface RenderedMarkdownDocument {
	html: string;
	toc: TocItem[];
	frontMatter: FrontMatterEntry[];
}

// Line-based YAML-lite parser: `key: value`, inline `[a, b]` arrays and
// simple `- item` lists. Nested structures are skipped, not mangled.
export function extractYamlFrontMatterEntries(source: string): FrontMatterEntry[] {
	const lines = source.match(/.*(?:\r\n|\n|\r|$)/g) ?? [];
	if (lines.length > 0 && lines[lines.length - 1] === '') lines.pop();
	if (lines.length < 2) return [];

	const firstLine = lineContent(lines[0] ?? '').replace(/^﻿/, '');
	if (!/^---[ \t]*$/.test(firstLine)) return [];

	let closingLineIndex = -1;
	for (let i = 1; i < lines.length; i += 1) {
		if (/^(---|\.\.\.)[ \t]*$/.test(lineContent(lines[i] ?? ''))) {
			closingLineIndex = i;
			break;
		}
	}
	if (closingLineIndex === -1) return [];

	const cleanScalar = (raw: string) => raw.trim().replace(/^["']|["']$/g, '');
	const entries: FrontMatterEntry[] = [];
	let pendingListKey: string | null = null;
	let pendingListValues: string[] = [];

	const flushPendingList = () => {
		if (pendingListKey !== null && pendingListValues.length > 0) {
			entries.push({ key: pendingListKey, value: pendingListValues.join('、') });
		}
		pendingListKey = null;
		pendingListValues = [];
	};

	for (let i = 1; i < closingLineIndex; i += 1) {
		const line = lineContent(lines[i] ?? '');

		const listItem = /^\s+-\s+(.+)$/.exec(line);
		if (listItem && pendingListKey !== null) {
			pendingListValues.push(cleanScalar(listItem[1]));
			continue;
		}

		const kv = /^([A-Za-z0-9_][\w .-]*):\s*(.*)$/.exec(line);
		if (!kv) continue;
		flushPendingList();

		const key = kv[1].trim();
		let value = kv[2].trim();
		if (value === '') {
			pendingListKey = key;
			continue;
		}
		if (/^\[.*\]$/.test(value)) {
			value = value
				.slice(1, -1)
				.split(',')
				.map(cleanScalar)
				.filter(Boolean)
				.join('、');
		} else {
			value = cleanScalar(value);
		}
		if (value !== '') entries.push({ key, value });
	}
	flushPendingList();

	return entries;
}

function decodeHtmlEntities(text: string): string {
	return text.replace(/&(#x?[0-9a-fA-F]+|[a-zA-Z]+);/g, (full, entity) => {
		if (entity[0] === '#') {
			const isHex = entity[1]?.toLowerCase() === 'x';
			const value = Number.parseInt(entity.slice(isHex ? 2 : 1), isHex ? 16 : 10);
			if (Number.isFinite(value)) {
				try {
					return String.fromCodePoint(value);
				} catch {
					return full;
				}
			}
			return full;
		}

		const namedEntities: Record<string, string> = {
			amp: '&',
			apos: "'",
			gt: '>',
			lt: '<',
			nbsp: '\u00a0',
			quot: '"'
		};
		return namedEntities[entity] ?? full;
	});
}

function textFromHast(node: any): string {
	if (!node) return '';
	if (node.type === 'text') return node.value ?? '';
	if (!node.children) return '';
	return node.children.map((child: any) => textFromHast(child)).join('');
}

function stripHeadingHtml(content: string): string {
	return decodeHtmlEntities(content.replace(/<[^>]*>/g, ''));
}

function generateHeadingId(text: string, seenIds: Map<string, number>): string {
	let id = text.trim().toLowerCase()
		.replace(/[^\w\u4e00-\u9fff]+/g, '-')
		.replace(/^-|-$/g, '');
	// After sanitization, if nothing remains (e.g. "!!!"), fallback to 'unnamed'
	if (!id) {
		id = 'unnamed';
	}
	// IDs starting with digits are invalid in HTML4 and break querySelector
	if (/^\d/.test(id)) {
		id = 'heading-' + id;
	}
	// Ensure uniqueness by appending counter for duplicates
	const count = seenIds.get(id) || 0;
	seenIds.set(id, count + 1);
	if (count > 0) {
		id = `${id}-${count}`;
	}
	return id;
}

function rehypeDocumentMetadata(toc: TocItem[]) {
	const blockTags = new Set(['p', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'blockquote', 'pre', 'table', 'ul', 'ol', 'li', 'hr']);
	const seenIds = new Map<string, number>();

	function walk(node: any) {
		if (node.type === 'element') {
			if (!node.properties) node.properties = {};

			if (node.position && blockTags.has(node.tagName)) {
				node.properties.dataSourceStart = node.position.start.line;
				node.properties.dataSourceEnd = node.position.end.line;
			}

			const headingMatch = /^h([1-6])$/.exec(node.tagName);
			if (headingMatch) {
				const text = textFromHast(node);
				if (!node.properties.id) {
					node.properties.id = generateHeadingId(text, seenIds);
				} else {
					seenIds.set(String(node.properties.id), (seenIds.get(String(node.properties.id)) || 0) + 1);
				}
				toc.push({
					level: Number.parseInt(headingMatch[1], 10),
					text,
					id: String(node.properties.id)
				});
			}
		}
		if (node.children) {
			for (const child of node.children) walk(child);
		}
	}
	return (tree: any) => walk(tree);
}

function collectCodeLanguages(html: string): string[] {
	const langs = new Set<string>();
	const regex = /<pre[^>]*><code class="language-([\w+-]+)">/g;
	let match;
	while ((match = regex.exec(html)) !== null) {
		langs.add(match[1]);
	}
	return [...langs];
}

async function highlightCodeBlocks(html: string): Promise<string> {
	const langs = collectCodeLanguages(html);
	if (langs.length === 0) return html;

	const hl = await ensureLanguages(langs);
	return html.replace(
		/<pre([^>]*)><code class="language-([\w+-]+)">([\s\S]*?)<\/code><\/pre>/g,
		(_, preAttrs, lang, code) => {
			const decoded = decodeHtmlEntities(code);
			try {
				const safeLang = normalizeLanguage(lang);
				if (safeLang !== 'plaintext' && !hl.getLoadedLanguages().includes(safeLang)) {
					return `<pre${preAttrs}><code class="language-${lang}">${escapeHtml(decoded)}</code></pre>`;
				}
				const highlighted = hl.codeToHtml(decoded, {
					lang: safeLang,
					themes: { light: 'github-light', dark: 'github-dark' }
				});
				return highlighted.replace(/^<pre\b/, `<pre${preAttrs}`);
			} catch {
				return `<pre${preAttrs}><code class="language-${lang}">${escapeHtml(decoded)}</code></pre>`;
			}
		}
	);
}

export async function renderMarkdownDocument(source: string): Promise<RenderedMarkdownDocument> {
	const toc: TocItem[] = [];
	const renderSource = stripYamlFrontMatterForRender(source);

	// Math support is loaded lazily — documents without $…$ pay nothing.
	const hasMath = /\$[^\s$]/.test(renderSource);
	let remarkMath: any = null;
	let rehypeKatex: any = null;
	if (hasMath) {
		try {
			[remarkMath, rehypeKatex] = await Promise.all([
				import('remark-math').then((m) => m.default),
				import('rehype-katex').then((m) => m.default)
			]);
		} catch {
			remarkMath = null;
			rehypeKatex = null;
		}
	}

	let pipeline: any = unified().use(remarkParse).use(remarkGfm);
	if (remarkMath) pipeline = pipeline.use(remarkMath);
	pipeline = pipeline
		.use(remarkRehype, { allowDangerousHtml: true })
		.use(rehypeRaw)
		.use(rehypeSanitize, sanitizeSchema);
	if (rehypeKatex) pipeline = pipeline.use(rehypeKatex, { throwOnError: false });
	pipeline = pipeline.use(rehypeDocumentMetadata, toc).use(rehypeStringify);

	const result = await pipeline.process(renderSource);

	return {
		html: await highlightCodeBlocks(String(result)),
		toc,
		frontMatter: extractYamlFrontMatterEntries(source)
	};
}

export async function renderMarkdown(source: string): Promise<string> {
	return (await renderMarkdownDocument(source)).html;
}

export function extractToc(html: string): TocItem[] {
	const items: TocItem[] = [];
	const seenIds = new Map<string, number>();
	const regex = /<h([1-6])[^>]*id="([^"]*)"[^>]*>([\s\S]*?)<\/h[1-6]>/g;
	let match;
	while ((match = regex.exec(html)) !== null) {
		const text = stripHeadingHtml(match[3]);
		items.push({
			level: parseInt(match[1]),
			text,
			id: match[2]
		});
		seenIds.set(match[2], (seenIds.get(match[2]) || 0) + 1);
	}

	// If no IDs in headings, generate them
	if (items.length === 0) {
		const regex2 = /<h([1-6])[^>]*>([\s\S]*?)<\/h[1-6]>/g;
		while ((match = regex2.exec(html)) !== null) {
			const text = stripHeadingHtml(match[2]);
			const id = generateHeadingId(text, seenIds);
			items.push({ level: parseInt(match[1]), text, id });
		}
	}

	return items;
}

export function addHeadingIds(html: string): string {
	const seenIds = new Map<string, number>();
	return html.replace(/<h([1-6])([^>]*)>([\s\S]*?)<\/h[1-6]>/g, (full, level, attrs, content) => {
		// Require whitespace or start-of-tag before id=, and id= must be followed by a quote
		if (/(^|\s)id\s*=\s*["']/i.test(attrs)) return full;
		const text = stripHeadingHtml(content);
		const id = generateHeadingId(text, seenIds);
		return `<h${level}${attrs} id="${id}">${content}</h${level}>`;
	});
}
