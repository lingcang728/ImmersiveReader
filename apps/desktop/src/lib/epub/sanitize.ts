// HTML sanitization for the EPUB surface. The backend already sanitizes
// chapter XHTML (E5); this is the belt-and-suspenders client pass before the
// content reaches {@html}. Snippet/markdown helpers stay pure for tests.

import type { Config } from "dompurify";

type Purifier = { sanitize(source: string, config?: Config): string };

let purifierPromise: Promise<Purifier | null> | null = null;

async function getPurifier(): Promise<Purifier | null> {
	if (typeof window === "undefined" || typeof document === "undefined") return null;
	purifierPromise ??= import("dompurify")
		.then((mod) => {
			const instance = mod.default as unknown as Purifier;
			return typeof instance?.sanitize === "function" ? instance : null;
		})
		.catch((error) => {
			console.warn("DOMPurify unavailable:", error);
			return null;
		});
	return purifierPromise;
}

// Spec §4: forbid interactive/embedded payloads but keep <style> (chapter
// styles get scoped to .epub-content during post-processing). `on*` event
// attributes are not in DOMPurify's attribute allowlist at all, so they are
// always stripped; FORBID_ATTR additionally hard-blocks the script-capable
// ones that are. (DOMPurify resolves FORBID_ATTR as a name map — regexes are
// not supported there.)
// data-ir-remote-src must survive — it marks remote media the backend
// neutralized and the DOM pass swaps for a placeholder.
const EPUB_PURIFY_CONFIG: Config = {
	FORBID_TAGS: [
		"script",
		"iframe",
		"object",
		"embed",
		"form",
		"input",
		"button",
		"textarea",
		"select",
		"audio",
		"video",
	],
	FORBID_ATTR: ["srcdoc", "formaction"],
};

/**
 * Client-side sanitize of backend-provided chapter markup. Falls back to a
 * conservative tag strip when DOMPurify cannot run (e.g. tests/SSR) — in
 * that case block-level text still renders, only markup is degraded.
 */
export async function sanitizeChapterHtml(html: string): Promise<string> {
	if (!html) return "";
	const purifier = await getPurifier();
	if (purifier) {
		return String(purifier.sanitize(html, EPUB_PURIFY_CONFIG));
	}
	return stripDangerousMarkup(html);
}

/**
 * Last-resort scrub used only when DOMPurify is unavailable: removes the
 * most dangerous constructs with regexes. Never ship this as the primary
 * sanitizer — real DOM parsing is strictly stronger.
 */
function stripDangerousMarkup(html: string): string {
	return html
		.replace(
			/<\/?(script|iframe|object|embed|form|input|button|textarea|select|audio|video)\b[^>]*>/gi,
			"",
		)
		.replace(/\son\w+\s*=\s*("[^"]*"|'[^']*'|[^\s>]+)/gi, "")
		.replace(/(href|src|xlink:href)\s*=\s*(["'])\s*javascript:[^"']*\2/gi, '$1="#"');
}

const ESCAPE_MAP: Record<string, string> = {
	"&": "&amp;",
	"<": "&lt;",
	">": "&gt;",
	'"': "&quot;",
	"'": "&#39;",
};

export function escapeHtml(text: string): string {
	return text.replace(/[&<>"']/g, (ch) => ESCAPE_MAP[ch] ?? ch);
}

/**
 * `search_book` snippets use `<b>` for query highlights. Escape everything,
 * then re-allow only literal `<b>`/`</b>` so nothing else can inject markup.
 * Rendering still goes through `{@html}` — this whitelist keeps it safe.
 */
export function sanitizeSearchSnippet(snippet: string): string {
	return escapeHtml(snippet)
		.replace(/&lt;b&gt;/g, "<b>")
		.replace(/&lt;\/b&gt;/g, "</b>");
}

/**
 * Minimal Markdown fallback for `format === "markdown"` chapters (legacy
 * books served through the same command): escape, then split blank-line
 * paragraphs. Not a full renderer — just keeps text readable.
 */
export function markdownFallbackHtml(markdown: string): string {
	return markdown
		.split(/\r?\n\s*\r?\n/)
		.map((block) => block.trim())
		.filter(Boolean)
		.map((block) => `<p>${escapeHtml(block).replace(/\r?\n/g, "<br>")}</p>`)
		.join("\n");
}

/** Collapse whitespace and cut a display excerpt — used for bookmark labels. */
export function textExcerpt(text: string, max = 40): string {
	const compact = text.replace(/\s+/g, " ").trim();
	if (compact.length <= max) return compact;
	return `${compact.slice(0, max)}…`;
}

const UNESCAPE_MAP: Record<string, string> = {
	"&amp;": "&",
	"&lt;": "<",
	"&gt;": ">",
	"&quot;": '"',
	"&#39;": "'",
	"&apos;": "'",
	"&nbsp;": " ",
};

export function unescapeHtml(text: string): string {
	return text.replace(
		/&(?:amp|lt|gt|quot|apos|nbsp|#39);/g,
		(entity) => UNESCAPE_MAP[entity] ?? entity,
	);
}

/**
 * Recover the query a `search_book` snippet highlights — the text inside its
 * `<b>` marks. Used to locate a hit in the DOM when the backend returned no
 * locator. Entities are unescaped so the result matches document text.
 */
export function snippetQueryText(snippet: string): string {
	const parts: string[] = [];
	for (const match of snippet.matchAll(/<b>([\s\S]*?)<\/b>/gi)) {
		parts.push(match[1]);
	}
	return unescapeHtml(parts.join("")).trim();
}
