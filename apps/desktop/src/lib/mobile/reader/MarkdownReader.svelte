<!--
	Mobile Markdown reader — library multi-chapter book and single-file modes.

	Responsibilities:
	  - chapter loading (openBook chapter list → getBookChapterPath → readMarkdown)
	  - off-main-thread render via the desktop worker pipeline (mobile profile)
	  - progress within a chapter + whole book, restored/persisted with the same
	    data structures the desktop app uses (`nextReadingState` /
	    `save_book_progress` / `save_reading_state`)
	  - progressive mount for >1 MiB chapters, KaTeX CSS on demand, lazy images
	  - touch gestures (center tap = chrome, edge tap = page, swipe = chapter)
	  - in-chapter search with <mark> highlights

	Focus mode lives in FocusMode.svelte; ReaderScreen owns the chrome/sheets.
-->
<script lang="ts">
	import { onDestroy, onMount, tick } from 'svelte';
	import * as api from '$lib/mobile/api';
	import type { BookChapter, BookReadingState } from '$lib/mobile/contracts';
	import { nextReadingState } from '$lib/library/books';
	import {
		MOBILE_RENDER_OPTIONS,
		cancelPendingRenders,
		ensureKatexCss,
		renderMarkdownForMobile,
		resolveRenderedAssets,
		splitArticleHtmlIntoChunks
	} from './markdownRender';
	import {
		bookOverallProgress,
		chapterRatioFromScroll,
		clampChapterIndex,
		fileOverallProgress,
		isChapterComplete,
		pageScrollTarget,
		resolveBookRestoreRatio,
		resolveFileRestore,
		resolveStartChapterIndex,
		restoreScrollTop,
		shouldPreloadNext,
		shouldPromptNext,
		buildReaderMeta,
		displayChapterNumber
	} from './progress';
	import {
		gestureExcluded,
		hasActiveSelection,
		resolveGesture,
		LONG_PRESS_MS
	} from './gestures';
	import {
		clearSearchHighlights,
		highlightChapterMatches,
		setCurrentMatch,
		type SearchHighlights
	} from './search';
	import type { ChapterSearchResult, ReaderMeta, ScrollRestore } from './types';

	interface MarkdownBookInput {
		bookId: string;
		title: string;
		chapters: BookChapter[];
		progress: BookReadingState;
		requestedChapterId?: string;
	}

	interface Props {
		/** Book mode payload; null for single-file mode. */
		book?: MarkdownBookInput | null;
		/** Absolute path of the Markdown file in single-file mode. */
		filePath?: string;
		/** Display name used when the backend does not give a chapter title. */
		title: string;
		gesturesEnabled?: boolean;
		onMeta: (meta: ReaderMeta) => void;
		onToggleChrome: () => void;
		onNotice: (message: string) => void;
	}

	let {
		book = null,
		filePath = '',
		title,
		gesturesEnabled = true,
		onMeta,
		onToggleChrome,
		onNotice
	}: Props = $props();

	let scrollerEl = $state<HTMLElement | null>(null);
	let articleEl = $state<HTMLElement | null>(null);
	let loading = $state(true);
	let errorMessage = $state('');
	let articleChunks = $state<string[]>([]);
	let chapterIndex = $state(0);
	let chapterId = $state('');
	let chapterTitle = $state('');
	let chapterRatio = $state(0);
	let overall = $state(0);
	let contentKey = $state('');
	let seamVisible = $state(false);

	const isBook = $derived(book !== null);
	const chapters = $derived(book?.chapters ?? []);
	const bookId = $derived(book?.bookId ?? '');
	const currentPath = $derived(filePath);

	function emptyProgressState(): BookReadingState {
		return {
			schemaVersion: 1,
			current: '',
			position: 0,
			read: [],
			updated: new Date().toISOString()
		};
	}

	// Live metadata (not reactive — read by the exported getters). Seeded from
	// the book detail inside onMount, where reading props is unproblematic.
	let progressState: BookReadingState = emptyProgressState();
	let mountGeneration = 0;
	let loadGeneration = 0;
	let pendingChunks: string[] = [];
	let pendingScrollRestore: ScrollRestore | null = null;
	let pendingFragment = '';
	/** Re-anchor restored position while late-loading images reflow the page. */
	let imageRestore: { restore: ScrollRestore; deadline: number } | null = null;
	/** Headings of the mounted chapter (single-file TOC). */
	let lastToc: { id: string; text: string; level: number }[] = [];

	// Next-chapter preload cache (at most one chapter).
	let preloadCache: {
		index: number;
		path: string;
		rendered: Awaited<ReturnType<typeof renderMarkdownForMobile>>;
	} | null = null;
	let preloadInFlight: Promise<void> | null = null;

	// Progress saving (serialized + throttled).
	const SAVE_THROTTLE_MS = 1000;
	let saveTimer: ReturnType<typeof setTimeout> | null = null;
	let saveTail: Promise<void> = Promise.resolve();
	let saveFailureNotified = false;
	let lastMetaKey = '';
	let destroyed = false;

	// Search.
	let searchHighlights: SearchHighlights | null = null;
	let searchIndex = -1;

	// Gestures.
	let touchStartX = 0;
	let touchStartY = 0;
	let touchStartTime = 0;
	let touchStartScrollTop = 0;
	let touchExcluded = false;
	let touchMulti = false;
	let touchLongPress = false;
	let longPressTimer: ReturnType<typeof setTimeout> | null = null;
	let scrollFrame: number | null = null;

	const canPrev = $derived(chapterIndex > 0);
	const canNext = $derived(isBook ? chapterIndex < chapters.length - 1 : false);
	const chapterCount = $derived(isBook ? Math.max(1, chapters.length) : 1);

	// ---------------------------------------------------------------------
	// Meta reporting

	function emitMeta(force = false) {
		if (destroyed) return;
		const ratio = chapterRatio;
		const meta = isBook
			? buildReaderMeta({
					title: book?.title ?? title,
					chapterId,
					chapterTitle,
					chapterIndex,
					chapterCount,
					chapterRatio: ratio,
					overall: bookOverallProgress(chapters, progressState, chapterId, ratio),
					canPrev,
					canNext
				})
			: buildReaderMeta({
					title,
					chapterId,
					chapterTitle,
					chapterIndex: 0,
					chapterCount: 1,
					chapterRatio: ratio,
					overall: fileOverallProgress(ratio),
					canPrev: false,
					canNext: false
				});
		overall = meta.overall;
		const key = `${meta.chapterId}|${Math.round(meta.chapterRatio * 200)}|${Math.round(
			meta.overall * 400
		)}|${meta.canPrev ? 1 : 0}${meta.canNext ? 1 : 0}|${meta.chapterIndex}`;
		if (!force && key === lastMetaKey) return;
		lastMetaKey = key;
		onMeta(meta);
	}

	// ---------------------------------------------------------------------
	// Progress persistence

	async function persistProgress() {
		if (destroyed || !scrollerEl) return;
		try {
			if (isBook) {
				if (!chapterId) return;
				const isLast = chapterIndex >= chapters.length - 1;
				const completed = isChapterComplete(chapterRatio, isLast);
				progressState = nextReadingState(progressState, chapterId, chapterRatio, completed);
				await api.saveBookProgress(bookId, progressState);
			} else if (currentPath) {
				await api.saveFileReadingState(currentPath, {
					scroll_position: scrollerEl.scrollTop,
					bookmarks: [],
					progress: chapterRatio
				});
			}
			saveFailureNotified = false;
		} catch (error) {
			console.warn('[mobile-reader] progress save failed', error);
			if (!saveFailureNotified) {
				saveFailureNotified = true;
				onNotice('阅读进度暂时无法保存，退出前会再试一次');
			}
		}
	}

	function saveProgressNow(): Promise<void> {
		const run = saveTail.then(() => persistProgress());
		saveTail = run.catch(() => {});
		return run;
	}

	function scheduleProgressSave() {
		if (saveTimer) return;
		saveTimer = setTimeout(() => {
			saveTimer = null;
			void saveProgressNow();
		}, SAVE_THROTTLE_MS);
	}

	/** Flush pending progress (chapter change, exit, app hide). */
	export async function flushProgress(): Promise<void> {
		if (saveTimer) {
			clearTimeout(saveTimer);
			saveTimer = null;
		}
		await saveProgressNow();
	}

	function handleVisibilityChange() {
		if (typeof document !== 'undefined' && document.visibilityState === 'hidden') {
			void flushProgress();
		}
	}

	function handlePageHide() {
		void flushProgress();
	}

	// ---------------------------------------------------------------------
	// Chapter loading

	async function chapterPathFor(index: number): Promise<string> {
		if (!isBook) return currentPath;
		const chapter = chapters[index];
		if (!chapter) throw new Error('章节不存在');
		return api.getBookChapterPath(bookId, chapter.id);
	}

	async function settleChunks(token: number) {
		if (pendingChunks.length === 0) return;
		while (pendingChunks.length > 0) {
			if (token !== loadGeneration || destroyed) return;
			await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
			if (token !== loadGeneration || destroyed) return;
			const next = pendingChunks[0];
			pendingChunks = pendingChunks.slice(1);
			articleChunks = [...articleChunks, next];
			await tick();
		}
	}

	function applyScrollRestore(restore: ScrollRestore | null, fragment: string) {
		if (!scrollerEl) return;
		if (fragment && articleEl) {
			const target =
				articleEl.querySelector(`#${cssEscape(fragment)}`) ??
				articleEl.querySelector(`[name="${fragment.replace(/"/g, '\\"')}"]`);
			if (target instanceof HTMLElement) {
				target.scrollIntoView({ block: 'start' });
				return;
			}
		}
		const target = restoreScrollTop(restore, scrollerEl.scrollHeight, scrollerEl.clientHeight);
		if (target !== null) {
			const previousBehavior = scrollerEl.style.scrollBehavior;
			scrollerEl.style.scrollBehavior = 'auto';
			scrollerEl.scrollTop = target;
			scrollerEl.style.scrollBehavior = previousBehavior;
			// Images below the restored line can land later and shift it; keep
			// re-anchoring on resource loads until the settle window closes.
			imageRestore = restore ? { restore, deadline: Date.now() + 4000 } : null;
		} else {
			scrollerEl.scrollTop = 0;
			imageRestore = null;
		}
	}

	/** Capture-phase resource load: keep the restored ratio pinned. */
	function handleResourceLoad() {
		if (!imageRestore || !scrollerEl) return;
		if (Date.now() > imageRestore.deadline) {
			imageRestore = null;
			return;
		}
		const target = restoreScrollTop(
			imageRestore.restore,
			scrollerEl.scrollHeight,
			scrollerEl.clientHeight
		);
		if (target === null) return;
		const previousBehavior = scrollerEl.style.scrollBehavior;
		scrollerEl.style.scrollBehavior = 'auto';
		scrollerEl.scrollTop = target;
		scrollerEl.style.scrollBehavior = previousBehavior;
	}

	function cssEscape(value: string): string {
		if (typeof CSS !== 'undefined' && typeof CSS.escape === 'function') return CSS.escape(value);
		return value.replace(/[^\w-]/g, (ch) => `\\${ch}`);
	}

	/** Mount one chapter; `restore` is applied after the progressive mount settles. */
	async function loadChapterAt(index: number, restore: ScrollRestore | null, fragment = '') {
		if (!isBook && !currentPath) {
			loading = false;
			errorMessage = '文件路径为空';
			return;
		}
		const safeIndex = isBook ? clampChapterIndex(index, chapters.length) : 0;
		if (safeIndex < 0) {
			loading = false;
			errorMessage = '这本文集还没有可读章节';
			return;
		}
		const token = ++loadGeneration;
		loading = true;
		errorMessage = '';
		seamVisible = false;
		chapterRatio = 0;
		overall = 0;
		clearSearch();

		try {
			const path = await chapterPathFor(safeIndex);
			if (token !== loadGeneration || destroyed) return;
			let rendered: Awaited<ReturnType<typeof renderMarkdownForMobile>> | null = null;
			if (preloadCache && preloadCache.index === safeIndex && preloadCache.path === path) {
				rendered = preloadCache.rendered;
			}
			const result = rendered ? null : await api.readMarkdown(path);
			if (token !== loadGeneration || destroyed) return;
			if (!rendered) {
				rendered = await renderMarkdownForMobile(result!.content, MOBILE_RENDER_OPTIONS);
			}
			if (token !== loadGeneration || destroyed) return;

			const html = resolveRenderedAssets(rendered.html, path);
			ensureKatexCss(html);
			lastToc = rendered.toc ?? [];
			const chunks = splitArticleHtmlIntoChunks(html);
			articleChunks = chunks.slice(0, 1);
			pendingChunks = chunks.slice(1);
			pendingScrollRestore = restore;
			pendingFragment = fragment;

			chapterIndex = safeIndex;
			if (isBook) {
				const chapter = chapters[safeIndex];
				chapterId = chapter?.id ?? '';
				chapterTitle = chapter?.title ?? '';
				progressState = { ...progressState, current: chapterId };
			} else {
				chapterId = currentPath;
				chapterTitle = title;
			}
			contentKey = `${chapterId}:${++mountGeneration}`;
			loading = false;
			await tick();
			if (token !== loadGeneration || destroyed) return;
			await settleChunks(token);
			if (token !== loadGeneration || destroyed) return;
			applyScrollRestore(pendingScrollRestore, pendingFragment);
			pendingScrollRestore = null;
			pendingFragment = '';
			updateFromScroll();
			emitMeta(true);
			if (preloadCache && preloadCache.index === safeIndex) preloadCache = null;
		} catch (error) {
			if (token !== loadGeneration || destroyed) return;
			console.error('[mobile-reader] chapter load failed', error);
			errorMessage = api.errorText(error);
			loading = false;
			chapterRatio = 0;
		}
	}

	/** Persist the chapter we are leaving, then open the target chapter. */
	async function switchChapter(
		targetIndex: number,
		direction: -1 | 1,
		completePrevious = direction > 0
	) {
		if (loading) return;
		const clamped = isBook ? clampChapterIndex(targetIndex, chapters.length) : 0;
		if (clamped < 0 || clamped === chapterIndex) return;
		if (isBook && chapterId && completePrevious) {
			// Sequential forward reading marks the finished chapter as read. A
			// TOC jump does not (desktop parity: jumping is not "reading it").
			progressState = nextReadingState(progressState, chapterId, 1, true);
			void api.saveBookProgress(bookId, progressState).catch(() => {});
		}
		await loadChapterAt(clamped, direction < 0 ? { kind: 'ratio', value: 1 } : { kind: 'ratio', value: 0 });
	}

	export async function navigateChapter(direction: -1 | 1): Promise<boolean> {
		const target = chapterIndex + direction;
		if (isBook && (target < 0 || target >= chapters.length)) {
			if (direction > 0) void flushProgress();
			return false;
		}
		await switchChapter(target, direction);
		return true;
	}

	export async function goToChapterId(id: string): Promise<boolean> {
		if (!isBook) return false;
		const index = chapters.findIndex((chapter) => chapter.id === id);
		if (index < 0) return false;
		if (index === chapterIndex) {
			scrollerEl?.scrollTo({ top: 0, behavior: 'smooth' });
			return true;
		}
		await switchChapter(index, index > chapterIndex ? 1 : -1, false);
		return true;
	}

	// ---------------------------------------------------------------------
	// Scroll + preload

	function updateFromScroll() {
		if (!scrollerEl || loading) return;
		chapterRatio = chapterRatioFromScroll(
			scrollerEl.scrollTop,
			scrollerEl.scrollHeight,
			scrollerEl.clientHeight
		);
		seamVisible = shouldPromptNext(chapterRatio, canNext);
		emitMeta();
		scheduleProgressSave();
		if (isBook && shouldPreloadNext(chapterRatio, canNext)) void preloadNextChapter();
	}

	function handleScroll() {
		if (scrollFrame !== null) return;
		scrollFrame = requestAnimationFrame(() => {
			scrollFrame = null;
			updateFromScroll();
		});
	}

	async function preloadNextChapter() {
		if (!isBook || preloadInFlight || preloadCache) return;
		if (typeof document !== 'undefined' && document.visibilityState === 'hidden') return;
		const nextIndex = chapterIndex + 1;
		if (nextIndex >= chapters.length) return;
		const token = loadGeneration;
		preloadInFlight = (async () => {
			try {
				const path = await chapterPathFor(nextIndex);
				const result = await api.readMarkdown(path);
				const rendered = await renderMarkdownForMobile(result.content, MOBILE_RENDER_OPTIONS);
				if (destroyed || token !== loadGeneration) return;
				preloadCache = { index: nextIndex, path, rendered };
			} catch {
				// Normal navigation will surface a concrete error if needed.
			}
		})().finally(() => {
			preloadInFlight = null;
		});
		await preloadInFlight;
	}

	// ---------------------------------------------------------------------
	// Touch gestures

	function clearLongPress() {
		if (longPressTimer) {
			clearTimeout(longPressTimer);
			longPressTimer = null;
		}
	}

	function handleTouchStart(event: TouchEvent) {
		// User intent wins over the image-reflow re-anchor window.
		imageRestore = null;
		if (!gesturesEnabled || event.touches.length !== 1) {
			touchMulti = true;
			clearLongPress();
			return;
		}
		touchMulti = false;
		touchLongPress = false;
		const touch = event.touches[0];
		touchStartX = touch.clientX;
		touchStartY = touch.clientY;
		touchStartTime = Date.now();
		touchStartScrollTop = scrollerEl?.scrollTop ?? 0;
		touchExcluded = gestureExcluded(event.target, scrollerEl);
		clearLongPress();
		if (!touchExcluded) {
			longPressTimer = setTimeout(() => {
				longPressTimer = null;
				touchLongPress = true;
			}, LONG_PRESS_MS);
		}
	}

	function handleTouchMove(event: TouchEvent) {
		if (touchMulti || touchLongPress) return;
		const touch = event.touches[0];
		if (!touch) return;
		if (
			Math.abs(touch.clientX - touchStartX) > 10 ||
			Math.abs(touch.clientY - touchStartY) > 10
		) {
			clearLongPress();
		}
	}

	function handleTouchCancel() {
		clearLongPress();
		touchMulti = false;
		touchExcluded = false;
		touchLongPress = false;
	}

	function handleTouchEnd(event: TouchEvent) {
		clearLongPress();
		const wasMulti = touchMulti;
		const wasExcluded = touchExcluded;
		const wasLongPress = touchLongPress;
		touchMulti = false;
		touchExcluded = false;
		touchLongPress = false;
		if (!gesturesEnabled || wasMulti || wasExcluded || wasLongPress) return;
		if (event.changedTouches.length !== 1) return;
		const touch = event.changedTouches[0];
		const scrolled =
			Math.abs((scrollerEl?.scrollTop ?? 0) - touchStartScrollTop) > 4;
		const verdict = resolveGesture({
			start: { x: touchStartX, y: touchStartY, time: touchStartTime },
			end: { x: touch.clientX, y: touch.clientY, time: Date.now() },
			viewportWidth: window.innerWidth,
			multiTouch: false,
			cancelled: false,
			excluded: false,
			longPressed: false,
			selectionActive: hasActiveSelection(),
			scrolled
		});
		if (verdict.type === 'swipe') {
			void navigateChapter(verdict.direction);
			return;
		}
		if (verdict.type === 'tap') {
			if (verdict.zone === 'center') {
				onToggleChrome();
			} else {
				scrollPage(verdict.zone === 'prev' ? -1 : 1);
			}
		}
	}

	export function scrollPage(direction: -1 | 1) {
		if (!scrollerEl) return;
		const target = pageScrollTarget(
			scrollerEl.scrollTop,
			scrollerEl.clientHeight,
			scrollerEl.scrollHeight,
			scrollerEl.clientHeight,
			direction
		);
		scrollerEl.scrollTo({ top: target, behavior: 'smooth' });
	}

	// ---------------------------------------------------------------------
	// Links

	function handleContentClick(event: MouseEvent) {
		const target = event.target instanceof Element ? event.target : null;
		const anchor = target?.closest('a[href]') as HTMLAnchorElement | null;
		if (!anchor || !articleEl?.contains(anchor)) return;
		const href = (anchor.getAttribute('href') ?? '').trim();
		if (!href) return;
		event.preventDefault();
		event.stopPropagation();
		if (href.startsWith('#')) {
			const id = href.slice(1);
			const fragmentTarget =
				articleEl.querySelector(`#${cssEscape(decodeFragment(id))}`) ??
				articleEl.querySelector(`[name="${decodeFragment(id).replace(/"/g, '\\"')}"]`);
			if (fragmentTarget instanceof HTMLElement) fragmentTarget.scrollIntoView({ block: 'start' });
			return;
		}
		if (/^https?:\/\//i.test(href)) {
			void api.openExternalUrl(href).catch(() => onNotice('无法打开外部链接'));
			return;
		}
		onNotice('暂不支持该链接');
	}

	function decodeFragment(value: string): string {
		try {
			return decodeURIComponent(value);
		} catch {
			return value;
		}
	}

	// ---------------------------------------------------------------------
	// Search

	function clearSearch() {
		if (articleEl) clearSearchHighlights(articleEl);
		searchHighlights = null;
		searchIndex = -1;
	}

	export function search(query: string): ChapterSearchResult {
		if (!articleEl) return { query, count: 0, index: -1 };
		searchHighlights = highlightChapterMatches(articleEl, query);
		searchIndex = searchHighlights.marks.length > 0 ? setCurrentMatch(searchHighlights.marks, 0, scrollerEl) : -1;
		return {
			query,
			count: searchHighlights.total,
			index: searchIndex
		};
	}

	export function searchNext(): ChapterSearchResult {
		if (!searchHighlights || searchHighlights.marks.length === 0) {
			return { query: '', count: 0, index: -1 };
		}
		searchIndex = setCurrentMatch(searchHighlights.marks, searchIndex + 1, scrollerEl);
		return { query: '', count: searchHighlights.total, index: searchIndex };
	}

	export function searchPrev(): ChapterSearchResult {
		if (!searchHighlights || searchHighlights.marks.length === 0) {
			return { query: '', count: 0, index: -1 };
		}
		searchIndex = setCurrentMatch(searchHighlights.marks, searchIndex - 1, scrollerEl);
		return { query: '', count: searchHighlights.total, index: searchIndex };
	}

	export function clearSearchState() {
		clearSearch();
	}

	export function isSearchTruncated(): boolean {
		return searchHighlights?.truncated ?? false;
	}

	// ---------------------------------------------------------------------
	// Focus-mode bridge + ReaderScreen accessors

	export function getFocusContext(): {
		articleEl: HTMLElement;
		scrollerEl: HTMLElement;
		contentKey: string;
	} | null {
		if (!articleEl || !scrollerEl || loading) return null;
		return { articleEl, scrollerEl, contentKey };
	}

	export function getMeta(): ReaderMeta | null {
		if (!chapterId) return null;
		return buildReaderMeta({
			title: isBook ? (book?.title ?? title) : title,
			chapterId,
			chapterTitle,
			chapterIndex,
			chapterCount,
			chapterRatio,
			overall,
			canPrev,
			canNext
		});
	}

	/** Heading TOC of the mounted chapter (single-file mode). */
	export function getToc(): { id: string; text: string; level: number }[] {
		return lastToc;
	}

	/** Scroll to one heading id of the mounted chapter (single-file TOC). */
	export function goToHeading(id: string): void {
		if (!articleEl) return;
		const target = articleEl.querySelector(`#${cssEscape(id)}`);
		if (target instanceof HTMLElement) target.scrollIntoView({ block: 'start' });
	}

	export function getChapterOrdinal(): number {
		return displayChapterNumber(chapterIndex);
	}

	// ---------------------------------------------------------------------
	// Lifecycle

	onMount(() => {
		document.addEventListener('visibilitychange', handleVisibilityChange);
		window.addEventListener('pagehide', handlePageHide);
		void initialize();
	});

	onDestroy(() => {
		destroyed = true;
		loadGeneration += 1;
		cancelPendingRenders();
		if (scrollFrame !== null) cancelAnimationFrame(scrollFrame);
		clearLongPress();
		if (saveTimer) clearTimeout(saveTimer);
		document.removeEventListener('visibilitychange', handleVisibilityChange);
		window.removeEventListener('pagehide', handlePageHide);
		// Drop large strings / DOM references so the chapter can be GC'd.
		articleChunks = [];
		pendingChunks = [];
		preloadCache = null;
		searchHighlights = null;
	});

	async function initialize() {
		if (isBook) {
			progressState = book?.progress ?? emptyProgressState();
			const start = resolveStartChapterIndex(chapters, progressState, book?.requestedChapterId);
			const restore: ScrollRestore | null = book?.requestedChapterId
				? null
				: { kind: 'ratio', value: resolveBookRestoreRatio(progressState.position) };
			await loadChapterAt(start, restore);
			return;
		}
		let restore: ScrollRestore | null = null;
		if (currentPath) {
			try {
				const state = await api.loadFileReadingState(currentPath);
				restore = resolveFileRestore(state);
			} catch {
				// No saved state — start at the top.
			}
		}
		await loadChapterAt(0, restore);
	}

	async function handleSeamClick() {
		if (!canNext) return;
		await navigateChapter(1);
	}
</script>

<div class="md-reader" data-testid="markdown-reader">
	<div class="md-scroll" bind:this={scrollerEl} onscroll={handleScroll} data-testid="reader-scroll">
		{#if loading}
			<div class="md-skeleton" aria-hidden="true" data-testid="reader-loading">
				<div class="md-skel-title"></div>
				<div class="md-skel"></div>
				<div class="md-skel"></div>
				<div class="md-skel short"></div>
				<div class="md-skel"></div>
				<div class="md-skel short"></div>
			</div>
		{:else if errorMessage}
			<div class="md-error" role="alert">
				<p class="md-error-msg">{errorMessage}</p>
				<button
					type="button"
					class="md-error-btn"
					onclick={() => void loadChapterAt(chapterIndex, null)}
				>重试</button>
			</div>
		{:else}
			<!-- svelte-ignore a11y_click_events_have_key_events -->
			<!-- svelte-ignore a11y_no_static_element_interactions -->
			<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
			<!-- The article is a touch surface: gestures are decided in JS from
			     raw touch coordinates (tap zone / swipe / long press), which
			     keyboard activation cannot express. -->
			<article
				class="md-article"
				bind:this={articleEl}
				onclick={handleContentClick}
				onloadcapture={handleResourceLoad}
				ontouchstart={handleTouchStart}
				ontouchmove={handleTouchMove}
				ontouchend={handleTouchEnd}
				ontouchcancel={handleTouchCancel}
			>
				{#each articleChunks as chunk, index (index)}{@html chunk}{/each}
			</article>
			{#if seamVisible}
				<button type="button" class="md-seam" data-testid="reader-chapter-seam" onclick={handleSeamClick}>
					{#if canNext}
						<span>下一篇 · {chapters[chapterIndex + 1]?.title ?? '下一章'}</span>
						<small>点击继续</small>
					{:else}
						<span>全书读完</span>
						<small>最后一章达到 95% 后记为已读</small>
					{/if}
				</button>
			{/if}
			<div class="md-end-spacer" aria-hidden="true"></div>
		{/if}
	</div>
</div>

<style>
	.md-reader {
		position: relative;
		height: 100%;
		min-height: 0;
		background: var(--bg);
		color: var(--text);
	}

	.md-scroll {
		height: 100%;
		overflow-y: auto;
		overflow-x: hidden;
		overscroll-behavior: contain;
		-webkit-overflow-scrolling: touch;
		scrollbar-width: none;
	}

	.md-scroll::-webkit-scrollbar {
		width: 0;
		height: 0;
	}

	.md-article {
		max-width: var(--article-max-width, 760px);
		margin: 0 auto;
		padding: var(--reader-article-top, 64px) 20px 24px;
		font-size: calc(17px * var(--font-scale, 1));
		line-height: var(--article-line-height, 1.8);
		font-family: var(--article-font-family, inherit);
		overflow-wrap: break-word;
		line-break: strict;
		/* No transition/filter on the content container: focus mode paints its
		   spotlight from a fixed overlay instead of restyling each block. */
	}

	/* Content-visibility skips offscreen layout for long chapters. The first
	   screenful is exempt so initial paint never waits on containment. */
	:global(.md-article > *:nth-child(n + 8)) {
		content-visibility: auto;
		contain-intrinsic-size: auto 600px;
	}
	:global(.md-article > pre:nth-child(n + 8)),
	:global(.md-article > table:nth-child(n + 8)),
	:global(.md-article > blockquote:nth-child(n + 8)) {
		contain-intrinsic-size: auto 12em;
	}

	.md-article :global(h1),
	.md-article :global(h2),
	.md-article :global(h3),
	.md-article :global(h4),
	.md-article :global(h5),
	.md-article :global(h6) {
		color: var(--heading);
		scroll-margin-top: 76px;
		line-height: 1.35;
	}
	.md-article :global(h1) {
		font-size: 1.6em;
		margin: 1.4em 0 0.7em;
	}
	.md-article :global(h2) {
		font-size: 1.35em;
		margin: 1.5em 0 0.6em;
		padding-bottom: 0.25em;
		border-bottom: 1px solid var(--hr);
	}
	.md-article :global(h3) {
		font-size: 1.2em;
		margin: 1.3em 0 0.5em;
	}
	.md-article :global(h4),
	.md-article :global(h5),
	.md-article :global(h6) {
		font-size: 1.05em;
		margin: 1.1em 0 0.4em;
	}
	.md-article :global(p) {
		margin: 0.9em 0;
	}
	.md-article :global(a) {
		color: var(--link);
		text-decoration: none;
		border-bottom: 1px solid transparent;
	}
	.md-article :global(strong) {
		color: var(--heading);
		font-weight: 600;
	}
	.md-article :global(blockquote) {
		margin: 1em 0;
		padding: 0.2em 0 0.2em 0.9em;
		border-left: 3px solid var(--blockquote-border);
		color: var(--blockquote-text);
		background: transparent;
	}
	.md-article :global(hr) {
		border: 0;
		border-top: 1px solid var(--hr);
		margin: 1.6em 0;
	}
	.md-article :global(ul),
	.md-article :global(ol) {
		padding-left: 1.4em;
		margin: 0.8em 0;
	}
	.md-article :global(li) {
		margin: 0.3em 0;
	}
	.md-article :global(li > input[type='checkbox']) {
		margin-right: 0.4em;
	}
	.md-article :global(img) {
		max-width: 100%;
		height: auto;
		display: block;
		margin: 1em auto;
		border-radius: 8px;
	}
	.md-article :global(figure) {
		margin: 1.2em 0;
	}
	.md-article :global(figcaption) {
		font-size: 0.85em;
		color: var(--text-faded);
		text-align: center;
	}
	.md-article :global(code) {
		font-family: 'Cascadia Code', Consolas, 'Noto Sans Mono', monospace;
		font-size: 0.88em;
		background: var(--code-bg);
		color: var(--code-text);
		padding: 0.1em 0.35em;
		border-radius: 4px;
	}
	.md-article :global(pre) {
		background: var(--code-bg);
		color: var(--code-text);
		border-radius: 10px;
		padding: 12px 14px;
		overflow-x: auto;
		-webkit-overflow-scrolling: touch;
		margin: 1em 0;
		font-size: 13px;
		line-height: 1.6;
	}
	.md-article :global(pre code) {
		background: transparent;
		padding: 0;
		font-size: inherit;
	}
	.md-article :global(table) {
		display: block;
		max-width: 100%;
		overflow-x: auto;
		-webkit-overflow-scrolling: touch;
		border-collapse: collapse;
		margin: 1em 0;
		font-size: 0.92em;
	}
	.md-article :global(th),
	.md-article :global(td) {
		border: 1px solid var(--table-border);
		padding: 6px 10px;
		white-space: nowrap;
	}
	.md-article :global(th) {
		background: var(--bg-secondary);
	}
	.md-article :global(tr:nth-child(even) td) {
		background: color-mix(in srgb, var(--table-stripe) 65%, transparent);
	}
	.md-article :global(mark) {
		background: var(--search-highlight);
		color: inherit;
		border-radius: 2px;
		padding: 1px 0;
	}
	.md-article :global(.reader-search-mark) {
		background: var(--search-highlight);
		color: inherit;
		border-radius: 2px;
		padding: 1px 0;
	}
	.md-article :global(.reader-search-current) {
		background: var(--link);
		color: var(--on-accent);
	}
	.md-article :global(.sr-only) {
		position: absolute;
		width: 1px;
		height: 1px;
		padding: 0;
		margin: -1px;
		overflow: hidden;
		clip: rect(0 0 0 0);
		white-space: nowrap;
		border: 0;
	}
	.md-article :global(.remote-blocked) {
		border: 1px dashed var(--text-faded);
		border-radius: 8px;
		padding: 18px;
		margin: 12px 0;
		text-align: center;
		font-size: 0.85em;
		color: var(--text-faded);
	}
	.md-article :global(.katex-display) {
		overflow-x: auto;
		overflow-y: hidden;
		padding: 0.2em 0;
	}
	.md-article :global(.katex) {
		font-size: 1.05em;
	}
	/* Podcast bilingual blocks: the original is a collapsible blockquote. */
	.md-article :global(blockquote.podcast-original) {
		border-left: 2px solid var(--hr);
		color: var(--text-faded);
		font-size: 0.94em;
		max-height: 0;
		overflow: hidden;
		opacity: 0;
		padding-top: 0;
		padding-bottom: 0;
		transition: max-height 0.24s ease, opacity 0.2s ease;
	}
	.md-article :global(blockquote.podcast-original.is-revealed) {
		max-height: 40em;
		opacity: 1;
		padding-top: 0.2em;
		padding-bottom: 0.2em;
	}
	.md-article :global(p.podcast-translation) {
		margin: 0.4em 0;
	}

	.md-skeleton {
		padding: 28px 20px;
		display: flex;
		flex-direction: column;
		gap: 16px;
	}
	.md-skel {
		height: 15px;
		border-radius: 6px;
		background: var(--bg-secondary);
		animation: mdPulse 1.4s ease-in-out infinite;
	}
	.md-skel-title {
		height: 24px;
		width: 46%;
		margin-bottom: 8px;
		border-radius: 8px;
		background: var(--bg-secondary);
		animation: mdPulse 1.4s ease-in-out infinite;
	}
	.md-skel.short {
		width: 68%;
	}
	@media (prefers-reduced-motion: reduce) {
		.md-skel,
		.md-skel-title {
			animation: none;
		}
	}
	@keyframes mdPulse {
		0%,
		100% {
			opacity: 1;
		}
		50% {
			opacity: 0.45;
		}
	}

	.md-error {
		padding: 18vh 24px 0;
		text-align: center;
	}
	.md-error-msg {
		font-size: 15px;
		line-height: 1.7;
		color: var(--text);
		margin-bottom: 18px;
	}
	.md-error-btn {
		border: 1px solid var(--hr);
		background: var(--bg-secondary);
		color: var(--text);
		font-size: 14px;
		min-height: 44px;
		padding: 8px 24px;
		border-radius: 12px;
	}

	.md-seam {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 2px;
		width: calc(100% - 40px);
		max-width: var(--article-max-width, 760px);
		margin: 8px auto 0;
		padding: 14px 16px;
		border: 1px solid var(--hr);
		border-radius: 14px;
		background: var(--bg-secondary);
		color: var(--link);
		font-size: 14px;
		min-height: 56px;
	}

	.md-seam small {
		color: var(--text-faded);
		font-size: 12px;
	}

	.md-end-spacer {
		height: calc(max(var(--ir-safe-bottom, 0px), env(safe-area-inset-bottom, 0px)) + 96px);
	}
</style>
