<script lang="ts">
	// EPUB 阅读主界面 — 纵滚、按 spine 单章渲染。
	//
	// Props（外部契约，勿改）:
	//   bookId          manifest.bookId
	//   bookDir         书籍目录绝对路径（BookDetail.bookDir）
	//   publication     解析后的 publication.json（format === "epub"）
	//   isMobile        手机布局/行为开关
	//   initialLocator  续读位置（null → 从 spine[0] 开始）
	// 回调:
	//   onExit()                 返回书架
	//   onLocator(locator)       节流后的 ReaderLocator 上报（滚动 ~2s、换章、退出时）
	import { onDestroy, onMount, tick } from "svelte";
	import { convertFileSrc } from "@tauri-apps/api/core";
	import { openUrl } from "@tauri-apps/plugin-opener";
	import EpubToc from "$lib/components/epub/EpubToc.svelte";
	import EpubSearch from "$lib/components/epub/EpubSearch.svelte";
	import EpubBookmarks from "$lib/components/epub/EpubBookmarks.svelte";
	import {
		getReadableChapter,
		saveReaderLocator,
		listBookmarks,
		addBookmark,
		removeBookmark,
	} from "./ipc";
	import { describeUnsupportedFlag, reportEpubError } from "./errors";
	import {
		markdownFallbackHtml,
		sanitizeChapterHtml,
		snippetQueryText,
		textExcerpt,
	} from "./sanitize";
	import {
		buildChapterHrefMap,
		hasUriScheme,
		isExternalHttpUrl,
		isNativeAbsolutePath,
		isProtocolRelative,
		joinNativePath,
		normalizeBookPath,
		resolveHrefToChapterId,
		splitHref,
	} from "./paths";
	import {
		clearFocusLite,
		excerptAtAnchor,
		findTextBlock,
		firstVisibleBlockAnchor,
		postProcessChapterDom,
		restoreLocatorScroll,
		scrollToFragment,
		swipeExcluded,
		updateFocusLite,
	} from "./dom";
	import {
		makeLocator,
		normalizeReaderLocator,
		ratioFromScroll,
		sameLocator,
	} from "./locator";
	import { throttled } from "./timing";
	import type {
		Bookmark,
		EpubNavItem,
		Publication,
		ReadableChapter,
		ReaderLocator,
		SearchHit,
	} from "./types";

	export let bookId: string;
	export let bookDir: string;
	export let publication: Publication;
	export let isMobile: boolean = false;
	export let initialLocator: ReaderLocator | null = null;
	export let onExit: () => void = () => {};
	export let onLocator: (locator: ReaderLocator) => void = () => {};

	type Panel = "" | "toc" | "search" | "bookmarks" | "type";

	// ---------- state ----------
	let scrollerEl: HTMLElement | null = null;
	let articleEl: HTMLElement | null = null;
	let chapter: ReadableChapter | null = null;
	let chapterHtml = "";
	let loading = true;
	let errorMsg = "";
	let errorChapterId = "";
	let panel: Panel = "";
	let focusLite = false;
	let fontPct = loadFontPct();
	let noticeDismissed = false;
	let transientNotice = "";
	let transientTimer: ReturnType<typeof setTimeout> | null = null;

	let bookmarks: Bookmark[] = [];
	let bookmarksLoading = false;

	// currentLocator mirrors the freshest sampled position; lastEmitted guards
	// redundant IPC/callback writes.
	let currentLocator: ReaderLocator | null = null;
	let lastEmitted: ReaderLocator | null = null;
	let pendingTextTarget = "";
	let loadSeq = 0;
	let destroyed = false;

	const hrefMap = buildChapterHrefMap(publication);
	const navTitle = new Map<string, string>();
	flattenNavTitles(publication.nav ?? [], navTitle);

	function flattenNavTitles(items: EpubNavItem[], out: Map<string, string>) {
		for (const item of items) {
			if (item.chapterId) out.set(item.chapterId, item.title);
			if (item.children?.length) flattenNavTitles(item.children, out);
		}
	}

	// ---------- derived ----------
	$: spineIndex = chapter ? publication.spine.indexOf(chapter.chapterId) : -1;
	$: spineCount = publication.spine.length;
	$: progressPct =
		chapter && spineCount > 0 && spineIndex >= 0
			? Math.min(
					100,
					Math.round(
						((spineIndex + (currentLocator?.ratio ?? 0)) / spineCount) * 100,
					),
				)
			: currentLocator
				? Math.round((currentLocator.ratio ?? 0) * 100)
				: 0;
	$: prevTitle = chapter?.prevChapterId
		? (navTitle.get(chapter.prevChapterId) ?? "上一章")
		: "";
	$: nextTitle = chapter?.nextChapterId
		? (navTitle.get(chapter.nextChapterId) ?? "下一章")
		: "";
	$: unsupportedLabels = (publication.unsupported ?? []).map(describeUnsupportedFlag);
	$: showUnsupported = !noticeDismissed && unsupportedLabels.length > 0;

	// ---------- font size ----------
	const FONT_PCT_KEY = "ir-epub-font-pct";
	const FONT_PCT_MIN = 80;
	const FONT_PCT_MAX = 160;

	function clampFontPct(value: number): number {
		if (!Number.isFinite(value)) return 100;
		return Math.min(FONT_PCT_MAX, Math.max(FONT_PCT_MIN, Math.round(value)));
	}

	function loadFontPct(): number {
		try {
			const raw =
				typeof localStorage !== "undefined" ? localStorage.getItem(FONT_PCT_KEY) : null;
			if (raw !== null) return clampFontPct(Number.parseFloat(raw));
		} catch {
			// localStorage disabled
		}
		return 100;
	}

	function persistFontPct() {
		try {
			localStorage.setItem(FONT_PCT_KEY, String(fontPct));
		} catch {
			// localStorage disabled
		}
	}

	async function setFontPct(value: number) {
		const anchorSnapshot = currentLocator;
		fontPct = clampFontPct(value);
		persistFontPct();
		// Reflow shifts offsets — re-anchor at the same element/ratio.
		await tick();
		if (articleEl && scrollerEl && anchorSnapshot) {
			restoreLocatorScroll(articleEl, scrollerEl, anchorSnapshot);
			sampleScroll();
			emitLocatorNow();
		}
	}

	// ---------- chapter loading ----------
	function chapterBaseDir(ch: ReadableChapter): string {
		const rel = normalizeBookPath(ch.resourceDir ?? "");
		return rel ? joinNativePath(bookDir, rel) : bookDir;
	}

	async function loadChapter(
		chapterId: string,
		restore: ReaderLocator | null = null,
		fragment = "",
	) {
		const seq = ++loadSeq;
		loading = true;
		errorMsg = "";
		try {
			const ch = await getReadableChapter(bookId, chapterId);
			if (seq !== loadSeq || destroyed) return;
			const html =
				ch.format === "markdown"
					? markdownFallbackHtml(ch.content)
					: await sanitizeChapterHtml(ch.content);
			if (seq !== loadSeq || destroyed) return;
			chapter = ch;
			chapterHtml = html;
			loading = false;
			await tick();
			if (seq !== loadSeq || destroyed || !articleEl || !scrollerEl) return;
			postProcessChapterDom(articleEl, chapterBaseDir(ch), convertFileSrc);

			if (restore) {
				restoreLocatorScroll(articleEl, scrollerEl, restore);
			} else if (fragment) {
				if (!scrollToFragment(articleEl, fragment)) scrollerEl.scrollTop = 0;
			} else {
				scrollerEl.scrollTop = 0;
			}
			// 搜索结果跳转：locator 缺失时退化为正文文本定位。
			if (pendingTextTarget) {
				const hit = findTextBlock(articleEl, pendingTextTarget);
				pendingTextTarget = "";
				if (hit) hit.scrollIntoView({ block: "start" });
			}
			if (focusLite) {
				articleEl.classList.add("focus-lite");
				updateFocusLite(articleEl, scrollerEl);
			}
			sampleScroll();
			emitLocatorNow();
		} catch (error) {
			if (seq !== loadSeq || destroyed) return;
			errorMsg = reportEpubError("epub-chapter", error);
			errorChapterId = chapterId;
			chapter = null;
			chapterHtml = "";
			loading = false;
		}
	}

	function navigateChapter(direction: 1 | -1) {
		const target =
			direction > 0 ? chapter?.nextChapterId : chapter?.prevChapterId;
		if (!target) return;
		void loadChapter(target);
	}

	function goToChapter(chapterId: string, fragment = "") {
		if (!chapterId) return;
		if (chapter?.chapterId === chapterId) {
			if (fragment && articleEl) {
				scrollToFragment(articleEl, fragment);
			} else if (scrollerEl) {
				scrollerEl.scrollTo({ top: 0 });
			}
			sampleScroll();
			emitLocatorNow();
			return;
		}
		void loadChapter(chapterId, null, fragment);
	}

	// ---------- locator sampling / emitting ----------
	function sampleScroll() {
		if (destroyed || !scrollerEl || !articleEl || !chapter) return;
		const ratio = ratioFromScroll(
			scrollerEl.scrollTop,
			scrollerEl.scrollHeight,
			scrollerEl.clientHeight,
		);
		const anchor = firstVisibleBlockAnchor(articleEl, scrollerEl);
		currentLocator = makeLocator(bookId, chapter.chapterId, anchor, ratio);
	}

	const throttledSample = throttled(() => sampleScroll(), 500);
	const throttledFocus = throttled(() => {
		if (focusLite && articleEl && scrollerEl) updateFocusLite(articleEl, scrollerEl);
	}, 160);

	function emitLocatorNow() {
		if (!currentLocator || destroyed) return;
		// 换章竞态：currentLocator 仍指向上章时绝不能上报。
		if (currentLocator.chapterId !== chapter?.chapterId) return;
		if (sameLocator(currentLocator, lastEmitted)) return;
		lastEmitted = currentLocator;
		onLocator(currentLocator);
		// 持久化兜底：父级也可能在 onLocator 里落库，重复写同值是无害的。
		void saveReaderLocator(bookId, currentLocator).catch((error) => {
			console.warn("[epub] save_reader_locator:", error);
		});
	}

	// Throttled (not debounced): updates must land ~every 2s *while* scrolling,
	// not only after the scroll ends.
	const throttledEmit = throttled(() => {
		sampleScroll();
		emitLocatorNow();
	}, 1600);

	function handleScroll() {
		throttledSample();
		throttledEmit();
		if (focusLite) throttledFocus();
	}

	// ---------- links ----------
	function handleContentClick(event: MouseEvent) {
		const target = event.target instanceof Element ? event.target : null;
		const anchorEl = target?.closest("a[href]") ?? null;
		if (!anchorEl || !articleEl || !articleEl.contains(anchorEl)) return;
		const href = (anchorEl.getAttribute("href") ?? "").trim();
		event.preventDefault();
		event.stopPropagation();
		if (!href || href === "#") return;

		const { path, fragment } = splitHref(href);

		// 纯 #fragment：本章内滚动（脚注/内联锚点）。
		if (!path) {
			if (fragment && articleEl) scrollToFragment(articleEl, fragment);
			sampleScroll();
			throttledEmit();
			return;
		}

		// 外部 http(s)：交给系统浏览器，绝不让 WebView 自己导航。
		if (isExternalHttpUrl(href) || isProtocolRelative(href)) {
			const url = isProtocolRelative(href) ? `https:${href}` : href;
			void openUrl(url).catch((error) => {
				console.warn("[epub] openUrl:", error);
				showTransient("无法打开外部链接");
			});
			return;
		}

		// 书内章节链接：resourceDir 相对 href → spine chapterId。
		const resolved = chapter
			? resolveHrefToChapterId(hrefMap, chapter.resourceDir, href)
			: null;
		if (resolved) {
			goToChapter(resolved.chapterId, resolved.fragment);
			return;
		}

		if (hasUriScheme(path) || isNativeAbsolutePath(path)) {
			showTransient("暂不支持该链接");
		} else {
			showTransient("链接目标不在本书内");
		}
	}

	function showTransient(message: string) {
		transientNotice = message;
		if (transientTimer) clearTimeout(transientTimer);
		transientTimer = setTimeout(() => {
			transientNotice = "";
			transientTimer = null;
		}, 2400);
	}

	// ---------- touch swipe（与主阅读器同一排除规则） ----------
	let touchX = 0;
	let touchY = 0;
	let touchTime = 0;
	let touchExcluded = false;
	let touchMulti = false;

	function handleTouchStart(event: TouchEvent) {
		if (event.touches.length !== 1) {
			touchMulti = true;
			return;
		}
		touchMulti = false;
		touchX = event.touches[0].clientX;
		touchY = event.touches[0].clientY;
		touchTime = Date.now();
		touchExcluded = scrollerEl ? swipeExcluded(event.target, scrollerEl) : false;
	}

	function handleTouchCancel() {
		touchMulti = false;
		touchExcluded = false;
	}

	function handleTouchEnd(event: TouchEvent) {
		if (touchMulti || touchExcluded || event.changedTouches.length !== 1) return;
		const dx = event.changedTouches[0].clientX - touchX;
		const dy = event.changedTouches[0].clientY - touchY;
		const elapsed = Date.now() - touchTime;
		if (Math.abs(dx) > 60 && Math.abs(dy) < 45 && elapsed < 400) {
			navigateChapter(dx < 0 ? 1 : -1);
		}
	}

	// ---------- keyboard ----------
	function isEditableTarget(target: EventTarget | null): boolean {
		return (
			target instanceof HTMLElement &&
			!!target.closest("input, textarea, select, [contenteditable='true']")
		);
	}

	function handleKeydown(event: KeyboardEvent) {
		if (event.defaultPrevented) return;
		const isMac =
			typeof navigator !== "undefined" && /mac/i.test(navigator.platform);
		const mod = isMac ? event.metaKey : event.ctrlKey;

		if (mod && (event.key === "f" || event.key === "F")) {
			event.preventDefault();
			panel = "search";
			return;
		}
		if (event.key === "Escape") {
			if (panel) {
				event.preventDefault();
				panel = "";
			}
			return;
		}
		if (isEditableTarget(event.target)) return;
		if (mod && (event.key === "=" || event.key === "+")) {
			event.preventDefault();
			void setFontPct(fontPct + 10);
			return;
		}
		if (mod && event.key === "-") {
			event.preventDefault();
			void setFontPct(fontPct - 10);
			return;
		}
		if (mod && event.key === "0") {
			event.preventDefault();
			void setFontPct(100);
			return;
		}
		if (panel || event.ctrlKey || event.metaKey || event.altKey) return;
		if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
			// 与横滑同一排除规则：焦点在表格/代码块/横向滚动容器里时，
			// 方向键留给内容自身滚动。
			if (scrollerEl && swipeExcluded(event.target, scrollerEl)) return;
			event.preventDefault();
			navigateChapter(event.key === "ArrowLeft" ? -1 : 1);
		}
	}

	// ---------- focus-lite ----------
	function toggleFocusLite() {
		focusLite = !focusLite;
		if (focusLite) {
			void tick().then(() => {
				articleEl?.classList.add("focus-lite");
				if (articleEl && scrollerEl) updateFocusLite(articleEl, scrollerEl);
			});
		} else if (articleEl) {
			articleEl.classList.remove("focus-lite");
			clearFocusLite(articleEl);
		}
	}

	// ---------- bookmarks ----------
	async function refreshBookmarks() {
		bookmarksLoading = true;
		try {
			bookmarks = await listBookmarks(bookId);
		} catch (error) {
			console.warn("[epub] list_bookmarks:", error);
			bookmarks = [];
		} finally {
			bookmarksLoading = false;
		}
	}

	async function addBookmarkHere() {
		if (!chapter) return;
		sampleScroll();
		if (!currentLocator) return;
		const excerpt = articleEl
			? excerptAtAnchor(articleEl, currentLocator.anchor)
			: "";
		const label = excerpt
			? `${chapter.title} · ${excerpt}`
			: chapter.title || "书签";
		try {
			await addBookmark(bookId, currentLocator, textExcerpt(label, 80));
			await refreshBookmarks();
			showTransient("已收藏当前位置");
		} catch (error) {
			showTransient(reportEpubError("epub-bookmark", error));
		}
	}

	function removeBookmarkById(bookmarkId: string) {
		void removeBookmark(bookId, bookmarkId)
			.then(() => refreshBookmarks())
			.catch((error) => showTransient(reportEpubError("epub-bookmark", error)));
	}

	function jumpToLocator(locator: ReaderLocator) {
		const valid = normalizeReaderLocator(locator, bookId);
		if (!valid) return;
		if (chapter?.chapterId === valid.chapterId) {
			if (articleEl && scrollerEl) restoreLocatorScroll(articleEl, scrollerEl, valid);
			sampleScroll();
			emitLocatorNow();
			return;
		}
		void loadChapter(valid.chapterId, valid);
	}

	// ---------- search jump ----------
	function jumpToSearchHit(hit: SearchHit) {
		const query = snippetQueryText(hit.snippet);
		const locator = hit.locator ? normalizeReaderLocator(hit.locator, bookId) : null;
		if (chapter?.chapterId === hit.chapterId) {
			if (locator && articleEl && scrollerEl) {
				restoreLocatorScroll(articleEl, scrollerEl, locator);
			} else if (query && articleEl) {
				findTextBlock(articleEl, query)?.scrollIntoView({ block: "start" });
			}
			sampleScroll();
			emitLocatorNow();
			return;
		}
		if (!locator && query) pendingTextTarget = query;
		void loadChapter(hit.chapterId, locator);
	}

	// ---------- panels ----------
	function togglePanel(next: Exclude<Panel, "">) {
		panel = panel === next ? "" : next;
		if (panel === "bookmarks") void refreshBookmarks();
	}

	// ---------- exit ----------
	function handleExit() {
		sampleScroll();
		throttledEmit.flush();
		emitLocatorNow();
		onExit();
	}

	function handleVisibility() {
		if (typeof document !== "undefined" && document.visibilityState === "hidden") {
			sampleScroll();
			throttledEmit.flush();
			emitLocatorNow();
		}
	}

	// ---------- lifecycle ----------
	function init() {
		const locator = normalizeReaderLocator(initialLocator, bookId);
		const startId =
			locator && publication.spine.includes(locator.chapterId)
				? locator.chapterId
				: publication.spine[0];
		if (!startId) {
			loading = false;
			errorMsg = "本书没有可阅读的章节";
			return;
		}
		if (locator && locator.chapterId === startId) {
			void loadChapter(startId, locator);
		} else {
			void loadChapter(startId);
		}
	}

	onMount(() => {
		init();
		window.addEventListener("keydown", handleKeydown);
		document.addEventListener("visibilitychange", handleVisibility);
		if (scrollerEl) {
			scrollerEl.addEventListener("scroll", handleScroll, { passive: true });
			scrollerEl.addEventListener("click", handleContentClick);
			scrollerEl.addEventListener("touchstart", handleTouchStart, { passive: true });
			scrollerEl.addEventListener("touchend", handleTouchEnd, { passive: true });
			scrollerEl.addEventListener("touchcancel", handleTouchCancel, {
				passive: true,
			});
		}
	});

	onDestroy(() => {
		// 卸载前把最后位置发出去——父级通常正好要在这时持久化。必须在
		// destroyed 置位之前完成，否则 sample/emit 会提前返回。
		sampleScroll();
		throttledEmit.flush();
		emitLocatorNow();
		destroyed = true;
		loadSeq += 1;
		window.removeEventListener("keydown", handleKeydown);
		document.removeEventListener("visibilitychange", handleVisibility);
		if (scrollerEl) {
			scrollerEl.removeEventListener("scroll", handleScroll);
			scrollerEl.removeEventListener("click", handleContentClick);
			scrollerEl.removeEventListener("touchstart", handleTouchStart);
			scrollerEl.removeEventListener("touchend", handleTouchEnd);
			scrollerEl.removeEventListener("touchcancel", handleTouchCancel);
		}
		throttledSample.cancel();
		throttledEmit.cancel();
		throttledFocus.cancel();
		if (transientTimer) clearTimeout(transientTimer);
	});

	// bookId 变化时重载（正常由父级 {#key} 保证，这里是保险）。
	let mountedBookId = "";
	$: if (mountedBookId !== bookId) {
		if (mountedBookId) init();
		mountedBookId = bookId;
	}
</script>

<div class="epub-reader" class:mobile={isMobile}>
	<header class="epub-top">
		<button
			type="button"
			class="epub-back"
			on:click={handleExit}
			aria-label="返回书架"
		>
			<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true">
				<path d="M15 5l-7 7 7 7" />
			</svg>
			<span class="epub-back-text">返回书架</span>
		</button>
		<div class="epub-titles">
			<span class="epub-book-title">{publication.title}</span>
			{#if chapter?.title}
				<span class="epub-chapter-title">{chapter.title}</span>
			{/if}
		</div>
		<div class="epub-tools">
			{#if !isMobile}
				<span class="epub-progress-text">{progressPct}%</span>
			{/if}
			<button
				type="button"
				class="epub-tool"
				class:active={panel === "toc"}
				on:click={() => togglePanel("toc")}
				disabled={!publication.nav?.length}
				title="目录"
			>目录</button>
			<button
				type="button"
				class="epub-tool"
				class:active={panel === "search"}
				on:click={() => togglePanel("search")}
				title="搜索 (Ctrl+F)"
			>搜索</button>
			<button
				type="button"
				class="epub-tool"
				class:active={panel === "bookmarks"}
				on:click={() => togglePanel("bookmarks")}
				title="书签"
			>书签</button>
			<button
				type="button"
				class="epub-tool"
				class:active={panel === "type"}
				on:click={() => togglePanel("type")}
				title="字号与显示"
			>字号</button>
		</div>
	</header>

	{#if showUnsupported}
		<div class="epub-unsupported" role="status">
			<span>本书包含暂不支持的特性：{unsupportedLabels.join("、")}</span>
			<button
				type="button"
				class="epub-unsupported-close"
				on:click={() => (noticeDismissed = true)}
				aria-label="关闭提示"
			>×</button>
		</div>
	{/if}

	<div class="epub-scroll" bind:this={scrollerEl}>
		{#if loading}
			<div class="epub-skeleton" aria-hidden="true">
				<div class="epub-skel epub-skel-title"></div>
				<div class="epub-skel"></div>
				<div class="epub-skel"></div>
				<div class="epub-skel short"></div>
				<div class="epub-skel"></div>
				<div class="epub-skel short"></div>
			</div>
		{:else if errorMsg}
			<div class="epub-error">
				<p class="epub-error-msg">{errorMsg}</p>
				<div class="epub-error-actions">
					<button
						type="button"
						class="epub-error-btn"
						on:click={() => void loadChapter(errorChapterId || publication.spine[0])}
					>重试</button>
					<button type="button" class="epub-error-btn" on:click={handleExit}>
						返回书架
					</button>
				</div>
			</div>
		{:else}
			<article
				class="epub-content"
				bind:this={articleEl}
				style:--epub-font-pct={fontPct / 100}
			>{@html chapterHtml}</article>
			<nav class="epub-chapter-nav" aria-label="章节导航">
				{#if chapter?.prevChapterId}
					<button
						type="button"
						class="epub-chapter-btn"
						on:click={() => navigateChapter(-1)}
					>← {prevTitle}</button>
				{:else}
					<span></span>
				{/if}
				{#if chapter?.nextChapterId}
					<button
						type="button"
						class="epub-chapter-btn"
						on:click={() => navigateChapter(1)}
					>{nextTitle} →</button>
				{/if}
			</nav>
		{/if}
	</div>

	{#if isMobile}
		<footer class="epub-bottom">
			<button
				type="button"
				class="epub-bottom-btn"
				disabled={!chapter?.prevChapterId}
				on:click={() => navigateChapter(-1)}
			>上一章</button>
			<span class="epub-bottom-progress">{progressPct}%</span>
			<button
				type="button"
				class="epub-bottom-btn"
				disabled={!chapter?.nextChapterId}
				on:click={() => navigateChapter(1)}
			>下一章</button>
		</footer>
	{/if}

	{#if transientNotice}
		<div class="epub-toast" role="status">{transientNotice}</div>
	{/if}

	{#if panel === "toc"}
		<EpubToc
			items={publication.nav}
			activeChapterId={chapter?.chapterId ?? ""}
			{isMobile}
			onJump={(id) => goToChapter(id)}
			onClose={() => (panel = "")}
		/>
	{:else if panel === "search"}
		<EpubSearch
			{bookId}
			{isMobile}
			onJump={(hit) => jumpToSearchHit(hit)}
			onClose={() => (panel = "")}
		/>
	{:else if panel === "bookmarks"}
		<EpubBookmarks
			{bookmarks}
			loading={bookmarksLoading}
			canAdd={!!chapter && !loading}
			{isMobile}
			onAdd={() => void addBookmarkHere()}
			onJump={(loc) => jumpToLocator(loc)}
			onRemove={(id) => removeBookmarkById(id)}
			onClose={() => (panel = "")}
		/>
	{:else if panel === "type"}
		<!-- svelte-ignore a11y-click-events-have-key-events -->
		<!-- svelte-ignore a11y-no-static-element-interactions -->
		<div class="epub-popover-overlay" on:click={() => (panel = "")}>
			<div
				class="epub-popover"
				role="dialog"
				tabindex="-1"
				aria-label="字号与显示"
				on:click|stopPropagation
				on:keydown={(e) => {
					if (e.key === "Escape") {
						e.preventDefault();
						e.stopPropagation();
						panel = "";
					}
				}}
			>
				<div class="epub-pop-row">
					<span class="epub-pop-label">字号</span>
					<button
						type="button"
						class="epub-pop-btn"
						disabled={fontPct <= 80}
						on:click={() => void setFontPct(fontPct - 10)}
						aria-label="减小字号"
					>A−</button>
					<span class="epub-pop-value">{fontPct}%</span>
					<button
						type="button"
						class="epub-pop-btn"
						disabled={fontPct >= 160}
						on:click={() => void setFontPct(fontPct + 10)}
						aria-label="增大字号"
					>A＋</button>
				</div>
				<input
					type="range"
					class="epub-pop-range"
					min="80"
					max="160"
					step="5"
					value={fontPct}
					on:input={(e) =>
						void setFontPct(Number((e.target as HTMLInputElement).value))}
					aria-label="字号百分比"
				/>
				<div class="epub-pop-row">
					<span class="epub-pop-label">专注模式</span>
					<button
						type="button"
						class="epub-pop-toggle"
						class:on={focusLite}
						on:click={toggleFocusLite}
						aria-pressed={focusLite}
						aria-label="专注模式"
					>
						<span class="epub-pop-knob"></span>
					</button>
				</div>
				<button
					type="button"
					class="epub-pop-reset"
					on:click={() => void setFontPct(100)}
				>重置字号</button>
			</div>
		</div>
	{/if}
</div>

<style>
	.epub-reader {
		position: relative;
		display: flex;
		flex-direction: column;
		height: 100%;
		min-height: 0;
		background: var(--bg);
		color: var(--text);
		overflow: hidden;
	}

	/* ---------- top chrome ---------- */
	.epub-top {
		display: flex;
		align-items: center;
		gap: 10px;
		height: calc(48px + env(safe-area-inset-top, 0px));
		padding: env(safe-area-inset-top, 0px)
			max(12px, env(safe-area-inset-right, 0px)) 0
			max(12px, env(safe-area-inset-left, 0px));
		border-bottom: 1px solid var(--hr);
		background: var(--bg);
		flex-shrink: 0;
	}
	.epub-back {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		border: none;
		background: transparent;
		color: var(--text-secondary);
		font-size: 13px;
		padding: 6px 8px;
		border-radius: 8px;
		cursor: pointer;
		flex-shrink: 0;
	}
	.epub-back:hover {
		color: var(--text);
		background: var(--bg-secondary);
	}
	.epub-back:focus-visible {
		outline: 2px solid var(--link);
		outline-offset: -2px;
	}
	.mobile .epub-back {
		min-height: 40px;
		padding: 6px 12px;
	}
	.mobile .epub-back-text {
		/* 顶栏空间留给标题与工具按钮 */
		display: none;
	}
	.epub-titles {
		flex: 1;
		min-width: 0;
		display: flex;
		flex-direction: column;
		line-height: 1.25;
	}
	.epub-book-title {
		font-size: 13.5px;
		font-weight: 600;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.epub-chapter-title {
		font-size: 11.5px;
		color: var(--text-faded);
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.epub-tools {
		display: flex;
		align-items: center;
		gap: 4px;
		flex-shrink: 0;
	}
	.epub-progress-text {
		font-size: 11.5px;
		color: var(--text-faded);
		font-variant-numeric: tabular-nums;
		margin-right: 4px;
	}
	.epub-tool {
		border: none;
		background: transparent;
		color: var(--text-secondary);
		font-size: 13px;
		padding: 6px 10px;
		border-radius: 8px;
		cursor: pointer;
		white-space: nowrap;
	}
	.epub-tool:hover:not(:disabled) {
		background: var(--bg-secondary);
		color: var(--text);
	}
	.epub-tool.active {
		color: var(--link);
		background: var(--bg-secondary);
	}
	.epub-tool:disabled {
		opacity: 0.4;
		cursor: default;
	}
	.epub-tool:focus-visible {
		outline: 2px solid var(--link);
		outline-offset: -2px;
	}
	.mobile .epub-tool {
		min-height: 40px;
		font-size: 14px;
		padding: 6px 8px;
	}

	/* ---------- unsupported notice ---------- */
	.epub-unsupported {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
		padding: 8px 16px;
		background: var(--bg-secondary);
		border-bottom: 1px solid var(--hr);
		font-size: 12.5px;
		color: var(--text-secondary);
		flex-shrink: 0;
	}
	.epub-unsupported-close {
		border: none;
		background: transparent;
		color: var(--text-faded);
		font-size: 16px;
		cursor: pointer;
		padding: 2px 8px;
		border-radius: 6px;
		line-height: 1;
	}
	.epub-unsupported-close:hover {
		color: var(--text);
	}
	.epub-unsupported-close:focus-visible {
		outline: 2px solid var(--link);
	}

	/* ---------- scroll area ---------- */
	.epub-scroll {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
		overflow-x: hidden;
		overscroll-behavior-y: contain;
	}

	/* 注入的书籍样式经 postProcessChapterDom 统一限定在 .epub-content 下；
	   此处对子元素必须用 :global — {@html} 节点不带 svelte 作用域属性。 */
	.epub-content {
		max-width: var(--article-max-width, 760px);
		margin: 0 auto;
		padding: 28px max(20px, env(safe-area-inset-right, 0px)) 48px
			max(20px, env(safe-area-inset-left, 0px));
		font-size: calc(1rem * var(--font-scale, 1) * var(--epub-font-pct, 1));
		line-height: var(--article-line-height, var(--reading-line-height, 1.9));
		font-family: var(--article-font-family, inherit);
		color: var(--text);
		overflow-wrap: break-word;
	}
	.epub-content :global(img) {
		max-width: 100%;
		height: auto;
	}
	.epub-content :global(table) {
		max-width: 100%;
		overflow-x: auto;
		display: block;
	}
	.epub-content :global(pre) {
		overflow-x: auto;
		white-space: pre-wrap;
	}
	.epub-content :global(a) {
		color: var(--link);
	}
	.epub-content :global(sup a) {
		text-decoration: none;
		opacity: 0.75;
		padding: 0 1px;
	}
	.epub-content :global(h1),
	.epub-content :global(h2),
	.epub-content :global(h3),
	.epub-content :global(h4),
	.epub-content :global(h5),
	.epub-content :global(h6) {
		line-height: 1.4;
	}
	.epub-content :global(.remote-blocked) {
		border: 1px dashed var(--text-faded);
		border-radius: 8px;
		padding: 18px;
		margin: 12px 0;
		text-align: center;
		font-size: 0.85em;
		color: var(--text-faded);
	}
	.epub-content :global(.remote-blocked)::before {
		content: "⊘ ";
	}

	/* focus-lite：非当前块的直接子级压暗。 */
	:global(.epub-content.focus-lite > *) {
		transition: opacity 0.25s ease;
	}
	:global(.epub-content.focus-lite > *:not(.epub-focus-current)) {
		opacity: 0.18;
	}
	:global(.epub-content.focus-lite > .epub-focus-current) {
		opacity: 1;
	}

	/* ---------- skeleton / error ---------- */
	.epub-skeleton {
		max-width: var(--article-max-width, 760px);
		margin: 0 auto;
		padding: 40px 24px;
		display: flex;
		flex-direction: column;
		gap: 16px;
	}
	.epub-skel {
		height: 14px;
		border-radius: 6px;
		background: var(--bg-secondary);
		animation: epubPulse 1.4s ease-in-out infinite;
	}
	.epub-skel-title {
		height: 22px;
		width: 42%;
		margin-bottom: 10px;
	}
	.epub-skel.short {
		width: 68%;
	}
	@media (prefers-reduced-motion: reduce) {
		.epub-skel {
			animation: none;
		}
	}
	@keyframes epubPulse {
		0%,
		100% {
			opacity: 1;
		}
		50% {
			opacity: 0.45;
		}
	}

	.epub-error {
		max-width: 460px;
		margin: 18vh auto 0;
		padding: 24px;
		text-align: center;
	}
	.epub-error-msg {
		font-size: 15px;
		color: var(--text);
		line-height: 1.7;
		margin-bottom: 20px;
	}
	.epub-error-actions {
		display: flex;
		justify-content: center;
		gap: 12px;
	}
	.epub-error-btn {
		border: 1px solid var(--hr);
		background: var(--bg-secondary);
		color: var(--text);
		font-size: 13.5px;
		padding: 8px 22px;
		border-radius: 10px;
		cursor: pointer;
	}
	.epub-error-btn:hover {
		color: var(--link);
	}
	.epub-error-btn:focus-visible {
		outline: 2px solid var(--link);
	}

	/* ---------- chapter nav row ---------- */
	.epub-chapter-nav {
		max-width: var(--article-max-width, 760px);
		margin: 0 auto;
		padding: 0 max(20px, env(safe-area-inset-right, 0px)) 56px
			max(20px, env(safe-area-inset-left, 0px));
		display: flex;
		justify-content: space-between;
		gap: 16px;
	}
	.epub-chapter-btn {
		border: 1px solid var(--hr);
		background: transparent;
		color: var(--link);
		font-size: 13.5px;
		padding: 10px 18px;
		border-radius: 10px;
		cursor: pointer;
		max-width: 48%;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.epub-chapter-btn:hover {
		background: var(--bg-secondary);
	}
	.epub-chapter-btn:focus-visible {
		outline: 2px solid var(--link);
	}
	.mobile .epub-chapter-btn {
		min-height: 44px;
		font-size: 14px;
	}

	/* ---------- mobile bottom bar ---------- */
	.epub-bottom {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
		padding: 8px max(16px, env(safe-area-inset-right, 0px))
			max(10px, env(safe-area-inset-bottom, 0px))
			max(16px, env(safe-area-inset-left, 0px));
		border-top: 1px solid var(--hr);
		background: var(--bg);
		flex-shrink: 0;
	}
	.epub-bottom-btn {
		flex: 1;
		border: 1px solid var(--hr);
		background: transparent;
		color: var(--link);
		font-size: 14px;
		padding: 10px 0;
		border-radius: 10px;
		cursor: pointer;
	}
	.epub-bottom-btn:disabled {
		color: var(--text-faded);
		opacity: 0.5;
		cursor: default;
	}
	.epub-bottom-btn:focus-visible {
		outline: 2px solid var(--link);
	}
	.epub-bottom-progress {
		min-width: 52px;
		text-align: center;
		font-size: 12.5px;
		color: var(--text-faded);
		font-variant-numeric: tabular-nums;
	}

	/* ---------- toast ---------- */
	.epub-toast {
		position: absolute;
		left: 50%;
		bottom: calc(72px + env(safe-area-inset-bottom, 0px));
		transform: translateX(-50%);
		background: var(--bg-secondary);
		border: 1px solid var(--hr);
		color: var(--text);
		font-size: 12.5px;
		padding: 8px 18px;
		border-radius: 999px;
		box-shadow: 0 8px 24px rgba(0, 0, 0, 0.25);
		z-index: 900;
		animation: epubFadeIn 0.15s ease;
		white-space: nowrap;
	}

	/* ---------- 字号 popover ---------- */
	.epub-popover-overlay {
		position: fixed;
		inset: 0;
		z-index: 1000;
		background: transparent;
	}
	.epub-popover {
		position: absolute;
		top: calc(52px + env(safe-area-inset-top, 0px));
		right: max(12px, env(safe-area-inset-right, 0px));
		width: 240px;
		background: var(--bg-secondary);
		border: 1px solid var(--hr);
		border-radius: 14px;
		box-shadow: 0 16px 48px rgba(0, 0, 0, 0.3);
		padding: 14px;
		display: flex;
		flex-direction: column;
		gap: 12px;
		animation: epubFadeIn 0.15s ease;
	}
	.epub-pop-row {
		display: flex;
		align-items: center;
		gap: 10px;
	}
	.epub-pop-label {
		flex: 1;
		font-size: 13px;
		color: var(--text);
	}
	.epub-pop-btn {
		border: 1px solid var(--hr);
		background: transparent;
		color: var(--text);
		font-size: 13px;
		width: 40px;
		height: 32px;
		border-radius: 8px;
		cursor: pointer;
	}
	.epub-pop-btn:disabled {
		opacity: 0.4;
		cursor: default;
	}
	.epub-pop-btn:focus-visible {
		outline: 2px solid var(--link);
	}
	.epub-pop-value {
		min-width: 44px;
		text-align: center;
		font-size: 12.5px;
		color: var(--text-secondary);
		font-variant-numeric: tabular-nums;
	}
	.epub-pop-range {
		width: 100%;
		accent-color: var(--link);
	}
	.epub-pop-toggle {
		position: relative;
		width: 44px;
		height: 24px;
		border: none;
		border-radius: 999px;
		background: var(--hr);
		cursor: pointer;
		transition: background 0.2s ease;
	}
	.epub-pop-toggle.on {
		background: var(--link);
	}
	.epub-pop-toggle:focus-visible {
		outline: 2px solid var(--link);
		outline-offset: 2px;
	}
	.epub-pop-knob {
		position: absolute;
		top: 3px;
		left: 3px;
		width: 18px;
		height: 18px;
		border-radius: 50%;
		background: var(--bg);
		transition: transform 0.2s ease;
	}
	.epub-pop-toggle.on .epub-pop-knob {
		transform: translateX(20px);
	}
	.epub-pop-reset {
		border: none;
		background: transparent;
		color: var(--link);
		font-size: 12.5px;
		padding: 6px;
		border-radius: 8px;
		cursor: pointer;
	}
	.epub-pop-reset:hover {
		background: var(--bg);
	}
	.epub-pop-reset:focus-visible {
		outline: 2px solid var(--link);
	}

	@keyframes epubFadeIn {
		from {
			opacity: 0;
		}
		to {
			opacity: 1;
		}
	}
</style>
