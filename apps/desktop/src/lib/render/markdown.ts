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
		],
		blockquote: [
			...((defaultSchema.attributes?.blockquote as any[]) ?? []),
			['className', 'podcast-original'],
			['lang', 'en'],
			['dataBilingualId', /^[\w-]+$/],
			['tabIndex'],
			// 双语原文块是可聚焦的展开/收起控件（见 markPodcastOriginal）。
			['role', 'button'],
			['ariaExpanded', 'true', 'false'],
			['ariaLabel']
		],
		p: [
			...((defaultSchema.attributes?.p as any[]) ?? []),
			['className', 'podcast-translation'],
			['dataBilingualId', /^[\w-]+$/]
		],
		// remark-rehype 给脚注区 <h2 id="footnote-label"> 加 sr-only；
		// 白名单放行，否则中文界面会冒出一个英文大标题。
		h2: [
			...((defaultSchema.attributes?.h2 as any[]) ?? []),
			['className', 'sr-only']
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

interface FrontMatterBlock {
	lines: string[];
	closingLineIndex: number;
}

// P3-7: the render path used to split the whole document twice (once per
// front-matter helper). This is the single shared split, and the first-line
// gate early-exits without a full-document match() when there is no `---`.
function splitFrontMatter(source: string): FrontMatterBlock | null {
	const firstBreak = source.search(/\r\n|\n|\r/);
	const firstLine = (firstBreak === -1 ? source : source.slice(0, firstBreak)).replace(/^\ufeff/, '');
	if (!/^---[ \t]*$/.test(firstLine)) return null;

	const lines = source.match(/.*(?:\r\n|\n|\r|$)/g) ?? [];
	if (lines.length > 0 && lines[lines.length - 1] === '') lines.pop();
	if (lines.length < 2) return null;

	let closingLineIndex = -1;
	for (let i = 1; i < lines.length; i += 1) {
		const line = lines[i];
		if (line !== undefined && /^(---|\.\.\.)[ \t]*$/.test(lineContent(line))) {
			closingLineIndex = i;
			break;
		}
	}
	if (closingLineIndex === -1) return null;

	return { lines, closingLineIndex };
}

function blankFrontMatterBlock(frontMatter: FrontMatterBlock): string {
	return frontMatter.lines
		.map((line, index) => (index <= frontMatter.closingLineIndex ? blankLineContent(line) : line))
		.join('');
}

export function stripYamlFrontMatterForRender(source: string): string {
	const frontMatter = splitFrontMatter(source);
	return frontMatter ? blankFrontMatterBlock(frontMatter) : source;
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
function parseFrontMatterEntries(frontMatter: FrontMatterBlock): FrontMatterEntry[] {
	const { lines, closingLineIndex } = frontMatter;

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

export function extractYamlFrontMatterEntries(source: string): FrontMatterEntry[] {
	const frontMatter = splitFrontMatter(source);
	return frontMatter ? parseFrontMatterEntries(frontMatter) : [];
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

function cjkRatio(text: string): number {
	const cleaned = text.replace(/\s+/g, '');
	if (!cleaned) return 0;
	let cjk = 0;
	let latin = 0;
	for (const ch of cleaned) {
		if (/[\u4e00-\u9fff]/.test(ch)) cjk += 1;
		else if (/[A-Za-z]/.test(ch)) latin += 1;
	}
	const total = cjk + latin;
	return total === 0 ? 0 : cjk / total;
}

function isMostlyLatin(text: string): boolean {
	const cleaned = text.replace(/\s+/g, '');
	if (cleaned.length < 12) return false;
	return cjkRatio(text) < 0.2 && /[A-Za-z]{3,}/.test(cleaned);
}

function isMostlyChinese(text: string): boolean {
	const cleaned = text.replace(/\s+/g, '');
	if (cleaned.length < 4) return false;
	// Allow mixed titles (e.g. “欢迎收听 Huberman Lab”) while still requiring real CJK body.
	const cjk = (cleaned.match(/[\u4e00-\u9fff]/g) ?? []).length;
	return cjk >= 4 && cjkRatio(text) >= 0.28;
}

function classListOf(node: any): string[] {
	const value = node?.properties?.className;
	if (Array.isArray(value)) return value.map(String);
	if (typeof value === 'string') return value.split(/\s+/).filter(Boolean);
	return [];
}

function markPodcastTranslation(node: any, bilingualId?: string) {
	if (!node.properties) node.properties = {};
	const classes = new Set(classListOf(node));
	classes.add('podcast-translation');
	node.properties.className = [...classes];
	if (bilingualId) node.properties.dataBilingualId = bilingualId;
}

function markPodcastOriginal(node: any, bilingualId?: string) {
	if (!node.properties) node.properties = {};
	const classes = new Set(classListOf(node));
	classes.add('podcast-original');
	node.properties.className = [...classes];
	node.properties.lang = 'en';
	// 可聚焦的展开/收起控件：点击、Enter、Space 都会切 is-revealed
	//（+page.svelte 的 togglePodcastOriginal 同步 aria-expanded）。
	node.properties.tabIndex = 0;
	node.properties.role = 'button';
	node.properties.ariaExpanded = 'false';
	node.properties.ariaLabel = '显示英文原文';
	if (bilingualId) node.properties.dataBilingualId = bilingualId;
}

/**
 * Normalize podcast bilingual blocks for both new and legacy Markdown:
 * - plain EN paragraph + ZH paragraph → ZH then blockquote.podcast-original
 * - ZH paragraph + untagged blockquote EN → tag the blockquote
 * - all English originals → a single English section at the end
 *
 * Does not rewrite Library files. The generated `data-bilingual-id` lets the
 * reader reveal a matching original without relying on DOM adjacency.
 */
function rehypeNormalizePodcastBilingual() {
	return (tree: any) => {
		const children: any[] = tree.children ?? [];
		const normalized: any[] = [];
		const usedIds = new Set<string>();
		let nextId = 0;
		const isWhitespace = (node: any) =>
			node?.type === 'text' && !String(node.value ?? '').trim();
		const isElement = (node: any) => node?.type === 'element';
		const hasClass = (node: any, className: string) => classListOf(node).includes(className);
		const existingId = (node: any) => {
			const value = node?.properties?.dataBilingualId;
			return value === undefined || value === null ? '' : String(value);
		};
		const createId = () => {
			let candidate = `podcast-${nextId}`;
			while (usedIds.has(candidate)) {
				nextId += 1;
				candidate = `podcast-${nextId}`;
			}
			usedIds.add(candidate);
			nextId += 1;
			return candidate;
		};
		const pairId = (translation: any, original: any) => {
			const current = existingId(translation) || existingId(original);
			const id = current || createId();
			usedIds.add(id);
			markPodcastTranslation(translation, id);
			markPodcastOriginal(original, id);
			return id;
		};

		const nextElementIndex = (from: number) => {
			for (let j = from; j < children.length; j += 1) {
				if (isElement(children[j])) return j;
				if (!isWhitespace(children[j])) return -1;
			}
			return -1;
		};

		// Track the last element pushed instead of rescanning `normalized` for
		// every Latin blockquote — the scan made each lookup O(n) → O(n²) total.
		let lastNormalizedElement: any = null;
		const pushNormalized = (...nodes: any[]) => {
			for (const pushed of nodes) {
				normalized.push(pushed);
				if (isElement(pushed)) lastNormalizedElement = pushed;
			}
		};

		for (let i = 0; i < children.length; i += 1) {
			const node = children[i];
			if (isWhitespace(node)) {
				pushNormalized(node);
				continue;
			}
			if (isElement(node) && node.tagName === 'p') {
				const followIdx = nextElementIndex(i + 1);
				const following = followIdx >= 0 ? children[followIdx] : null;
				if (isElement(following)) {
					const left = textFromHast(node).trim();
					const right = textFromHast(following).trim();
					if (following.tagName === 'p' && isMostlyLatin(left) && isMostlyChinese(right)) {
						const original = {
							type: 'element',
							tagName: 'blockquote',
							properties: {},
							children: [{ type: 'text', value: left }]
						};
						pairId(following, original);
						// Preserve interstitial whitespace between the pair.
						for (let w = i + 1; w < followIdx; w += 1) pushNormalized(children[w]);
						pushNormalized(following, original);
						i = followIdx;
						continue;
					}
					if (
						following.tagName === 'blockquote' &&
						isMostlyChinese(left) &&
						isMostlyLatin(right)
					) {
						pairId(node, following);
						for (let w = i + 1; w < followIdx; w += 1) pushNormalized(children[w]);
						pushNormalized(node, following);
						i = followIdx;
						continue;
					}
				}
			}
			if (
			isElement(node) &&
				node.tagName === 'blockquote' &&
				isMostlyLatin(textFromHast(node)) &&
				!hasClass(node, 'podcast-original')
			) {
				const prev = lastNormalizedElement;
				if (prev?.tagName === 'p' && isMostlyChinese(textFromHast(prev))) {
					pairId(prev, node);
				}
			}
			if (isElement(node) && node.tagName === 'p' && hasClass(node, 'podcast-translation')) {
				markPodcastTranslation(node, existingId(node) || createId());
			}
			if (isElement(node) && node.tagName === 'blockquote' && hasClass(node, 'podcast-original')) {
				markPodcastOriginal(node, existingId(node) || createId());
			}
			pushNormalized(node);
		}

		const originals = normalized.filter(
			(node) => isElement(node) && node.tagName === 'blockquote' && hasClass(node, 'podcast-original'),
		);
		if (originals.length === 0) {
			tree.children = normalized;
			return;
		}

		// `includes` made this filter O(n·originals); a Set keeps it linear.
		const originalSet = new Set(originals);
		const content = normalized.filter((node) => !originalSet.has(node));
		const heading = content.find(
			(node) => isElement(node) && node.tagName === 'h2' && textFromHast(node).trim() === '英文原文',
		) as any;
		if (heading) {
			const classes = new Set(classListOf(heading));
			classes.add('podcast-originals-heading');
			heading.properties = heading.properties ?? {};
			heading.properties.className = [...classes];
		} else {
			content.push({ type: 'text', value: '\n' });
			content.push({
				type: 'element',
				tagName: 'h2',
				properties: { className: ['podcast-originals-heading'] },
				children: [{ type: 'text', value: '英文原文' }]
			});
		}
		content.push({ type: 'text', value: '\n' });
		content.push(...originals);
		tree.children = content;
	};
}

// P3-7: the processor is composed once and shared, so per-document state —
// the toc sink and the heading-id counter — rides on the VFile (`process`
// receives `{ value, data: { toc } }`) instead of `.use()` options.
function rehypeDocumentMetadata() {
	const blockTags = new Set(['p', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'blockquote', 'pre', 'table', 'ul', 'ol', 'li', 'hr']);

	return (tree: any, file: any) => {
		const toc: TocItem[] = file?.data?.toc ?? [];
		const seenIds = new Map<string, number>();

		// Sanitize runs with clobberPrefix '' (it can't rewrite hrefs, so a
		// prefix there would break footnote links) — meaning document-supplied
		// ids/names pass through verbatim. A crafted `id="user-content-fn-1"`
		// could then hijack footnote jumps or collide with generated heading
		// ids. Rename non-footnote document ids here instead and fix up the
		// matching `#fragment` references ourselves. `user-content-*` is the
		// remark-rehype footnote prefix: those id↔href pairs are already
		// consistent and must stay untouched.
		// `user-content-*` is remark-rehype's footnote id prefix; GFM only ever
		// puts those ids on footnote refs/backs (<sup>/<a>), footnote items
		// (<li>) inside the footnotes <section>, and the section itself. The
		// same id on anything else is a document-supplied duplicate
		// (footnote-jump hijack) and gets renamed like any other document id.
		const footnoteIdTags = new Set(['a', 'sup', 'li', 'section']);
		const isFootnotesSection = (node: any) =>
			node.tagName === 'section' &&
			(node.properties?.dataFootnotes !== undefined ||
				(Array.isArray(node.properties?.className)
					? node.properties.className
					: String(node.properties?.className ?? '').split(' ')
				).includes('footnotes'));
		const isFootnoteMachinery = (node: any, inside: boolean) =>
			node.tagName === 'sup' || (inside && footnoteIdTags.has(node.tagName));

		// Pass 1: which `user-content-*` ids genuinely belong to footnote
		// machinery — they keep their names so the ref↔target pair survives —
		// and every id/name the document declares (an href can point forward).
		const keptFootnoteIds = new Set<string>();
		const declaredIds = new Set<string>();
		(function scan(node: any, inFootnotes: boolean) {
			if (node.type === 'element') {
				const inside = inFootnotes || isFootnotesSection(node);
				for (const key of ['id', 'name'] as const) {
					const value = node.properties?.[key];
					if (typeof value !== 'string' || value === '') continue;
					declaredIds.add(value);
					if (
						key === 'id' &&
						value.startsWith('user-content-') &&
						isFootnoteMachinery(node, inside)
					) {
						keptFootnoteIds.add(value);
					}
				}
				for (const child of node.children ?? []) scan(child, inside);
				return;
			}
			for (const child of node.children ?? []) scan(child, inFootnotes);
		})(tree, false);

		// Every declared id/name that no footnote element keeps is renamed to a
		// `md-` name; `#fragment` references are rewritten through this map so
		// in-document anchors keep working while footnote hrefs stay pointed at
		// the kept machinery ids.
		const renamedIds = new Map<string, string>();
		for (const id of declaredIds) {
			if (!keptFootnoteIds.has(id)) renamedIds.set(id, `md-${id}`);
		}

		function walk(node: any, inFootnotes: boolean) {
			if (node.type === 'element') {
				if (!node.properties) node.properties = {};
				const inside = inFootnotes || isFootnotesSection(node);

				for (const key of ['id', 'name'] as const) {
					const value = node.properties[key];
					if (typeof value !== 'string' || value === '') continue;
					const keep =
						key === 'id' &&
						keptFootnoteIds.has(value) &&
						isFootnoteMachinery(node, inside);
					if (keep) continue;
					// A `user-content-*` id that machinery keeps elsewhere isn't in
					// renamedIds (its hrefs must stay) — rename this duplicate
					// anyway so it stops competing for the fragment.
					node.properties[key] = renamedIds.get(value) ?? `md-${value}`;
				}
				const href = node.properties.href;
				if (typeof href === 'string' && href.startsWith('#')) {
					const target = href.slice(1);
					const renamed =
						renamedIds.get(target) ??
						(() => {
							try {
								return renamedIds.get(decodeURIComponent(target));
							} catch {
								return undefined;
							}
						})();
					if (renamed) node.properties.href = `#${renamed}`;
				}
				// idref attributes (footnote aria-describedby → footnote-label)
				// must move with a renamed target or the a11y link dangles.
				for (const key of ['ariaDescribedBy', 'ariaLabelledBy'] as const) {
					const value = node.properties[key];
					if (Array.isArray(value)) {
						node.properties[key] = value.map(
							(id: any) => renamedIds.get(id) ?? id
						);
					}
				}

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
				const inside =
					node.type === 'element'
						? inFootnotes || isFootnotesSection(node)
						: inFootnotes;
				for (const child of node.children) walk(child, inside);
			}
		}
		walk(tree, false);
	};
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

// P3-7: the plugin list is static, so each processor is composed once and
// reused — unified freezes it on the first `process()` call. Per-document
// state (toc sink, heading-id counters) travels on the VFile via
// `process({ value, data })`, keeping the shared processors stateless and
// safe for concurrent renders.
let baseProcessor: any = null;
let mathProcessorPromise: Promise<any> | null = null;

function baseMarkdownProcessor() {
	if (!baseProcessor) {
		baseProcessor = unified()
			.use(remarkParse)
			.use(remarkGfm)
			.use(remarkRehype, { allowDangerousHtml: true, footnoteLabel: '脚注' })
			.use(rehypeRaw)
			.use(rehypeSanitize, sanitizeSchema)
			.use(rehypeNormalizePodcastBilingual)
			.use(rehypeDocumentMetadata)
			.use(rehypeStringify);
	}
	return baseProcessor;
}

// Math support stays lazy behind a second processor: documents without $…$
// never pay the remark-math/rehype-katex import cost.
function mathMarkdownProcessor(): Promise<any> {
	if (!mathProcessorPromise) {
		mathProcessorPromise = Promise.all([
			import('remark-math').then((m) => m.default),
			import('rehype-katex').then((m) => m.default)
		])
			.then(([remarkMath, rehypeKatex]: [any, any]) =>
				unified()
					.use(remarkParse)
					.use(remarkGfm)
					.use(remarkMath)
					.use(remarkRehype, { allowDangerousHtml: true, footnoteLabel: '脚注' })
					.use(rehypeRaw)
					.use(rehypeSanitize, sanitizeSchema)
					.use(rehypeNormalizePodcastBilingual)
					.use(rehypeKatex, { throwOnError: false })
					.use(rehypeDocumentMetadata)
					.use(rehypeStringify)
			)
			.catch(() => {
				// A failed lazy import must not poison the cache — retry next render.
				mathProcessorPromise = null;
				return null;
			});
	}
	return mathProcessorPromise;
}

// Gate the lazy math pipeline on the same shape remark-math requires: an
// opening `$` not followed by whitespace and a closing `$` not preceded by
// whitespace and not followed by a digit. "$5 和 $10" is currency, not
// math — it must not even pay the remark-math/KaTeX import cost.
export function mayContainMath(source: string): boolean {
	return /\$(?!\s)[^\n$]*[^\s$]\$(?!\d)|\$\$/.test(source);
}

export async function renderMarkdownDocument(source: string): Promise<RenderedMarkdownDocument> {
	const toc: TocItem[] = [];
	const frontMatterBlock = splitFrontMatter(source);
	const renderSource = frontMatterBlock ? blankFrontMatterBlock(frontMatterBlock) : source;

	const pipeline = mayContainMath(renderSource)
		? ((await mathMarkdownProcessor()) ?? baseMarkdownProcessor())
		: baseMarkdownProcessor();

	const result = await pipeline.process({ value: renderSource, data: { toc } });

	return {
		html: await highlightCodeBlocks(String(result)),
		toc,
		frontMatter: frontMatterBlock ? parseFrontMatterEntries(frontMatterBlock) : []
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
