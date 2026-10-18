// In-chapter Markdown search for the mobile reader.
//
// The desktop surface has a much larger search machinery (focus integration,
// right-edge tick marks, search-in-focus). The phone only needs: find matches
// in the currently mounted chapter, highlight them, jump previous/next.
// Pure matching lives in `findOccurrences` so it is unit-testable; the DOM
// wrapper only splits text nodes and wraps matches in <mark>.

const SEARCH_MARK_CLASS = 'reader-search-mark';
const SEARCH_CURRENT_CLASS = 'reader-search-current';

/** Maximum matches highlighted in one chapter (keeps long chapters snappy). */
export const MAX_CHAPTER_MATCHES = 300;

/** Case-insensitive occurrence offsets of `query` in `text`. Pure. */
export function findOccurrences(text: string, query: string): number[] {
	const needle = query.trim().toLowerCase();
	if (!needle) return [];
	const haystack = text.toLowerCase();
	const offsets: number[] = [];
	let from = 0;
	for (;;) {
		const index = haystack.indexOf(needle, from);
		if (index === -1) return offsets;
		offsets.push(index);
		from = index + Math.max(1, needle.length);
		if (offsets.length > 10_000) return offsets;
	}
}

function shouldSkipTextNode(node: Text): boolean {
	const parent = node.parentElement;
	if (!parent) return true;
	return parent.closest(`mark, script, style, .katex`) !== null;
}

function collectTextNodes(root: HTMLElement): Text[] {
	const walker = root.ownerDocument.createTreeWalker(root, NodeFilter.SHOW_TEXT);
	const nodes: Text[] = [];
	for (let node = walker.nextNode(); node; node = walker.nextNode()) {
		const text = node as Text;
		if (!text.data.trim() || shouldSkipTextNode(text)) continue;
		nodes.push(text);
	}
	return nodes;
}

/** Wrap every occurrence inside one text node; returns created <mark>s. */
function wrapNodeOccurrences(node: Text, offsets: number[], queryLength: number): HTMLElement[] {
	if (offsets.length === 0) return [];
	const text = node.data;
	const doc = node.ownerDocument;
	const fragment = doc.createDocumentFragment();
	const marks: HTMLElement[] = [];
	let cursor = 0;
	for (const offset of offsets) {
		if (offset < cursor) continue;
		if (offset > cursor) fragment.append(text.slice(cursor, offset));
		const mark = doc.createElement('mark');
		mark.className = SEARCH_MARK_CLASS;
		mark.textContent = text.slice(offset, offset + queryLength);
		fragment.append(mark);
		marks.push(mark);
		cursor = offset + queryLength;
	}
	if (marks.length === 0) return [];
	if (cursor < text.length) fragment.append(text.slice(cursor));
	node.replaceWith(fragment);
	return marks;
}

export interface SearchHighlights {
	marks: HTMLElement[];
	total: number;
	/** True when the chapter had more matches than the cap. */
	truncated: boolean;
}

/** Remove all highlight marks previously created by this module. */
export function clearSearchHighlights(root: HTMLElement): void {
	for (const mark of Array.from(root.querySelectorAll<HTMLElement>(`.${SEARCH_MARK_CLASS}`))) {
		const parent = mark.parentNode;
		if (!parent) continue;
		parent.replaceChild(mark.ownerDocument.createTextNode(mark.textContent ?? ''), mark);
	}
	// Merge the split text nodes back so the DOM does not fragment per search.
	root.normalize();
}

/** Highlight `query` in the chapter; returns the created marks in order. */
export function highlightChapterMatches(
	root: HTMLElement,
	query: string,
	limit = MAX_CHAPTER_MATCHES,
): SearchHighlights {
	clearSearchHighlights(root);
	const needle = query.trim().toLowerCase();
	if (!needle) return { marks: [], total: 0, truncated: false };
	const marks: HTMLElement[] = [];
	let truncated = false;
	for (const node of collectTextNodes(root)) {
		const offsets = findOccurrences(node.data, needle);
		if (offsets.length === 0) continue;
		const remaining = limit - marks.length;
		if (offsets.length > remaining) {
			truncated = true;
			marks.push(...wrapNodeOccurrences(node, offsets.slice(0, remaining), needle.length));
			break;
		}
		marks.push(...wrapNodeOccurrences(node, offsets, needle.length));
		if (marks.length >= limit) {
			truncated = true;
			break;
		}
	}
	return { marks, total: marks.length, truncated };
}

/** Mark the current match and scroll it into view. */
export function setCurrentMatch(
	marks: HTMLElement[],
	index: number,
	scroller: HTMLElement | null,
): number {
	if (marks.length === 0) return -1;
	const next = Math.min(marks.length - 1, Math.max(0, index));
	for (const [i, mark] of marks.entries()) {
		mark.classList.toggle(SEARCH_CURRENT_CLASS, i === next);
	}
	const current = marks[next];
	if (current && scroller) {
		const scrollerRect = scroller.getBoundingClientRect();
		const rect = current.getBoundingClientRect();
		const fullyVisible = rect.top >= scrollerRect.top + 8 && rect.bottom <= scrollerRect.bottom - 8;
		if (!fullyVisible) {
			scroller.scrollTo({
				top: scroller.scrollTop + rect.top - scrollerRect.top - scrollerRect.height * 0.32,
				behavior: 'smooth'
			});
		}
	}
	return next;
}
