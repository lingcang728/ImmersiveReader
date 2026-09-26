// DOM-dependent helpers for the EPUB surface: resource resolution, anchor
// capture/restore, link interception, swipe arbitration, focus-lite.
// Every export is safe to *import* in a DOM-less environment (vitest node);
// functions that need a document early-return instead of throwing.

import type { ReaderLocator, ReaderAnchor } from "./types";
import {
	isExternalHttpUrl,
	isInlineOrAssetUrl,
	isNativeAbsolutePath,
	isProtocolRelative,
	hasUriScheme,
	joinNativePath,
	normalizeBookPath,
	safeDecodeUri,
	splitHref,
} from "./paths";
import { textExcerpt } from "./sanitize";

export type FileSrcConverter = (path: string) => string;

/** Transparent 1px stand-in for remote media that must render inline-safe. */
const BLOCKED_REMOTE_PIXEL =
	"data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";

/** Block-level elements that can carry a scroll anchor. */
export const ANCHOR_BLOCK_SELECTOR =
	"p, h1, h2, h3, h4, h5, h6, li, blockquote, pre, figure, figcaption, section, aside, table, div:not(.remote-blocked)";

function escapeIdent(ident: string): string {
	if (typeof CSS !== "undefined" && typeof CSS.escape === "function") {
		return CSS.escape(ident);
	}
	return ident.replace(/[^\w-]/g, (ch) => `\\${ch}`);
}

function isRemoteUrl(value: string): boolean {
	return isExternalHttpUrl(value) || isProtocolRelative(value);
}

function isResolvableRelative(value: string): boolean {
	if (!value || value.startsWith("#")) return false;
	if (isRemoteUrl(value) || isInlineOrAssetUrl(value)) return false;
	// file:/native-absolute/other schemes resolve outside the book — refuse.
	if (isNativeAbsolutePath(value) || hasUriScheme(value)) return false;
	return true;
}

function makeRemotePlaceholder(doc: Document): HTMLElement {
	const placeholder = doc.createElement("div");
	placeholder.className = "remote-blocked";
	placeholder.setAttribute("role", "note");
	placeholder.textContent = "远程图片已离线阻止";
	return placeholder;
}

/**
 * Replace one element marked `data-ir-remote-src` (or leaking an http(s)
 * reference) with a visible offline placeholder. Media-ish elements become
 * a labelled block; structural ones (`link`, `source`) are removed instead —
 * `picture` fallbacks keep working and no outbound request can fire.
 */
export function neutralizeRemoteElement(el: Element): void {
	const tag = el.tagName.toLowerCase();
	if (tag === "link" || tag === "source") {
		el.remove();
		return;
	}
	if (el.namespaceURI === "http://www.w3.org/2000/svg") {
		// Inside <svg> an HTML placeholder cannot render — swap the ref.
		el.setAttribute("href", BLOCKED_REMOTE_PIXEL);
		el.setAttribute("xlink:href", BLOCKED_REMOTE_PIXEL);
		el.removeAttribute("data-ir-remote-src");
		return;
	}
	el.replaceWith(makeRemotePlaceholder(el.ownerDocument));
}

function resolveResourceUrl(raw: string, baseDir: string, convert: FileSrcConverter): string {
	const decoded = safeDecodeUri(raw.trim());
	const { path, query } = splitHref(decoded);
	if (!isResolvableRelative(path)) return raw;
	const absolute = joinNativePath(baseDir, normalizeBookPath(path));
	return `${convert(absolute)}${query}`;
}

/**
 * Rewrite `srcset` candidates, resolving relative URLs against the chapter
 * directory. Remote candidates are dropped entirely — a `source` with only
 * remote candidates ends up with an empty (ignored) srcset.
 */
function resolveSrcset(raw: string, baseDir: string, convert: FileSrcConverter): string {
	const resolved = raw
		.split(",")
		.map((candidate) => candidate.trim())
		.filter(Boolean)
		.map((candidate) => {
			const space = candidate.search(/\s/);
			const url = space === -1 ? candidate : candidate.slice(0, space);
			const descriptor = space === -1 ? "" : candidate.slice(space);
			if (isRemoteUrl(url)) return "";
			return `${resolveResourceUrl(url, baseDir, convert)}${descriptor}`;
		})
		.filter(Boolean);
	return resolved.join(", ");
}

/** Rewrite `url(...)` references inside CSS text (scoped styles, attrs). */
export function rewriteCssUrls(
	cssText: string,
	baseDir: string,
	convert: FileSrcConverter,
): string {
	return cssText.replace(
		/url\(\s*(["']?)([^"')]+)\1\s*\)/gi,
		(full, _quote: string, url: string) => {
			const trimmed = url.trim();
			if (isRemoteUrl(trimmed)) return "url(\"data:,\")";
			if (!isResolvableRelative(trimmed)) return full;
			const absolute = joinNativePath(baseDir, normalizeBookPath(trimmed));
			return `url("${convert(absolute)}")`;
		},
	);
}

/**
 * Scope one `<style>` element's rules under `.epub-content` so book CSS
 * cannot restyle app chrome. Keeps @media/@supports/@font-face; drops
 * @import/@namespace (remote-capable). Returns null when the rules cannot
 * be parsed — caller should then remove the element rather than leak
 * unscoped CSS into the document.
 */
export function scopeCssText(
	cssText: string,
	scopeSelector: string,
	baseDir: string,
	convert: FileSrcConverter,
): string | null {
	if (typeof CSSStyleSheet === "undefined") return null;
	const sheet = new CSSStyleSheet();
	try {
		sheet.replaceSync(cssText);
	} catch {
		return null;
	}
	const scoped = scopeRuleList(sheet.cssRules, scopeSelector, baseDir, convert);
	return scoped ?? null;
}

function scopeSelectorText(selectorText: string, scopeSelector: string): string {
	return selectorText
		.split(",")
		.map((selector) => {
			let s = selector.trim();
			if (!s) return "";
			if (s === ":root" || s === "html" || s === "body") return scopeSelector;
			// `html body p` / `body p` → scope stays the containing element.
			s = s.replace(/^(?:\s*(?:html|body|:root)\s+)+/i, "");
			s = s.replace(/^(?:html|body|:root)(?=[.#:\[])/i, "");
			if (!s) return scopeSelector;
			if (s.startsWith(scopeSelector)) return s;
			return `${scopeSelector} ${s}`;
		})
		.filter(Boolean)
		.join(", ");
}

function scopeRuleList(
	rules: CSSRuleList,
	scopeSelector: string,
	baseDir: string,
	convert: FileSrcConverter,
): string | null {
	const out: string[] = [];
	for (const rule of Array.from(rules)) {
		if (rule instanceof CSSStyleRule) {
			const selectors = scopeSelectorText(rule.selectorText, scopeSelector);
			if (!selectors) continue;
			const body = rewriteCssUrls(rule.style.cssText, baseDir, convert);
			out.push(`${selectors} { ${body} }`);
		} else if (rule instanceof CSSMediaRule || rule instanceof CSSSupportsRule) {
			const inner = scopeRuleList(rule.cssRules, scopeSelector, baseDir, convert);
			if (inner) {
				const condition =
					rule instanceof CSSMediaRule ? rule.media.mediaText : rule.conditionText;
				out.push(
					`@${rule instanceof CSSMediaRule ? "media" : "supports"} ${condition} { ${inner} }`,
				);
			}
		} else if (rule instanceof CSSFontFaceRule) {
			out.push(`@font-face { ${rewriteCssUrls(rule.style.cssText, baseDir, convert)} }`);
		}
		// @import/@namespace/@keyframes/@page dropped: remote fetches and
		// document-global rules are not wanted inside the reader surface.
	}
	return out.length ? out.join("\n") : "";
}

/**
 * Post-process rendered chapter markup in place: scope <style> blocks,
 * resolve relative resource URLs to asset-protocol URLs, neutralize remote
 * media. `baseDir` is the chapter's absolute native directory
 * (`bookDir + sep + resourceDir`, already normalized by the caller).
 */
export function postProcessChapterDom(
	articleEl: HTMLElement,
	baseDir: string,
	convert: FileSrcConverter,
	scopeSelector = ".epub-content",
): void {
	if (typeof document === "undefined") return;

	for (const styleEl of Array.from(articleEl.querySelectorAll("style"))) {
		const scoped = scopeCssText(styleEl.textContent ?? "", scopeSelector, baseDir, convert);
		if (scoped === null) styleEl.remove();
		else styleEl.textContent = scoped;
	}

	const urlAttrTargets = articleEl.querySelectorAll(
		"img[src], img[srcset], source[src], source[srcset], image[href], image[xlink\\:href], link[href], [data-ir-remote-src]",
	);
	for (const el of Array.from(urlAttrTargets)) {
		if (el.hasAttribute("data-ir-remote-src")) {
			neutralizeRemoteElement(el);
			continue;
		}
		for (const attr of ["src", "href", "xlink:href"]) {
			const value = el.getAttribute(attr);
			if (!value) continue;
			const decoded = safeDecodeUri(value.trim());
			if (isRemoteUrl(decoded)) {
				neutralizeRemoteElement(el);
				break;
			}
			if (isResolvableRelative(decoded)) {
				el.setAttribute(attr, resolveResourceUrl(decoded, baseDir, convert));
			}
		}
		if (!el.isConnected) continue;
		const srcset = el.getAttribute("srcset");
		if (srcset) {
			const rewritten = resolveSrcset(srcset, baseDir, convert);
			if (rewritten) el.setAttribute("srcset", rewritten);
			else el.removeAttribute("srcset");
		}
	}

	// Inline style attributes can carry url() — rewrite relative refs,
	// blank remote ones.
	for (const el of Array.from(articleEl.querySelectorAll<HTMLElement>("[style]"))) {
		const style = el.getAttribute("style");
		if (!style || !/url\(/i.test(style)) continue;
		el.setAttribute("style", rewriteCssUrls(style, baseDir, convert));
	}
}

// ---------------------------------------------------------------------------
// Anchors

/**
 * A querySelector-compatible path for `el` relative to `root`: `#id` when an
 * id exists, else a `tag:nth-of-type(n)` chain up to (and including) the
 * nearest id'd ancestor or `root` itself.
 */
export function elementCssPath(el: Element, root: Element): string {
	const segments: string[] = [];
	let current: Element | null = el;
	while (current && current !== root) {
		const id = current.getAttribute("id");
		if (id) {
			segments.unshift(`#${escapeIdent(id)}`);
			return segments.join(" > ");
		}
		const tag = current.tagName.toLowerCase();
		let index = 1;
		for (let sib = current.previousElementSibling; sib; sib = sib.previousElementSibling) {
			if (sib.tagName === current.tagName) index += 1;
		}
		segments.unshift(`${tag}:nth-of-type(${index})`);
		current = current.parentElement;
	}
	if (!segments.length) return "";
	return `${root.tagName.toLowerCase()} > ${segments.join(" > ")}`;
}

/** First block-level element intersecting the scroller's top edge. */
export function firstVisibleBlockAnchor(
	articleEl: HTMLElement,
	scrollerEl: HTMLElement,
): ReaderAnchor {
	if (typeof document === "undefined") return { kind: "ratio" };
	const viewTop = scrollerEl.getBoundingClientRect().top;
	const viewBottom = viewTop + scrollerEl.clientHeight;
	const blocks = articleEl.querySelectorAll(ANCHOR_BLOCK_SELECTOR);
	for (const block of Array.from(blocks)) {
		const rect = block.getBoundingClientRect();
		if (rect.bottom > viewTop + 8 && rect.top < viewBottom) {
			return { kind: "element", path: elementCssPath(block, articleEl) };
		}
		if (rect.top >= viewBottom) break;
	}
	return { kind: "ratio" };
}

function queryWithin(articleEl: HTMLElement, path: string): Element | null {
	try {
		const found = articleEl.querySelector(path);
		if (found) return found;
	} catch {
		// fall through to id lookup
	}
	if (path.startsWith("#")) {
		const byId = articleEl.ownerDocument.getElementById(path.slice(1));
		if (byId && articleEl.contains(byId)) return byId;
	}
	return null;
}

/** Text-node walk to find an element whose text contains `quote`. */
export function findTextBlock(articleEl: HTMLElement, quote: string): Element | null {
	if (typeof document === "undefined" || !quote) return null;
	const walker = articleEl.ownerDocument.createTreeWalker(articleEl, NodeFilter.SHOW_TEXT);
	let node = walker.nextNode();
	while (node) {
		if (node.nodeValue?.includes(quote)) {
			const parent = node.parentElement;
			return parent?.closest(ANCHOR_BLOCK_SELECTOR) ?? parent;
		}
		node = walker.nextNode();
	}
	// Quote may span element boundaries — check per-block textContent.
	for (const block of Array.from(articleEl.querySelectorAll(ANCHOR_BLOCK_SELECTOR))) {
		if (block.textContent?.includes(quote)) return block;
	}
	return null;
}

/** Scroll `#fragment` (or `<a name>`) into view inside the scroller. */
export function scrollToFragment(articleEl: HTMLElement, fragment: string): boolean {
	if (typeof document === "undefined" || !fragment) return false;
	const id = safeDecodeUri(fragment);
	const escaped = escapeIdent(id);
	let target = queryWithin(articleEl, `#${escaped}`);
	if (!target) {
		try {
			target = articleEl.querySelector(`[name="${id.replace(/"/g, '\\"')}"]`);
		} catch {
			target = null;
		}
	}
	if (!target) return false;
	target.scrollIntoView({ block: "start" });
	return true;
}

/**
 * Best-effort locator restore: element anchor → querySelector/scrollIntoView;
 * text anchor → TreeWalker quote match; anything else → scroll ratio.
 * Falls back down the chain when the preferred anchor misses.
 */
export function restoreLocatorScroll(
	articleEl: HTMLElement,
	scrollerEl: HTMLElement,
	locator: Pick<ReaderLocator, "anchor" | "ratio">,
): void {
	if (typeof document === "undefined") return;
	const { anchor, ratio } = locator;

	if (anchor.kind === "element") {
		const el = queryWithin(articleEl, anchor.path);
		if (el) {
			el.scrollIntoView({ block: "start" });
			return;
		}
	} else if (anchor.kind === "text") {
		const el = findTextBlock(articleEl, anchor.quote);
		if (el) {
			el.scrollIntoView({ block: "start" });
			return;
		}
	}

	const span = scrollerEl.scrollHeight - scrollerEl.clientHeight;
	scrollerEl.scrollTop = Math.max(0, Math.round(ratio * span));
}

/** Excerpt of the block an anchor points at — bookmark label material. */
export function excerptAtAnchor(articleEl: HTMLElement, anchor: ReaderAnchor): string {
	if (typeof document === "undefined") return "";
	if (anchor.kind === "text") return textExcerpt(anchor.quote);
	if (anchor.kind === "element") {
		const el = queryWithin(articleEl, anchor.path);
		if (el?.textContent) return textExcerpt(el.textContent);
	}
	return "";
}

// ---------------------------------------------------------------------------
// Swipe arbitration — same exclusions as the main reader (B2): a horizontal
// swipe that starts inside a table / pre / image / horizontally-scrollable
// container is content interaction, not chapter navigation.

const SWIPE_EXCLUDE_SELECTOR =
	"table, pre, img, figure, svg, video, audio, iframe, [data-no-swipe], [contenteditable='true']";

export function swipeExcluded(target: EventTarget | null, boundary: HTMLElement): boolean {
	if (!(target instanceof Element)) return false;
	if (target.closest(SWIPE_EXCLUDE_SELECTOR)) return true;
	let current: Element | null = target;
	let hops = 0;
	while (current && current !== boundary && hops < 8) {
		if (current instanceof HTMLElement) {
			const overflowX = getComputedStyle(current).overflowX;
			if (
				(overflowX === "auto" || overflowX === "scroll") &&
				current.scrollWidth > current.clientWidth + 8
			) {
				return true;
			}
		}
		current = current.parentElement;
		hops += 1;
	}
	return false;
}

// ---------------------------------------------------------------------------
// Focus-lite: dim every direct child of the article except the block under
// the viewport's reading line. Deliberately independent of the main app's
// focus machinery (locked behavior — do not reuse).

const FOCUS_LINE_RATIO = 0.42;

export function updateFocusLite(articleEl: HTMLElement, scrollerEl: HTMLElement): void {
	if (typeof document === "undefined") return;
	const scrollerRect = scrollerEl.getBoundingClientRect();
	const focusLine = scrollerRect.top + scrollerRect.height * FOCUS_LINE_RATIO;
	let active: Element | null = null;
	for (const child of Array.from(articleEl.children)) {
		const rect = child.getBoundingClientRect();
		if (rect.bottom >= focusLine) {
			active = child;
			break;
		}
	}
	if (!active && articleEl.children.length > 0) {
		active = articleEl.children[articleEl.children.length - 1];
	}
	for (const child of Array.from(articleEl.children)) {
		child.classList.toggle("epub-focus-current", child === active);
	}
}

export function clearFocusLite(articleEl: HTMLElement): void {
	for (const child of Array.from(articleEl.children)) {
		child.classList.remove("epub-focus-current");
	}
}
