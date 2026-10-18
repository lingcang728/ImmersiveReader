<!--
	Android mobile reader screen.

	Owns the full-viewport reading surface: top chrome, bottom progress bar,
	sheets (TOC / search / bookmarks / reading settings), focus mode, Android
	back layering, and volume-key routing. Data/IPC only through
	`$lib/mobile/api.ts`; EPUB is mounted through the existing kernel via
	EpubReaderMobile (dynamic import), Markdown through MarkdownReader.

	Shell contract (fixed): props { target: ReaderTarget; onExit: () => void }.
	The shell owns history; it dispatches `ir:android-back` when the Android
	back gesture fires and only calls onExit after we report nothing left to
	close. Volume presses arrive as `ir:volume-key` ({ dir: -1 | 1 }).
-->
<script lang="ts">
	import { onDestroy, onMount, tick } from 'svelte';
	import * as api from '$lib/mobile/api';
	import type { BookDetail, ReaderTarget } from '$lib/mobile/contracts';
	import type { Bookmark, EpubNavItem, ReaderLocator, SearchHit } from '$lib/epub/types';
	import { makeLocator } from '$lib/epub/locator';
	import { sanitizeSearchSnippet } from '$lib/epub/sanitize';
	import { fontScale, readingFontFamily, readingLineHeight, readingWidth } from '$lib/stores/app';
	import ReaderChrome from './ReaderChrome.svelte';
	import MarkdownReader from './MarkdownReader.svelte';
	import EpubReaderMobile from './EpubReaderMobile.svelte';
	import FocusMode from './FocusMode.svelte';
	import TocSheet from './TocSheet.svelte';
	import SearchSheet from './SearchSheet.svelte';
	import BookmarksSheet from './BookmarksSheet.svelte';
	import ReadingSettingsSheet from './ReadingSettingsSheet.svelte';
	import {
		emptyMeta,
		type ChapterSearchResult,
		type EpubMeta,
		type ReaderMeta,
		type ReaderMode,
		type ReaderPanel,
		type SearchResultView,
		type TocEntryView
	} from './types';
	import { percentOf } from './progress';
	import {
		LONG_PRESS_MS,
		gestureExcluded,
		hasActiveSelection,
		resolveGesture
	} from './gestures';

	interface Props {
		target: ReaderTarget;
		onExit: () => void;
	}

	let { target, onExit }: Props = $props();

	type FocusContext = { articleEl: HTMLElement; scrollerEl: HTMLElement; contentKey: string };

	type MarkdownReaderApi = {
		flushProgress(): Promise<void>;
		navigateChapter(direction: -1 | 1): Promise<boolean>;
		goToChapterId(id: string): Promise<boolean>;
		goToHeading(id: string): void;
		scrollPage(direction: -1 | 1): void;
		search(query: string): ChapterSearchResult;
		searchNext(): ChapterSearchResult;
		searchPrev(): ChapterSearchResult;
		clearSearchState(): void;
		isSearchTruncated(): boolean;
		getFocusContext(): FocusContext | null;
		getMeta(): ReaderMeta | null;
		getToc(): { id: string; text: string; level: number }[];
	};

	type EpubReaderApi = {
		navigateChapterBy(direction: 1 | -1): void;
		scrollPage(direction: 1 | -1): void;
		goToChapterId(chapterId: string): void;
		goToSearchHit(hit: SearchHit): void;
		goToLocator(locator: ReaderLocator): void;
		bookmarkCurrent(): Promise<void>;
		flushLocator(): void;
		handleBack(): boolean;
		getFocusContext(): FocusContext | null;
		isReady(): boolean;
	};

	type FocusApi = {
		next(): void;
		prev(): void;
		exit(): void;
		isActive(): boolean;
	};

	let mode = $state<ReaderMode>('loading');
	let loadError = $state('');
	let bookDetail = $state<BookDetail | null>(null);
	let bookTitle = $state('');
	let meta = $state<ReaderMeta>(emptyMeta(''));
	let epubLocator = $state<ReaderLocator | null>(null);
	let fileToc = $state<{ id: string; text: string; level: number }[]>([]);

	let panel = $state<ReaderPanel>('');
	let chromeVisible = $state(true);
	let focusActive = $state(false);
	let focusContext = $state<FocusContext | null>(null);
	let reflowKey = $state(0);
	let toast = $state('');
	let exiting = false;

	let mdRef = $state<MarkdownReaderApi | null>(null);
	let epubRef = $state<EpubReaderApi | null>(null);
	let focusRef = $state<FocusApi | null>(null);

	// Search sheet state.
	let searchQuery = $state('');
	let searchBusy = $state(false);
	let searchStatus = $state('');
	let searchResults = $state<SearchResultView[]>([]);
	let searchIndex = $state(0);
	let searchTruncated = $state(false);
	let searchTimer: ReturnType<typeof setTimeout> | null = null;
	let lastHits: SearchHit[] = [];

	// Bookmarks.
	let bookmarks = $state<Bookmark[]>([]);
	let bookmarksLoading = $state(false);

	let hideTimer: ReturnType<typeof setTimeout> | null = null;
	let toastTimer: ReturnType<typeof setTimeout> | null = null;

	const bookInput = $derived(
		bookDetail && target.kind === 'book'
			? {
					bookId: bookDetail.manifest.bookId,
					title: bookDetail.manifest.title || target.title,
					chapters: bookDetail.manifest.chapters,
					progress: bookDetail.progress,
					requestedChapterId: target.chapterId
				}
			: null
	);

	const epubNav = $derived(flattenNav(bookDetail?.publication?.nav ?? []));

	const tocItems = $derived.by<TocEntryView[]>(() => {
		if (mode === 'epub') return epubNav;
		if (mode === 'markdown-book') {
			return (bookDetail?.manifest.chapters ?? []).map((chapter) => ({
				id: chapter.id,
				label: chapter.title,
				level: 1
			}));
		}
		return fileToc.map((item) => ({ id: item.id, label: item.text, level: item.level }));
	});

	const hasToc = $derived(tocItems.length > 0);
	const hasBookmarks = $derived(mode === 'epub');
	const hasMeta = $derived(meta.chapterId !== '');
	const showProgress = $derived(chromeVisible && hasMeta && mode !== 'error');
	/** Single-file chapters share the book title — do not print it twice. */
	const chromeChapterTitle = $derived(mode === 'markdown-file' ? '' : meta.chapterTitle);

	function flattenNav(items: EpubNavItem[], level = 1, out: TocEntryView[] = []): TocEntryView[] {
		for (const item of items) {
			if (item.chapterId) {
				out.push({ id: item.chapterId, label: item.title, level: Math.min(3, level) });
			}
			if (item.children?.length) flattenNav(item.children, level + 1, out);
		}
		return out;
	}

	// ---------------------------------------------------------------------
	// Target loading

	async function loadTarget() {
		try {
			if (target.kind === 'file') {
				bookTitle = target.title;
				mode = 'markdown-file';
				return;
			}
			const detail = await api.openBook(target.bookId);
			bookDetail = detail;
			bookTitle = detail.manifest.title || target.title;
			if (detail.publication?.format === 'epub') {
				const locator = target.chapterId
					? makeLocator(detail.manifest.bookId, target.chapterId, { kind: 'ratio' }, 0)
					: await api.getReaderLocator(detail.manifest.bookId).catch(() => null);
				epubLocator = locator;
				mode = 'epub';
				return;
			}
			mode = 'markdown-book';
		} catch (error) {
			console.error('[mobile-reader] open target failed', error);
			loadError = api.errorText(error);
			mode = 'error';
		}
	}

	// ---------------------------------------------------------------------
	// Chrome / layering

	function clearHideTimer() {
		if (hideTimer) {
			clearTimeout(hideTimer);
			hideTimer = null;
		}
	}

	function scheduleAutoHide() {
		clearHideTimer();
		if (panel || focusActive || !chromeVisible) return;
		hideTimer = setTimeout(() => {
			hideTimer = null;
			if (!panel && !focusActive) chromeVisible = false;
		}, 3200);
	}

	function toggleChrome() {
		if (focusActive) return;
		chromeVisible = !chromeVisible;
		if (chromeVisible) scheduleAutoHide();
		else clearHideTimer();
	}

	function showToast(message: string) {
		toast = message;
		if (toastTimer) clearTimeout(toastTimer);
		toastTimer = setTimeout(() => {
			toast = '';
			toastTimer = null;
		}, 2600);
	}

	function openPanel(next: Exclude<ReaderPanel, ''>) {
		if (next === 'toc' && !hasToc) return;
		panel = panel === next ? '' : next;
		if (panel) {
			chromeVisible = true;
			clearHideTimer();
		} else {
			scheduleAutoHide();
		}
	}

	function closePanel() {
		panel = '';
		if (searchTimer) {
			clearTimeout(searchTimer);
			searchTimer = null;
		}
		// A closed search starts fresh next time: stale input without live
		// highlights is worse than an empty field.
		searchQuery = '';
		searchResults = [];
		searchStatus = '';
		searchBusy = false;
		searchIndex = 0;
		searchTruncated = false;
		lastHits = [];
		if (mode !== 'epub' && mode !== 'error' && mode !== 'loading') {
			mdRef?.clearSearchState();
		}
		scheduleAutoHide();
	}

	// Android back layering: sheet → focus mode → leave reader.
	export function handleBack(): boolean {
		if (panel) {
			closePanel();
			return true;
		}
		if (focusActive) {
			focusRef?.exit();
			return true;
		}
		if (epubRef?.handleBack()) return true;
		return false;
	}

	async function exitReader() {
		if (exiting) return;
		exiting = true;
		try {
			if (mode === 'markdown-book' || mode === 'markdown-file') {
				await mdRef?.flushProgress();
			} else if (mode === 'epub') {
				epubRef?.flushLocator();
			}
		} catch {
			// Exit must never block on persistence.
		}
		onExit();
	}

	function handleAndroidBack() {
		if (handleBack()) return;
		void exitReader();
	}

	// ---------------------------------------------------------------------
	// Meta / focus wiring

	function handleMarkdownMeta(next: ReaderMeta) {
		const chapterChanged = next.chapterId !== meta.chapterId;
		meta = next;
		if (next.title) bookTitle = next.title;
		if (mode === 'markdown-file') {
			const toc = mdRef?.getToc() ?? [];
			if (toc.length > 0) fileToc = toc;
		}
		if (chapterChanged) {
			void tick().then(() => {
				syncFocusContext();
				if (panel === 'search' && searchQuery.trim()) void runSearch(searchQuery);
			});
		}
	}

	function handleEpubMeta(next: EpubMeta) {
		const chapterChanged = next.chapterId !== meta.chapterId;
		meta = {
			...emptyMeta(bookTitle),
			...next,
			title: bookTitle
		};
		if (chapterChanged) {
			void tick().then(() => {
				syncFocusContext();
				if (panel === 'search' && searchQuery.trim()) void runSearch(searchQuery);
				if (panel === 'bookmarks') void refreshBookmarks();
			});
		}
	}

	function syncFocusContext() {
		const context = mdRef?.getFocusContext() ?? epubRef?.getFocusContext() ?? null;
		if (context) focusContext = context;
	}

	function toggleFocus() {
		if (focusActive) {
			focusRef?.exit();
			return;
		}
		const context = mdRef?.getFocusContext() ?? epubRef?.getFocusContext() ?? null;
		if (!context) {
			showToast('正文尚未就绪，请稍候');
			return;
		}
		focusContext = context;
		chromeVisible = false;
		clearHideTimer();
		focusActive = true;
	}

	function handleFocusExit() {
		focusActive = false;
		chromeVisible = true;
		scheduleAutoHide();
	}

	async function handleFocusBoundary(direction: -1 | 1) {
		if (mode === 'epub') {
			epubRef?.navigateChapterBy(direction);
		} else {
			await mdRef?.navigateChapter(direction);
		}
	}

	// ---------------------------------------------------------------------
	// Chapter navigation (progress bar)

	async function previousChapter() {
		if (!meta.canPrev) return;
		if (mode === 'epub') epubRef?.navigateChapterBy(-1);
		else await mdRef?.navigateChapter(-1);
	}

	async function nextChapter() {
		if (!meta.canNext) return;
		if (mode === 'epub') epubRef?.navigateChapterBy(1);
		else await mdRef?.navigateChapter(1);
	}

	// ---------------------------------------------------------------------
	// TOC

	function handleTocSelect(id: string) {
		closePanel();
		if (mode === 'epub') {
			epubRef?.goToChapterId(id);
			return;
		}
		if (mode === 'markdown-book') {
			void mdRef?.goToChapterId(id);
			return;
		}
		mdRef?.goToHeading(id);
	}

	// ---------------------------------------------------------------------
	// Search

	function handleSearchQuery(query: string) {
		searchQuery = query;
		if (searchTimer) {
			clearTimeout(searchTimer);
			searchTimer = null;
		}
		if (!query.trim()) {
			searchResults = [];
			searchStatus = '';
			searchBusy = false;
			searchIndex = 0;
			mdRef?.clearSearchState();
			return;
		}
		searchTimer = setTimeout(() => {
			searchTimer = null;
			void runSearch(query);
		}, mode === 'epub' ? 360 : 200);
	}

	async function runSearch(query: string) {
		if (mode === 'epub') {
			if (!bookDetail) return;
			searchBusy = true;
			try {
				const hits = await api.searchBook(bookDetail.manifest.bookId, query);
				if (query !== searchQuery) return;
				lastHits = hits;
				searchResults = hits.map((hit, index) => ({
					id: `${hit.chapterId}:${index}`,
					title: hit.title || '结果',
					snippetHtml: sanitizeSearchSnippet(hit.snippet),
					ordinal: index + 1
				}));
				searchIndex = 0;
				searchStatus = hits.length > 0 ? `${hits.length} 条结果` : '没有找到匹配内容';
			} catch (error) {
				searchResults = [];
				searchStatus = api.errorText(error);
			} finally {
				searchBusy = false;
			}
			return;
		}
		const result = mdRef?.search(query);
		if (query !== searchQuery) return;
		searchTruncated = mdRef?.isSearchTruncated() ?? false;
		searchResults = [];
		searchStatus = result && result.count > 0
			? `第 ${result.index + 1} / ${result.count} 处`
			: '没有找到匹配内容';
	}

	function searchPrev() {
		if (mode === 'epub') {
			if (searchResults.length === 0) return;
			searchIndex = (searchIndex - 1 + searchResults.length) % searchResults.length;
			return;
		}
		const result = mdRef?.searchPrev();
		if (result) {
			searchStatus = result.count > 0 ? `第 ${result.index + 1} / ${result.count} 处` : '没有找到匹配内容';
		}
	}

	function searchNext() {
		if (mode === 'epub') {
			if (searchResults.length === 0) return;
			searchIndex = (searchIndex + 1) % searchResults.length;
			return;
		}
		const result = mdRef?.searchNext();
		if (result) {
			searchStatus = result.count > 0 ? `第 ${result.index + 1} / ${result.count} 处` : '没有找到匹配内容';
		}
	}

	function selectSearchResult(index: number) {
		if (mode !== 'epub') return;
		const hit = lastHits[index];
		if (!hit) return;
		searchIndex = index;
		closePanel();
		epubRef?.goToSearchHit(hit);
	}

	// ---------------------------------------------------------------------
	// Bookmarks (EPUB)

	async function refreshBookmarks() {
		if (!bookDetail) return;
		bookmarksLoading = true;
		try {
			bookmarks = await api.listBookmarks(bookDetail.manifest.bookId);
		} catch (error) {
			console.warn('[mobile-reader] list_bookmarks failed', error);
			bookmarks = [];
		} finally {
			bookmarksLoading = false;
		}
	}

	async function addBookmark() {
		if (!bookDetail || !epubRef) return;
		try {
			await epubRef.bookmarkCurrent();
			await refreshBookmarks();
			showToast('已添加书签');
		} catch (error) {
			showToast(api.errorText(error));
		}
	}

	async function deleteBookmark(bookmarkId: string) {
		if (!bookDetail) return;
		try {
			await api.removeBookmark(bookDetail.manifest.bookId, bookmarkId);
			await refreshBookmarks();
		} catch (error) {
			showToast(api.errorText(error));
		}
	}

	function jumpBookmark(bookmark: Bookmark) {
		closePanel();
		epubRef?.goToLocator(bookmark.locator);
	}

	function openBookmarks() {
		openPanel('bookmarks');
		void refreshBookmarks();
	}

	// ---------------------------------------------------------------------
	// Volume keys (shell owns capture enable/disable)

	function handleVolumeKey(event: Event) {
		const detail = (event as CustomEvent<{ dir?: number }>).detail;
		const direction = detail?.dir;
		if (direction !== -1 && direction !== 1) return;
		if (panel) return;
		if (focusActive) {
			if (direction === -1) focusRef?.prev();
			else focusRef?.next();
			return;
		}
		if (mode === 'epub') {
			epubRef?.scrollPage(direction);
		} else if (mode === 'markdown-book' || mode === 'markdown-file') {
			mdRef?.scrollPage(direction);
		}
	}

	// ---------------------------------------------------------------------
	// EPUB touch surface: the EPUB kernel owns swipe-to-chapter and links, so
	// the shell layer only adds the gestures it does not implement — center tap
	// toggles chrome, edge taps page.

	let epubTouchStart: { x: number; y: number; time: number } | null = null;
	let epubTouchMulti = false;
	let epubTouchExcluded = false;
	let epubTouchLongPress = false;
	let epubTouchMoved = false;
	let epubLongPressTimer: ReturnType<typeof setTimeout> | null = null;

	function clearEpubLongPress() {
		if (epubLongPressTimer) {
			clearTimeout(epubLongPressTimer);
			epubLongPressTimer = null;
		}
	}

	function handleEpubTouchStart(event: TouchEvent) {
		if (mode !== 'epub' || focusActive || panel) return;
		if (event.touches.length !== 1) {
			epubTouchMulti = true;
			clearEpubLongPress();
			return;
		}
		const touch = event.touches[0];
		epubTouchStart = { x: touch.clientX, y: touch.clientY, time: Date.now() };
		epubTouchMulti = false;
		epubTouchLongPress = false;
		epubTouchMoved = false;
		epubTouchExcluded = gestureExcluded(event.target, event.currentTarget as HTMLElement);
		clearEpubLongPress();
		if (!epubTouchExcluded) {
			epubLongPressTimer = setTimeout(() => {
				epubLongPressTimer = null;
				epubTouchLongPress = true;
			}, LONG_PRESS_MS);
		}
	}

	function handleEpubTouchMove(event: TouchEvent) {
		if (!epubTouchStart || epubTouchLongPress) return;
		const touch = event.touches[0];
		if (!touch) return;
		if (
			Math.abs(touch.clientX - epubTouchStart.x) > 10 ||
			Math.abs(touch.clientY - epubTouchStart.y) > 10
		) {
			epubTouchMoved = true;
			clearEpubLongPress();
		}
	}

	function handleEpubTouchCancel() {
		clearEpubLongPress();
		epubTouchStart = null;
		epubTouchMulti = false;
		epubTouchExcluded = false;
		epubTouchLongPress = false;
		epubTouchMoved = false;
	}

	function handleEpubTouchEnd(event: TouchEvent) {
		clearEpubLongPress();
		const start = epubTouchStart;
		const blocked =
			epubTouchMulti || epubTouchExcluded || epubTouchLongPress || epubTouchMoved;
		epubTouchStart = null;
		epubTouchMulti = false;
		epubTouchExcluded = false;
		epubTouchLongPress = false;
		epubTouchMoved = false;
		if (!start || blocked || mode !== 'epub' || focusActive || panel) return;
		if (event.changedTouches.length !== 1) return;
		const touch = event.changedTouches[0];
		const verdict = resolveGesture({
			start,
			end: { x: touch.clientX, y: touch.clientY, time: Date.now() },
			viewportWidth: window.innerWidth,
			multiTouch: false,
			cancelled: false,
			excluded: false,
			selectionActive: hasActiveSelection()
		});
		// Swipes are the EPUB kernel's own chapter navigation.
		if (verdict.type !== 'tap') return;
		if (verdict.zone === 'center') toggleChrome();
		else epubRef?.scrollPage(verdict.zone === 'prev' ? -1 : 1);
	}

	// ---------------------------------------------------------------------
	// Lifecycle

	/** Keep the EPUB locator on the last position when the app is frozen. */
	function handleReaderPageHide() {
		if (mode === 'epub') epubRef?.flushLocator();
	}

	function handleReaderVisibility() {
		if (typeof document === 'undefined' || document.visibilityState !== 'hidden') return;
		if (mode === 'epub') epubRef?.flushLocator();
	}

	onMount(() => {
		bookTitle = target.title;
		meta = emptyMeta(target.title);
		const unsubscribeScale = fontScale.subscribe(() => (reflowKey += 1));
		const unsubscribeLine = readingLineHeight.subscribe(() => (reflowKey += 1));
		const unsubscribeWidth = readingWidth.subscribe(() => (reflowKey += 1));
		const unsubscribeFamily = readingFontFamily.subscribe(() => (reflowKey += 1));
		window.addEventListener('ir:android-back', handleAndroidBack);
		window.addEventListener('ir:volume-key', handleVolumeKey);
		window.addEventListener('keydown', handleKeydown);
		window.addEventListener('pagehide', handleReaderPageHide);
		document.addEventListener('visibilitychange', handleReaderVisibility);
		void loadTarget().then(() => {
			chromeVisible = true;
			scheduleAutoHide();
			void tick().then(syncFocusContext);
		});
		return () => {
			unsubscribeScale();
			unsubscribeLine();
			unsubscribeWidth();
			unsubscribeFamily();
			window.removeEventListener('ir:android-back', handleAndroidBack);
			window.removeEventListener('ir:volume-key', handleVolumeKey);
			window.removeEventListener('keydown', handleKeydown);
			window.removeEventListener('pagehide', handleReaderPageHide);
			document.removeEventListener('visibilitychange', handleReaderVisibility);
		};
	});

	onDestroy(() => {
		clearHideTimer();
		if (toastTimer) clearTimeout(toastTimer);
		if (searchTimer) clearTimeout(searchTimer);
		focusContext = null;
	});

	function handleKeydown(event: KeyboardEvent) {
		if (event.key === 'Escape') {
			if (handleBack()) {
				event.preventDefault();
				return;
			}
			void exitReader();
		}
	}
</script>

<div class="reader-screen" data-testid="reader-screen">
	<ReaderChrome
		visible={chromeVisible}
		title={bookTitle}
		chapterTitle={chromeChapterTitle}
		hasToc={hasToc}
		hasBookmarks={hasBookmarks}
		onBack={handleAndroidBack}
		onToc={() => openPanel('toc')}
		onSearch={() => openPanel('search')}
		onBookmarks={openBookmarks}
		onMore={() => openPanel('settings')}
	/>

	<main class="reader-body" data-testid="reader-content">
		{#if mode === 'loading'}
			<div class="reader-skeleton" data-testid="reader-loading" aria-hidden="true">
				<div class="skel-title"></div>
				<div class="skel"></div>
				<div class="skel"></div>
				<div class="skel short"></div>
				<div class="skel"></div>
			</div>
		{:else if mode === 'error'}
			<div class="reader-error" role="alert">
				<p class="reader-error-msg">{loadError || '无法打开这本书'}</p>
				<div class="reader-error-actions">
					<button type="button" class="reader-error-btn" onclick={() => { mode = 'loading'; void loadTarget(); }}>重试</button>
					<button type="button" class="reader-error-btn" onclick={handleAndroidBack}>返回书架</button>
				</div>
			</div>
		{:else if mode === 'epub' && bookDetail?.publication}
			<!-- svelte-ignore a11y_no_static_element_interactions -->
			<!-- Touch shell for the EPUB kernel: taps toggle chrome / page; swipes
			     stay with the kernel. -->
			<div
				class="epub-host"
				ontouchstart={handleEpubTouchStart}
				ontouchmove={handleEpubTouchMove}
				ontouchend={handleEpubTouchEnd}
				ontouchcancel={handleEpubTouchCancel}
			>
				<EpubReaderMobile
					bind:this={epubRef}
					bookId={bookDetail.manifest.bookId}
					bookDir={bookDetail.bookDir}
					publication={bookDetail.publication}
					initialLocator={epubLocator}
					onExit={exitReader}
					onLocator={() => {}}
					onMeta={handleEpubMeta}
				/>
			</div>
		{:else if mode === 'markdown-book'}
			<MarkdownReader
				bind:this={mdRef}
				book={bookInput}
				title={bookTitle}
				gesturesEnabled={!focusActive}
				onMeta={handleMarkdownMeta}
				onToggleChrome={toggleChrome}
				onNotice={showToast}
			/>
		{:else if mode === 'markdown-file'}
			<MarkdownReader
				bind:this={mdRef}
				book={null}
				filePath={target.kind === 'file' ? target.path : ''}
				title={bookTitle}
				gesturesEnabled={!focusActive}
				onMeta={handleMarkdownMeta}
				onToggleChrome={toggleChrome}
				onNotice={showToast}
			/>
		{/if}
	</main>

	{#if showProgress}
		<footer class="reader-progress" data-testid="reader-progress">
			<div class="progress-track">
				<div class="progress-fill" style:width="{percentOf(meta.chapterRatio)}%"></div>
			</div>
			<div class="progress-row">
				<button
					type="button"
					class="progress-chapter-btn"
					aria-label="上一章"
					data-testid="reader-prev-chapter"
					disabled={!meta.canPrev}
					onclick={previousChapter}
				>
					<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M15 6l-6 6 6 6" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" /></svg>
				</button>
				<span class="progress-label" data-testid="reader-progress-label">
					{#if mode === 'epub' || mode === 'markdown-book'}
						第 {meta.chapterIndex + 1}/{meta.chapterCount} 章 · 全书 {percentOf(meta.overall)}%
					{:else}
						全书 {percentOf(meta.overall)}%
					{/if}
				</span>
				<button
					type="button"
					class="progress-chapter-btn"
					aria-label="下一章"
					data-testid="reader-next-chapter"
					disabled={!meta.canNext}
					onclick={nextChapter}
				>
					<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M9 6l6 6-6 6" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" /></svg>
				</button>
			</div>
		</footer>
	{/if}

	{#if toast}
		<div class="reader-toast" role="status">{toast}</div>
	{/if}

	<FocusMode
		bind:this={focusRef}
		active={focusActive}
		articleEl={focusContext?.articleEl ?? null}
		scrollerEl={focusContext?.scrollerEl ?? null}
		contentKey={focusContext?.contentKey ?? ''}
		reflowKey={reflowKey}
		onExit={handleFocusExit}
		onBoundary={handleFocusBoundary}
	/>

	<TocSheet
		open={panel === 'toc'}
		items={tocItems}
		activeId={mode === 'markdown-file' ? '' : meta.chapterId}
		onSelect={handleTocSelect}
		onClose={closePanel}
	/>

	<SearchSheet
		open={panel === 'search'}
		mode={mode === 'epub' ? 'epub' : 'markdown'}
		query={searchQuery}
		busy={searchBusy}
		statusText={searchStatus}
		results={searchResults}
		activeIndex={searchIndex}
		truncated={searchTruncated}
		onQueryChange={handleSearchQuery}
		onPrev={searchPrev}
		onNext={searchNext}
		onSelect={selectSearchResult}
		onClose={closePanel}
	/>

	{#if mode === 'epub'}
		<BookmarksSheet
			open={panel === 'bookmarks'}
			{bookmarks}
			loading={bookmarksLoading}
			canAdd={hasMeta}
			onAdd={() => void addBookmark()}
			onJump={jumpBookmark}
			onRemove={(bookmarkId) => void deleteBookmark(bookmarkId)}
			onClose={closePanel}
		/>
	{/if}

	<ReadingSettingsSheet
		open={panel === 'settings'}
		mode={mode === 'epub' ? 'epub' : mode === 'markdown-file' ? 'file' : 'markdown'}
		focusActive={focusActive}
		onToggleFocus={toggleFocus}
		onClose={closePanel}
	/>
</div>

<style>
	.reader-screen {
		position: fixed;
		inset: 0;
		z-index: 40;
		display: flex;
		flex-direction: column;
		background: var(--bg);
		color: var(--text);
		overscroll-behavior: none;
		/* Floating chrome reserves this much top space in the reading content. */
		--reader-article-top: calc(max(var(--ir-safe-top, 0px), env(safe-area-inset-top, 0px)) + 64px);
	}

	.reader-body {
		position: absolute;
		inset: 0;
		min-height: 0;
	}

	.epub-host {
		height: 100%;
	}

	.reader-skeleton {
		display: flex;
		flex-direction: column;
		gap: 16px;
		padding: 84px 20px 0;
	}

	.skel,
	.skel-title {
		height: 15px;
		border-radius: 6px;
		background: var(--bg-secondary);
		animation: readerPulse 1.4s ease-in-out infinite;
	}

	.skel-title {
		height: 24px;
		width: 46%;
		margin-bottom: 10px;
	}

	.skel.short {
		width: 68%;
	}

	.reader-error {
		padding: 18vh 24px 0;
		text-align: center;
	}

	.reader-error-msg {
		font-size: 15px;
		line-height: 1.7;
		margin-bottom: 20px;
	}

	.reader-error-actions {
		display: flex;
		justify-content: center;
		gap: 12px;
	}

	.reader-error-btn {
		min-height: 44px;
		padding: 8px 22px;
		border: 1px solid var(--hr);
		border-radius: 12px;
		background: var(--bg-secondary);
		color: var(--text);
		font-size: 14px;
	}

	.reader-progress {
		position: fixed;
		left: 0;
		right: 0;
		bottom: 0;
		z-index: 50;
		padding: 8px 16px calc(max(var(--ir-safe-bottom, 0px), env(safe-area-inset-bottom, 0px)) + 10px);
		background: var(--bg);
		border-top: 1px solid var(--hr);
	}

	.progress-track {
		height: 4px;
		border-radius: 999px;
		background: var(--bg-secondary);
		overflow: hidden;
	}

	.progress-fill {
		height: 100%;
		background: var(--link);
		border-radius: 999px;
		transition: width 140ms linear;
	}

	.progress-row {
		display: flex;
		align-items: center;
		gap: 6px;
		padding-top: 2px;
	}

	.progress-chapter-btn {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 44px;
		height: 44px;
		border: 0;
		border-radius: 12px;
		background: transparent;
		color: var(--text);
	}

	.progress-chapter-btn:disabled {
		opacity: 0.3;
	}

	.progress-chapter-btn svg {
		width: 20px;
		height: 20px;
	}

	.progress-label {
		flex: 1;
		text-align: center;
		font-size: 12.5px;
		color: var(--text-secondary);
		font-variant-numeric: tabular-nums;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}

	.reader-toast {
		position: fixed;
		left: 50%;
		bottom: calc(max(var(--ir-safe-bottom, 0px), env(safe-area-inset-bottom, 0px)) + 108px);
		transform: translateX(-50%);
		z-index: 90;
		max-width: calc(100vw - 48px);
		padding: 9px 18px;
		border-radius: 999px;
		background: var(--bg-secondary);
		border: 1px solid var(--hr);
		color: var(--text);
		font-size: 12.5px;
		box-shadow: 0 8px 24px rgba(0, 0, 0, 0.24);
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}

	@keyframes readerPulse {
		0%,
		100% {
			opacity: 1;
		}
		50% {
			opacity: 0.45;
		}
	}

	@media (prefers-reduced-motion: reduce) {
		.skel,
		.skel-title {
			animation: none;
		}
		.progress-fill {
			transition: none;
		}
	}
</style>
