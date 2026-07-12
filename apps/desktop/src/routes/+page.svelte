<script lang="ts">
	import { onMount, tick } from "svelte";
	import { convertFileSrc, invoke } from "@tauri-apps/api/core";
	import { listen } from "@tauri-apps/api/event";
	import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
	import { open } from "@tauri-apps/plugin-dialog";
	import { openUrl } from "@tauri-apps/plugin-opener";
	import {
		getMarkdownSourceBlock,
		normalizeMarkdownEditText,
		replaceMarkdownSourceBlock
	} from "$lib/edit/sourceBlock";
	import { getFocusScrollTarget } from "$lib/focus/scroll";
	import { splitSentences } from "$lib/focus/segment";
	import { resolveMarkdownImageSources } from "$lib/render/images";
	import type { FrontMatterEntry, RenderedMarkdownDocument, TocItem } from "$lib/render/markdown";
	import {
		applyTaskEvent,
		snapshotState,
		taskList,
		type AcquisitionSnapshot,
		type TaskEvent,
		type TaskSnapshot,
		type TaskSyncState
	} from "$lib/tasks/sync";
	import {
		currentFilePath,
		markdownSource,
		renderedHtml,
		isLoading,
		currentTheme,
		focusMode,
		searchOpen,
		searchQuery,
		tocOpen,
		settingsOpen,
		fontScale,
		clampFontScale,
		FONT_SCALE_STEP,
		readingLineHeight,
		readingWidth,
		readingFontFamily,
	} from "$lib/stores/app";
	import SearchBar from "$lib/components/SearchBar.svelte";
	import TocPanel from "$lib/components/TocPanel.svelte";
	import SettingsPanel from "$lib/components/SettingsPanel.svelte";
	import Bookshelf from "$lib/components/Bookshelf.svelte";
	import TrashPanel from "$lib/components/TrashPanel.svelte";
	import type { TrashDeleteResult, TrashItem } from "$lib/trash/types";
	import {
		chapterTocItems,
		nextReadingState,
		resolveChapterIndex,
		type AppSettings,
		type BookDetail,
		type LibraryIssue,
		type BookSummary,
		type TemporaryItem,
	} from "$lib/library/books";

	let contentEl: HTMLElement;
	let tocItems: TocItem[] = [];
	let frontMatterEntries: FrontMatterEntry[] = [];

	// 最近打开的文件（欢迎页展示，localStorage 持久化）
	interface RecentFile {
		path: string;
		name: string;
		openedAt: number;
	}
	interface RecentFilesLoad {
		json: string;
		store_exists: boolean;
	}
	const RECENT_FILES_KEY = "mmbook.recentFiles";
	const RECENT_FILES_LIMIT = 8;
	let recentFiles: RecentFile[] = [];
	let continueProgress = 0;

	function isRecord(value: unknown): value is Record<string, unknown> {
		return typeof value === "object" && value !== null && !Array.isArray(value);
	}

	function parseRecentFiles(raw: string | null): RecentFile[] {
		if (!raw) return [];
		try {
			const parsed: unknown = JSON.parse(raw);
			if (!Array.isArray(parsed)) return [];
			const files: RecentFile[] = [];
			for (const item of parsed) {
				if (!isRecord(item)) continue;
				const { path, name, openedAt } = item;
				if (typeof path !== "string" || typeof name !== "string") continue;
				files.push({
					path,
					name,
					openedAt: typeof openedAt === "number" ? openedAt : 0,
				});
			}
			return files.slice(0, RECENT_FILES_LIMIT);
		} catch {
			return [];
		}
	}

	function loadLocalRecentFiles(): RecentFile[] {
		try {
			return parseRecentFiles(localStorage.getItem(RECENT_FILES_KEY));
		} catch {
			return [];
		}
	}

	// 最近文件存后端 AppData(清 WebView 缓存不丢);旧 localStorage 数据一次性迁移。
	async function loadRecentFiles(): Promise<RecentFile[]> {
		try {
			const result = await invoke<RecentFilesLoad>("load_recent_files");
			const loaded = parseRecentFiles(result.json);
			if (result.store_exists) return loaded;

			const localFiles = loadLocalRecentFiles();
			if (localFiles.length > 0) {
				return await persistRecentFiles(localFiles);
			}
			return loaded;
		} catch {
			return loadLocalRecentFiles();
		}
	}

	async function persistRecentFiles(files: RecentFile[], refreshContinue = false): Promise<RecentFile[]> {
		const limited = files.slice(0, RECENT_FILES_LIMIT);
		try {
			const cleanedJson = await invoke<string>("save_recent_files", {
				json: JSON.stringify(limited),
			});
			const cleaned = parseRecentFiles(cleanedJson);
			recentFiles = cleaned;
			try {
				localStorage.removeItem(RECENT_FILES_KEY);
			} catch {
				// ignore
			}
			if (refreshContinue) await loadContinueInfo();
			return cleaned;
		} catch {
			recentFiles = limited;
			try {
				localStorage.setItem(RECENT_FILES_KEY, JSON.stringify(limited));
			} catch {
				// ignore
			}
			if (refreshContinue) await loadContinueInfo();
			return limited;
		}
	}

	function recordRecentFile(path: string, name: string) {
		const nextFiles = [
			{ path, name, openedAt: Date.now() },
			...recentFiles.filter((f) => f.path !== path),
		].slice(0, RECENT_FILES_LIMIT);
		recentFiles = nextFiles;
		void persistRecentFiles(nextFiles);
	}

	async function markdownFileStillExists(path: string): Promise<boolean> {
		try {
			return await invoke<boolean>("markdown_file_exists", { path });
		} catch {
			return true;
		}
	}

	async function removeRecentFile(path: string, showNotice = false) {
		recentFiles = recentFiles.filter((f) => f.path !== path);
		void invoke("delete_reading_state", { path }).catch(() => {});
		await persistRecentFiles(recentFiles, true);
		if (showNotice) {
			showAppNotice("文件不存在或已移动，已从历史记录中移除");
		}
	}

	async function openRecentFile(path: string) {
		if (!(await markdownFileStillExists(path))) {
			await removeRecentFile(path, true);
			return;
		}

		const opened = await openFile(path, { suppressFailureNotice: true });
		if (opened) return;

		if (!(await markdownFileStillExists(path))) {
			await removeRecentFile(path, true);
			return;
		}

		showAppNotice("无法打开文件，请检查路径或权限");
		if (contentEl) contentEl.scrollTop = 0;
	}

	async function loadContinueInfo() {
		continueProgress = 0;
		const first = recentFiles[0];
		if (!first) return;
		try {
			const state = await invoke<{ progress?: number }>("load_reading_state", {
				path: first.path,
			});
			continueProgress = Math.max(0, Math.min(1, state.progress ?? 0));
		} catch {
			// 没有阅读状态就不显示进度
		}
	}
	let fileName = "";
	let fileEncoding = 'utf-8';
	let fileError = "";
	type SearchMatch = {
		block: HTMLElement;
		occurrence: number;
	};
	let searchMatches: SearchMatch[] = [];
	let currentMatchIndex = -1;
	let currentSearchMark: HTMLElement | null = null;
	let editingParagraph: HTMLElement | null = null;
	let editUnwrappedSegments = false;
	let originalText = "";
	let originalMarkdownBlock = "";
	// A focus unit is one *or more* sibling elements highlighted together —
	// adjacent low-density lines (short list items, one-line paragraphs, table
	// rows) merge into one unit so the spotlight doesn't step line by line.
	type FocusUnit = HTMLElement[];
	let focusUnits: FocusUnit[] = [];
	let focusBlockMetrics: { top: number; bottom: number; center: number }[] = [];
	let focusMetricsValid = false;
	let focusUpdateFrame: number | null = null;
	let queuedFocusIndex: number | undefined;
	let isFocusScrollActive = false;
	let isFocusKeyScrollActive = false;
	let focusScrollEndTimer: ReturnType<typeof setTimeout> | null = null;
	let focusLockedIndex: number | null = null;
	let focusProgrammaticScrollIndex: number | null = null;
	let focusProgrammaticScrollTarget: number | null = null;
	let focusProgrammaticScrollTimer: ReturnType<typeof setTimeout> | null = null;
	let focusKeyScrollDirection: -1 | 0 | 1 = 0;
	let focusKeyScrollFrame: number | null = null;
	let focusKeyScrollLastTime = 0;
	let focusInlineScrollBehavior: string | null = null;
	let searchMatchBlocks = new Set<HTMLElement>();
	let searchDebounceTimer: ReturnType<typeof setTimeout> | null = null;
	let saveStateTimer: ReturnType<typeof setTimeout> | null = null;
	let currentLoadToken: string = '';
	let finishEditPromise: Promise<boolean> | null = null;
	let lastFocusRenderSignature = "";
	let focusStyleCache = new WeakMap<HTMLElement, string>();
	let styledFocusIndices = new Set<number>();
	let focusEntryStickyIndices: Set<number> | null = null;
	let focusEntryStickyUntil = 0;
	let cachedFocusGlow: { theme: string; color: string; has: boolean } | null = null;
	let lastSpotlightVars = new Map<string, string>();
	let markdownWorker: Worker | null = null;
	let markdownWorkerFailed = false;
	let nextRenderRequestId = 1;
	const pendingMarkdownRenders = new Map<number, {
		resolve: (result: RenderedMarkdownDocument) => void;
		reject: (error: Error) => void;
	}>();
	let pendingArticleLinkOpenTimer: ReturnType<typeof setTimeout> | null = null;
	let pendingArticleLinkOpenUrl = "";
	let spotlightHeight = 100;

	let isEditingInDarkFocus = false;
	let editOrbitContainerStyle = '';
	const EDIT_ORBIT_COUNT = 20;
	let editOrbitParticles: {id: number, size: number, duration: number, opacity: number, stagger: number}[] = [];

	let particles: {id: number, left: number, size: number, duration: number, delay: number, opacity: number}[] = [];
	let isLightTheme = false;

	const isMac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform);
	const modLabel = isMac ? '⌘' : 'Ctrl+';
	function isModKey(e: KeyboardEvent): boolean {
		return isMac ? e.metaKey : e.ctrlKey;
	}

	$: isLightTheme = $currentTheme.name.toLowerCase().includes('light');
	$: if ($focusMode && !isLightTheme && typeof window !== "undefined") {
		if (particles.length === 0) {
			particles = Array.from({length: 80}).map((_, i) => ({
				id: i,
				left: Math.random() * 100,
				size: Math.random() * 2 + 1,
				duration: Math.random() * 12 + 6,
				delay: Math.random() * -20,
				opacity: Math.random() * 0.7 + 0.3
			}));
		}
	} else {
		particles = [];
	}
	$: if (!editingParagraph && isEditingInDarkFocus) cleanupEditOrbit();

	let readingProgress = 0;
	let libraryBooks: BookSummary[] = [];
	let libraryIssues: LibraryIssue[] = [];
	let temporaryItems: TemporaryItem[] = [];
	let trashItems: TrashItem[] = [];
	let trashOpen = false;
	let trashLoading = false;
	let libraryWritable = true;
	let libraryLoading = true;
	let appSettings: AppSettings | null = null;
	let taskSyncState: TaskSyncState = snapshotState([]);
	let acquisitionTasks: readonly TaskSnapshot[] = [];
	let taskRefreshNonce = 0;
	let flowReaderSession: { sessionId: string; url: string } | null = null;
	let activeBook: BookDetail | null = null;
	let activeChapterIndex = -1;
	let chapterSwitching = false;
	let preloadedChapter: {
		bookId: string;
		index: number;
		path: string;
		result: { content: string; encoding: string };
		rendered: RenderedMarkdownDocument;
	} | null = null;
	let zoomIndicatorText = "";
	let zoomIndicatorTimer: ReturnType<typeof setTimeout> | null = null;

	$: acquisitionTasks = taskList(taskSyncState);

	async function refreshAcquisitionSnapshot(): Promise<void> {
		const nonce = ++taskRefreshNonce;
		try {
			const snapshot = await invoke<AcquisitionSnapshot>("get_acquisition_snapshot", {
				kind: null
			});
			if (nonce !== taskRefreshNonce) return;
			taskSyncState = snapshotState(snapshot.tasks);
		} catch (error) {
			const message = error instanceof Error ? error.message : String(error);
			showAppNotice(`无法同步任务队列：${message}`);
		}
	}

	function receiveTaskEvent(event: TaskEvent): void {
		const result = applyTaskEvent(taskSyncState, event);
		if (result.kind === "refresh") {
			void refreshAcquisitionSnapshot();
			return;
		}
		if (result.kind === "applied") {
			taskRefreshNonce += 1;
			taskSyncState = result.state;
		}
	}

	function setFontScale(next: number) {
		const clamped = clampFontScale(next);
		zoomIndicatorText = `字号 ${Math.round(clamped * 100)}%`;
		if (zoomIndicatorTimer) clearTimeout(zoomIndicatorTimer);
		zoomIndicatorTimer = setTimeout(() => {
			zoomIndicatorText = "";
			zoomIndicatorTimer = null;
		}, 1200);
		if (clamped !== $fontScale) $fontScale = clamped;
	}

	function adjustFontScale(direction: number) {
		setFontScale($fontScale + direction * FONT_SCALE_STEP);
	}

	// Any typography change (zoom, line height, width, font) reflows the whole
	// document. Re-anchor by reading progress — computed before the change —
	// and re-align focus mode on the same unit.
	let lastTypographySignature: string | null = null;
	$: {
		const signature = `${$fontScale}|${$readingLineHeight}|${$readingWidth}|${$readingFontFamily}`;
		if (lastTypographySignature === null) {
			lastTypographySignature = signature;
		} else if (signature !== lastTypographySignature) {
			lastTypographySignature = signature;
			reflowAfterTypographyChange();
		}
	}

	function reflowAfterTypographyChange() {
		if (!contentEl) return;
		const progress = readingProgress;
		requestAnimationFrame(() => {
			if (!contentEl) return;
			contentEl.scrollTop = progress * Math.max(0, contentEl.scrollHeight - contentEl.clientHeight);
			invalidateFocusMetrics();
			if ($focusMode) {
				rebuildFocusMetrics();
				const units = getFocusUnits();
				if (lastFocusedIdx >= 0 && units[lastFocusedIdx]) {
					focusLockedIndex = lastFocusedIdx;
					markFocusScrollActive();
					scrollUnitToFocusPosition(units[lastFocusedIdx], lastFocusedIdx);
				}
				scheduleFocusUpdate(lastFocusedIdx >= 0 ? lastFocusedIdx : undefined);
			}
		});
	}

	// Wide tables scroll inside a wrapper instead of getting squashed.
	function wrapArticleTables() {
		const article = getArticleElement();
		if (!article) return;
		article.querySelectorAll("table").forEach((table) => {
			if (table.parentElement?.classList.contains("table-scroll")) return;
			const wrapper = document.createElement("div");
			wrapper.className = "table-scroll";
			const start = table.getAttribute("data-source-start");
			const end = table.getAttribute("data-source-end");
			if (start) wrapper.setAttribute("data-source-start", start);
			if (end) wrapper.setAttribute("data-source-end", end);
			table.parentNode?.insertBefore(wrapper, table);
			wrapper.appendChild(table);
		});
	}

	// ===== Code copy button (single floating button over the hovered pre) =====
	let codeCopyVisible = false;
	let codeCopyStyle = "";
	let codeCopySuccess = false;
	let codeCopyTarget: HTMLElement | null = null;
	let codeCopyHideTimer: ReturnType<typeof setTimeout> | null = null;

	function cancelCodeCopyHide() {
		if (codeCopyHideTimer) {
			clearTimeout(codeCopyHideTimer);
			codeCopyHideTimer = null;
		}
	}

	function scheduleCodeCopyHide(delay = 250) {
		cancelCodeCopyHide();
		codeCopyHideTimer = setTimeout(() => {
			codeCopyHideTimer = null;
			codeCopyVisible = false;
			codeCopyTarget = null;
		}, delay);
	}

	function showCodeCopyButton(pre: HTMLElement) {
		cancelCodeCopyHide();
		if (codeCopyTarget !== pre) codeCopySuccess = false;
		codeCopyTarget = pre;
		const rect = pre.getBoundingClientRect();
		codeCopyStyle = `top:${Math.max(8, rect.top + 8)}px;left:${rect.right - 8}px`;
		codeCopyVisible = true;
	}

	async function copyCodeBlock() {
		const text =
			codeCopyTarget?.querySelector("code")?.textContent ??
			codeCopyTarget?.textContent ??
			"";
		try {
			await navigator.clipboard.writeText(text);
		} catch {
			const textarea = document.createElement("textarea");
			textarea.value = text;
			textarea.style.position = "fixed";
			textarea.style.opacity = "0";
			document.body.appendChild(textarea);
			textarea.select();
			document.execCommand("copy");
			textarea.remove();
		}
		codeCopySuccess = true;
		scheduleCodeCopyHide(900);
	}

	// ===== Footnote hover preview =====
	let footnotePreviewHtml = "";
	let footnotePreviewStyle = "";
	let footnoteHideTimer: ReturnType<typeof setTimeout> | null = null;

	function getFootnoteLink(node: EventTarget | null): HTMLAnchorElement | null {
		const el = node instanceof HTMLElement ? node : null;
		return (el?.closest("a[data-footnote-ref], sup > a[href^='#']") ?? null) as HTMLAnchorElement | null;
	}

	function showFootnotePreview(link: HTMLAnchorElement) {
		const href = link.getAttribute("href") ?? "";
		if (!href.startsWith("#")) return;
		let target: Element | null = null;
		try {
			target = contentEl?.querySelector(
				`#${CSS.escape(decodeURIComponent(href.slice(1)))}`,
			) ?? null;
		} catch {
			return;
		}
		if (!target || !target.closest("section.footnotes, section[data-footnotes]")) return;

		const clone = target.cloneNode(true) as HTMLElement;
		clone
			.querySelectorAll("a[data-footnote-backref], .data-footnote-backref")
			.forEach((node) => node.remove());
		footnotePreviewHtml = clone.innerHTML;

		const rect = link.getBoundingClientRect();
		const margin = 12;
		const width = Math.min(380, window.innerWidth - margin * 2);
		const left = Math.max(
			margin,
			Math.min(rect.left + rect.width / 2 - width / 2, window.innerWidth - width - margin),
		);
		const showAbove = rect.top > window.innerHeight * 0.4;
		const vertical = showAbove
			? `bottom:${window.innerHeight - rect.top + 8}px`
			: `top:${rect.bottom + 8}px`;
		footnotePreviewStyle = `left:${left}px;max-width:${width}px;${vertical}`;
	}

	function hideFootnotePreview() {
		footnotePreviewHtml = "";
	}

	// ===== Image lightbox =====
	let lightboxSrc = "";
	let lightboxAlt = "";
	let lightboxScale = 1;

	function openLightbox(img: HTMLImageElement) {
		lightboxSrc = img.currentSrc || img.src;
		lightboxAlt = img.alt || "";
		lightboxScale = 1;
	}

	function closeLightbox() {
		lightboxSrc = "";
		lightboxAlt = "";
		lightboxScale = 1;
	}

	function handleLightboxWheel(e: WheelEvent) {
		e.preventDefault();
		const factor = e.deltaY < 0 ? 1.12 : 1 / 1.12;
		lightboxScale = Math.min(6, Math.max(0.2, lightboxScale * factor));
	}

	// Reading mode can hide chrome; temporary / non-book files keep a discoverable back control.
	let topbarRevealed = false;
	let topbarHideTimer: ReturnType<typeof setTimeout> | null = null;
	// true when opened as a single file (not library book chapter)
	let isTemporaryReading = false;

	function revealTopbar() {
		if (topbarHideTimer) {
			clearTimeout(topbarHideTimer);
			topbarHideTimer = null;
		}
		topbarRevealed = true;
	}

	function scheduleTopbarHide() {
		// Keep chrome visible for temporary files so "back to shelf" is always findable.
		if (isTemporaryReading || !activeBook) return;
		if (topbarHideTimer) clearTimeout(topbarHideTimer);
		topbarHideTimer = setTimeout(() => {
			topbarRevealed = false;
			topbarHideTimer = null;
		}, 350);
	}

	const FOCUS_ANCHOR_RATIO = 0.5;
	const FOCUS_EDGE_SPACE_RATIO = 0.34;
	const FOCUS_EDGE_SPACE_MIN = 140;
	const FOCUS_WHEEL_STEP = 48;
	const FOCUS_SCROLL_ACTIVE_MS = 120;
	const FOCUS_LONG_BLOCK_RATIO = 0.78;
	const FOCUS_LONG_BLOCK_EDGE_RATIO = 0.12;
	const FOCUS_LONG_BLOCK_EDGE_MIN = 48;
	const FOCUS_LONG_BLOCK_EDGE_MAX = 140;
	const FOCUS_SPOTLIGHT_MAX_RATIO = 0.62;
	const FOCUS_SPOTLIGHT_MIN = 48;
	const FOCUS_PROGRAMMATIC_SCROLL_SETTLE_MS = 180;
	const FOCUS_KEY_SCROLL_RATIO = 1.26;
	const FOCUS_KEY_SCROLL_MIN_SPEED = 780;
	const FOCUS_KEY_SCROLL_MAX_SPEED = 1260;
	const FOCUS_KEY_SCROLL_MAX_FRAME_MS = 32;
	const ARTICLE_LINK_OPEN_DELAY_MS = 240;
	let focusEdgeSpace = 0;
	let focusWheelDelta = 0;
	let focusWheelResetTimer: ReturnType<typeof setTimeout> | null = null;
	let lastFocusedIdx = -1;

	function isTextInputTarget(target: EventTarget | null) {
		const el = target instanceof HTMLElement ? target : null;
		return !!el?.closest("input, textarea, select, [contenteditable='true']");
	}

	function isMarkdownPath(path: string): boolean {
		const lower = path.toLowerCase();
		return lower.endsWith('.md') || lower.endsWith('.markdown');
	}

	function rejectPendingMarkdownRenders(error: Error) {
		pendingMarkdownRenders.forEach(({ reject }) => reject(error));
		pendingMarkdownRenders.clear();
	}

	function getMarkdownWorker() {
		if (markdownWorkerFailed || typeof Worker === "undefined") return null;
		if (markdownWorker) return markdownWorker;

		try {
			markdownWorker = new Worker(
				new URL("../lib/render/markdown.worker.ts", import.meta.url),
				{ type: "module" },
			);
			markdownWorker.onmessage = (
				event: MessageEvent<
					| { id: number; result: RenderedMarkdownDocument }
					| { id: number; error: string }
				>,
			) => {
				const pending = pendingMarkdownRenders.get(event.data.id);
				if (!pending) return;
				pendingMarkdownRenders.delete(event.data.id);
				if ("result" in event.data) {
					pending.resolve(event.data.result);
				} else {
					pending.reject(new Error(event.data.error));
				}
			};
			markdownWorker.onerror = (event) => {
				markdownWorkerFailed = true;
				markdownWorker?.terminate();
				markdownWorker = null;
				rejectPendingMarkdownRenders(new Error(event.message || "Markdown worker failed"));
			};
		} catch (err) {
			markdownWorkerFailed = true;
			console.warn("Markdown worker unavailable.", err);
			return null;
		}

		return markdownWorker;
	}

	async function renderMarkdownForUi(source: string): Promise<RenderedMarkdownDocument> {
		const worker = getMarkdownWorker();
		if (worker) {
			try {
				const id = nextRenderRequestId++;
				return await new Promise<RenderedMarkdownDocument>((resolve, reject) => {
					pendingMarkdownRenders.set(id, { resolve, reject });
					worker.postMessage({ id, source });
				});
			} catch (err) {
				markdownWorkerFailed = true;
				markdownWorker?.terminate();
				markdownWorker = null;
				console.warn("Markdown worker render failed.", err);
			}
		}

		const { renderMarkdownDocument } = await import("$lib/render/markdown");
		return renderMarkdownDocument(source);
	}

	function resolveRenderedHtmlAssets(html: string, path: string | null): string {
		if (!path) return html;
		return resolveMarkdownImageSources(html, path, convertFileSrc);
	}

	// ===== Display layer: KaTeX styles + Mermaid diagrams (both lazy) =====
	let katexCssLoaded = false;
	function ensureKatexCss(html: string) {
		if (katexCssLoaded || !html.includes("katex")) return;
		katexCssLoaded = true;
		void import("katex/dist/katex.min.css");
	}
	$: ensureKatexCss($renderedHtml);

	let mermaidModule: any = null;
	let nextMermaidId = 1;

	function getMermaidTheme() {
		return isLightTheme ? "neutral" : "dark";
	}

	async function renderMermaidBlocks() {
		const article = getArticleElement();
		if (!article) return;
		const codes = Array.from(article.querySelectorAll("pre > code.language-mermaid"));
		if (codes.length === 0) return;

		try {
			if (!mermaidModule) {
				mermaidModule = (await import("mermaid")).default;
			}
		} catch (err) {
			console.warn("Mermaid unavailable:", err);
			return;
		}

		mermaidModule.initialize({ startOnLoad: false, theme: getMermaidTheme() });
		let replaced = false;
		for (const code of codes) {
			const pre = code.closest("pre");
			if (!pre || !pre.isConnected) continue;
			const src = code.textContent ?? "";
			const renderId = `mmbook-mermaid-${nextMermaidId++}`;
			try {
				const { svg } = await mermaidModule.render(renderId, src);
				const container = document.createElement("div");
				container.className = "mermaid-diagram";
				container.dataset.mermaidSource = src;
				const start = pre.getAttribute("data-source-start");
				const end = pre.getAttribute("data-source-end");
				if (start) container.setAttribute("data-source-start", start);
				if (end) container.setAttribute("data-source-end", end);
				container.innerHTML = svg;
				pre.replaceWith(container);
				replaced = true;
			} catch (err) {
				console.warn("Mermaid render failed:", err);
				document.getElementById(`d${renderId}`)?.remove();
			}
		}

		if (replaced) {
			if ($focusMode || $searchQuery.trim()) {
				reindexAfterArticleDomChange();
			} else {
				invalidateFocusMetrics();
				clearFocusBlockIndex();
			}
		}
	}

	// Mermaid bakes theme colors into the SVG — re-render on theme switch.
	let lastMermaidThemeName = "";
	$: if ($currentTheme.name !== lastMermaidThemeName) {
		lastMermaidThemeName = $currentTheme.name;
		void rethemeMermaidDiagrams();
	}

	async function rethemeMermaidDiagrams() {
		if (!mermaidModule) return;
		const article = getArticleElement();
		const containers = article
			? (Array.from(article.querySelectorAll(".mermaid-diagram")) as HTMLElement[])
			: [];
		if (containers.length === 0) return;
		mermaidModule.initialize({ startOnLoad: false, theme: getMermaidTheme() });
		for (const container of containers) {
			const src = container.dataset.mermaidSource ?? "";
			if (!src) continue;
			const renderId = `mmbook-mermaid-${nextMermaidId++}`;
			try {
				const { svg } = await mermaidModule.render(renderId, src);
				container.innerHTML = svg;
			} catch {
				document.getElementById(`d${renderId}`)?.remove();
			}
		}
	}

	function getExternalUrlToOpen(rawHref: string | null): string | null {
		if (!rawHref || !/^[a-z][a-z0-9+.-]*:/i.test(rawHref)) return null;
		try {
			const url = new URL(rawHref, window.location.href);
			if (["http:", "https:", "mailto:", "tel:"].includes(url.protocol)) {
				return url.href;
			}
		} catch {
			// Ignore malformed links and let the browser keep its default behavior.
		}
		return null;
	}

	function cancelPendingArticleLinkOpen() {
		if (!pendingArticleLinkOpenTimer) return;
		clearTimeout(pendingArticleLinkOpenTimer);
		pendingArticleLinkOpenTimer = null;
		pendingArticleLinkOpenUrl = "";
	}

	function handleArticleLinkClick(event: MouseEvent, link: HTMLAnchorElement): boolean {
		const rawHref = link.getAttribute("href");

		// In-document anchors go through the same jump logic as the TOC, so focus
		// mode stays anchored and long jumps don't fly.
		if (rawHref && rawHref.startsWith("#")) {
			event.preventDefault();
			event.stopPropagation();
			hideFootnotePreview();
			scrollToHeading(decodeURIComponent(rawHref.slice(1)));
			return true;
		}

		const externalUrl = getExternalUrlToOpen(rawHref);
		if (!externalUrl) return false;

		event.preventDefault();
		event.stopPropagation();

		cancelPendingArticleLinkOpen();
		if (event.detail > 1) return true;

		pendingArticleLinkOpenUrl = externalUrl;
		pendingArticleLinkOpenTimer = setTimeout(() => {
			const url = pendingArticleLinkOpenUrl;
			pendingArticleLinkOpenTimer = null;
			pendingArticleLinkOpenUrl = "";
			void openUrl(url).catch((err) => {
				console.error("Failed to open external URL:", err);
			});
		}, ARTICLE_LINK_OPEN_DELAY_MS);
		return true;
	}

	function updateWindowTitle(name: string) {
		void getCurrentWebviewWindow()
			.setTitle(name ? `${name} — 沉浸阅读` : "沉浸阅读")
			.catch(() => {});
	}

	// One-time hint after the first file open: buttons are gone, keys matter.
	let firstOpenHintVisible = false;
	let firstOpenHintTimer: ReturnType<typeof setTimeout> | null = null;
	function maybeShowFirstOpenHint() {
		try {
			if (localStorage.getItem("mmbook-hint-shown")) return;
			localStorage.setItem("mmbook-hint-shown", "1");
		} catch {
			return;
		}
		firstOpenHintVisible = true;
		firstOpenHintTimer = setTimeout(() => {
			firstOpenHintVisible = false;
			firstOpenHintTimer = null;
		}, 9000);
	}

	// Quiet transient notice (e.g. "updated in background").
	let appNoticeText = "";
	let appNoticeTimer: ReturnType<typeof setTimeout> | null = null;
	function showAppNotice(text: string) {
		appNoticeText = text;
		if (appNoticeTimer) clearTimeout(appNoticeTimer);
		appNoticeTimer = setTimeout(() => {
			appNoticeText = "";
			appNoticeTimer = null;
		}, 5000);
	}

	async function refreshLibrary() {
		libraryLoading = true;
		try {
			appSettings = await invoke<AppSettings>("get_app_settings");
			const scan = await invoke<{ books: BookSummary[]; issues: LibraryIssue[]; writable: boolean }>("scan_library");
			libraryBooks = scan.books;
			libraryIssues = scan.issues;
			libraryWritable = scan.writable;
			try {
				temporaryItems = await invoke<TemporaryItem[]>("list_temporary_content");
			} catch {
				temporaryItems = [];
			}
			try {
				trashItems = await invoke<TrashItem[]>("list_trash");
			} catch {
				trashItems = [];
			}
		} catch (error) {
			libraryBooks = [];
			libraryIssues = [{ path: appSettings?.libraryRoot ?? "书库", message: String(error) }];
			libraryWritable = false;
		} finally {
			libraryLoading = false;
		}
	}

	async function importFolderToLibrary() {
		const selected = await open({ directory: true, multiple: false, title: "选择 Markdown 文件夹" });
		if (!selected || Array.isArray(selected)) return;
		try {
			const manifest = await invoke<{ bookId: string }>("import_markdown_folder", { path: selected });
			await refreshLibrary();
			await openLibraryBook(manifest.bookId);
		} catch (error) {
			showAppNotice(`导入失败：${String(error)}`);
		}
	}

	async function chooseLibraryRoot() {
		const selected = await open({ directory: true, multiple: false, title: "选择沉浸阅读书库" });
		if (!selected || Array.isArray(selected)) return;
		try {
			const current = appSettings ?? await invoke<AppSettings>("get_app_settings");
			const next: AppSettings = { ...current, libraryRoot: selected };
			await invoke("update_app_settings", { value: next });
			appSettings = next;
			await refreshLibrary();
			showAppNotice("书库位置已更新");
		} catch (error) {
			showAppNotice(`无法更新书库：${String(error)}`);
		}
	}

	async function launchCompanionTool(tool: "zhihu" | "podcast") {
		try {
			const result = await invoke<{ message: string; url?: string }>("launch_companion_tool", { tool });
			showAppNotice(result.message);
			if (result.url) {
				setTimeout(() => void openUrl(result.url!).catch(() => {}), 900);
			}
		} catch (error) {
			showAppNotice(`无法启动工具：${String(error)}`);
		}
	}

	async function removeLibraryBook(bookId: string, title: string, chapterCount: number) {
		const ok = window.confirm(
			`将《${title}》移出书架？\n\n${chapterCount} 篇内容会移到书库 .trash，可手动恢复，不会立刻永久删除。`
		);
		if (!ok) return;
		try {
			const message = await invoke<string>("remove_book", { bookId });
			if (activeBook?.manifest.bookId === bookId) {
				await returnToBookshelf();
			} else {
				await refreshLibrary();
			}
			showAppNotice(message);
		} catch (error) {
			showAppNotice(`移出失败：${String(error)}`);
		}
	}

	async function deleteLibraryBook(bookId: string, title: string, chapterCount: number) {
		const ok = window.confirm(
			`永久删除《${title}》？\n\n将删除磁盘上的 ${chapterCount} 篇 Markdown 与阅读进度，不可恢复。`
		);
		if (!ok) return;
		const again = window.confirm("再次确认：此操作不可撤销。");
		if (!again) return;
		try {
			const message = await invoke<string>("delete_book", { bookId });
			if (activeBook?.manifest.bookId === bookId) {
				await returnToBookshelf();
			} else {
				await refreshLibrary();
			}
			showAppNotice(message);
		} catch (error) {
			showAppNotice(`删除失败：${String(error)}`);
		}
	}

	async function openBrowserReader(bookId: string) {
		try {
			if (editingParagraph && !(await finishEdit())) return;
			await flushSaveState();
			if (flowReaderSession) {
				await invoke("close_reader_session", { sessionId: flowReaderSession.sessionId });
			}
			flowReaderSession = await invoke<{ sessionId: string; url: string }>(
				"start_reader_session",
				{ bookId }
			);
		} catch (error) {
			showAppNotice(`无法启动连读：${String(error)}`);
		}
	}

	async function startPodcastTask(taskId: string) {
		try {
			await invoke("start_podcast_task", { taskId });
			showAppNotice("播客任务已启动");
		} catch (error) {
			showAppNotice(`无法启动播客任务：${String(error)}`);
		}
	}

	async function restartPodcastTask(taskId: string) {
		try {
			await invoke("restart_podcast_task", { taskId });
			showAppNotice("已创建新的 Podcast revision");
			await refreshAcquisitionSnapshot();
		} catch (error) {
			showAppNotice(`无法重新开始播客任务：${String(error)}`);
		}
	}

	async function openPodcastTaskResult(taskId: string) {
		try {
			const detail = await invoke<BookDetail>("open_task_result", { taskId });
			await openLibraryBook(detail.manifest.bookId);
		} catch (error) {
			showAppNotice(`无法打开播客结果：${String(error)}`);
		}
	}

	async function controlPodcastTask(
		taskId: string,
		action: "pause" | "resume" | "cancel" | "cancel_and_discard",
		expectedRevision: number
	) {
		if (action === "cancel_and_discard" && !window.confirm("取消并丢弃该播客任务及缓存？")) return;
		try {
			await invoke("control_podcast_task", {
				taskId,
				action,
				expectedRevision,
				requestId: crypto.randomUUID()
			});
			showAppNotice(action === "pause" ? "任务已暂停" : action === "resume" ? "任务已恢复" : "任务已取消");
		} catch (error) {
			showAppNotice(`任务控制失败：${String(error)}`);
		}
	}

	async function refreshTrash(): Promise<void> {
		trashLoading = true;
		try {
			trashItems = await invoke<TrashItem[]>("list_trash");
		} catch (error) {
			const message = error instanceof Error ? error.message : String(error);
			showAppNotice(`无法读取回收站：${message}`);
		} finally {
			trashLoading = false;
		}
	}

	function openTrashPanel(): void {
		trashOpen = true;
		void refreshTrash();
	}

	async function restoreTrashItem(item: TrashItem): Promise<void> {
		try {
			await invoke("restore_trash_item", {
				trashId: item.trashId,
				expectedRevision: item.revision,
				requestId: crypto.randomUUID()
			});
			await Promise.all([refreshTrash(), refreshLibrary()]);
			showAppNotice(`已恢复《${item.title}》`);
		} catch (error) {
			const message = error instanceof Error ? error.message : String(error);
			showAppNotice(`恢复失败：${message}`);
		}
	}

	async function permanentlyDeleteTrashItem(item: TrashItem): Promise<void> {
		const confirmed = window.confirm(
			`永久删除《${item.title}》？\n\n这会删除回收站中的全部正文、资源与阅读状态，无法恢复。`
		);
		if (!confirmed || !window.confirm("再次确认：只删除这个受管回收站条目。")) return;
		try {
			const result = await invoke<TrashDeleteResult>("permanently_delete_trash_item", {
				trashId: item.trashId,
				expectedRevision: item.revision,
				requestId: crypto.randomUUID()
			});
			await refreshTrash();
			showAppNotice(`已永久删除 ${result.deletedItems} 个项目`);
		} catch (error) {
			const message = error instanceof Error ? error.message : String(error);
			showAppNotice(`永久删除失败：${message}`);
		}
	}

	async function closeFlowReader() {
		const session = flowReaderSession;
		flowReaderSession = null;
		if (!session) return;
		try {
			await invoke("close_reader_session", { sessionId: session.sessionId });
		} catch (error) {
			showAppNotice(`无法关闭连读会话：${String(error)}`);
		}
	}

	async function openLibraryBook(bookId: string) {
		try {
			const detail = await invoke<BookDetail>("open_book", { bookId });
			const index = resolveChapterIndex(detail.manifest.chapters, detail.progress.current, detail.progress.read);
			if (index < 0) {
				showAppNotice("这本文集还没有可读章节");
				return;
			}
			const chapter = detail.manifest.chapters[index];
			const path = await invoke<string>("get_book_chapter_path", { bookId, chapterId: chapter.id });
			const opened = await openFile(path, {
				bookChapter: true,
				restoreRatio: detail.progress.position,
				suppressRecent: true,
			});
			if (!opened) return;
			activeBook = detail;
			activeChapterIndex = index;
			tocItems = chapterTocItems(detail.manifest.chapters);
			updateWindowTitle(`${detail.manifest.title} · ${chapter.title}`);
		} catch (error) {
			showAppNotice(`无法打开文集：${String(error)}`);
		}
	}

	async function persistActiveBookProgress(forceComplete = false) {
		if (!activeBook || activeChapterIndex < 0) return;
		const chapter = activeBook.manifest.chapters[activeChapterIndex];
		if (!chapter) return;
		const isLast = activeChapterIndex === activeBook.manifest.chapters.length - 1;
		const completed = forceComplete || readingProgress >= 0.999 || (isLast && readingProgress >= 0.95);
		const progress = nextReadingState(activeBook.progress, chapter.id, readingProgress, completed);
		activeBook = { ...activeBook, progress };
		await invoke("save_book_progress", { bookId: activeBook.manifest.bookId, progress });
	}

	async function preloadNextBookChapter() {
		if (!activeBook || activeChapterIndex < 0 || readingProgress < 0.84 || preloadedChapter) return;
		const index = activeChapterIndex + 1;
		const chapter = activeBook.manifest.chapters[index];
		if (!chapter) return;
		const bookId = activeBook.manifest.bookId;
		try {
			const path = await invoke<string>("get_book_chapter_path", { bookId, chapterId: chapter.id });
			const result = await invoke<{ content: string; encoding: string }>("read_markdown_file", { path });
			const rendered = await renderMarkdownForUi(result.content);
			if (activeBook?.manifest.bookId === bookId && activeChapterIndex + 1 === index) {
				preloadedChapter = { bookId, index, path, result, rendered };
			}
		} catch {
			// The normal open path will show a concrete error if the next chapter is unavailable.
		}
	}

	async function openBookChapter(index: number, position = 0, completePrevious = false) {
		if (!activeBook || chapterSwitching) return;
		const chapter = activeBook.manifest.chapters[index];
		if (!chapter) return;
		chapterSwitching = true;
		try {
			if (index !== activeChapterIndex && activeChapterIndex >= 0) {
				const previous = activeBook.manifest.chapters[activeChapterIndex];
				let progress = nextReadingState(activeBook.progress, previous.id, readingProgress, completePrevious);
				progress = { ...progress, current: chapter.id, position, updated: new Date().toISOString() };
				activeBook = { ...activeBook, progress };
				await invoke("save_book_progress", { bookId: activeBook.manifest.bookId, progress });
			}
			const cached = preloadedChapter?.bookId === activeBook.manifest.bookId && preloadedChapter.index === index
				? preloadedChapter
				: null;
			const path = cached?.path ?? await invoke<string>("get_book_chapter_path", {
				bookId: activeBook.manifest.bookId,
				chapterId: chapter.id,
			});
			const opened = await openFile(path, {
				bookChapter: true,
				restoreRatio: position,
				suppressRecent: true,
				skipFlush: true,
				preloaded: cached ? { result: cached.result, rendered: cached.rendered } : undefined,
			});
			if (!opened) return;
			activeChapterIndex = index;
			tocItems = chapterTocItems(activeBook.manifest.chapters);
			preloadedChapter = null;
			updateWindowTitle(`${activeBook.manifest.title} · ${chapter.title}`);
		} finally {
			chapterSwitching = false;
		}
	}

	async function advanceBookChapter() {
		if (!activeBook || chapterSwitching) return;
		const nextIndex = activeChapterIndex + 1;
		const next = activeBook.manifest.chapters[nextIndex];
		if (!next) {
			await persistActiveBookProgress(true);
			return;
		}
		await openBookChapter(nextIndex, 0, true);
	}

	async function returnToBookshelf() {
		await flushSaveState();
		activeBook = null;
		trashOpen = false;
		activeChapterIndex = -1;
		preloadedChapter = null;
		isTemporaryReading = false;
		topbarRevealed = false;
		$currentFilePath = "";
		$markdownSource = "";
		$renderedHtml = "";
		fileName = "";
		tocItems = [];
		readingProgress = 0;
		updateWindowTitle("");
		await refreshLibrary();
	}

	function clearFocusScrollActive() {
		isFocusScrollActive = false;
		focusLockedIndex = null;
		stopFocusKeyScroll({ restoreStyles: false });
		clearProgrammaticFocusScrollLock();
		if (focusScrollEndTimer) {
			clearTimeout(focusScrollEndTimer);
			focusScrollEndTimer = null;
		}
	}

	function markFocusScrollActive() {
		if (!isFocusScrollActive) {
			isFocusScrollActive = true;
		}
		if (focusScrollEndTimer) {
			clearTimeout(focusScrollEndTimer);
		}
		focusScrollEndTimer = setTimeout(() => {
			isFocusScrollActive = false;
			focusLockedIndex = null;
			focusScrollEndTimer = null;
		}, FOCUS_SCROLL_ACTIVE_MS);
	}

	function clearProgrammaticFocusScrollLock() {
		if (focusProgrammaticScrollTimer) {
			clearTimeout(focusProgrammaticScrollTimer);
			focusProgrammaticScrollTimer = null;
		}
		focusProgrammaticScrollIndex = null;
		focusProgrammaticScrollTarget = null;
	}

	function scheduleProgrammaticFocusScrollUnlock(delay = FOCUS_PROGRAMMATIC_SCROLL_SETTLE_MS) {
		if (focusProgrammaticScrollTimer) {
			clearTimeout(focusProgrammaticScrollTimer);
		}
		focusProgrammaticScrollTimer = setTimeout(() => {
			focusProgrammaticScrollTimer = null;
			focusProgrammaticScrollIndex = null;
			focusProgrammaticScrollTarget = null;
		}, delay);
	}

	function lockProgrammaticFocusScroll(index: number, targetScrollTop: number) {
		if (index < 0) return;
		focusProgrammaticScrollIndex = index;
		focusProgrammaticScrollTarget = targetScrollTop;
		focusLockedIndex = index;
		scheduleProgrammaticFocusScrollUnlock();
	}

	function getScheduledFocusIndex() {
		return focusProgrammaticScrollIndex ?? focusLockedIndex ?? undefined;
	}

	function getFocusKeyScrollSpeed() {
		if (!contentEl) return FOCUS_KEY_SCROLL_MIN_SPEED;
		return Math.max(
			FOCUS_KEY_SCROLL_MIN_SPEED,
			Math.min(FOCUS_KEY_SCROLL_MAX_SPEED, contentEl.clientHeight * FOCUS_KEY_SCROLL_RATIO),
		);
	}

	function setFocusImmediateScrollMode(enabled: boolean) {
		if (!contentEl) return;
		if (enabled) {
			if (focusInlineScrollBehavior === null) {
				focusInlineScrollBehavior = contentEl.style.scrollBehavior;
			}
			contentEl.style.scrollBehavior = "auto";
			return;
		}

		if (focusInlineScrollBehavior !== null) {
			contentEl.style.scrollBehavior = focusInlineScrollBehavior;
			focusInlineScrollBehavior = null;
		}
	}

	function activateFocusKeyReadableMode() {
		const wasActive = isFocusKeyScrollActive;
		isFocusKeyScrollActive = true;
		setFocusImmediateScrollMode(true);
		if (!wasActive) {
			scheduleFocusUpdate();
		}
	}

	function deactivateFocusKeyReadableMode({ restoreStyles = true }: { restoreStyles?: boolean } = {}) {
		const wasActive = isFocusKeyScrollActive;
		isFocusKeyScrollActive = false;
		setFocusImmediateScrollMode(false);
		if (wasActive && restoreStyles && $focusMode) {
			scheduleFocusUpdate();
		}
	}

	function startFocusKeyScroll(direction: -1 | 1) {
		if (!contentEl || !$focusMode) return;

		if (focusKeyScrollDirection !== direction) {
			focusKeyScrollDirection = direction;
			focusKeyScrollLastTime = 0;
		}

		clearProgrammaticFocusScrollLock();
		focusLockedIndex = null;
		markFocusScrollActive();
		activateFocusKeyReadableMode();

		if (focusKeyScrollFrame === null) {
			focusKeyScrollFrame = requestAnimationFrame(stepFocusKeyScroll);
		}
	}

	function stopFocusKeyScroll({ restoreStyles = true }: { restoreStyles?: boolean } = {}) {
		const wasActive = isFocusKeyScrollActive;
		if (focusKeyScrollFrame !== null) {
			cancelAnimationFrame(focusKeyScrollFrame);
			focusKeyScrollFrame = null;
		}
		focusKeyScrollDirection = 0;
		focusKeyScrollLastTime = 0;
		deactivateFocusKeyReadableMode({ restoreStyles: restoreStyles && wasActive });
	}

	function stepFocusKeyScroll(timestamp: number) {
		focusKeyScrollFrame = null;
		if (!contentEl || !$focusMode || focusKeyScrollDirection === 0) {
			stopFocusKeyScroll();
			return;
		}

		const elapsedMs = focusKeyScrollLastTime === 0
			? 16
			: Math.min(timestamp - focusKeyScrollLastTime, FOCUS_KEY_SCROLL_MAX_FRAME_MS);
		focusKeyScrollLastTime = timestamp;

		const maxScrollTop = Math.max(0, contentEl.scrollHeight - contentEl.clientHeight);
		const currentScrollTop = contentEl.scrollTop;
		const delta = focusKeyScrollDirection * getFocusKeyScrollSpeed() * (elapsedMs / 1000);
		const nextScrollTop = Math.max(0, Math.min(maxScrollTop, currentScrollTop + delta));

		if (Math.abs(nextScrollTop - currentScrollTop) < 0.5) {
			stopFocusKeyScroll();
			return;
		}

		contentEl.scrollTop = nextScrollTop;
		markFocusScrollActive();

		focusKeyScrollFrame = requestAnimationFrame(stepFocusKeyScroll);
	}

	function scheduleFocusUpdate(preferredIdx?: number) {
		if (!$focusMode) return;
		if (preferredIdx !== undefined) {
			queuedFocusIndex = preferredIdx;
		}
		if (focusUpdateFrame !== null) return;
		focusUpdateFrame = requestAnimationFrame(() => {
			focusUpdateFrame = null;
			const nextIdx = queuedFocusIndex;
			queuedFocusIndex = undefined;
			updateFocusParagraph(nextIdx);
			updateSpotlightPosition();
		});
	}

	// Entering/leaving focus mode reflows the whole document (the article font
	// scales up/down), so raw scrollTop no longer points at the same content.
	// Capture the block under the anchor line (plus the ratio inside it) before
	// the reflow, and scroll it back to the anchor line afterwards.
	type ViewportAnchor = { element: HTMLElement; ratio: number };
	let pendingExitAnchor: ViewportAnchor | null = null;

	function captureViewportAnchor(): ViewportAnchor | null {
		const article = getArticleElement();
		if (!article || !contentEl) return null;
		const anchorY = getFocusAnchorY();
		let candidate: HTMLElement | null = null;
		for (const child of Array.from(article.children)) {
			if (!(child instanceof HTMLElement) || child.offsetHeight <= 0) continue;
			const rect = child.getBoundingClientRect();
			if (rect.top <= anchorY) {
				candidate = child;
			} else {
				break;
			}
		}
		if (!candidate) return null;
		const rect = candidate.getBoundingClientRect();
		const ratio = rect.height > 0 ? (anchorY - rect.top) / rect.height : 0;
		return { element: candidate, ratio: Math.max(0, Math.min(1, ratio)) };
	}

	function restoreViewportAnchor(anchor: ViewportAnchor | null) {
		if (!anchor || !contentEl || !anchor.element.isConnected) return;
		const rect = anchor.element.getBoundingClientRect();
		const currentY = rect.top + anchor.ratio * rect.height;
		contentEl.scrollTop += currentY - getFocusAnchorY();
	}

	async function toggleFocusMode(nextState = !$focusMode) {
		if (nextState === $focusMode) return;
		if (nextState && !$currentFilePath) return;
		if (nextState) {
			const anchor = captureViewportAnchor();
			$focusMode = true;
			await tick();
			enterFocusMode(anchor);
			return;
		}
		const exited = await exitFocusMode();
		if (exited) {
			$focusMode = false;
			await tick();
			restoreViewportAnchor(pendingExitAnchor);
			pendingExitAnchor = null;
			invalidateFocusMetrics();
		}
	}

	onMount(() => {
		const checkInitialFile = () => {
			if ((window as any).__INITIAL_FILE__) {
				openFile((window as any).__INITIAL_FILE__);
				delete (window as any).__INITIAL_FILE__;
			}
		};
		checkInitialFile();
		setTimeout(checkInitialFile, 200);

		const handleKeydown = (e: KeyboardEvent) => {
			if (isModKey(e) && e.key === "o") {
				e.preventDefault();
				openFileDialog();
				return;
			}

			if (isModKey(e) && e.key === "f") {
				e.preventDefault();
				$searchOpen = !$searchOpen;
				if (!$searchOpen) {
					$searchQuery = "";
					clearSearchHighlights();
				}
				// 打开时的聚焦由 SearchBar 组件自身完成
				return;
			}

		if (e.key === "Escape") {
			// Esc only cancels the innermost layer: an active edit reverts and we
			// stay wherever we were (including focus mode).
			if (editingParagraph) {
				e.preventDefault();
				cancelEdit();
			} else if (lightboxSrc) {
				e.preventDefault();
				closeLightbox();
			} else if ($searchOpen) {
				$searchOpen = false;
				$searchQuery = "";
				clearSearchHighlights();
			} else if ($tocOpen) {
				$tocOpen = false;
			} else if ($settingsOpen) {
				$settingsOpen = false;
			} else if ($focusMode) {
				e.preventDefault();
				void toggleFocusMode(false);
			}
			return;
		}

			if (isModKey(e) && e.key === "t") {
				e.preventDefault();
				$tocOpen = !$tocOpen;
				return;
			}

			if (e.key === "F11" || (isModKey(e) && e.shiftKey && e.key === "F")) {
				e.preventDefault();
				void toggleFocusMode(!$focusMode);
				return;
			}

			if ($focusMode && !e.ctrlKey && !e.metaKey && !e.altKey && !isTextInputTarget(e.target)) {
				// Short taps move block-by-block; OS key repeat scrolls continuously.
				if (e.key === "ArrowUp") {
					e.preventDefault();
					if (e.repeat) {
						startFocusKeyScroll(-1);
					} else {
						stopFocusKeyScroll();
						moveFocus(-1);
					}
					return;
				}
				if (e.key === "ArrowDown") {
					e.preventDefault();
					if (e.repeat) {
						startFocusKeyScroll(1);
					} else {
						stopFocusKeyScroll();
						moveFocus(1);
					}
					return;
				}
				if (e.key === "ArrowLeft") {
					e.preventDefault();
					moveFocus(-1);
					return;
				}
				if (e.key === "ArrowRight") {
					e.preventDefault();
					moveFocus(1);
					return;
				}
			}

			if ($searchOpen && e.key === "Enter") {
				e.preventDefault();
				if (e.shiftKey) {
					navigateSearch(-1);
				} else {
					navigateSearch(1);
				}
				return;
			}

			if (isModKey(e) && (e.key === "=" || e.key === "+")) {
				e.preventDefault();
				adjustFontScale(1);
				return;
			}
			if (isModKey(e) && e.key === "-") {
				e.preventDefault();
				adjustFontScale(-1);
				return;
			}
			if (isModKey(e) && e.key === "0") {
				e.preventDefault();
				setFontScale(1);
				return;
			}

			if (isModKey(e) && e.key === ",") {
				e.preventDefault();
				$settingsOpen = !$settingsOpen;
			}
		};

		const handleKeyup = (e: KeyboardEvent) => {
			if ((e.key === "ArrowUp" || e.key === "ArrowDown") && focusKeyScrollDirection !== 0) {
				stopFocusKeyScroll();
			}
		};

		const handleWindowBlur = () => {
			stopFocusKeyScroll();
		};

		const handleScroll = () => {
			if (!contentEl) return;
			if (codeCopyVisible) scheduleCodeCopyHide(0);
			const scrollTop = contentEl.scrollTop;
			const scrollHeight = contentEl.scrollHeight - contentEl.clientHeight;
			readingProgress = scrollHeight > 0 ? scrollTop / scrollHeight : 0;
			if (activeBook && readingProgress >= 0.84) void preloadNextBookChapter();

			if ($focusMode) {
				markFocusScrollActive();
				if (focusProgrammaticScrollIndex !== null) {
					const targetReached =
						focusProgrammaticScrollTarget !== null &&
						Math.abs(scrollTop - focusProgrammaticScrollTarget) < 1;
					scheduleProgrammaticFocusScrollUnlock(
						targetReached ? FOCUS_SCROLL_ACTIVE_MS : FOCUS_PROGRAMMATIC_SCROLL_SETTLE_MS,
					);
				}
				scheduleFocusUpdate(getScheduledFocusIndex());
			}

			// Update edit orbit particle position when scrolling during edit in focus mode
			if (editingParagraph && isEditingInDarkFocus) {
				updateEditOrbitPosition();
			}

			if ($currentFilePath && !$focusMode && !editingParagraph) {
				pulseStatusLine();
			}

			if ($currentFilePath && !saveStateTimer) {
				saveStateTimer = setTimeout(() => {
					saveStateTimer = null;
					void saveState();
				}, 5000);
			}
		};

		const handleWheel = (e: WheelEvent) => {
			// Ctrl+wheel (and touchpad pinch on Windows) zooms the reader font.
			if (e.ctrlKey) {
				e.preventDefault();
				if (Math.abs(e.deltaY) > 0.01) {
					adjustFontScale(e.deltaY < 0 ? 1 : -1);
				}
				return;
			}
			if (contentEl && activeBook && !$focusMode && e.deltaY > 0) {
				const remaining = contentEl.scrollHeight - contentEl.clientHeight - contentEl.scrollTop;
				if (remaining <= 1) {
					e.preventDefault();
					void advanceBookChapter();
					return;
				}
			}
			if (!contentEl || !$focusMode) return;
			const units = getFocusUnits();
			if (units.length === 0 || Math.abs(e.deltaY) < 0.01) return;

			const direction = e.deltaY > 0 ? 1 : -1;
			if (allowNativeLongBlockWheel(units, direction)) {
				focusWheelDelta = 0;
				if (focusWheelResetTimer) {
					clearTimeout(focusWheelResetTimer);
					focusWheelResetTimer = null;
				}
				return;
			}

			e.preventDefault();
			focusWheelDelta += e.deltaY;

			if (focusWheelResetTimer) {
				clearTimeout(focusWheelResetTimer);
			}
			focusWheelResetTimer = setTimeout(() => {
				focusWheelDelta = 0;
				focusWheelResetTimer = null;
			}, 120);

			if (Math.abs(focusWheelDelta) < FOCUS_WHEEL_STEP) return;

			const stepDirection = focusWheelDelta > 0 ? 1 : -1;
			focusWheelDelta = 0;
			moveFocus(stepDirection);
		};

		const handleContentClick = (e: MouseEvent) => {
			const target = e.target as HTMLElement | null;
			if (!target || !contentEl.querySelector(".article")?.contains(target)) return;

			const link = target.closest("a[href]") as HTMLAnchorElement | null;
			if (link && handleArticleLinkClick(e, link)) {
				return;
			}

			// Plain images open in the lightbox (linked images keep link behavior).
			if (!link && target instanceof HTMLImageElement && !editingParagraph) {
				e.preventDefault();
				openLightbox(target);
				return;
			}

			if (!$focusMode || editingParagraph) return;
			focusBlockFromInteraction(target);
		};

		const handleDblClick = (e: MouseEvent) => {
			cancelPendingArticleLinkOpen();
			e.preventDefault();
			e.stopPropagation();
			const target = (e.target as HTMLElement)?.closest("p, h1, h2, h3, h4, h5, h6, li, blockquote, pre");
			if (target && contentEl.querySelector(".article")?.contains(target)) {
				startEdit(target as HTMLElement);
			}
		};

		// Tauri drag-and-drop (replaces HTML5 drag-and-drop)
		const appWindow = getCurrentWebviewWindow();
		const unlistenDrop = appWindow.onDragDropEvent((event) => {
			if (event.payload.type === 'drop') {
				const mdFile = event.payload.paths.find(
					(p: string) => isMarkdownPath(p)
				);
				if (mdFile) {
					// openFile will handle canceling any active edit
					openFile(mdFile);
				}
			}
		});

		// macOS: file opened via Apple Event (double-click / Open With)
		const unlistenOpenFile = listen<string>('open-file', (event) => {
			// openFile will handle canceling any active edit
			openFile(event.payload);
		});
		const unlistenTaskEvents = listen<TaskEvent>("acquisition://task-event", (event) => {
			receiveTaskEvent(event.payload);
		});
		const refreshTasksOnResume = () => {
			if (document.visibilityState === "visible") {
				void refreshAcquisitionSnapshot();
			}
		};
		window.addEventListener("focus", refreshTasksOnResume);
		document.addEventListener("visibilitychange", refreshTasksOnResume);
		void refreshAcquisitionSnapshot();

		const handleContentMouseOver = (e: MouseEvent) => {
			const link = getFootnoteLink(e.target);
			if (link) {
				if (footnoteHideTimer) {
					clearTimeout(footnoteHideTimer);
					footnoteHideTimer = null;
				}
				showFootnotePreview(link);
			}

			const pre = (e.target instanceof HTMLElement
				? e.target.closest(".article pre")
				: null) as HTMLElement | null;
			if (pre && !editingParagraph) {
				showCodeCopyButton(pre);
			}
		};

		const handleContentMouseOut = (e: MouseEvent) => {
			if (getFootnoteLink(e.target)) {
				if (footnoteHideTimer) clearTimeout(footnoteHideTimer);
				footnoteHideTimer = setTimeout(() => {
					footnoteHideTimer = null;
					hideFootnotePreview();
				}, 200);
			}
			if (e.target instanceof HTMLElement && e.target.closest(".article pre")) {
				scheduleCodeCopyHide();
			}
		};

		window.addEventListener("keydown", handleKeydown);
		window.addEventListener("keyup", handleKeyup);
		window.addEventListener("blur", handleWindowBlur);
		contentEl?.addEventListener("scroll", handleScroll);
		contentEl?.addEventListener("wheel", handleWheel, { passive: false });
		contentEl?.addEventListener("click", handleContentClick);
		contentEl?.addEventListener("dblclick", handleDblClick);
		contentEl?.addEventListener("mouseover", handleContentMouseOver);
		contentEl?.addEventListener("mouseout", handleContentMouseOut);
		const handleResize = () => {
			invalidateFocusMetrics();
			if ($focusMode) {
				rebuildFocusMetrics();
				scheduleFocusUpdate(lastFocusedIdx >= 0 ? lastFocusedIdx : undefined);
			}
			updateEditOrbitPosition();
		};
		window.addEventListener("resize", handleResize);
		const handleBeforeUnload = () => {
			if ($currentFilePath && contentEl) void flushSaveState();
		};
		window.addEventListener("beforeunload", handleBeforeUnload);

		// Closing the window hides it; tray exit explicitly chooses lease-preserving
		// quit or cancel_and_discard cleanup.
		let isClosing = false;
		const finishExit = async (mode: "hide" | "preserve" | "cancel_and_discard") => {
			try {
				await appWindow.hide();
				if (editingParagraph) {
					await Promise.race([
						finishEdit(),
						new Promise<boolean>((resolve) => setTimeout(() => resolve(false), 1200)),
					]);
				}
				await Promise.race([
					flushSaveState(),
					new Promise<void>((resolve) => setTimeout(resolve, 500)),
				]);
				await closeFlowReader();
				if (mode === "preserve") {
					await invoke("quit_app");
				} else if (mode === "cancel_and_discard") {
					await invoke("cancel_and_discard");
				}
			} catch (err) {
				console.error("Failed to finish app exit:", err);
			} finally {
				isClosing = false;
			}
		};
		const unlistenExit = listen<{ mode?: string }>("request-app-exit", (event) => {
			if (isClosing) return;
			isClosing = true;
			const mode = event.payload?.mode === "cancel_and_discard" ? "cancel_and_discard" : "preserve";
			void finishExit(mode);
		});
		const unlistenClose = appWindow.onCloseRequested((event) => {
			if (isClosing) return;
			event.preventDefault();
			isClosing = true;
			void finishExit("hide");
		});

		void refreshLibrary();

		void loadRecentFiles().then((files) => {
			recentFiles = files;
			void loadContinueInfo();
		});

		// 外部变更自动重载：每 2s 轮询当前文件 mtime；
		// 内容与内存一致（如刚由本应用保存）或正在编辑时不打断。
		// 窗口失焦时降频到 1/5(约 10s)。
		let watchedPath = "";
		let watchedMtime = 0;
		let pollTick = 0;
		const fileWatchTimer = window.setInterval(async () => {
			pollTick += 1;
			if (!document.hasFocus() && pollTick % 5 !== 0) return;
			const path = $currentFilePath;
			if (!path) {
				watchedPath = "";
				return;
			}
			try {
				const mtime = await invoke<number>("get_file_mtime", { path });
				if (watchedPath !== path) {
					watchedPath = path;
					watchedMtime = mtime;
					return;
				}
				if (mtime <= watchedMtime) return;
				watchedMtime = mtime;
				if (editingParagraph) return;
				const result = await invoke<{ content: string; encoding: string }>(
					"read_markdown_file",
					{ path },
				);
				if (result.content === $markdownSource) return;
				const scrollTop = contentEl?.scrollTop ?? 0;
				await openFile(path, activeBook ? {
					bookChapter: true,
					restoreRatio: readingProgress,
					suppressRecent: true,
					skipFlush: true,
				} : {});
				if (activeBook) tocItems = chapterTocItems(activeBook.manifest.chapters);
				if (contentEl) contentEl.scrollTop = scrollTop;
			} catch {
				// 文件被移动/删除等情况：静默忽略，下次轮询再试
			}
		}, 2000);

		return () => {
			window.clearInterval(fileWatchTimer);
			unlistenDrop.then(fn => fn());
			unlistenOpenFile.then(fn => fn());
			unlistenTaskEvents.then(fn => fn());
			unlistenExit.then(fn => fn());
			unlistenClose.then(fn => fn());
			window.removeEventListener("focus", refreshTasksOnResume);
			document.removeEventListener("visibilitychange", refreshTasksOnResume);
			window.removeEventListener("keydown", handleKeydown);
			window.removeEventListener("keyup", handleKeyup);
			window.removeEventListener("blur", handleWindowBlur);
			contentEl?.removeEventListener("scroll", handleScroll);
			contentEl?.removeEventListener("wheel", handleWheel);
			contentEl?.removeEventListener("click", handleContentClick);
			contentEl?.removeEventListener("dblclick", handleDblClick);
			contentEl?.removeEventListener("mouseover", handleContentMouseOver);
			contentEl?.removeEventListener("mouseout", handleContentMouseOut);
			if (footnoteHideTimer) {
				clearTimeout(footnoteHideTimer);
			}
			if (codeCopyHideTimer) {
				clearTimeout(codeCopyHideTimer);
			}
			window.removeEventListener("resize", handleResize);
			window.removeEventListener("beforeunload", handleBeforeUnload);
			if (focusWheelResetTimer) {
				clearTimeout(focusWheelResetTimer);
			}
			if (searchDebounceTimer) {
				clearTimeout(searchDebounceTimer);
			}
			if (saveStateTimer) {
				clearTimeout(saveStateTimer);
			}
			if (topbarHideTimer) {
				clearTimeout(topbarHideTimer);
			}
			if (zoomIndicatorTimer) {
				clearTimeout(zoomIndicatorTimer);
			}
			if (statusLineTimer) {
				clearTimeout(statusLineTimer);
			}
			if (appNoticeTimer) {
				clearTimeout(appNoticeTimer);
			}
			if (firstOpenHintTimer) {
				clearTimeout(firstOpenHintTimer);
			}
			cancelPendingArticleLinkOpen();
			clearFocusScrollActive();
			if (focusUpdateFrame !== null) {
				cancelAnimationFrame(focusUpdateFrame);
				focusUpdateFrame = null;
			}
			markdownWorker?.terminate();
			markdownWorker = null;
			rejectPendingMarkdownRenders(new Error("Component destroyed"));
			const session = flowReaderSession;
			if (session) {
				void invoke("close_reader_session", { sessionId: session.sessionId });
			}
		};
	});

	async function openFileDialog() {
		const selected = await open({
			multiple: false,
			filters: [{ name: "Markdown", extensions: ["md", "markdown"] }],
		});
		if (selected) {
			openFile(selected as string);
		}
	}

	type OpenFileOptions = {
		suppressFailureNotice?: boolean;
		bookChapter?: boolean;
		restoreRatio?: number;
		suppressRecent?: boolean;
		skipFlush?: boolean;
		preloaded?: {
			result: { content: string; encoding: string };
			rendered: RenderedMarkdownDocument;
		};
	};

	async function openFile(path: string, options: OpenFileOptions = {}): Promise<boolean> {
		// Finish any active edit before switching files (saves changes)
		if (editingParagraph) {
			const saved = await finishEdit();
			if (!saved) return false;
		}

		if ($currentFilePath && !options.skipFlush) {
			await flushSaveState();
		}
		if (!options.bookChapter) {
			activeBook = null;
			activeChapterIndex = -1;
			preloadedChapter = null;
			isTemporaryReading = true;
			topbarRevealed = true;
		} else {
			isTemporaryReading = false;
		}

		// Race condition protection: invalidate previous loads
		const loadToken = path + ':' + Date.now();
		currentLoadToken = loadToken;

		$isLoading = true;
		fileError = "";
		const nextFileName = path.split(/[\\/]/).pop() || "";
		let loadSucceeded = false;

		try {
			const result = options.preloaded?.result ?? await invoke<{content: string, encoding: string}>("read_markdown_file", {
				path,
			});
			if (currentLoadToken !== loadToken) return false;

			const rendered = options.preloaded?.rendered ?? await renderMarkdownForUi(result.content);
			if (currentLoadToken !== loadToken) return false;

			$currentFilePath = path;
			fileName = nextFileName;
			updateWindowTitle(
				options.bookChapter
					? nextFileName
					: `临时 · ${nextFileName}`
			);
			if (isTemporaryReading) {
				topbarRevealed = true;
			}
			$markdownSource = result.content;
			fileEncoding = result.encoding;
			$renderedHtml = resolveRenderedHtmlAssets(rendered.html, path);
			tocItems = rendered.toc;
			frontMatterEntries = rendered.frontMatter;
			void tick().then(() => renderMermaidBlocks());

			// Reset focus state when file changes
			stopFocusKeyScroll({ restoreStyles: false });
			lastFocusedIdx = -1;
			focusLockedIndex = null;
			spotlightHeight = 100;
			focusEdgeSpace = 0;
			focusUnits = [];
			focusStyleCache = new WeakMap<HTMLElement, string>();
			styledFocusIndices = new Set<number>();
			focusEntryStickyIndices = null;
			lastFocusRenderSignature = "";
			lastSpotlightVars = new Map<string, string>();
			clearProgrammaticFocusScrollLock();
			invalidateFocusMetrics();

			await tick();
			wrapArticleTables();

			if (options.restoreRatio !== undefined) {
				if (contentEl) {
					const maxScroll = Math.max(0, contentEl.scrollHeight - contentEl.clientHeight);
					contentEl.scrollTop = Math.max(0, Math.min(1, options.restoreRatio)) * maxScroll;
				}
			} else {
				try {
					const state: { scroll_position: number; bookmarks: number[]; progress?: number } =
						await invoke("load_reading_state", { path });
					if (currentLoadToken !== loadToken) return false;
					if (contentEl) {
						const maxScroll = Math.max(0, contentEl.scrollHeight - contentEl.clientHeight);
						contentEl.scrollTop = typeof state.progress === "number" && state.progress > 0
							? Math.max(0, Math.min(1, state.progress)) * maxScroll
							: state.scroll_position;
					}
				} catch {
					// No saved state, start from top
				}
			}

			loadSucceeded = true;
			if (!options.suppressRecent) recordRecentFile(path, nextFileName);
			maybeShowFirstOpenHint();

		} catch (err) {
			if (currentLoadToken !== loadToken) return false;
			console.error("Failed to open file:", err);
			fileError = `无法打开文件：${err instanceof Error ? err.message : String(err)}`;
			if (contentEl) contentEl.scrollTop = 0;
			if (!options.suppressFailureNotice) {
				showAppNotice("无法打开文件，请检查路径或权限");
			}
		} finally {
			if (currentLoadToken === loadToken) {
				$isLoading = false;
			}
			if (loadSucceeded && currentLoadToken === loadToken) {
				await tick();
				if (currentLoadToken === loadToken) {
					if ($searchQuery.trim()) {
						performSearch();
					} else if (!$focusMode) {
						clearFocusBlockIndex();
					}
					if ($focusMode) {
						await tick();
						if (currentLoadToken === loadToken) {
							enterFocusMode();
						}
					}
				}
			}
		}
		return loadSucceeded && currentLoadToken === loadToken;
	}

	function startEdit(el: HTMLElement) {
		// Ignore new edit attempts while already editing to prevent accidental data loss
		if (editingParagraph) return;
		if (!$currentFilePath) return;

		// Every edit works on the block's exact Markdown source line range — the
		// only write-back that can never corrupt other parts of the file.
		const sourceStart = parseInt(el.dataset.sourceStart || '-1');
		const sourceEnd = parseInt(el.dataset.sourceEnd || '-1');
		const sourceBlock = getMarkdownSourceBlock($markdownSource, sourceStart, sourceEnd);
		if (sourceBlock === null) return;

		editingParagraph = el;
		// A sentence-segmented paragraph must edit (and restore) its pristine DOM.
		editUnwrappedSegments = unwrapFocusSegments(el);
		if (editUnwrappedSegments && $focusMode) {
			refreshFocusBlocks();
		}
		originalText = el.innerHTML;
		originalMarkdownBlock = sourceBlock;
		el.textContent = sourceBlock;
		el.setAttribute("contenteditable", "plaintext-only");
		el.classList.add("editing", "editing-markdown-source");
		el.focus();
		placeCaretAtEnd(el);

		el.addEventListener("blur", finishEdit, { once: true });
		el.addEventListener("keydown", handleEditKeydown);
		el.addEventListener("input", updateEditOrbitPosition);

		if ($focusMode) {
			focusBlockFromInteraction(el);
		}

		const isDark = !$currentTheme.name.includes('light');
		if ($focusMode && isDark) {
			updateEditOrbitPosition();
		}
	}

	function updateEditOrbitPosition() {
		if (!editingParagraph) return;
		const el = editingParagraph;
		const isDark = !$currentTheme.name.includes('light');
		if (!$focusMode || !isDark) return;
		const rect = el.getBoundingClientRect();
		const pad = 8;
		editOrbitContainerStyle = `left:${rect.left - pad}px;top:${rect.top - pad}px;width:${rect.width + pad * 2}px;height:${rect.height + pad * 2}px`;
		if (editOrbitParticles.length === 0) {
			editOrbitParticles = Array.from({length: EDIT_ORBIT_COUNT}).map((_, i) => ({
				id: i,
				size: Math.random() * 2 + 1,
				duration: 8 + Math.random() * 4,
				opacity: Math.random() * 0.5 + 0.5,
				stagger: i * 0.025,
			}));
		}
		isEditingInDarkFocus = true;
	}

	function placeCaretAtEnd(el: HTMLElement) {
		const selection = window.getSelection();
		if (!selection) return;
		const range = document.createRange();
		range.selectNodeContents(el);
		range.collapse(false);
		selection.removeAllRanges();
		selection.addRange(range);
	}

	function handleEditKeydown(e: KeyboardEvent) {
		if (e.key === "Escape") {
			// Cancel only this layer; stop the window handler from also acting.
			e.preventDefault();
			e.stopPropagation();
			cancelEdit();
			return;
		}
		if (e.key === "Enter") {
			// Shift+Enter inserts a line break (browser default); Enter or
			// Ctrl+Enter saves.
			if (e.shiftKey) return;
			e.preventDefault();
			e.stopPropagation();
			void finishEdit();
		}
	}

	function cleanupEditOrbit() {
		isEditingInDarkFocus = false;
		editOrbitParticles = [];
	}

	function teardownEdit(el: HTMLElement) {
		editingParagraph = null;
		originalMarkdownBlock = "";
		el.removeEventListener("blur", finishEdit);
		el.removeEventListener("keydown", handleEditKeydown);
		el.removeEventListener("input", updateEditOrbitPosition);
		el.removeAttribute("contenteditable");
		el.classList.remove("editing");
		el.classList.remove("editing-markdown-source");
		cleanupEditOrbit();
	}

	function cancelEdit() {
		if (!editingParagraph) return;
		const el = editingParagraph;
		teardownEdit(el);
		restoreEditedBlockHtml(el, originalText);
	}

	// Restore a block's pre-edit HTML; if its sentence segmentation was unwrapped
	// for editing, the focus unit index refers to dead spans — rebuild it.
	function restoreEditedBlockHtml(el: HTMLElement, html: string) {
		el.innerHTML = html;
		if (editUnwrappedSegments) {
			editUnwrappedSegments = false;
			if ($focusMode || $searchQuery.trim()) {
				reindexAfterArticleDomChange();
			}
		}
	}

	// After any in-place article DOM change: drop stale block indices/metrics and
	// rebuild whatever the current mode (search/focus) needs.
	function reindexAfterArticleDomChange() {
		focusStyleCache = new WeakMap<HTMLElement, string>();
		styledFocusIndices = new Set<number>();
		lastFocusRenderSignature = "";
		invalidateFocusMetrics();
		clearFocusBlockIndex();

		if ($searchQuery.trim()) {
			performSearch();
		} else if ($focusMode) {
			refreshFocusBlocks();
			scheduleFocusUpdate(lastFocusedIdx >= 0 ? lastFocusedIdx : undefined);
		}
	}

	async function refreshRenderedMarkdownAfterEdit(scrollPos: number) {
		const rendered = await renderMarkdownForUi($markdownSource);
		$renderedHtml = resolveRenderedHtmlAssets(rendered.html, $currentFilePath);
		tocItems = rendered.toc;
		frontMatterEntries = rendered.frontMatter;
		await tick();
		wrapArticleTables();
		if (contentEl) contentEl.scrollTop = scrollPos;
		reindexAfterArticleDomChange();
		void renderMermaidBlocks();
	}

	// Re-render only the edited block and splice it into the live DOM. Bails out
	// (returns false) whenever the edit could have effects beyond the block —
	// callers then fall back to a full document re-render.
	async function tryLocalizedEditUpdate(
		el: HTMLElement,
		sourceStart: number,
		sourceEnd: number,
		newBlockSource: string,
	): Promise<boolean> {
		const article = getArticleElement();
		if (!article || el.parentElement !== article) return false;
		// Headings change the TOC; list items/table rows can't render in isolation.
		if (!/^(P|PRE|BLOCKQUOTE)$/.test(el.tagName)) return false;
		// A leading --- would be treated as front matter by the fragment pipeline.
		if (/^---/.test(newBlockSource)) return false;
		// Reference definitions and footnotes have document-wide effects.
		if (/^\s{0,3}\[[^\]\n]+\]:/m.test($markdownSource) || $markdownSource.includes("[^")) {
			return false;
		}

		// The block must stay isolated by blank lines (or document edges), or it
		// could merge with a neighbour when the full document is parsed.
		const lines = $markdownSource.split("\n");
		const newLines = newBlockSource.split("\n");
		const beforeIdx = sourceStart - 2;
		if (beforeIdx >= 0 && (lines[beforeIdx] ?? "").trim() !== "") return false;
		const afterIdx = sourceStart - 1 + newLines.length;
		if (afterIdx < lines.length && (lines[afterIdx] ?? "").trim() !== "") return false;

		let rendered: RenderedMarkdownDocument;
		try {
			rendered = await renderMarkdownForUi(newBlockSource);
		} catch {
			return false;
		}
		// The edit may have produced a heading (e.g. setext underline) — TOC change.
		if (rendered.toc.length > 0) return false;

		const html = resolveRenderedHtmlAssets(rendered.html, $currentFilePath);
		const template = document.createElement("template");
		template.innerHTML = html;

		// Fragment line numbers are relative to the block source; rebase them.
		const lineOffset = sourceStart - 1;
		template.content.querySelectorAll("[data-source-start]").forEach((node) => {
			const start = Number(node.getAttribute("data-source-start"));
			const end = Number(node.getAttribute("data-source-end"));
			if (Number.isFinite(start)) node.setAttribute("data-source-start", String(start + lineOffset));
			if (Number.isFinite(end)) node.setAttribute("data-source-end", String(end + lineOffset));
		});

		// Shift line ranges of everything after the edited block.
		const lineDelta = newLines.length - (sourceEnd - sourceStart + 1);
		if (lineDelta !== 0) {
			article.querySelectorAll("[data-source-start]").forEach((node) => {
				const start = Number(node.getAttribute("data-source-start"));
				if (!Number.isFinite(start) || start <= sourceEnd) return;
				const end = Number(node.getAttribute("data-source-end"));
				node.setAttribute("data-source-start", String(start + lineDelta));
				if (Number.isFinite(end)) node.setAttribute("data-source-end", String(end + lineDelta));
			});
		}

		el.replaceWith(...Array.from(template.content.children));
		wrapArticleTables();
		reindexAfterArticleDomChange();
		ensureKatexCss(html);
		void renderMermaidBlocks();
		return true;
	}

	async function finishEdit(): Promise<boolean> {
		if (!editingParagraph) return true;
		if (finishEditPromise) return finishEditPromise;

		finishEditPromise = (async (): Promise<boolean> => {
			const el = editingParagraph;
			const newTextContent = el.innerText || '';

			// Capture path/encoding and original source before any mutation
			const savePath = $currentFilePath;
			const saveEncoding = fileEncoding;
			const oldText = originalText;
			const oldMarkdownBlock = originalMarkdownBlock;
			const oldMarkdownSource = $markdownSource;

			const newMarkdownBlock = normalizeMarkdownEditText(newTextContent);
			if (newMarkdownBlock === oldMarkdownBlock) {
				teardownEdit(el);
				restoreEditedBlockHtml(el, oldText);
				return true;
			}

			if (!savePath) {
				teardownEdit(el);
				restoreEditedBlockHtml(el, oldText);
				return false;
			}

			const sourceStart = parseInt(el.dataset.sourceStart || '-1');
			const sourceEnd = parseInt(el.dataset.sourceEnd || '-1');
			const updatedMarkdownSource = replaceMarkdownSourceBlock(
				oldMarkdownSource,
				sourceStart,
				sourceEnd,
				newMarkdownBlock,
			);
			if (updatedMarkdownSource === null) {
				teardownEdit(el);
				restoreEditedBlockHtml(el, oldText);
				return false;
			}
			$markdownSource = updatedMarkdownSource;

			try {
				await invoke('save_markdown_file', { path: savePath, content: $markdownSource, encoding: saveEncoding });
				fileError = "";
			} catch (err) {
				console.error('Failed to save:', err);
				fileError = `保存失败：${err instanceof Error ? err.message : String(err)}`;
				$markdownSource = oldMarkdownSource;
				teardownEdit(el);
				restoreEditedBlockHtml(el, oldText);
				return false;
			}

			const scrollPos = contentEl?.scrollTop ?? 0;
			teardownEdit(el);
			editUnwrappedSegments = false;
			const localized = await tryLocalizedEditUpdate(
				el,
				sourceStart,
				sourceEnd,
				newMarkdownBlock,
			);
			if (!localized) {
				await refreshRenderedMarkdownAfterEdit(scrollPos);
			}
			await saveState();
			return true;
		})();

		try {
			return await finishEditPromise;
		} finally {
			finishEditPromise = null;
		}
	}

	async function saveState() {
		if (!$currentFilePath || !contentEl) return;
		try {
			if (activeBook) {
				await persistActiveBookProgress();
				return;
			}
			await invoke("save_reading_state", {
				path: $currentFilePath,
				state: {
					scroll_position: contentEl.scrollTop,
					bookmarks: [],
					progress: readingProgress,
				},
			});
		} catch (e) {
			console.error("Failed to save state:", e);
		}
	}

	async function flushSaveState() {
		if (saveStateTimer) {
			clearTimeout(saveStateTimer);
			saveStateTimer = null;
		}
		await saveState();
	}

	function getArticleElement() {
		return contentEl?.querySelector(".article") as HTMLElement | null;
	}

	function collectFocusAtoms(article: HTMLElement) {
		const atoms: HTMLElement[] = [];
		for (const child of Array.from(article.children)) {
			if (!(child instanceof HTMLElement) || child.offsetHeight <= 0) {
				continue;
			}

			// Tables may sit inside a .table-scroll wrapper (wide-table support).
			const tableEl = child.tagName === "TABLE"
				? child
				: child.classList.contains("table-scroll")
					? child.querySelector(":scope > table")
					: null;
			if (tableEl) {
				const rows = Array.from(
					tableEl.querySelectorAll(":scope > thead > tr, :scope > tbody > tr, :scope > tr"),
				).filter(
					(row): row is HTMLElement =>
						row instanceof HTMLElement && row.offsetHeight > 0,
				);
				if (rows.length > 0) {
					atoms.push(...rows);
					continue;
				}
			}

			if (child.tagName === "UL" || child.tagName === "OL") {
				const items = Array.from(child.children).filter(
					(item): item is HTMLElement =>
						item instanceof HTMLElement && item.tagName === "LI" && item.offsetHeight > 0,
				);
				if (items.length > 0) {
					atoms.push(...items);
					continue;
				}
			}

			atoms.push(child);
		}
		return atoms;
	}

	// Only these tags may merge with a same-tag sibling into one focus unit.
	const FOCUS_MERGEABLE_TAGS = new Set(["LI", "TR", "P"]);
	// Atoms up to ~1.5 text lines count as "low density" and are mergeable.
	const FOCUS_MERGE_MAX_ATOM_LINES = 1.6;
	// A merged unit stops growing once it reaches ~3 lines of text.
	const FOCUS_MERGE_TARGET_LINES = 3.2;
	const FOCUS_MERGE_MAX_ATOMS = 5;

	function getArticleLineHeight(article: HTMLElement) {
		const lineHeight = Number.parseFloat(getComputedStyle(article).lineHeight);
		return Number.isFinite(lineHeight) && lineHeight > 0 ? lineHeight : 28;
	}

	// Paragraphs taller than this get sentence-level focus: the paragraph's
	// layout is untouched, but the spotlight advances by sentence groups.
	const FOCUS_SEGMENT_MIN_LINES = 4.5;
	const FOCUS_SEGMENT_GROUP_LINES = 2.8;
	const FOCUS_SEGMENT_MIN_TAIL_LINES = 1.2;

	function getInlineRunHeight(el: HTMLElement) {
		const rects = el.getClientRects();
		if (rects.length === 0) return 0;
		return rects[rects.length - 1].bottom - rects[0].top;
	}

	// Wrap a long paragraph's sentences into <span class="focus-seg"> groups of
	// roughly FOCUS_SEGMENT_GROUP_LINES lines each. Text nodes are split at
	// sentence starts; inline elements (links, code…) stay whole.
	function segmentLongParagraph(p: HTMLElement, lineHeight: number): HTMLElement[] {
		if (p.dataset.focusSegmented === "true") {
			return Array.from(p.querySelectorAll(":scope > .focus-seg")) as HTMLElement[];
		}

		const text = p.textContent ?? "";
		const sentences = splitSentences(text);
		if (sentences.length < 2) return [];

		const splitAtOffset = (target: number) => {
			let acc = 0;
			for (let node = p.firstChild; node; node = node.nextSibling) {
				const len = node.textContent?.length ?? 0;
				if (target < acc + len) {
					if (target > acc && node.nodeType === Node.TEXT_NODE) {
						(node as Text).splitText(target - acc);
					}
					// Inside an inline element: leave it whole in the current sentence.
					return;
				}
				acc += len;
			}
		};
		for (let i = 1; i < sentences.length; i++) {
			splitAtOffset(sentences[i].start);
		}

		// Assign every top-level node to the sentence containing its start offset.
		const bins: Node[][] = sentences.map(() => []);
		let acc = 0;
		let si = 0;
		for (let node = p.firstChild; node; node = node.nextSibling) {
			while (si < sentences.length - 1 && acc >= sentences[si].end) si++;
			bins[si].push(node);
			acc += node.textContent?.length ?? 0;
		}

		const sentenceSpans: HTMLElement[] = [];
		for (const bin of bins) {
			if (bin.length === 0) continue;
			const span = document.createElement("span");
			span.className = "focus-sentence";
			p.insertBefore(span, bin[0]);
			for (const node of bin) span.appendChild(node);
			sentenceSpans.push(span);
		}
		if (sentenceSpans.length < 2) {
			p.dataset.focusSegmented = "true";
			return [];
		}

		// Merge sentences into groups of ~GROUP_LINES lines, measured for real.
		const targetHeight = lineHeight * FOCUS_SEGMENT_GROUP_LINES;
		const groups: HTMLElement[] = [];
		let currentGroup: HTMLElement | null = null;
		for (const sentence of sentenceSpans) {
			if (!currentGroup || getInlineRunHeight(currentGroup) >= targetHeight) {
				currentGroup = document.createElement("span");
				currentGroup.className = "focus-seg";
				p.insertBefore(currentGroup, sentence);
				groups.push(currentGroup);
			}
			currentGroup.appendChild(sentence);
		}

		// A tiny tail group reads as an orphan line — fold it into the previous.
		if (groups.length >= 2) {
			const tail = groups[groups.length - 1];
			if (getInlineRunHeight(tail) < lineHeight * FOCUS_SEGMENT_MIN_TAIL_LINES) {
				const prev = groups[groups.length - 2];
				while (tail.firstChild) prev.appendChild(tail.firstChild);
				tail.remove();
				groups.pop();
			}
		}

		p.dataset.focusSegmented = "true";
		return groups.length >= 2 ? groups : [];
	}

	function unwrapFocusSegments(p: HTMLElement): boolean {
		if (p.dataset.focusSegmented !== "true") return false;
		p.querySelectorAll(".focus-seg, .focus-sentence").forEach((span) => {
			const parent = span.parentNode;
			if (!parent) return;
			while (span.firstChild) parent.insertBefore(span.firstChild, span);
			span.remove();
		});
		delete p.dataset.focusSegmented;
		p.normalize();
		return true;
	}

	function collectFocusUnits(article: HTMLElement): FocusUnit[] {
		const atoms = collectFocusAtoms(article);
		const lineHeight = getArticleLineHeight(article);
		const maxAtomHeight = lineHeight * FOCUS_MERGE_MAX_ATOM_LINES;
		const targetHeight = lineHeight * FOCUS_MERGE_TARGET_LINES;
		const segmentMinHeight = lineHeight * FOCUS_SEGMENT_MIN_LINES;

		const units: FocusUnit[] = [];
		let group: HTMLElement[] = [];
		let groupHeight = 0;

		const flush = () => {
			if (group.length > 0) {
				units.push(group);
				group = [];
				groupHeight = 0;
			}
		};

		for (const atom of atoms) {
			const height = atom.offsetHeight;

			// Long top-level paragraphs advance sentence group by sentence group.
			// Focus mode only: outside it, content-visibility skips offscreen layout
			// and inline rects can't be measured for real.
			if (
				$focusMode &&
				atom.tagName === "P" &&
				atom.parentElement === article &&
				atom !== editingParagraph &&
				height > segmentMinHeight
			) {
				const segments = segmentLongParagraph(atom, lineHeight);
				if (segments.length >= 2) {
					flush();
					for (const segment of segments) {
						units.push([segment]);
					}
					continue;
				}
			}

			const mergeable = FOCUS_MERGEABLE_TAGS.has(atom.tagName) && height <= maxAtomHeight;

			if (!mergeable) {
				flush();
				units.push([atom]);
				continue;
			}

			const head = group[0];
			const sameRun = head !== undefined &&
				head.tagName === atom.tagName &&
				head.parentElement === atom.parentElement;
			if (!sameRun || groupHeight >= targetHeight || group.length >= FOCUS_MERGE_MAX_ATOMS) {
				flush();
			}
			group.push(atom);
			groupHeight += height;
		}
		flush();

		return units;
	}

	function invalidateFocusMetrics() {
		focusMetricsValid = false;
		focusBlockMetrics = [];
	}

	function getUnitBoundingRect(unit: FocusUnit) {
		const first = unit[0].getBoundingClientRect();
		if (unit.length === 1) return first;
		const last = unit[unit.length - 1].getBoundingClientRect();
		return new DOMRect(
			Math.min(first.left, last.left),
			first.top,
			Math.max(first.right, last.right) - Math.min(first.left, last.left),
			last.bottom - first.top,
		);
	}

	function rebuildFocusMetrics(units = focusUnits) {
		if (!contentEl || units.length === 0) {
			focusBlockMetrics = [];
			focusMetricsValid = true;
			return focusBlockMetrics;
		}

		const contentRect = contentEl.getBoundingClientRect();
		const scrollTop = contentEl.scrollTop;
		focusBlockMetrics = units.map((unit) => {
			const rect = getUnitBoundingRect(unit);
			const top = scrollTop + rect.top - contentRect.top;
			const bottom = top + rect.height;
			return {
				top,
				bottom,
				center: (top + bottom) / 2,
			};
		});
		focusMetricsValid = true;
		return focusBlockMetrics;
	}

	function getFocusMetrics(units = focusUnits) {
		return focusMetricsValid && focusBlockMetrics.length === units.length
			? focusBlockMetrics
			: rebuildFocusMetrics(units);
	}

	function refreshFocusBlocks() {
		const article = getArticleElement();
		if (!article) {
			focusUnits = [];
			invalidateFocusMetrics();
			return focusUnits;
		}

		// The styled window is tracked by index — indices shift when the unit list
		// is rebuilt, so drop all inline focus styles before re-collecting.
		article.querySelectorAll("[data-focus-near='true']").forEach((node) => {
			if (node instanceof HTMLElement) clearBlockFocusStyle(node);
		});
		styledFocusIndices = new Set<number>();
		article.querySelectorAll("[data-focus-block='true']").forEach((node) => {
			node.removeAttribute("data-focus-block");
			node.removeAttribute("data-focus-index");
		});

		focusUnits = collectFocusUnits(article);
		focusUnits.forEach((unit, index) => {
			for (const member of unit) {
				member.dataset.focusBlock = "true";
				member.dataset.focusIndex = `${index}`;
			}
		});
		rebuildFocusMetrics(focusUnits);
		return focusUnits;
	}

	function clearFocusBlockIndex() {
		focusUnits = [];
		invalidateFocusMetrics();
	}

	function getFocusUnits() {
		return focusUnits.length > 0 ? focusUnits : refreshFocusBlocks();
	}

	function getFocusStyleTargets(block: HTMLElement) {
		if (block.tagName === "TR") {
			const cells = Array.from(block.children).filter(
				(cell): cell is HTMLElement => cell instanceof HTMLElement,
			);
			return cells.length > 0 ? cells : [block];
		}
		return [block];
	}

	function clearBlockFocusStyle(block: HTMLElement) {
		block.removeAttribute("data-focus-near");
		for (const target of getFocusStyleTargets(block)) {
			target.style.filter = "";
			target.style.opacity = "";
			target.style.transform = "";
			target.style.textShadow = "";
			target.style.color = "";
			focusStyleCache.delete(target);
		}
	}

	function getBlockForNode(node: Node | null) {
		const element = node instanceof Element ? node : node?.parentElement ?? null;
		return element?.closest("[data-focus-block='true']") as HTMLElement | null;
	}

	function getFocusIndexForBlock(block: HTMLElement | null) {
		const index = Number(block?.dataset.focusIndex ?? "-1");
		return Number.isFinite(index) && index >= 0 ? index : -1;
	}

	function getSearchPreferredFocusIndex() {
		const block = currentMatchIndex >= 0 ? searchMatches[currentMatchIndex]?.block ?? null : null;
		const index = getFocusIndexForBlock(block);
		return index >= 0 ? index : undefined;
	}

	function focusBlockFromInteraction(node: Node | null): boolean {
		if (!$focusMode || !contentEl) return false;
		const block = getBlockForNode(node);
		if (!block) return false;
		const index = getFocusIndexForBlock(block);
		if (index < 0) return false;
		const unit = getFocusUnits()[index];
		if (!unit) return false;

		focusLockedIndex = index;
		markFocusScrollActive();
		updateFocusParagraph(index);
		scrollUnitToFocusPosition(unit, index);

		requestAnimationFrame(() => {
			updateSpotlightPosition();
			if (editingParagraph && isEditingInDarkFocus) {
				updateEditOrbitPosition();
			}
		});
		return true;
	}

	function getFocusAnchorY() {
		if (typeof window === "undefined") return 0;
		return window.innerHeight * FOCUS_ANCHOR_RATIO;
	}

	function getFocusAnchorOffset() {
		if (!contentEl) return 0;
		return getFocusAnchorY() - contentEl.getBoundingClientRect().top;
	}

	function getFocusEdgeSpacing() {
		if (!contentEl) return 0;
		return Math.max(
			Math.round(contentEl.clientHeight * FOCUS_EDGE_SPACE_RATIO),
			FOCUS_EDGE_SPACE_MIN,
		);
	}

	function getClosestFocusIndex(units: FocusUnit[]) {
		if (!contentEl || units.length === 0) return 0;
		const metrics = getFocusMetrics(units);
		const anchorY = contentEl.scrollTop + getFocusAnchorOffset();

		let low = 0;
		let high = metrics.length - 1;
		while (low <= high) {
			const mid = Math.floor((low + high) / 2);
			const metric = metrics[mid];
			if (anchorY < metric.top) {
				high = mid - 1;
			} else if (anchorY > metric.bottom) {
				low = mid + 1;
			} else {
				return mid;
			}
		}

		const before = clampFocusIndex(high, metrics.length);
		const after = clampFocusIndex(low, metrics.length);
		const beforeDistance = Math.abs(metrics[before].center - anchorY);
		const afterDistance = Math.abs(metrics[after].center - anchorY);
		return beforeDistance <= afterDistance ? before : after;
	}

	function clampFocusIndex(index: number, length: number) {
		return Math.max(0, Math.min(index, length - 1));
	}

	function getLongFocusBlockEdgeBuffer() {
		if (!contentEl) return FOCUS_LONG_BLOCK_EDGE_MIN;
		return Math.max(
			FOCUS_LONG_BLOCK_EDGE_MIN,
			Math.min(
				FOCUS_LONG_BLOCK_EDGE_MAX,
				Math.round(contentEl.clientHeight * FOCUS_LONG_BLOCK_EDGE_RATIO),
			),
		);
	}

	function getCurrentFocusIndexForWheel(units: FocusUnit[]) {
		const lockedIndex = focusProgrammaticScrollIndex ?? focusLockedIndex;
		if (lockedIndex !== null) {
			return clampFocusIndex(lockedIndex, units.length);
		}
		return lastFocusedIdx >= 0
			? clampFocusIndex(lastFocusedIdx, units.length)
			: getClosestFocusIndex(units);
	}

	function allowNativeLongBlockWheel(units: FocusUnit[], direction: number) {
		if (!contentEl || units.length === 0) return false;
		const currentIdx = getCurrentFocusIndexForWheel(units);
		const metric = getFocusMetrics(units)[currentIdx];
		const block = units[currentIdx];
		if (!metric || !block) return false;

		const blockHeight = metric.bottom - metric.top;
		if (blockHeight <= contentEl.clientHeight * FOCUS_LONG_BLOCK_RATIO) return false;

		const anchorY = contentEl.scrollTop + getFocusAnchorOffset();
		const edgeBuffer = getLongFocusBlockEdgeBuffer();
		const canScrollInsideBlock =
			direction > 0
				? anchorY < metric.bottom - edgeBuffer
				: anchorY > metric.top + edgeBuffer;

		if (!canScrollInsideBlock) return false;

		clearProgrammaticFocusScrollLock();
		focusLockedIndex = currentIdx;
		markFocusScrollActive();
		scheduleFocusUpdate(currentIdx);
		return true;
	}

	function applyFocusSpacing({ preservePosition }: { preservePosition: boolean }) {
		if (!contentEl) return;
		const article = getArticleElement();
		if (!article) return;

		const nextFocusEdgeSpace = getFocusEdgeSpacing();
		const spacingDelta = nextFocusEdgeSpace - focusEdgeSpace;
		article.style.paddingTop = `calc(var(--article-padding-top) + ${nextFocusEdgeSpace}px)`;
		article.style.paddingBottom = `calc(var(--article-padding-bottom) + ${nextFocusEdgeSpace}px)`;

		if (preservePosition && spacingDelta !== 0) {
			contentEl.scrollTop = Math.max(0, contentEl.scrollTop + spacingDelta);
		}

		focusEdgeSpace = nextFocusEdgeSpace;
		rebuildFocusMetrics();
	}

	function scrollUnitToFocusPosition(unit: FocusUnit, focusIndex: number) {
		if (!contentEl || unit.length === 0) return;
		const contentRect = contentEl.getBoundingClientRect();
		const blockRect = getUnitBoundingRect(unit);
		const blockTop = contentEl.scrollTop + blockRect.top - contentRect.top;
		const blockBottom = blockTop + blockRect.height;
		const targetScrollTop = getFocusScrollTarget({
			viewportHeight: contentEl.clientHeight,
			currentScrollTop: contentEl.scrollTop,
			blockTop,
			blockBottom,
			anchorOffset: getFocusAnchorOffset(),
			longBlockRatio: FOCUS_LONG_BLOCK_RATIO,
			longBlockEdgeBuffer: getLongFocusBlockEdgeBuffer(),
		});
		if (targetScrollTop === null) return;

		const maxScrollTop = Math.max(0, contentEl.scrollHeight - contentEl.clientHeight);
		const nextScrollTop = Math.max(0, Math.min(targetScrollTop, maxScrollTop));
		lockProgrammaticFocusScroll(focusIndex, nextScrollTop);
		contentEl.scrollTo({
			top: nextScrollTop,
			behavior: getJumpBehavior(Math.abs(nextScrollTop - contentEl.scrollTop)),
		});
	}

	function applyStylesToBlock(
		block: HTMLElement,
		styles: {
			filter: string;
			opacity: string;
			transform: string;
			textShadow: string;
			color: string;
		},
	) {
		const allowScale = block.tagName !== "TR";
		const transform = allowScale ? styles.transform : "";
		const signature = [
			styles.filter,
			styles.opacity,
			transform,
			styles.textShadow,
			styles.color,
		].join("|");
		for (const target of getFocusStyleTargets(block)) {
			if (focusStyleCache.get(target) === signature) continue;
			target.style.filter = styles.filter;
			target.style.opacity = styles.opacity;
			target.style.transform = transform;
			target.style.textShadow = styles.textShadow;
			target.style.color = styles.color;
			focusStyleCache.set(target, signature);
		}
	}

	const FOCUS_STYLE_RADIUS = 6;
	// Extra ring of blocks kept inline-styled beyond the visible radius so that a
	// block always reaches the fully-hidden state before its inline styles are
	// dropped and the CSS baseline (opacity: 0) takes over with identical values.
	const FOCUS_STYLE_HALO = 2;
	const FOCUS_RELEASE_FADE_MS = 450;

	const FOCUS_HIDDEN_STYLES = {
		filter: "none",
		opacity: "0",
		transform: "none",
		textShadow: "none",
		color: "",
	};

	function getFocusGlow() {
		if (!cachedFocusGlow || cachedFocusGlow.theme !== $currentTheme.name) {
			const color = getComputedStyle(document.documentElement)
				.getPropertyValue("--focus-text-glow")
				.trim();
			cachedFocusGlow = {
				theme: $currentTheme.name,
				color,
				has: !!color && !color.includes("0, 0, 0, 0") && color !== "transparent",
			};
		}
		return cachedFocusGlow;
	}

	function getViewportFocusIndices(units: FocusUnit[]) {
		const indices: number[] = [];
		if (!contentEl) return indices;
		const metrics = getFocusMetrics(units);
		const viewTop = contentEl.scrollTop - 100;
		const viewBottom = contentEl.scrollTop + contentEl.clientHeight + 100;
		for (let i = 0; i < metrics.length; i++) {
			if (metrics[i].bottom >= viewTop && metrics[i].top <= viewBottom) {
				indices.push(i);
			}
		}
		return indices;
	}

	function releaseFocusUnit(unit: FocusUnit) {
		for (const block of unit) {
			// Blocks that already reached the hidden state can drop their inline
			// styles immediately — the CSS baseline holds the exact same values.
			const probe = getFocusStyleTargets(block)[0];
			if (!probe || probe.style.opacity === "0") {
				clearBlockFocusStyle(block);
				continue;
			}
			// Still visible (focus jumped far away): fade to the hidden state first,
			// then drop inline styles once the transition has finished.
			applyStylesToBlock(block, FOCUS_HIDDEN_STYLES);
			setTimeout(() => {
				const index = Number(block.dataset.focusIndex ?? "-1");
				if (block.dataset.focusBlock === "true" && styledFocusIndices.has(index)) return;
				clearBlockFocusStyle(block);
			}, FOCUS_RELEASE_FADE_MS);
		}
	}

	function applyStylesToUnit(
		unit: FocusUnit,
		styles: {
			filter: string;
			opacity: string;
			transform: string;
			textShadow: string;
			color: string;
		},
	) {
		for (const block of unit) {
			applyStylesToBlock(block, styles);
		}
	}

	function applyFocusStyles(units: FocusUnit[], hitIdx: number) {
		const signature = `${hitIdx}|${currentMatchIndex}|${searchMatchBlocks.size}|${$currentTheme.name}|${isFocusKeyScrollActive}`;
		if (signature === lastFocusRenderSignature) {
			lastFocusedIdx = hitIdx;
			return;
		}
		lastFocusRenderSignature = signature;

		const focusGlow = getFocusGlow();
		const currentSearchBlock = currentMatchIndex >= 0
			? searchMatches[currentMatchIndex]?.block ?? null
			: null;
		const currentSearchIndex = getFocusIndexForBlock(currentSearchBlock);

		// Only the styled window gets inline styles; everything beyond it is hidden
		// by the CSS baseline. This keeps every update O(radius) instead of O(doc),
		// which is what made focus mode freeze on long documents.
		const nextStyleIndices = new Set<number>();
		const start = Math.max(0, hitIdx - FOCUS_STYLE_RADIUS - FOCUS_STYLE_HALO);
		const end = Math.min(units.length - 1, hitIdx + FOCUS_STYLE_RADIUS + FOCUS_STYLE_HALO);
		for (let i = start; i <= end; i++) {
			nextStyleIndices.add(i);
		}
		if (currentSearchIndex >= 0) {
			nextStyleIndices.add(currentSearchIndex);
		}
		// Units visible at focus-mode entry keep inline styles (and transitions)
		// until their entry fade has finished, then fall back to the baseline.
		if (focusEntryStickyIndices) {
			if (performance.now() < focusEntryStickyUntil) {
				for (const i of focusEntryStickyIndices) nextStyleIndices.add(i);
			} else {
				focusEntryStickyIndices = null;
			}
		}

		for (const i of styledFocusIndices) {
			if (nextStyleIndices.has(i)) continue;
			const unit = units[i];
			if (unit) releaseFocusUnit(unit);
		}
		styledFocusIndices = nextStyleIndices;

		for (const i of nextStyleIndices) {
			const unit = units[i];
			if (!unit) continue;
			for (const member of unit) {
				if (member.dataset.focusNear !== "true") {
					member.dataset.focusNear = "true";
				}
			}
			const dist = Math.abs(i - hitIdx);

			// Units beyond the focus window are fully hidden WITHOUT filter/transform.
			// blur() and scale() each force a persistent GPU compositing layer, so if we
			// leave them on far blocks, every block ever scrolled past keeps a blurred
			// layer alive, and the compositor occasionally drops/recreates a frame —
			// the full-screen flicker. Plain opacity keeps the layer count bounded.
			if (dist > FOCUS_STYLE_RADIUS) {
				applyStylesToUnit(unit, FOCUS_HIDDEN_STYLES);
				continue;
			}

			if (i === hitIdx) {
				const glow = focusGlow.has
					? `0 0 8px ${focusGlow.color}, 0 0 20px ${focusGlow.color}`
					: "none";
				for (const member of unit) {
					applyStylesToBlock(member, {
						filter: "none",
						opacity: "1",
						transform: "none",
						textShadow: glow,
						color: /^H[1-6]$/.test(member.tagName) ? "var(--heading)" : "",
					});
				}
				continue;
			}

			if (isFocusKeyScrollActive) {
				const blur = dist === 1
					? 0.3
					: dist === 2
						? 0.6
						: dist === 3
							? 0.9
							: Math.min(1.2 + (dist - 3) * 0.25, 2.8);
				const opacity = dist === 1
					? 0.86
					: dist === 2
						? 0.74
						: dist === 3
							? 0.62
							: Math.max(0.32, 0.54 - (dist - 3) * 0.035);
				applyStylesToUnit(unit, {
					filter: `blur(${blur}px)`,
					opacity: `${opacity}`,
					transform: "none",
					textShadow: "none",
					color: "",
				});
			} else if (dist === 1) {
				applyStylesToUnit(unit, {
					filter: "blur(2.5px)",
					opacity: "0.2",
					transform: "scale(0.995)",
					textShadow: "none",
					color: "",
				});
			} else if (dist === 2) {
				applyStylesToUnit(unit, {
					filter: "blur(5px)",
					opacity: "0.08",
					transform: "scale(0.99)",
					textShadow: "none",
					color: "",
				});
			} else if (dist === 3) {
				applyStylesToUnit(unit, {
					filter: "blur(8px)",
					opacity: "0.04",
					transform: "scale(0.985)",
					textShadow: "none",
					color: "",
				});
			} else {
				const blur = Math.min(8 + (dist - 3) * 2, 16);
				const opacity = Math.max(0.01, 0.03 - (dist - 3) * 0.005);
				applyStylesToUnit(unit, {
					filter: `blur(${blur}px)`,
					opacity: `${opacity}`,
					transform: "scale(0.98)",
					textShadow: "none",
					color: "",
				});
			}
		}

		lastFocusedIdx = hitIdx;
	}

	function updateFocusParagraph(preferredIdx?: number) {
		if (!contentEl) return;
		const units = getFocusUnits();
		if (units.length === 0) return;

		const preferredSearchIdx = preferredIdx ?? getSearchPreferredFocusIndex();
		const hitIdx = preferredSearchIdx === undefined
			? getClosestFocusIndex(units)
			: clampFocusIndex(preferredSearchIdx, units.length);

		applyFocusStyles(units, hitIdx);
	}

	function moveFocus(direction: number) {
		const units = getFocusUnits();
		if (units.length === 0) return;

		const baseIdx = lastFocusedIdx >= 0 ? lastFocusedIdx : getClosestFocusIndex(units);
		const nextIdx = clampFocusIndex(baseIdx + direction, units.length);
		focusLockedIndex = nextIdx;
		markFocusScrollActive();
		updateFocusParagraph(nextIdx);
		scrollUnitToFocusPosition(units[nextIdx], nextIdx);
		updateSpotlightPosition();
	}

	function clearFocusStyles() {
		// Only blocks inside the styled window ever carry inline styles.
		const article = getArticleElement();
		article?.querySelectorAll("[data-focus-near='true']").forEach((node) => {
			if (node instanceof HTMLElement) clearBlockFocusStyle(node);
		});
		focusStyleCache = new WeakMap<HTMLElement, string>();
		styledFocusIndices = new Set<number>();
		focusEntryStickyIndices = null;
		lastFocusedIdx = -1;
		lastFocusRenderSignature = "";
		lastSpotlightVars = new Map<string, string>();
	}

	function setRootStylePropertyIfChanged(style: CSSStyleDeclaration, name: string, value: string) {
		if (lastSpotlightVars.get(name) === value) return;
		style.setProperty(name, value);
		lastSpotlightVars.set(name, value);
	}

	function updateSpotlightPosition() {
		if (!contentEl) return;
		const units = getFocusUnits();
		if (lastFocusedIdx < 0 || lastFocusedIdx >= units.length) return;

		const rect = getUnitBoundingRect(units[lastFocusedIdx]);
		const anchorY = getFocusAnchorY();
		const maxSpotlightHeight = Math.max(
			FOCUS_SPOTLIGHT_MIN,
			Math.round(contentEl.clientHeight * FOCUS_SPOTLIGHT_MAX_RATIO),
		);
		const targetSpotlightHeight = Math.min(
			Math.max(rect.height, FOCUS_SPOTLIGHT_MIN),
			maxSpotlightHeight,
		);
		spotlightHeight += (targetSpotlightHeight - spotlightHeight) * 0.24;
		if (Math.abs(targetSpotlightHeight - spotlightHeight) < 0.5) {
			spotlightHeight = targetSpotlightHeight;
		}
		const docStyle = document.documentElement.style;
		setRootStylePropertyIfChanged(docStyle, "--spotlight-top", `${anchorY - spotlightHeight / 2}px`);
		setRootStylePropertyIfChanged(docStyle, "--spotlight-height", `${spotlightHeight}px`);
		setRootStylePropertyIfChanged(docStyle, "--spotlight-left", `${rect.left}px`);
		setRootStylePropertyIfChanged(docStyle, "--spotlight-right", `${rect.right}px`);
		setRootStylePropertyIfChanged(docStyle, "--anchor-y", `${anchorY}px`);
	}

	function enterFocusMode(anchor: ViewportAnchor | null = null) {
		refreshFocusBlocks();
		const preservePosition = (contentEl?.scrollTop ?? 0) > 0;
		applyFocusSpacing({ preservePosition });
		// Correct for the focus-mode font reflow: put the block the reader was
		// on back at the anchor line, so the closest-unit pick below hits it.
		restoreViewportAnchor(anchor);
		focusWheelDelta = 0;
		lastFocusedIdx = -1;
		lastFocusRenderSignature = "";
		spotlightHeight = 100;
		lastSpotlightVars = new Map<string, string>();

		// Blocks visible at entry must fade out like everything else, so they get
		// inline styles (with transitions) until the entry fade has completed.
		focusEntryStickyIndices = new Set(getViewportFocusIndices(getFocusUnits()));
		focusEntryStickyUntil = performance.now() + 600;

		const preferredIdx = getSearchPreferredFocusIndex();
		updateFocusParagraph(preferredIdx);
		const units = getFocusUnits();
		if (units.length === 0 || lastFocusedIdx < 0) return;

		focusLockedIndex = lastFocusedIdx;
		markFocusScrollActive();
		scrollUnitToFocusPosition(units[lastFocusedIdx], lastFocusedIdx);
		updateFocusParagraph(lastFocusedIdx);
		updateSpotlightPosition();
	}

	async function exitFocusMode(): Promise<boolean> {
		// Finish any active edit before exiting focus mode (awaits save + re-render)
		if (editingParagraph) {
			const saved = await finishEdit();
			if (!saved) return false; // Block focus mode exit if save failed
		}
		// Capture while the focus-mode layout is still in effect; restored by
		// toggleFocusMode after the exit reflow.
		pendingExitAnchor = captureViewportAnchor();
		clearFocusStyles();
		clearFocusScrollActive();
		if (focusUpdateFrame !== null) {
			cancelAnimationFrame(focusUpdateFrame);
			focusUpdateFrame = null;
		}
		queuedFocusIndex = undefined;
		const article = getArticleElement();
		if (article) {
			const previousFocusEdgeSpace = focusEdgeSpace;
			article.style.paddingTop = "";
			article.style.paddingBottom = "";
			if (contentEl && previousFocusEdgeSpace > 0) {
				contentEl.scrollTop = Math.max(0, contentEl.scrollTop - previousFocusEdgeSpace);
			}
		}
		if (focusWheelResetTimer) {
			clearTimeout(focusWheelResetTimer);
			focusWheelResetTimer = null;
		}
		focusWheelDelta = 0;
		focusEdgeSpace = 0;
		spotlightHeight = 100;
		focusLockedIndex = null;
		clearProgrammaticFocusScrollLock();
		lastSpotlightVars = new Map<string, string>();
		return true;
	}

	function clearSearchHighlights() {
		if (searchDebounceTimer) { clearTimeout(searchDebounceTimer); searchDebounceTimer = null; }
		clearCurrentSearchMark();

		searchMatchBlocks.forEach((block) => block.classList.remove("search-result-block"));
		searchMatchBlocks = new Set<HTMLElement>();
		searchMatches = [];
		searchTickPositions = [];
		currentMatchIndex = -1;
		lastFocusRenderSignature = "";
		if ($focusMode) {
			scheduleFocusUpdate(lastFocusedIdx >= 0 ? lastFocusedIdx : undefined);
		}
	}

	function clearCurrentSearchMark() {
		const mark = currentSearchMark;
		currentSearchMark = null;
		if (!mark) return;
		const parent = mark.parentNode;
		if (parent) {
			parent.replaceChild(document.createTextNode(mark.textContent || ""), mark);
			parent.normalize();
			invalidateFocusMetrics();
		}
	}

	function collectMatchesInBlock(block: HTMLElement, query: string, blockMatches: SearchMatch[]) {
		const walker = document.createTreeWalker(block, NodeFilter.SHOW_TEXT, {
			acceptNode(node) {
				const parent = node.parentElement;
				if (!parent || parent.closest("script, style")) return NodeFilter.FILTER_REJECT;
				return NodeFilter.FILTER_ACCEPT;
			}
		});
		let occurrence = 0;
		while (walker.nextNode()) {
			const node = walker.currentNode as Text;
			const text = node.textContent?.toLowerCase() || "";
			let idx = text.indexOf(query);
			while (idx !== -1) {
				blockMatches.push({ block, occurrence });
				occurrence += 1;
				idx = text.indexOf(query, idx + query.length);
			}
		}
		if (occurrence > 0) {
			searchMatchBlocks.add(block);
			block.classList.add("search-result-block");
		}
	}

	function materializeSearchMark(match: SearchMatch, query: string) {
		const walker = document.createTreeWalker(match.block, NodeFilter.SHOW_TEXT);
		let occurrence = 0;
		while (walker.nextNode()) {
			const node = walker.currentNode as Text;
			const text = node.textContent?.toLowerCase() || "";
			let idx = text.indexOf(query);
			while (idx !== -1) {
				if (occurrence === match.occurrence) {
					const range = document.createRange();
					range.setStart(node, idx);
					range.setEnd(node, idx + query.length);
					const mark = document.createElement("mark");
					mark.className = "search-match search-current";
					range.surroundContents(mark);
					currentSearchMark = mark;
					return mark;
				}
				occurrence += 1;
				idx = text.indexOf(query, idx + query.length);
			}
		}
		return null;
	}

	function performSearch() {
		clearSearchHighlights();
		const query = $searchQuery.trim().toLowerCase();
		if (!query || !contentEl) return;

		const article = contentEl.querySelector(".article");
		if (!article) return;

		searchMatches = [];
		for (const unit of getFocusUnits()) {
			for (const member of unit) {
				collectMatchesInBlock(member, query, searchMatches);
			}
		}

		lastFocusRenderSignature = "";
		rebuildSearchTicks();
		if (searchMatches.length > 0) {
			currentMatchIndex = 0;
			highlightCurrentMatch();
		} else if ($focusMode) {
			scheduleFocusUpdate(lastFocusedIdx >= 0 ? lastFocusedIdx : undefined);
		}
	}

	// ===== Search tick marks along the right edge =====
	let searchTickPositions: number[] = [];

	function rebuildSearchTicks() {
		if (!contentEl || searchMatchBlocks.size === 0) {
			searchTickPositions = [];
			return;
		}
		const contentRect = contentEl.getBoundingClientRect();
		const scrollTop = contentEl.scrollTop;
		const total = Math.max(1, contentEl.scrollHeight);
		const positions: number[] = [];
		searchMatchBlocks.forEach((block) => {
			const rect = block.getBoundingClientRect();
			positions.push((scrollTop + rect.top - contentRect.top) / total);
		});
		searchTickPositions = positions.sort((a, b) => a - b);
	}

	function jumpToSearchTick(position: number) {
		if (!contentEl) return;
		const target = Math.max(
			0,
			position * contentEl.scrollHeight - contentEl.clientHeight * 0.4,
		);
		contentEl.scrollTo({
			top: target,
			behavior: getJumpBehavior(Math.abs(target - contentEl.scrollTop)),
		});
	}

	function navigateSearch(direction: number) {
		if (searchMatches.length === 0) return;
		clearCurrentSearchMark();
		currentMatchIndex =
			(currentMatchIndex + direction + searchMatches.length) %
			searchMatches.length;
		highlightCurrentMatch();
	}

	function highlightCurrentMatch() {
		const match = searchMatches[currentMatchIndex];
		if (!match) return;

		clearCurrentSearchMark();
		const mark = materializeSearchMark(match, $searchQuery.trim().toLowerCase());
		if (!mark) return;
		invalidateFocusMetrics();
		if ($focusMode) {
			if (focusBlockFromInteraction(match.block)) {
				return;
			}
		}

		mark.scrollIntoView({
			behavior: getJumpBehavior(getViewportDistance(mark)),
			block: "center",
		});
	}

	// Long jumps must not animate across thousands of pixels — they'd "fly" for
	// seconds on long documents. Anything beyond a few viewports cuts instantly.
	const LONG_JUMP_VIEWPORTS = 3;

	function getJumpBehavior(distancePx: number): ScrollBehavior {
		if (!contentEl) return "smooth";
		return distancePx > contentEl.clientHeight * LONG_JUMP_VIEWPORTS ? "instant" : "smooth";
	}

	function getViewportDistance(el: Element) {
		if (!contentEl) return 0;
		const rect = el.getBoundingClientRect();
		const contentRect = contentEl.getBoundingClientRect();
		return Math.abs(rect.top - contentRect.top);
	}

	function scrollToHeading(id: string) {
		if (!contentEl) return;
		const el = contentEl.querySelector(`#${CSS.escape(id)}`);
		if (!el) return;
		$tocOpen = false;

		// In focus mode a jump must land on the focus anchor with spotlight and
		// blur window in sync — plain scrollIntoView leaves them misaligned.
		if ($focusMode && el instanceof HTMLElement && focusBlockFromInteraction(el)) {
			return;
		}

		el.scrollIntoView({ behavior: getJumpBehavior(getViewportDistance(el)), block: "start" });
	}

	function jumpToToc(id: string) {
		if (activeBook) {
			const index = activeBook.manifest.chapters.findIndex((chapter) => chapter.id === id);
			if (index >= 0) {
				void openBookChapter(index);
				return;
			}
		}
		scrollToHeading(id);
	}

	function scheduleSearch() {
		if (searchDebounceTimer) clearTimeout(searchDebounceTimer);
		searchDebounceTimer = setTimeout(() => performSearch(), 180);
	}

	// Current section = last heading above ~35% of the viewport.
	let tocActiveId = "";
	$: if ($tocOpen) tocActiveId = activeBook
		? activeBook.manifest.chapters[activeChapterIndex]?.id ?? ""
		: getCurrentHeading()?.id ?? "";

	function getCurrentHeading(): TocItem | null {
		if (activeBook) return tocItems[activeChapterIndex] ?? null;
		if (!contentEl || tocItems.length === 0) return null;
		const threshold =
			contentEl.getBoundingClientRect().top + contentEl.clientHeight * 0.35;
		let current: TocItem | null = null;
		for (const item of tocItems) {
			const el = contentEl.querySelector(`#${CSS.escape(item.id)}`);
			if (!el) continue;
			if (el.getBoundingClientRect().top <= threshold) {
				current = item;
			} else {
				break;
			}
		}
		return current;
	}

	// ===== Bottom status line: chapter · progress · time left =====
	let statusLineVisible = false;
	let statusLineTimer: ReturnType<typeof setTimeout> | null = null;
	let statusChapterText = "";
	let lastStatusChapterUpdate = 0;

	// Mixed-language reading speed: ~400 CJK chars/min + ~230 latin words/min.
	function estimateReadingMinutes(text: string): number {
		const cjk = text.match(/[一-鿿぀-ヿ가-힯]/g)?.length ?? 0;
		const words = text.match(/[A-Za-z0-9]+/g)?.length ?? 0;
		return cjk / 400 + words / 230;
	}

	$: totalReadingMinutes = estimateReadingMinutes($markdownSource);
	$: remainingMinutes = Math.ceil(totalReadingMinutes * (1 - readingProgress));

	function pulseStatusLine() {
		statusLineVisible = true;
		const now = Date.now();
		if (now - lastStatusChapterUpdate > 300) {
			lastStatusChapterUpdate = now;
			statusChapterText = getCurrentHeading()?.text ?? "";
		}
		if (statusLineTimer) clearTimeout(statusLineTimer);
		statusLineTimer = setTimeout(() => {
			statusLineVisible = false;
			statusLineTimer = null;
		}, 2200);
	}

</script>

<div
	class="app"
	class:focus-mode={$focusMode}
	class:focus-scroll-active={isFocusScrollActive}
	class:focus-key-scroll-active={isFocusKeyScrollActive}
	class:is-light-theme={isLightTheme}
	class:editing-in-focus={isEditingInDarkFocus}
	role="application"
>
	<!-- Reading progress indicator -->
	{#if $currentFilePath}
		<div
			class="progress-line"
			style="width: {readingProgress * 100}%"
		></div>
	{/if}

	<!-- Reveal zone: book reading can hide chrome; temporary keeps back control visible -->
	{#if $currentFilePath && !$focusMode && activeBook && !isTemporaryReading}
		<div
			class="topbar-hover-zone"
			role="presentation"
			on:mouseenter={revealTopbar}
		></div>
	{/if}

	<!-- Top bar - always recoverable when a file is open -->
	<header
		class="topbar"
		class:hidden={!$currentFilePath || $focusMode || (!topbarRevealed && !!activeBook && !isTemporaryReading)}
		class:topbar-pinned={isTemporaryReading || !activeBook}
		role="toolbar"
		tabindex="-1"
		on:mouseenter={revealTopbar}
		on:mouseleave={scheduleTopbarHide}
	>
		<div class="topbar-left">
			{#if $currentFilePath}
				<!-- Icon-only: avoids clip of “书架” text against window chrome -->
				<button
					type="button"
					class="icon-btn shelf-back"
					on:click={() => void returnToBookshelf()}
					title="返回书架"
					aria-label="返回书架"
				>
					<svg width="18" height="18" viewBox="0 0 24 24" fill="none" aria-hidden="true">
						<path
							d="M15 6L9 12l6 6"
							stroke="currentColor"
							stroke-width="1.85"
							stroke-linecap="round"
							stroke-linejoin="round"
						/>
					</svg>
				</button>
			{/if}
			{#if fileName}
				<span class="filename">{isTemporaryReading ? `临时 · ${fileName}` : fileName}</span>
			{/if}
		</div>
		<div class="topbar-right">
			{#if $currentFilePath}
				<button
					class="icon-btn"
					class:active={$focusMode}
					on:click={() => void toggleFocusMode(!$focusMode)}
					title="专注模式 (F11 / Esc)"
				>
					<svg
						width="18"
						height="18"
						viewBox="0 0 24 24"
						fill="none"
						stroke="currentColor"
						stroke-width="1.5"
					>
						<circle cx="12" cy="12" r="3" />
						<path d="M3 12h3M18 12h3M12 3v3M12 18v3" />
					</svg>
				</button>
				<button
					class="icon-btn"
					class:active={$tocOpen}
					on:click={() => ($tocOpen = !$tocOpen)}
					title="目录 ({modLabel}T)"
				>
					<svg
						width="18"
						height="18"
						viewBox="0 0 24 24"
						fill="none"
						stroke="currentColor"
						stroke-width="1.5"
					>
						<path d="M4 6h16M4 12h12M4 18h8" />
					</svg>
				</button>
				<button
					class="icon-btn"
					class:active={$searchOpen}
					on:click={() => ($searchOpen = !$searchOpen)}
					title="搜索 ({modLabel}F)"
				>
					<svg
						width="18"
						height="18"
						viewBox="0 0 24 24"
						fill="none"
						stroke="currentColor"
						stroke-width="1.5"
					>
						<circle cx="11" cy="11" r="7" />
						<path d="M21 21l-4.35-4.35" />
					</svg>
				</button>
			{/if}
			<button
				class="icon-btn"
				class:active={$settingsOpen}
				on:click={() => ($settingsOpen = !$settingsOpen)}
				title="设置 ({modLabel},)"
			>
				<svg
					width="18"
					height="18"
					viewBox="0 0 24 24"
					fill="none"
					stroke="currentColor"
					stroke-width="1.5"
				>
					<circle cx="12" cy="12" r="3" />
					<path
						d="M12 1v4M12 19v4M4.22 4.22l2.83 2.83M16.95 16.95l2.83 2.83M1 12h4M19 12h4M4.22 19.78l2.83-2.83M16.95 7.05l2.83-2.83"
					/>
				</svg>
			</button>
		</div>
	</header>

	<!-- Search bar -->
	<SearchBar
		matchCount={searchMatches.length}
		currentIndex={currentMatchIndex}
		onInput={scheduleSearch}
		onNavigate={navigateSearch}
		onClose={clearSearchHighlights}
	/>

	<!-- TOC overlay -->
	<TocPanel items={tocItems} activeId={tocActiveId} onJump={jumpToToc} />

	<!-- Settings overlay -->
	<SettingsPanel />

	<!-- Main content -->
	<main class="content" class:flow-active={!!flowReaderSession} bind:this={contentEl}>
		{#if flowReaderSession}
			<section class="flow-reader-workspace" aria-label="连续阅读">
				<header>
					<div>
						<strong>连续阅读</strong>
						<span>所有章节留在沉浸阅读窗口内</span>
					</div>
					<button type="button" on:click={() => void closeFlowReader()}>返回书库</button>
				</header>
				<iframe
					title="连续阅读器"
					src={flowReaderSession.url}
					sandbox="allow-same-origin allow-scripts allow-forms"
				></iframe>
			</section>
		{:else if trashOpen}
			<TrashPanel
				items={trashItems}
				loading={trashLoading}
				onClose={() => (trashOpen = false)}
				onRefresh={() => void refreshTrash()}
				onRestore={(item) => void restoreTrashItem(item)}
				onDelete={(item) => void permanentlyDeleteTrashItem(item)}
			/>
		{:else}
		{#if fileError}
			<div class="file-error" role="alert">{fileError}</div>
		{/if}
		{#if $isLoading}
			<div class="loading">
				<div class="loading-dot"></div>
			</div>
		{:else if $renderedHtml}
			<article class="article">
				{#if frontMatterEntries.length > 0}
					<div class="frontmatter-card">
						{#each frontMatterEntries as entry (entry.key)}
							{#if entry.key.toLowerCase() === "title"}
								<div class="fm-title">{entry.value}</div>
							{:else}
								<div class="fm-row">
									<span class="fm-key">{entry.key}</span>
									<span class="fm-value">{entry.value}</span>
								</div>
							{/if}
						{/each}
					</div>
				{/if}
				{@html $renderedHtml}
				{#if activeBook}
					<div class="book-seam" aria-live="polite">
						{#if activeChapterIndex < activeBook.manifest.chapters.length - 1}
							<span>下一篇 · {activeBook.manifest.chapters[activeChapterIndex + 1].title}</span>
							<small>继续向下滚动</small>
						{:else}
							<span>全书读完</span><small>最后一章达到 95% 后记为已读</small>
						{/if}
					</div>
				{/if}
			</article>
		{:else if $currentFilePath}
			<article class="article empty-file">
				<p>空白 Markdown 文件</p>
			</article>
		{:else}
			<Bookshelf
				books={libraryBooks}
				issues={libraryIssues}
				{temporaryItems}
				tasks={acquisitionTasks}
				trashCount={trashItems.length}
				loading={libraryLoading}
				writable={libraryWritable}
				libraryRoot={appSettings?.libraryRoot ?? ""}
				onOpenBook={(bookId) => void openLibraryBook(bookId)}
				onFlowBook={(bookId) => void openBrowserReader(bookId)}
				onRefresh={() => void refreshLibrary()}
				onImport={() => void importFolderToLibrary()}
				onOpenFile={() => void openFileDialog()}
				onOpenTemporary={(path) => void openFile(path)}
				onLaunchTool={(tool) => void launchCompanionTool(tool)}
				onStartTask={(taskId) => void startPodcastTask(taskId)}
				onOpenTaskResult={(taskId) => void openPodcastTaskResult(taskId)}
				onRestartTask={(taskId) => void restartPodcastTask(taskId)}
				onControlTask={(taskId, action, revision) => void controlPodcastTask(taskId, action, revision)}
				onChooseLibrary={() => void chooseLibraryRoot()}
				onOpenTrash={openTrashPanel}
				onRemoveBook={(bookId, title, chapterCount) =>
					void removeLibraryBook(bookId, title, chapterCount)}
				onDeleteBook={(bookId, title, chapterCount) =>
					void deleteLibraryBook(bookId, title, chapterCount)}
			/>
		{/if}
		{/if}
	</main>

	<!-- Spotlight overlays for focus mode -->
	{#if $focusMode && $renderedHtml}
		<div class="stage-spotlight-layer">
			<div class="dust-container">
				<!-- Limit particles strictly to the text reading width -->
				<div class="dust-bounds">
					{#each particles as p (p.id)}
						<div class="dust-particle" style="
							left: {p.left}%; 
							width: {p.size}px; height: {p.size}px;
							animation-duration: {p.duration}s;
							animation-delay: {p.delay}s;
							--p-opacity: {p.opacity};
						"></div>
					{/each}
				</div>
			</div>

			<div class="stage-lamp left"></div>
			<div class="stage-beam left"></div>

			<div class="stage-lamp right"></div>
			<div class="stage-beam right"></div>
		</div>
	{/if}

	{#if editingParagraph}
		<div class="edit-hint">Enter 保存 · Shift+Enter 换行 · Esc 取消</div>
	{/if}

	{#if firstOpenHintVisible && !editingParagraph && !$focusMode}
		<div class="first-hint">
			<span>{modLabel}T 目录 · {modLabel}F 搜索 · F11 专注 · 双击编辑 · {modLabel}滚轮 字号</span>
			<button
				class="first-hint-close"
				on:click={() => (firstOpenHintVisible = false)}
				aria-label="关闭提示"
			>
				×
			</button>
		</div>
	{/if}

	{#if zoomIndicatorText}
		<div class="zoom-indicator">{zoomIndicatorText}</div>
	{/if}

	{#if appNoticeText}
		<div class="zoom-indicator app-notice">{appNoticeText}</div>
	{/if}

	<!-- Search hit tick marks along the right edge -->
	{#if searchTickPositions.length > 0}
		<div class="search-ticks">
			{#each searchTickPositions as pos, i (i)}
				<button
					class="search-tick"
					style="top: {pos * 100}%"
					aria-label="搜索命中位置"
					on:click={() => jumpToSearchTick(pos)}
				></button>
			{/each}
		</div>
	{/if}

	<!-- Code copy button -->
	{#if codeCopyVisible}
		<button
			class="code-copy-btn"
			style={codeCopyStyle}
			on:mouseenter={cancelCodeCopyHide}
			on:mouseleave={() => scheduleCodeCopyHide()}
			on:click={copyCodeBlock}
		>
			{codeCopySuccess ? "已复制" : "复制"}
		</button>
	{/if}

	<!-- Footnote hover preview -->
	{#if footnotePreviewHtml}
		<div class="footnote-preview" style={footnotePreviewStyle}>
			{@html footnotePreviewHtml}
		</div>
	{/if}

	<!-- Image lightbox -->
	{#if lightboxSrc}
		<!-- svelte-ignore a11y-click-events-have-key-events -->
		<!-- svelte-ignore a11y-no-static-element-interactions -->
		<div
			class="lightbox-overlay"
			role="presentation"
			on:click={closeLightbox}
			on:wheel={handleLightboxWheel}
		>
			<img
				class="lightbox-image"
				src={lightboxSrc}
				alt={lightboxAlt}
				style="transform: scale({lightboxScale})"
			/>
		</div>
	{/if}

	<!-- Bottom status line: appears while scrolling, breathes away when idle -->
	{#if $currentFilePath && !$focusMode && !editingParagraph}
		<div class="status-line-pill" class:visible={statusLineVisible}>
			{#if statusChapterText}
				<span class="status-chapter">{statusChapterText}</span>
				<span class="status-sep">·</span>
			{/if}
			<span>{Math.round(readingProgress * 100)}%</span>
			{#if totalReadingMinutes >= 1 && remainingMinutes > 0}
				<span class="status-sep">·</span>
				<span>约剩 {remainingMinutes} 分钟</span>
			{/if}
		</div>
	{/if}

	{#if isEditingInDarkFocus}
		<div class="edit-orbit-container" style={editOrbitContainerStyle}>
			{#each editOrbitParticles as p (p.id)}
				<div class="edit-orbit-particle" style="
					width: {p.size}px; height: {p.size}px;
					--orbit-dur: {p.duration}s;
					--orbit-delay: {-p.duration * p.id / EDIT_ORBIT_COUNT}s;
					--appear-delay: {p.stagger}s;
					--p-opacity: {p.opacity};
				"></div>
			{/each}
		</div>
	{/if}
</div>

<style>
	/* ========== Base ========== */
	:global(*) {
		margin: 0;
		padding: 0;
		box-sizing: border-box;
	}

	:global(body) {
		overflow: hidden;
		background: var(--bg);
		color: var(--text);
		font-family:
			"Segoe UI",
			-apple-system,
			BlinkMacSystemFont,
			"PingFang SC",
			"Microsoft YaHei",
			"Noto Sans SC",
			sans-serif;
		transition:
			background 0.3s ease,
			color 0.3s ease;
	}

	:global(::selection) {
		background: var(--selection);
	}

	:global(::-webkit-scrollbar) {
		width: 6px;
	}
	:global(::-webkit-scrollbar-track) {
		background: transparent;
	}
	:global(::-webkit-scrollbar-thumb) {
		background: var(--scrollbar);
		border-radius: 3px;
	}
	:global(::-webkit-scrollbar-thumb:hover) {
		background: var(--scrollbar-hover);
	}

	.app {
		height: 100vh;
		display: flex;
		flex-direction: column;
		position: relative;
	}

	/* ========== Progress line ========== */
	.progress-line {
		position: fixed;
		top: 0;
		left: 0;
		height: 2px;
		background: var(--link);
		opacity: 0.3;
		z-index: 100;
		transition: width 0.1s linear;
	}

	/* ========== Top bar ========== */
	/* Overlay bar: content flows underneath, so hiding it while reading leaves
	   nothing but text on screen. */
	.topbar {
		position: fixed;
		top: 0;
		left: 0;
		right: 0;
		display: flex;
		align-items: center;
		justify-content: space-between;
		height: 48px;
		padding: 0 16px;
		border-bottom: 1px solid var(--hr);
		-webkit-app-region: drag;
		user-select: none;
		transition: transform 0.4s cubic-bezier(0.2, 0.8, 0.2, 1), opacity 0.4s cubic-bezier(0.2, 0.8, 0.2, 1), visibility 0.4s ease;
		z-index: 50;
		background: var(--bg);
	}

	.topbar.hidden {
		transform: translateY(-100%);
		opacity: 0;
		pointer-events: none;
		visibility: hidden;
	}

	.shelf-back {
		/* Fixed square control — never clips text */
		flex: none;
		width: 36px;
		height: 36px;
		min-width: 36px;
		margin-right: 10px;
		padding: 0;
		border: 1px solid var(--hr);
		border-radius: 10px;
		background: var(--bg-secondary);
		color: var(--text);
		overflow: visible;
	}

	.shelf-back:hover {
		border-color: var(--link);
		color: var(--link);
		background: color-mix(in srgb, var(--link) 14%, var(--bg-secondary));
	}

	.topbar-pinned {
		/* temporary / non-book reading keeps chrome discoverable */
		box-shadow: 0 1px 0 color-mix(in srgb, var(--link) 12%, transparent);
	}

	.book-seam {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 8px;
		margin: 72px 0 28px;
		padding: 28px 16px;
		border-top: 1px solid var(--hr);
		color: var(--text-secondary);
		font-family: var(--font-body);
	}

	.book-seam small {
		color: var(--text-faded);
		font-family: var(--font-ui);
		font-size: 11px;
		letter-spacing: 0.12em;
	}

	/* Invisible strip along the top edge; hovering it summons the topbar and
	   it doubles as a window drag handle. */
	.topbar-hover-zone {
		position: fixed;
		top: 0;
		left: 0;
		right: 0;
		height: 14px;
		z-index: 60;
		-webkit-app-region: drag;
	}

	.topbar-left,
	.topbar-right {
		display: flex;
		align-items: center;
		gap: 4px;
		-webkit-app-region: no-drag;
		min-width: 0;
		overflow: visible;
	}

	.filename {
		font-size: 13px;
		color: var(--text-secondary);
		margin-left: 8px;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		max-width: min(48vw, 420px);
	}

	.icon-btn {
		display: flex;
		align-items: center;
		justify-content: center;
		width: 32px;
		height: 32px;
		border: 1px solid transparent;
		background: transparent;
		color: var(--text-secondary);
		border-radius: 10px;
		cursor: pointer;
		transition: all 0.3s cubic-bezier(0.2, 0.8, 0.2, 1);
		position: relative;
		overflow: hidden;
	}
	.icon-btn::after {
		content: ''; position: absolute; inset: 0;
		background: linear-gradient(135deg, rgba(255, 255, 255, 0.4) 0%, rgba(255, 255, 255, 0) 50%, rgba(255, 255, 255, 0.1) 100%);
		opacity: 0; transition: opacity 0.3s ease;
		pointer-events: none;
	}
	.icon-btn:hover {
		background: var(--bg-secondary);
		border-color: var(--hr);
		color: var(--text);
		transform: translateY(-2px);
		box-shadow: 0 4px 12px rgba(0, 0, 0, 0.1), inset 0 1px 1px rgba(255, 255, 255, 0.2);
		backdrop-filter: blur(8px);
		-webkit-backdrop-filter: blur(8px);
	}
	.icon-btn:hover::after {
		opacity: 1;
	}
	.icon-btn:active {
		transform: translateY(0);
		box-shadow: 0 2px 4px rgba(0, 0, 0, 0.05);
	}
	.icon-btn.active {
		color: var(--link);
		background: var(--bg-secondary);
		border-color: var(--hr);
	}

	/* ========== Main content ========== */
	.content {
		flex: 1;
		overflow-y: auto;
		overflow-x: hidden;
		scroll-behavior: smooth;
	}
	.content.flow-active {
		overflow: hidden;
	}
	.flow-reader-workspace {
		height: 100%;
		display: grid;
		grid-template-rows: 52px minmax(0, 1fr);
		background: var(--bg);
	}
	.flow-reader-workspace > header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 0 18px;
		border-bottom: 1px solid var(--hr);
		background: color-mix(in srgb, var(--bg) 92%, transparent);
	}
	.flow-reader-workspace > header div {
		display: flex;
		align-items: baseline;
		gap: 10px;
	}
	.flow-reader-workspace > header strong {
		font-size: 14px;
		font-weight: 650;
	}
	.flow-reader-workspace > header span {
		font-size: 11px;
		color: var(--text-secondary);
	}
	.flow-reader-workspace > header button {
		border: 1px solid var(--hr);
		border-radius: 8px;
		background: var(--bg-secondary);
		color: var(--text);
		padding: 7px 12px;
		cursor: pointer;
	}
	.flow-reader-workspace iframe {
		width: 100%;
		height: 100%;
		border: 0;
		background: var(--bg);
	}
	.app.focus-key-scroll-active .content {
		scroll-behavior: auto;
	}

	.article {
		--article-padding-x: 48px;
		--article-padding-top: 64px;
		--article-padding-bottom: 120px;
		max-width: var(--article-max-width, 760px);
		margin: 0 auto;
		padding: var(--article-padding-top) var(--article-padding-x)
			var(--article-padding-bottom);
		line-height: var(--article-line-height, 1.8);
		font-family: var(--article-font-family, inherit);
		font-size: calc(16px * var(--font-scale, 1));
		position: relative;
		z-index: 20;
		animation: contentFadeIn 0.3s ease;
		overflow-wrap: anywhere;
	}

	.app:not(.focus-mode) :global(.article > *) {
		content-visibility: auto;
		contain-intrinsic-size: auto 3em;
	}
	.app:not(.focus-mode) :global(.article pre),
	.app:not(.focus-mode) :global(.article table),
	.app:not(.focus-mode) :global(.article blockquote) {
		contain-intrinsic-size: auto 12em;
	}

	.file-error {
		margin: 16px auto 0;
		max-width: 760px;
		padding: 10px 14px;
		border: 1px solid var(--hr);
		border-radius: 8px;
		background: var(--bg-secondary);
		color: var(--text-secondary);
		font-size: 13px;
		line-height: 1.5;
		overflow-wrap: anywhere;
	}

	.empty-file {
		color: var(--text-faded);
	}
	
	.app.focus-mode .article {
		/* Focus mode enlarges relative to the user's zoom, capped so that
		   zoom + focus enlargement can never blow up the layout. */
		font-size: min(calc(16px * var(--font-scale, 1) * 1.125), 24px);
	}

	/* ========== Typography ========== */
	/* Breathing room when jumping to a heading — never glued to the top edge. */
	:global(.article h1),
	:global(.article h2),
	:global(.article h3),
	:global(.article h4),
	:global(.article h5),
	:global(.article h6) {
		scroll-margin-top: 72px;
	}

	:global(.article h1) {
		font-size: 2em;
		font-weight: 700;
		color: var(--heading);
		margin: 2em 0 0.8em;
		line-height: 1.3;
		letter-spacing: -0.02em;
	}
	:global(.article h1:first-child) {
		margin-top: 0;
	}
	:global(.article h2) {
		font-size: 1.5em;
		font-weight: 600;
		color: var(--heading);
		margin: 1.8em 0 0.6em;
		line-height: 1.35;
		padding-bottom: 0.3em;
		border-bottom: 1px solid var(--hr);
	}
	:global(.article h3) {
		font-size: 1.25em;
		font-weight: 600;
		color: var(--heading);
		margin: 1.5em 0 0.5em;
		line-height: 1.4;
	}
	:global(.article h4),
	:global(.article h5),
	:global(.article h6) {
		font-size: 1.05em;
		font-weight: 600;
		color: var(--heading);
		margin: 1.2em 0 0.4em;
	}

	:global(.article p) {
		margin: 0.8em 0;
		color: var(--text);
	}

	:global(.article a) {
		color: var(--link);
		text-decoration: none;
		border-bottom: 1px solid transparent;
		transition: border-color 0.15s ease;
	}
	:global(.article a:hover) {
		color: var(--link-hover);
		border-bottom-color: var(--link-hover);
	}

	:global(.article strong) {
		font-weight: 600;
		color: var(--heading);
	}

	:global(.article em) {
		font-style: italic;
	}

	:global(.article blockquote) {
		margin: 1.2em 0;
		padding: 0.8em 1.2em;
		border-left: 3px solid var(--blockquote-border);
		color: var(--blockquote-text);
		background: var(--bg-secondary);
		border-radius: 0 6px 6px 0;
	}
	:global(.article blockquote p) {
		margin: 0.3em 0;
	}

	:global(.article code) {
		font-family: "Cascadia Code", "Fira Code", "JetBrains Mono", "Consolas",
			monospace;
		font-size: 0.88em;
		background: var(--code-bg);
		color: var(--code-text);
		padding: 0.15em 0.4em;
		border-radius: 4px;
	}

	:global(.article pre) {
		margin: 1.2em 0;
		border-radius: 8px;
		overflow-x: auto;
		font-size: 0.88em;
		line-height: 1.6;
	}
	:global(.article pre code) {
		background: none;
		padding: 0;
		border-radius: 0;
	}
	:global(.article .shiki) {
		padding: 1em 1.2em;
		border-radius: 8px;
		overflow-x: auto;
	}

	:global(.article ul),
	:global(.article ol) {
		margin: 0.8em 0;
		padding-left: 1.8em;
	}
	:global(.article li) {
		margin: 0.3em 0;
	}
	:global(.article li > ul),
	:global(.article li > ol) {
		margin: 0.2em 0;
	}

	:global(.article hr) {
		border: none;
		height: 1px;
		background: var(--hr);
		margin: 2em 0;
	}

	:global(.article img) {
		max-width: 100%;
		height: auto;
		display: block;
		margin: 1.5em auto;
		border-radius: 6px;
		box-shadow: 0 2px 12px rgba(0, 0, 0, 0.06);
		cursor: zoom-in;
	}

	/* Night reading: dim images so they don't glare; hover restores. */
	.app:not(.is-light-theme) :global(.article img) {
		filter: brightness(0.85);
		transition: filter 0.25s ease;
	}
	.app:not(.is-light-theme) :global(.article img:hover) {
		filter: none;
	}

	/* ========== Front matter card ========== */
	.frontmatter-card {
		margin: 0 0 2.5em;
		padding: 16px 20px;
		border: 1px solid var(--hr);
		border-radius: 12px;
		background: var(--bg-secondary);
	}
	.fm-title {
		font-size: 1.35em;
		font-weight: 600;
		color: var(--heading);
		margin-bottom: 6px;
		line-height: 1.35;
	}
	.fm-row {
		display: flex;
		gap: 12px;
		font-size: 12.5px;
		line-height: 1.6;
		margin: 3px 0;
	}
	.fm-key {
		color: var(--text-faded);
		min-width: 64px;
		flex-shrink: 0;
	}
	.fm-value {
		color: var(--text-secondary);
		overflow-wrap: anywhere;
	}

	/* ========== Search tick marks ========== */
	.search-ticks {
		position: fixed;
		top: 0;
		right: 0;
		bottom: 0;
		width: 12px;
		z-index: 90;
		pointer-events: none;
	}
	.search-tick {
		position: absolute;
		right: 1px;
		width: 9px;
		height: 3px;
		border: none;
		border-radius: 2px;
		padding: 0;
		background: var(--link);
		opacity: 0.45;
		cursor: pointer;
		pointer-events: auto;
		transition: opacity 0.15s ease;
	}
	.search-tick:hover {
		opacity: 0.9;
	}

	/* ========== Code copy button ========== */
	.code-copy-btn {
		position: fixed;
		transform: translateX(-100%);
		z-index: 90;
		padding: 3px 10px;
		border-radius: 6px;
		border: 1px solid var(--hr);
		background: var(--bg-secondary);
		color: var(--text-secondary);
		font-size: 11px;
		cursor: pointer;
		opacity: 0.92;
		transition: opacity 0.2s ease, color 0.2s ease;
		animation: fadeIn 0.15s ease;
	}
	.code-copy-btn:hover {
		color: var(--text);
		opacity: 1;
	}

	/* ========== Footnote preview ========== */
	.footnote-preview {
		position: fixed;
		z-index: 95;
		padding: 10px 14px;
		border: 1px solid var(--hr);
		border-radius: 10px;
		background: var(--bg-secondary);
		color: var(--text-secondary);
		font-size: 13px;
		line-height: 1.6;
		box-shadow: 0 8px 32px rgba(0, 0, 0, 0.18);
		pointer-events: none;
		animation: fadeIn 0.15s ease;
	}
	.footnote-preview :global(p) {
		margin: 0.2em 0;
	}

	/* ========== Image lightbox ========== */
	.lightbox-overlay {
		position: fixed;
		inset: 0;
		z-index: 1100;
		background: rgba(0, 0, 0, 0.82);
		display: flex;
		align-items: center;
		justify-content: center;
		cursor: zoom-out;
		animation: fadeIn 0.18s ease;
	}
	.is-light-theme .lightbox-overlay {
		background: rgba(0, 0, 0, 0.65);
	}
	.lightbox-image {
		max-width: 92vw;
		max-height: 92vh;
		border-radius: 6px;
		box-shadow: 0 24px 80px rgba(0, 0, 0, 0.5);
		transition: transform 0.15s ease;
	}

	:global(.article .mermaid-diagram) {
		margin: 1.2em 0;
		display: flex;
		justify-content: center;
		overflow-x: auto;
	}
	:global(.article .mermaid-diagram svg) {
		max-width: 100%;
		height: auto;
	}
	:global(.article .katex-display) {
		overflow-x: auto;
		overflow-y: hidden;
		padding: 0.2em 0;
	}

	/* Wide tables scroll horizontally inside the wrapper instead of squashing. */
	:global(.article .table-scroll) {
		overflow-x: auto;
		margin: 1.2em 0;
	}
	:global(.article .table-scroll > table) {
		margin: 0;
	}

	:global(.article table) {
		width: 100%;
		border-collapse: collapse;
		margin: 1.2em 0;
		font-size: 0.92em;
	}
	:global(.article th),
	:global(.article td) {
		padding: 0.6em 1em;
		border: 1px solid var(--table-border);
		text-align: left;
	}
	:global(.article th) {
		font-weight: 600;
		background: var(--bg-secondary);
	}
	:global(.article tr:nth-child(even) td) {
		background: var(--table-stripe);
	}

	/* ========== Inline editing ========== */
	:global(.editing) {
		outline: none;
		border-radius: 4px;
		cursor: text;
		animation: editBreath 2.5s ease-in-out infinite;
	}

	:global(.editing-markdown-source) {
		white-space: pre-wrap;
	}

	@keyframes editBreath {
		0%, 100% { box-shadow: 0 0 0 1.5px var(--selection); }
		50% { box-shadow: 0 0 0 2px var(--selection), 0 0 10px var(--selection); }
	}

	.status-line-pill {
		position: fixed;
		bottom: 18px;
		left: 50%;
		transform: translateX(-50%);
		display: flex;
		align-items: center;
		gap: 8px;
		max-width: 70vw;
		padding: 5px 14px;
		border-radius: 999px;
		border: 1px solid var(--hr);
		background: var(--bg-secondary);
		color: var(--text-faded);
		font-size: 12px;
		white-space: nowrap;
		z-index: 80;
		pointer-events: none;
		user-select: none;
		opacity: 0;
		transition: opacity 0.45s ease;
	}
	.status-line-pill.visible {
		opacity: 0.95;
	}
	.status-chapter {
		overflow: hidden;
		text-overflow: ellipsis;
		color: var(--text-secondary);
	}
	.status-sep {
		opacity: 0.6;
	}

	.zoom-indicator {
		position: fixed;
		top: 64px;
		left: 50%;
		transform: translateX(-50%);
		padding: 4px 12px;
		border-radius: 999px;
		border: 1px solid var(--hr);
		background: var(--bg-secondary);
		color: var(--text-secondary);
		font-size: 12px;
		z-index: 110;
		pointer-events: none;
		user-select: none;
		animation: fadeIn 0.15s ease;
	}

	.app-notice {
		top: 100px;
	}

	.first-hint {
		position: fixed;
		bottom: 52px;
		left: 50%;
		transform: translateX(-50%);
		display: flex;
		align-items: center;
		gap: 10px;
		padding: 6px 8px 6px 14px;
		border-radius: 999px;
		border: 1px solid var(--hr);
		background: var(--bg-secondary);
		color: var(--text-secondary);
		font-size: 12px;
		white-space: nowrap;
		z-index: 85;
		animation: fadeIn 0.3s ease;
	}
	.first-hint-close {
		width: 20px;
		height: 20px;
		border: none;
		border-radius: 50%;
		background: transparent;
		color: var(--text-faded);
		font-size: 14px;
		line-height: 1;
		cursor: pointer;
	}
	.first-hint-close:hover {
		color: var(--text);
		background: var(--bg);
	}

	.edit-hint {
		position: fixed;
		bottom: 28px;
		left: 50%;
		transform: translateX(-50%);
		padding: 5px 14px;
		border-radius: 999px;
		border: 1px solid var(--hr);
		background: var(--bg-secondary);
		color: var(--text-faded);
		font-size: 12px;
		letter-spacing: 0.02em;
		white-space: nowrap;
		z-index: 90;
		pointer-events: none;
		user-select: none;
		animation: fadeIn 0.25s ease;
	}

	/* ========== Focus mode ========== */
	/* Baseline: any focus block without inline styles is fully hidden. JS only
	   maintains inline styles for the blocks near the focus point, so styling
	   work per update is O(radius) instead of O(document). Values must match
	   FOCUS_HIDDEN_STYLES exactly so dropping inline styles is invisible. */
	.focus-mode :global(.article [data-focus-block="true"]:not(tr)) {
		opacity: 0;
	}
	.focus-mode :global(tr[data-focus-block="true"] > td),
	.focus-mode :global(tr[data-focus-block="true"] > th) {
		opacity: 0;
	}
	/* Transitions only exist on blocks inside the styled window — putting them
	   on every block made entering focus mode animate thousands of layers. */
	.focus-mode :global(.article [data-focus-near="true"]) {
		transition:
			filter 0.4s ease,
			opacity 0.4s ease,
			transform 0.4s ease,
			text-shadow 0.4s ease,
			color 0.4s ease;
	}
	.focus-mode :global([data-focus-near="true"] > td),
	.focus-mode :global([data-focus-near="true"] > th) {
		transition:
			filter 0.4s ease,
			opacity 0.4s ease,
			color 0.4s ease,
			text-shadow 0.4s ease;
	}
	.focus-key-scroll-active :global(.article [data-focus-near="true"]),
	.focus-key-scroll-active :global([data-focus-near="true"] > td),
	.focus-key-scroll-active :global([data-focus-near="true"] > th) {
		transition: none;
	}
	.focus-mode :global(.article pre) {
		background: transparent;
		border-color: transparent;
	}


	/* ========== Edit orbit particles ========== */
	.dust-container {
		transition: opacity 0.5s ease;
	}
	.editing-in-focus .dust-container {
		opacity: 0;
	}

	.edit-orbit-container {
		position: fixed;
		pointer-events: none;
		z-index: 100;
	}
	.edit-orbit-particle {
		position: absolute;
		background: #fff;
		border-radius: 50%;
		filter: blur(0.5px);
		box-shadow: 0 0 6px 2px rgba(255,255,255,0.4);
		transform: translate(-50%, -50%);
		opacity: 0;
		animation-name: orbitBorder, editParticleAppear;
		animation-duration: var(--orbit-dur, 8s), 0.4s;
		animation-timing-function: linear, ease;
		animation-delay: var(--orbit-delay, 0s), var(--appear-delay, 0s);
		animation-iteration-count: infinite, 1;
		animation-fill-mode: none, forwards;
	}
	@keyframes orbitBorder {
		0%, 100% { left: 0; top: 0; }
		25% { left: 100%; top: 0; }
		50% { left: 100%; top: 100%; }
		75% { left: 0; top: 100%; }
	}
	@keyframes editParticleAppear {
		from { opacity: 0; transform: translate(-50%, -50%) scale(3); }
		to { opacity: var(--p-opacity); transform: translate(-50%, -50%) scale(1); }
	}

	/* Stage Spotlight layer */
	.stage-spotlight-layer {
		position: fixed;
		inset: 0;
		pointer-events: none;
		z-index: 10;
		animation: fadeIn 0.5s ease;
	}

	.dust-container {
		position: absolute;
		/* Lock vertical space roughly between lamps and spotlight target */
		top: calc(var(--anchor-y, 38vh)); 
		bottom: calc(100vh - (var(--spotlight-top, 50vh) + var(--spotlight-height, 100px)));
		left: 0; right: 0;
		overflow: hidden;
		pointer-events: none;
		z-index: 13;
	}
	.is-light-theme .dust-container {
		display: none; /* Removed particle effect in light mode completely */
	}
	.dust-bounds {
		position: absolute;
		/* Strict bounds: do not exceed the textual horizontal space */
		left: var(--spotlight-left, 30vw);
		width: calc(var(--spotlight-right, 70vw) - var(--spotlight-left, 30vw));
		height: 100%;
		/* Fade out particles progressively before they reach the bottom (text block) */
		mask-image: linear-gradient(to bottom, transparent 0%, black 15%, black 60%, transparent 100%);
		-webkit-mask-image: linear-gradient(to bottom, transparent 0%, black 15%, black 60%, transparent 100%);
	}
	.dust-particle {
		position: absolute;
		bottom: -10px; /* Start from below the view */
		background: #fff;
		border-radius: 50%;
		filter: blur(0.5px);
		box-shadow: 0 0 6px 2px rgba(255,255,255,0.4);
		animation-name: floatParticle;
		animation-timing-function: linear;
		animation-iteration-count: infinite;
	}
	@keyframes floatParticle {
		0% { transform: translateY(0); opacity: 0; }
		20% { opacity: var(--p-opacity); }
		60% { opacity: var(--p-opacity); }
		100% { transform: translateY(-40vh); opacity: 0; }
	}

	.stage-lamp {
		position: absolute;
		top: var(--anchor-y, 38vh);
		width: 12px;
		height: 48px;
		background: rgba(255, 255, 255, 0.9);
		border-radius: 6px;
		box-shadow: 0 0 20px 8px var(--spotlight-color, rgba(255, 255, 255, 0.2)), 0 0 10px rgba(255, 255, 255, 0.8) inset;
		transform: translateY(-50%);
		z-index: 12;
		transition: top 0.1s ease-out;
		filter: blur(1px);
	}
	.focus-scroll-active .stage-lamp {
		transition: none;
	}
	.is-light-theme .stage-lamp {
		background: rgba(0, 0, 0, 0.4);
		box-shadow: 0 0 20px 8px rgba(0, 0, 0, 0.15), 0 0 10px rgba(0, 0, 0, 0.2) inset;
	}
	.stage-lamp.left { left: -4px; }
	.stage-lamp.right { right: -4px; }

	.stage-beam {
		position: absolute;
		top: 0; left: 0; width: 100vw; height: 100vh;
		z-index: 11;
		transition: clip-path 0.4s cubic-bezier(0.2, 0.8, 0.2, 1);
		will-change: clip-path;
		opacity: 0.8;
	}
	.focus-scroll-active .stage-beam {
		transition: none;
	}
	.is-light-theme .stage-beam {
		opacity: 0.1;
	}

	.stage-beam.left {
		clip-path: polygon(
			-10px calc(var(--anchor-y, 38vh) - 8px), 
			calc(var(--spotlight-left, 30vw) + 10px) calc(var(--spotlight-top, 50vh) - 16px), 
			calc(var(--spotlight-left, 30vw) + 10px) calc(var(--spotlight-top, 50vh) + var(--spotlight-height, 100px) + 16px), 
			-10px calc(var(--anchor-y, 38vh) + 8px)
		);
		/* Subtle aberration exactly at the beam edge */
		background: linear-gradient(to right, 
			rgba(255,255,255,0.1) 0%, 
			var(--spotlight-color, rgba(255,255,255,0.08)) 25%, 
			transparent var(--spotlight-left, 30vw)
		);
		mask-image: linear-gradient(to right, #000 0%, transparent var(--spotlight-left, 30vw));
		-webkit-mask-image: linear-gradient(to right, #000 0%, transparent var(--spotlight-left, 30vw));
		/* Tighter blur for sharper beam */
		filter: blur(3px); 
	}
	.is-light-theme .stage-beam.left {
		background: linear-gradient(to right, rgba(0,0,0,0.2) 0%, rgba(0,0,0,0.1) 25%, transparent var(--spotlight-left, 30vw));
	}
	
	.stage-beam.right {
		clip-path: polygon(
			calc(100vw + 10px) calc(var(--anchor-y, 38vh) - 8px), 
			calc(var(--spotlight-right, 70vw) - 10px) calc(var(--spotlight-top, 50vh) - 16px), 
			calc(var(--spotlight-right, 70vw) - 10px) calc(var(--spotlight-top, 50vh) + var(--spotlight-height, 100px) + 16px), 
			calc(100vw + 10px) calc(var(--anchor-y, 38vh) + 8px)
		);
		background: linear-gradient(to left, 
			rgba(255,255,255,0.1) 0%, 
			var(--spotlight-color, rgba(255,255,255,0.08)) 25%, 
			transparent calc(100vw - var(--spotlight-right, 70vw))
		);
		mask-image: linear-gradient(to left, #000 0%, transparent calc(100vw - var(--spotlight-right, 70vw)));
		-webkit-mask-image: linear-gradient(to left, #000 0%, transparent calc(100vw - var(--spotlight-right, 70vw)));
		filter: blur(3px);
	}
	.is-light-theme .stage-beam.right {
		background: linear-gradient(to left, rgba(0,0,0,0.2) 0%, rgba(0,0,0,0.1) 25%, transparent calc(100vw - var(--spotlight-right, 70vw)));
	}

	/* ========== Search highlights ========== */
	:global(mark.search-match) {
		background: var(--search-highlight);
		color: inherit;
		border-radius: 2px;
		padding: 1px 0;
	}
	:global(mark.search-match.search-current) {
		background: var(--link);
		color: var(--bg);
	}

	/* ========== Loading ========== */
	.loading {
		display: flex;
		align-items: center;
		justify-content: center;
		height: 100%;
	}
	.loading-dot {
		width: 8px;
		height: 8px;
		border-radius: 50%;
		background: var(--text-faded);
		animation: pulse 1s ease-in-out infinite;
	}

	/* ========== Animations ========== */
	@keyframes fadeIn {
		from {
			opacity: 0;
		}
		to {
			opacity: 1;
		}
	}
	@keyframes slideIn {
		from {
			transform: translateX(20px);
			opacity: 0;
		}
		to {
			transform: translateX(0);
			opacity: 1;
		}
	}
	@keyframes scaleIn {
		from {
			transform: scale(0.95);
			opacity: 0;
		}
		to {
			transform: scale(1);
			opacity: 1;
		}
	}
	@keyframes contentFadeIn {
		from {
			opacity: 0;
			transform: translateY(8px);
		}
		to {
			opacity: 1;
			transform: translateY(0);
		}
	}
	@keyframes pulse {
		0%,
		100% {
			opacity: 0.3;
			transform: scale(0.8);
		}
		50% {
			opacity: 1;
			transform: scale(1);
		}
	}
</style>










