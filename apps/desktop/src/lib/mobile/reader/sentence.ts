// Sentence-level focus helpers for the mobile reader.
//
// Reuses the locked splitting + scroll-target algorithms from `$lib/focus`
// (`splitSentences`, `getFocusScrollTarget`) — the mobile focus mode is a new
// presentation, not a second focus engine. Everything here is DOM-dependent
// (Ranges, elementFromPoint); the pure parts live in `$lib/focus`.
//
// Performance model:
//   - The chapter DOM is scanned once when focus mode starts; only character
//     offsets are stored per sentence (no Range objects, no layout reads).
//   - The active sentence is located with `elementFromPoint` +
//     `caretRangeFromPoint` (no full-document rect scan, so
//     `content-visibility:auto` stays enabled).
//   - Each update measures exactly one Range rect (one forced layout read at
//     most) and never writes DOM inside the article.

import { splitSentences } from '$lib/focus/segment';
import { getFocusScrollTarget } from '$lib/focus/scroll';

/** Reading line as a fraction of the scroller viewport (desktop parity). */
export const FOCUS_ANCHOR_RATIO = 0.5;
export const FOCUS_LONG_BLOCK_RATIO = 0.78;
export const FOCUS_LONG_BLOCK_EDGE_RATIO = 0.12;
export const FOCUS_LONG_BLOCK_EDGE_MIN = 48;
export const FOCUS_LONG_BLOCK_EDGE_MAX = 140;

/** Leaf block elements that can carry a sentence. */
export const FOCUS_BLOCK_SELECTOR =
	'p, h1, h2, h3, h4, h5, h6, li, blockquote, figcaption, td, th, pre, dd, dt, div';

interface SentenceRange {
	start: number;
	end: number;
}

interface BlockInfo {
	block: HTMLElement;
	blockIndex: number;
	/** Character ranges of the block's sentences (masked text offsets). */
	ranges: SentenceRange[];
}

export interface FocusModel {
	blocks: BlockInfo[];
	/** First global sentence index for each block (cumulative counts). */
	blockStarts: number[];
	total: number;
}

export interface FocusLocation {
	blockIndex: number;
	sentenceIndex: number;
	/** Global 0-based sentence index. */
	globalIndex: number;
	start: number;
	end: number;
}

/** Replace code/KaTeX text with 'x' so their punctuation never splits prose. */
function maskedTextForBlock(block: HTMLElement): string {
	if (typeof document === 'undefined') return '';
	const walker = document.createTreeWalker(block, NodeFilter.SHOW_TEXT);
	let text = '';
	for (let node = walker.nextNode(); node; node = walker.nextNode()) {
		const data = (node as Text).data;
		const parent = (node as Text).parentElement;
		text += parent?.closest('code, .katex, pre') ? 'x'.repeat(data.length) : data;
	}
	return text;
}

/** Leaf text blocks in reading order (nested `<li><p>` keeps only the `<p>`). */
export function collectFocusBlocks(article: HTMLElement): HTMLElement[] {
	const blocks: HTMLElement[] = [];
	const candidates = article.querySelectorAll<HTMLElement>(FOCUS_BLOCK_SELECTOR);
	for (const element of Array.from(candidates)) {
		if (!element.textContent || !element.textContent.trim()) continue;
		// Only the innermost block owns its text run — otherwise a wrapping
		// <div> would duplicate every descendant sentence.
		if (element.querySelector(FOCUS_BLOCK_SELECTOR)) continue;
		if (element.closest('.katex')) continue;
		blocks.push(element);
	}
	return blocks;
}

/** Build the focus model. One pass, no layout reads. */
export function buildFocusModel(article: HTMLElement): FocusModel {
	const blocks: BlockInfo[] = [];
	const blockStarts: number[] = [];
	let total = 0;
	for (const [blockIndex, block] of collectFocusBlocks(article).entries()) {
		const masked = maskedTextForBlock(block);
		let ranges: SentenceRange[] = splitSentences(masked);
		if (ranges.length === 0) ranges = [{ start: 0, end: Math.max(0, masked.length) }];
		blockStarts.push(total);
		total += ranges.length;
		blocks.push({ block, blockIndex, ranges });
	}
	return { blocks, blockStarts, total };
}

/** Map a global sentence index back to its block + character range. */
export function locateUnit(model: FocusModel, globalIndex: number): FocusLocation | null {
	if (model.total <= 0) return null;
	const index = Math.min(model.total - 1, Math.max(0, Math.trunc(globalIndex)));
	// blockStarts is strictly increasing; binary search the owning block.
	let low = 0;
	let high = model.blockStarts.length - 1;
	while (low < high) {
		const mid = Math.ceil((low + high) / 2);
		if (model.blockStarts[mid] <= index) low = mid;
		else high = mid - 1;
	}
	const info = model.blocks[low];
	if (!info) return null;
	const sentenceIndex = index - model.blockStarts[low];
	const range = info.ranges[sentenceIndex];
	if (!range) return null;
	return {
		blockIndex: low,
		sentenceIndex,
		globalIndex: index,
		start: range.start,
		end: range.end,
	};
}

/** Absolute character offset of a (node, offset) pair inside `block`. */
function textOffsetOf(block: HTMLElement, node: Node, offset: number): number {
	const walker = document.createTreeWalker(block, NodeFilter.SHOW_TEXT);
	let accumulated = 0;
	for (let current = walker.nextNode(); current; current = walker.nextNode()) {
		if (current === node) return accumulated + Math.max(0, Math.min(offset, current.textContent?.length ?? 0));
		accumulated += current.textContent?.length ?? 0;
	}
	return accumulated;
}

function caretOffsetAtPoint(block: HTMLElement, x: number, y: number): number | null {
	const doc = block.ownerDocument as Document & {
		caretRangeFromPoint?: (x: number, y: number) => Range | null;
		caretPositionFromPoint?: (x: number, y: number) => { offsetNode: Node; offset: number } | null;
	};
	if (typeof doc.caretRangeFromPoint === 'function') {
		const range = doc.caretRangeFromPoint(x, y);
		if (range) return textOffsetOf(block, range.startContainer, range.startOffset);
	}
	if (typeof doc.caretPositionFromPoint === 'function') {
		const position = doc.caretPositionFromPoint(x, y);
		if (position) return textOffsetOf(block, position.offsetNode, position.offset);
	}
	return null;
}

function sentenceIndexForOffset(ranges: SentenceRange[], offset: number): number {
	if (ranges.length === 0) return 0;
	let low = 0;
	let high = ranges.length - 1;
	while (low < high) {
		const mid = Math.ceil((low + high) / 2);
		if (ranges[mid].start <= offset) low = mid;
		else high = mid - 1;
	}
	return low;
}

/** Block index for a DOM element, or -1 when it is not a focus block. */
function blockIndexForElement(model: FocusModel, element: Element | null): number {
	if (!element) return -1;
	const direct = model.blocks.findIndex((info) => info.block === element);
	if (direct >= 0) return direct;
	// elementFromPoint may land on an inline child / wrapper between blocks.
	const closest = element.closest(FOCUS_BLOCK_SELECTOR);
	if (!closest) return -1;
	return model.blocks.findIndex((info) => info.block === closest);
}

/** Locate the sentence under a client coordinate (tap → jump to sentence). */
export function locateUnitAtPoint(
	model: FocusModel,
	article: HTMLElement,
	clientX: number,
	clientY: number,
): FocusLocation | null {
	if (typeof document === 'undefined' || model.total === 0) return null;
	const hit = document.elementFromPoint(clientX, clientY);
	if (!hit || !article.contains(hit)) return null;
	let blockIndex = blockIndexForElement(model, hit);
	if (blockIndex < 0) return null;
	const info = model.blocks[blockIndex];
	let offset = caretOffsetAtPoint(info.block, clientX, clientY);
	if (offset === null) {
		// Fallback: nearest sentence by vertical position inside the block.
		const range = createUnitRange(info.block, { start: 0, end: info.ranges[0]?.end ?? 0 });
		if (!range) return null;
		const rect = range.getBoundingClientRect();
		offset = clientY >= rect.bottom ? Number.POSITIVE_INFINITY : 0;
	}
	const sentenceIndex = Number.isFinite(offset)
		? sentenceIndexForOffset(info.ranges, offset)
		: info.ranges.length - 1;
	const located = locateUnit(model, model.blockStarts[blockIndex] + sentenceIndex);
	return located;
}

/** Locate the sentence crossing the reading line (manual scroll syncing). */
export function locateUnitAtLine(
	model: FocusModel,
	article: HTMLElement,
	scroller: HTMLElement,
	lineY?: number,
): FocusLocation | null {
	if (typeof document === 'undefined' || model.total === 0) return null;
	const rect = scroller.getBoundingClientRect();
	const y = lineY ?? rect.top + rect.height * FOCUS_ANCHOR_RATIO;
	const x = Math.min(rect.right - 8, Math.max(rect.left + 8, rect.left + rect.width / 2));
	const hit = document.elementFromPoint(x, y);
	if (!hit || !article.contains(hit)) return null;
	const blockIndex = blockIndexForElement(model, hit);
	if (blockIndex < 0) return null;
	const info = model.blocks[blockIndex];
	let offset = caretOffsetAtPoint(info.block, x, y);
	if (offset === null) {
		offset = rangeOffsetAtLine(info, y);
	}
	return locateUnit(model, model.blockStarts[blockIndex] + sentenceIndexForOffset(info.ranges, offset));
}

/** Fallback for missing caret APIs: binary search a block's sentence rects. */
function rangeOffsetAtLine(info: BlockInfo, lineY: number): number {
	let low = 0;
	let high = info.ranges.length - 1;
	while (low < high) {
		const mid = Math.ceil((low + high) / 2);
		const range = createUnitRange(info.block, info.ranges[mid]);
		const rect = range?.getBoundingClientRect();
		if (rect && rect.bottom <= lineY) low = mid;
		else high = mid - 1;
	}
	return info.ranges[low]?.start ?? 0;
}

function findTextNodes(block: HTMLElement): Text[] {
	const walker = block.ownerDocument.createTreeWalker(block, NodeFilter.SHOW_TEXT);
	const nodes: Text[] = [];
	for (let node = walker.nextNode(); node; node = walker.nextNode()) nodes.push(node as Text);
	return nodes;
}

/** Build a DOM Range for one sentence's character offsets inside `block`. */
export function createUnitRange(
	block: HTMLElement,
	range: SentenceRange | null,
): Range | null {
	if (!range) return null;
	if (typeof document === 'undefined') return null;
	const nodes = findTextNodes(block);
	if (nodes.length === 0) return null;
	let accumulated = 0;
	let startNode: Text | null = null;
	let startOffset = 0;
	let endNode: Text | null = null;
	let endOffset = 0;
	for (const node of nodes) {
		const length = node.data.length;
		if (startNode === null && range.start <= accumulated + length) {
			startNode = node;
			startOffset = Math.max(0, Math.min(length, range.start - accumulated));
		}
		if (range.end <= accumulated + length) {
			endNode = node;
			endOffset = Math.max(0, Math.min(length, range.end - accumulated));
			break;
		}
		accumulated += length;
	}
	if (!startNode) {
		startNode = nodes[nodes.length - 1];
		startOffset = startNode.data.length;
	}
	if (!endNode) {
		endNode = nodes[nodes.length - 1];
		endOffset = endNode.data.length;
	}
	try {
		const domRange = block.ownerDocument.createRange();
		domRange.setStart(startNode, startOffset);
		domRange.setEnd(endNode, endOffset);
		return domRange;
	} catch {
		return null;
	}
}

/** Range for a global sentence index. */
export function rangeForUnit(model: FocusModel, globalIndex: number): Range | null {
	const located = locateUnit(model, globalIndex);
	if (!located) return null;
	const info = model.blocks[located.blockIndex];
	return createUnitRange(info.block, info.ranges[located.sentenceIndex]);
}

export interface UnitBox {
	top: number;
	bottom: number;
	left: number;
	right: number;
	height: number;
}

/** Sentence box in scroller content coordinates (for scrolling). */
export function unitBoxInContent(
	model: FocusModel,
	globalIndex: number,
	scroller: HTMLElement,
): UnitBox | null {
	const range = rangeForUnit(model, globalIndex);
	if (!range) return null;
	const rect = range.getBoundingClientRect();
	if (rect.width === 0 && rect.height === 0) return null;
	const scrollerRect = scroller.getBoundingClientRect();
	const top = scroller.scrollTop + rect.top - scrollerRect.top;
	return {
		top,
		bottom: top + rect.height,
		left: rect.left,
		right: rect.right,
		height: rect.height
	};
}

export function focusEdgeBuffer(scroller: HTMLElement): number {
	return Math.max(
		FOCUS_LONG_BLOCK_EDGE_MIN,
		Math.min(FOCUS_LONG_BLOCK_EDGE_MAX, Math.round(scroller.clientHeight * FOCUS_LONG_BLOCK_EDGE_RATIO)),
	);
}

/**
 * Scroll target that puts the sentence on the reading line — exactly the
 * locked desktop algorithm, with mobile viewport inputs.
 */
export function scrollTargetForUnit(
	model: FocusModel,
	globalIndex: number,
	scroller: HTMLElement,
): number | null {
	const box = unitBoxInContent(model, globalIndex, scroller);
	if (!box) return null;
	const scrollerRect = scroller.getBoundingClientRect();
	const anchorOffset = scrollerRect.height * FOCUS_ANCHOR_RATIO;
	const target = getFocusScrollTarget({
		viewportHeight: scroller.clientHeight,
		currentScrollTop: scroller.scrollTop,
		blockTop: box.top,
		blockBottom: box.bottom,
		anchorOffset,
		longBlockRatio: FOCUS_LONG_BLOCK_RATIO,
		longBlockEdgeBuffer: focusEdgeBuffer(scroller)
	});
	if (target === null) return null;
	const max = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
	return Math.max(0, Math.min(target, max));
}

/** Plain-text excerpt of one sentence (HUD / a11y label). */
export function unitText(model: FocusModel, globalIndex: number): string {
	const located = locateUnit(model, globalIndex);
	if (!located) return '';
	const info = model.blocks[located.blockIndex];
	const text = maskedTextForBlock(info.block);
	return text.slice(located.start, located.end).trim();
}
