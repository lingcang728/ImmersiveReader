// Mobile Markdown render client.
//
// Reuses the desktop render pipeline (`$lib/render/markdown.ts`) — there is
// exactly one Markdown parser in this repo. The worker is created with the
// same `type: 'module'` wiring as the desktop page, but always passes the
// mobile `lightweight` option and lazily falls back to a main-thread dynamic
// import when the worker cannot load (e.g. an old WebView). The pipeline
// module is never statically imported here, so it stays out of the mobile
// first-screen bundle.

import { convertFileSrc } from '@tauri-apps/api/core';
import type { MarkdownRenderOptions, RenderedMarkdownDocument } from '$lib/render/markdown';
import { resolveMarkdownImageSources } from '$lib/render/images';

/** Mobile render profile: no Shiki; KaTeX CSS is injected on demand. */
export const MOBILE_RENDER_OPTIONS: MarkdownRenderOptions = { lightweight: true };

const WORKER_TIMEOUT_MS = 60_000;
const MAX_TRANSPORT_FAILURES = 2;
/** Documents above this size mount progressively (docs: > 1 MiB). */
export const PROGRESSIVE_MOUNT_MIN_CHARS = 1024 * 1024;
const FIRST_CHUNK_MAX_NODES = 8;
const FIRST_CHUNK_MAX_CHARS = 48 * 1024;
const LATER_CHUNK_MAX_NODES = 48;
const LATER_CHUNK_MAX_CHARS = 160 * 1024;

interface PendingRender {
	resolve: (result: RenderedMarkdownDocument) => void;
	reject: (error: Error) => void;
	timer: ReturnType<typeof setTimeout>;
}

let worker: Worker | null = null;
let workerFailed = false;
let transportFailures = 0;
let nextRequestId = 1;
const pendingRenders = new Map<number, PendingRender>();

export class RenderSupersededError extends Error {
	constructor() {
		super('markdown render superseded');
		this.name = 'RenderSupersededError';
	}
}

function rejectAllPending(error: Error) {
	for (const [id, pending] of pendingRenders) {
		clearTimeout(pending.timer);
		pendingRenders.delete(id);
		pending.reject(error);
	}
}

function getWorker(): Worker | null {
	if (workerFailed || typeof Worker === 'undefined') return null;
	if (worker) return worker;
	try {
		worker = new Worker(new URL('../../render/markdown.worker.ts', import.meta.url), {
			type: 'module'
		});
		worker.onmessage = (
			event: MessageEvent<{ id: number; result?: RenderedMarkdownDocument; error?: string }>
		) => {
			const pending = pendingRenders.get(event.data.id);
			if (!pending) return;
			pendingRenders.delete(event.data.id);
			clearTimeout(pending.timer);
			if ('result' in event.data && event.data.result) {
				transportFailures = 0;
				pending.resolve(event.data.result);
			} else {
				pending.reject(new Error(event.data.error ?? 'Markdown 渲染失败'));
			}
		};
		worker.onerror = (event) => {
			transportFailures += 1;
			if (transportFailures >= MAX_TRANSPORT_FAILURES) workerFailed = true;
			const failed = worker;
			worker = null;
			failed?.terminate();
			rejectAllPending(new Error(event.message || 'Markdown worker failed'));
		};
	} catch (error) {
		workerFailed = true;
		console.warn('[mobile-reader] Markdown worker unavailable', error);
		return null;
	}
	return worker;
}

/** Terminate the worker and fail every in-flight render (chapter switch/exit). */
export function cancelPendingRenders() {
	const current = worker;
	worker = null;
	rejectAllPending(new RenderSupersededError());
	current?.terminate();
}

/**
 * Render Markdown off the main thread. Rejects with `RenderSupersededError`
 * when a newer navigation cancelled this request — callers should ignore it.
 */
export async function renderMarkdownForMobile(
	source: string,
	options: MarkdownRenderOptions = MOBILE_RENDER_OPTIONS
): Promise<RenderedMarkdownDocument> {
	const activeWorker = getWorker();
	if (activeWorker) {
		const id = nextRequestId++;
		try {
			return await new Promise<RenderedMarkdownDocument>((resolve, reject) => {
				const timer = setTimeout(() => {
					pendingRenders.delete(id);
					reject(new Error('Markdown 渲染超时'));
				}, WORKER_TIMEOUT_MS);
				pendingRenders.set(id, { resolve, reject, timer });
				try {
					activeWorker.postMessage({ id, source, options });
				} catch (postError) {
					pendingRenders.delete(id);
					clearTimeout(timer);
					reject(postError instanceof Error ? postError : new Error(String(postError)));
				}
			});
		} catch (error) {
			if (error instanceof RenderSupersededError) throw error;
			console.warn('[mobile-reader] worker render failed, falling back to main thread', error);
			if (worker === activeWorker) {
				transportFailures += 1;
				if (transportFailures >= MAX_TRANSPORT_FAILURES) workerFailed = true;
				worker = null;
			}
			activeWorker.terminate();
			rejectAllPending(new Error('Markdown worker failed'));
		}
	}

	const { renderMarkdownDocument } = await import('$lib/render/markdown');
	return renderMarkdownDocument(source, options);
}

/** Resolve relative `<img>` sources to asset-protocol URLs for one document. */
export function resolveRenderedAssets(html: string, markdownPath: string | null): string {
	if (!markdownPath || !html.includes('<img')) return html;
	return resolveMarkdownImageSources(html, markdownPath, convertFileSrc);
}

// ---------------------------------------------------------------------------
// Progressive mounting (large documents)

/** True when the rendered document is large enough to mount in chunks. */
export function needsProgressiveMount(html: string): boolean {
	return html.length >= PROGRESSIVE_MOUNT_MIN_CHARS;
}

/**
 * Split rendered HTML into top-level chunks. The first chunk is deliberately
 * small (visible screen only), later chunks are appended one frame at a time
 * so touch input never blocks behind a multi-MB `innerHTML` write.
 * Returns `[html]` when a single write is fine.
 */
export function splitArticleHtmlIntoChunks(html: string): string[] {
	if (!needsProgressiveMount(html) || typeof document === 'undefined') return [html];
	const template = document.createElement('template');
	template.innerHTML = html;
	const nodes = Array.from(template.content.childNodes);
	if (nodes.length <= FIRST_CHUNK_MAX_NODES + 1) return [html];

	const chunks: string[] = [];
	let group: ChildNode[] = [];
	let groupChars = 0;
	const flushGroup = () => {
		if (group.length === 0) return;
		const holder = document.createElement('template');
		holder.content.append(...group);
		chunks.push(holder.innerHTML);
		group = [];
		groupChars = 0;
	};
	for (const node of nodes) {
		const isFirstChunk = chunks.length === 0;
		const maxNodes = isFirstChunk ? FIRST_CHUNK_MAX_NODES : LATER_CHUNK_MAX_NODES;
		const maxChars = isFirstChunk ? FIRST_CHUNK_MAX_CHARS : LATER_CHUNK_MAX_CHARS;
		const nodeChars =
			node instanceof Element ? node.outerHTML.length : (node.textContent ?? '').length;
		if (group.length > 0 && (group.length >= maxNodes || groupChars + nodeChars > maxChars)) {
			flushGroup();
		}
		group.push(node);
		groupChars += nodeChars;
	}
	flushGroup();
	return chunks.length > 1 ? chunks : [html];
}

// ---------------------------------------------------------------------------
// KaTeX CSS — only paid for when the document actually contains math.

let katexCssRequested = false;

export function ensureKatexCss(html: string): void {
	if (katexCssRequested || !html.includes('katex')) return;
	katexCssRequested = true;
	void import('katex/dist/katex.min.css').catch((error) => {
		console.warn('[mobile-reader] KaTeX CSS unavailable', error);
	});
}

/** IntersectionObserver-based lazy wrapper is unnecessary; native loading=lazy
 * covers images. This marker keeps the intent explicit for the DOM pass. */
export function markLazyImages(html: string): string {
	return html;
}
