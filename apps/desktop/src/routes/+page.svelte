<script lang="ts">
	import { onMount, tick } from "svelte";
	import { convertFileSrc } from "@tauri-apps/api/core";
	import { invokeCommand as invoke, listenManaged } from "$lib/ipc";
	import { stableRequestId } from "$lib/requestId";
	import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
	import { open } from "@tauri-apps/plugin-dialog";
	import { openUrl } from "@tauri-apps/plugin-opener";
	import { checkForDesktopUpdate } from "$lib/update/service";
	import { describeError, reportError } from "$lib/errors";
	import {
		getMarkdownSourceBlock,
		normalizeMarkdownEditText,
		replaceMarkdownSourceBlock
	} from "$lib/edit/sourceBlock";
	import { getFocusScrollTarget } from "$lib/focus/scroll";
	import { splitSentences } from "$lib/focus/segment";
	import { isCurrentNavigation, type NavigationSnapshot } from "$lib/navigation/generation";
	import { resolveMarkdownImageSources } from "$lib/render/images";
	import type { FrontMatterEntry, RenderedMarkdownDocument, TocItem } from "$lib/render/markdown";
	import {
		applyTaskEvent,
		snapshotState,
		taskList,
		type AcquisitionSnapshot,
		type LifecycleState,
		type TaskEvent,
		type TaskSnapshot,
		type TaskSyncState
	} from "$lib/tasks/sync";
	import { taskEventPublishesToLibrary } from "$lib/tasks/libraryRefresh";
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
		READING_LINE_HEIGHTS,
		readingWidth,
		READING_WIDTHS,
		readingFontFamily,
		autoFocusMode,
	} from "$lib/stores/app";
	import { detectDevice, volumeKeyPaging, touchZonesEnabled, type DeviceInfo } from "$lib/platform/device";
	import { flowThemeVars } from "$lib/theme/themes";
	import SearchBar from "$lib/components/SearchBar.svelte";
	import TocPanel from "$lib/components/TocPanel.svelte";
	import SettingsPanel from "$lib/components/SettingsPanel.svelte";
	import ReaderWorkspace from "$lib/components/ReaderWorkspace.svelte";
	import Bookshelf from "$lib/components/Bookshelf.svelte";
	import PodcastWorkflow from "$lib/components/PodcastWorkflow.svelte";
	import ZhihuWorkflow from "$lib/components/ZhihuWorkflow.svelte";
	import TrashPanel from "$lib/components/TrashPanel.svelte";
	import WindowChrome from "$lib/components/WindowChrome.svelte";
	import WorkflowDialogShell from "$lib/components/WorkflowDialogShell.svelte";
	import WindowResizeHandles from "$lib/components/WindowResizeHandles.svelte";
	import BackButton from "$lib/components/BackButton.svelte";
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
	import {
		CHROME_HIDE_DELAY_MS,
		CHROME_TOP_EDGE_PX,
		WIDE_LAYOUT_MAX_WIDTH_PX,
		createChromeState,
		createFlowSetFontScaleMessage,
		createFlowSetLayoutModeMessage,
		createFlowSetThemeMessage,
		deriveChromeSurface,
		isAllowedFlowMessageOrigin,
		isFlowFontScaleChangeMessage,
		isFlowKeyDownMessage,
		isFlowReaderReadyMessage,
		isFlowReadingActivityMessage,
		isImmersiveSurface,
		isOverlaySurface,
		isReadingActivityKey,
		reduceChrome,
		type ChromeEvent,
		type ChromeState,
	} from "$lib/chrome/state";
	import {
		READING_CURSOR_HIDE_DELAY_MS,
		createReadingCursorState,
		reduceReadingCursor,
		type ReadingCursorEvent,
		type ReadingCursorState,
	} from "$lib/reading/cursor";
	import {
		createChapterNavigationKeyLatch,
		readingScrollIntentForKey,
		resolveChapterBoundaryScroll,
		resolveFocusStep,
		resolveReadingScroll,
		type ReadingScrollIntent,
	} from "$lib/reading/navigation";

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

	async function persistRecentFiles(files: RecentFile[]): Promise<RecentFile[]> {
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
			return cleaned;
		} catch {
			recentFiles = limited;
			try {
				localStorage.setItem(RECENT_FILES_KEY, JSON.stringify(limited));
			} catch {
				// ignore
			}
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

	let fileName = "";
	let fileEncoding = 'utf-8';
	let fileError = "";
	type SearchMatch = {
		block: HTMLElement;
		occurrence: number;
	};
	// A one-character query on a large document would otherwise materialize
	// tens of thousands of match records per keystroke — cap and say so.
	const MAX_SEARCH_MATCHES = 5000;
	let searchMatches: SearchMatch[] = [];
	let currentMatchIndex = -1;
	let currentSearchMark: HTMLElement | null = null;
	let editingParagraph: HTMLElement | null = null;
	let editUnwrappedSegments = false;
	let originalText = "";
	let originalMarkdownBlock = "";
	type NavigationGuardChoice = "save" | "discard" | "cancel";
	let navigationGuardOpen = false;
	let navigationGuardReason = "";
	let navigationGuardResolve: ((choice: NavigationGuardChoice) => void) | null = null;
	// Themed replacement for window.confirm: destructive actions get a real
	// dialog that follows the app theme instead of the OS-native prompt.
	interface ActionConfirm {
		message: string;
		confirmLabel: string;
		danger: boolean;
		resolve: (ok: boolean) => void;
	}
	let actionConfirm: ActionConfirm | null = null;
	// A focus unit is one *or more* sibling elements highlighted together —
	// adjacent low-density lines (short list items, one-line paragraphs, table
	// rows) merge into one unit so the spotlight doesn't step line by line.
	type FocusUnit = HTMLElement[];
	let focusUnits: FocusUnit[] = [];
	let focusBlockMetrics: { top: number; bottom: number; center: number }[] = [];
	let focusMetricsValid = false;
	let focusUpdateFrame: number | null = null;
	let focusInteractionFrame: number | null = null;
	// One-shot fade-out timers from releaseFocusUnit — tracked for teardown.
	const focusReleaseTimers = new Set<ReturnType<typeof setTimeout>>();
	let queuedFocusIndex: number | undefined;
	let isFocusScrollActive = false;
	let focusScrollEndTimer: ReturnType<typeof setTimeout> | null = null;
	let focusLockedIndex: number | null = null;
	let focusProgrammaticScrollIndex: number | null = null;
	let focusProgrammaticScrollTarget: number | null = null;
	let focusProgrammaticScrollTimer: ReturnType<typeof setTimeout> | null = null;
	let searchMatchBlocks = new Set<HTMLElement>();
	let searchDebounceTimer: ReturnType<typeof setTimeout> | null = null;
	let saveStateTimer: ReturnType<typeof setTimeout> | null = null;
	let currentLoadToken: string = '';
	let navigationGeneration = 0;
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
	let markdownWorkerTransportFailures = 0;
	const MARKDOWN_WORKER_MAX_TRANSPORT_FAILURES = 3;
	// A render that never answers would leave isLoading stuck forever —
	// generous bound for huge documents, then treat as transport failure
	// (worker gets recycled and the render falls back to the main thread).
	const MARKDOWN_WORKER_TIMEOUT_MS = 30_000;
	let nextRenderRequestId = 1;
	const pendingMarkdownRenders = new Map<number, {
		resolve: (result: RenderedMarkdownDocument) => void;
		reject: (error: Error) => void;
		speculative: boolean;
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

	let deviceInfo: DeviceInfo = detectDevice();
	$: isMobile = deviceInfo.isMobile;

	let mobileHistoryPushed = false;
	function syncMobileHistory() {
		if (!isMobile || typeof history === "undefined") return;
		const hasActiveLayer = Boolean(
			lightboxSrc ||
			editingParagraph ||
			$searchOpen ||
			$tocOpen ||
			$settingsOpen ||
			trashOpen ||
			podcastWorkflowOpen ||
			zhihuWorkflowOpen ||
			selectedBookDetail !== null ||
			actionConfirm !== null ||
			navigationGuardOpen ||
			$focusMode ||
			flowReaderSession ||
			$currentFilePath
		);
		if (hasActiveLayer && !mobileHistoryPushed) {
			mobileHistoryPushed = true;
			try {
				history.pushState({ mmbookActive: true }, "");
			} catch {}
		} else if (!hasActiveLayer && mobileHistoryPushed) {
			mobileHistoryPushed = false;
		}
	}

	$: if (isMobile) {
		void [
			lightboxSrc,
			editingParagraph,
			$searchOpen,
			$tocOpen,
			$settingsOpen,
			trashOpen,
			podcastWorkflowOpen,
			zhihuWorkflowOpen,
			selectedBookDetail,
			actionConfirm,
			navigationGuardOpen,
			$focusMode,
			flowReaderSession,
			$currentFilePath
		];
		syncMobileHistory();
	}

	let touchStartX = 0;
	let touchStartY = 0;
	let touchStartTime = 0;
	let lastTapTime = 0;
	let suppressNextDblClick = false;

	function toggleChromeForTouch() {
		if (chromeState.chromeVisible) {
			dispatchChrome({ type: "apply-hide" });
		} else {
			dispatchChrome({ type: "top-edge-enter" });
		}
	}

	function handleReaderTouchStart(e: TouchEvent) {
		if (e.touches.length !== 1) return;
		touchStartX = e.touches[0].clientX;
		touchStartY = e.touches[0].clientY;
		touchStartTime = Date.now();
	}

	function handleReaderTouchEnd(e: TouchEvent) {
		if (e.changedTouches.length !== 1) return;
		const touchEndX = e.changedTouches[0].clientX;
		const touchEndY = e.changedTouches[0].clientY;
		const deltaX = touchEndX - touchStartX;
		const deltaY = touchEndY - touchStartY;
		const elapsed = Date.now() - touchStartTime;

		// 1. Horizontal swipe gesture for chapter navigation
		if (Math.abs(deltaX) > 60 && Math.abs(deltaY) < 45 && elapsed < 400) {
			if (activeBook && !$focusMode) {
				if (deltaX < 0) {
					void navigateBookChapter(1);
					return;
				} else {
					void navigateBookChapter(-1);
					return;
				}
			}
		}

		// 2. Tap gesture (small movement, short duration)
		if (Math.abs(deltaX) < 14 && Math.abs(deltaY) < 14 && elapsed < 350) {
			if (typeof window !== "undefined" && window.getSelection()?.toString().trim()) {
				return;
			}
			const target = e.target as HTMLElement | null;
			if (target && target.closest("button, a, input, [data-no-tap], .mobile-focus-bar, .context-bar")) {
				return;
			}

			const now = Date.now();
			// Double tap to toggle focus mode
			if (now - lastTapTime < 320) {
				lastTapTime = 0;
				if ($currentFilePath && !flowReaderSession) {
					suppressNextDblClick = true;
					setTimeout(() => {
						suppressNextDblClick = false;
					}, 450);
					void toggleFocusMode(!$focusMode);
					return;
				}
			}
			lastTapTime = now;

			// Zone-based touch navigation or sentence selection
			if ($touchZonesEnabled && typeof window !== "undefined") {
				const w = window.innerWidth;
				const tapX = touchEndX;
				if (tapX < w * 0.25) {
					// Left 25%: Page Up / Prev Sentence
					if ($focusMode) {
						const chDir = moveFocus(-1);
						if (chDir !== null) void navigateBookChapter(chDir);
					} else {
						handleReadingScrollIntent({ direction: -1, kind: "page" }, "TouchLeft");
					}
					return;
				} else if (tapX > w * 0.75) {
					// Right 25%: Page Down / Next Sentence
					if ($focusMode) {
						const chDir = moveFocus(1);
						if (chDir !== null) void navigateBookChapter(chDir);
					} else {
						handleReadingScrollIntent({ direction: 1, kind: "page" }, "TouchRight");
					}
					return;
				} else {
					// Center 50%: If focus mode, clicking sentence focuses it; else toggle header
					if ($focusMode && target && focusBlockFromInteraction(target)) {
						return;
					}
					toggleChromeForTouch();
					return;
				}
			} else {
				// Touch zones disabled: if focus mode, tapping sentence sets focus; else toggle chrome
				if ($focusMode && target && focusBlockFromInteraction(target)) {
					return;
				}
				toggleChromeForTouch();
				return;
			}
		}
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
	let importFolderBusy = false;
	let flowReaderStarting = false;
	let appSettings: AppSettings | null = null;
	let podcastWorkflowOpen = false;
	let zhihuWorkflowOpen = false;
	let taskSyncState: TaskSyncState = snapshotState([]);
	let acquisitionTasks: readonly TaskSnapshot[] = [];
	let taskEventLog: TaskEvent[] = [];
	let taskRefreshNonce = 0;
	let flowReaderSession: { sessionId: string; url: string } | null = null;
	let activeBook: BookDetail | null = null;
	let selectedBookDetail: BookDetail | null = null;
	let activeChapterIndex = -1;
	let chapterSwitching = false;
	let chapterNavigationPending = false;
	let inFlightChapterDirection: -1 | 1 | null = null;
	let queuedChapterNavigation: { direction: -1 | 1; offsetPx: number } | null = null;
	let chapterBoundaryRestoreGeneration = 0;
	const chapterNavigationKeyLatch = createChapterNavigationKeyLatch();
	let preloadedChapter: {
		bookId: string;
		index: number;
		path: string;
		result: { content: string; encoding: string };
		rendered: RenderedMarkdownDocument;
	} | null = null;
	let chapterPreloadInFlight: Promise<void> | null = null;
	let zoomIndicatorText = "";
	let zoomIndicatorTimer: ReturnType<typeof setTimeout> | null = null;
	type DurableReaderPreferences = {
		schemaVersion: 1;
		fontScale: number;
		lineHeight: number;
		contentWidth: number;
		fontFamily: "sans" | "serif";
	};
	type DurableReaderPreferencesLoad = {
		preferences: DurableReaderPreferences;
		storeExists: boolean;
	};
	let readerPreferencesHydrated = false;
	let readerPreferencesBackendAvailable = false;
	let readerPreferencesSaveTimer: ReturnType<typeof setTimeout> | null = null;
	let lastReaderPreferencesSignature = "";

	$: acquisitionTasks = taskList(taskSyncState);

	// Backend event replays can fire in bursts — one in-flight snapshot is
	// shared by all callers instead of stacking one IPC per event.
	let acquisitionSnapshotInFlight: Promise<void> | null = null;
	function refreshAcquisitionSnapshot(): Promise<void> {
		if (acquisitionSnapshotInFlight) return acquisitionSnapshotInFlight;
		const nonce = ++taskRefreshNonce;
		acquisitionSnapshotInFlight = (async () => {
			try {
				const snapshot = await invoke<AcquisitionSnapshot>("get_acquisition_snapshot", {
					kind: null
				});
				if (nonce !== taskRefreshNonce) return;
				taskSyncState = snapshotState(snapshot.tasks);
			} catch (error) {
				noticeError("无法同步任务队列", error);
			} finally {
				acquisitionSnapshotInFlight = null;
			}
		})();
		return acquisitionSnapshotInFlight;
	}

	// 02-F5: `acquisition://task-event` is the fast path, but a dropped emit
	// leaves a row showing a stale state until the next manual refresh. While
	// any task is non-terminal, re-poll the snapshot on a slow cadence — the
	// interval only exists while something is still moving, and teardown in
	// onMount's cleanup clears it with the rest.
	const ACTIVE_LIFECYCLE_STATES: ReadonlySet<LifecycleState> = new Set([
		"queued",
		"starting",
		"running",
		"pausing",
		"paused",
		"stopping",
	]);
	let taskPollTimer: ReturnType<typeof setInterval> | null = null;
	$: {
		const hasActiveTask = acquisitionTasks.some((task) =>
			ACTIVE_LIFECYCLE_STATES.has(task.lifecycleState)
		);
		if (hasActiveTask && taskPollTimer === null) {
			taskPollTimer = setInterval(() => {
				void refreshAcquisitionSnapshot();
			}, 15_000);
		} else if (!hasActiveTask && taskPollTimer !== null) {
			clearInterval(taskPollTimer);
			taskPollTimer = null;
		}
	}

	// Terminal task events each triggered a full library refresh (4 invokes);
	// coalesce a burst into a single refresh after 300ms quiet.
	let libraryRefreshTimer: ReturnType<typeof setTimeout> | null = null;
	function scheduleLibraryRefresh() {
		if (libraryRefreshTimer) return;
		libraryRefreshTimer = setTimeout(() => {
			libraryRefreshTimer = null;
			void refreshLibrary();
		}, 300);
	}

	function receiveTaskEvent(event: TaskEvent): void {
		taskEventLog = [event, ...taskEventLog.filter((entry) => !(entry.taskId === event.taskId && entry.sequence === event.sequence))].slice(0, 60);
		if (taskEventPublishesToLibrary(event)) {
			scheduleLibraryRefresh();
		}
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

	// postMessage target: the reader session URL's exact origin instead of "*".
	function flowMessageTargetOrigin(): string {
		try {
			return flowReaderSession ? new URL(flowReaderSession.url).origin : "*";
		} catch {
			return "*";
		}
	}

	function postFlowFontScale(scale: number) {
		const win = flowIframeEl?.contentWindow;
		if (!win || !flowReaderSession) return;
		if (lastPostedFontScale === scale) return;
		lastPostedFontScale = scale;
		try {
			win.postMessage(
				createFlowSetFontScaleMessage(scale),
				flowMessageTargetOrigin(),
			);
		} catch {
			// iframe may be mid-navigation — the load handler re-sends on arrival
			lastPostedFontScale = null;
		}
	}

	let windowMaximized = false;
	let lastPostedLayoutWide: boolean | null = null;
	let lastPostedFontScale: number | null = null;
	let flowSyncTarget: {
		session: { sessionId: string; url: string };
		iframe: HTMLIFrameElement;
	} | null = null;
	// Restore frames scheduled by handleWindowMaximizedChange — tracked so
	// teardown can cancel a frame that has not run yet.
	const maximizedScrollFrames = new Set<number>();

	function postFlowLayoutMode(wide: boolean) {
		const win = flowIframeEl?.contentWindow;
		if (!win || !flowReaderSession) return;
		if (lastPostedLayoutWide === wide) return;
		lastPostedLayoutWide = wide;
		try {
			win.postMessage(
				createFlowSetLayoutModeMessage(
					wide,
					wide ? WIDE_LAYOUT_MAX_WIDTH_PX : $readingWidth,
				),
				flowMessageTargetOrigin(),
			);
		} catch {
			// iframe may be mid-navigation
		}
	}

	// 连读 iframe 跟随应用主题：模板暴露自己的 token 名，由
	// flowThemeVars 翻译后整包推过去（含 color-scheme）。
	let lastPostedThemeName: string | null = null;
	function postFlowTheme() {
		const win = flowIframeEl?.contentWindow;
		if (!win || !flowReaderSession) return;
		if (lastPostedThemeName === $currentTheme.name) return;
		lastPostedThemeName = $currentTheme.name;
		try {
			win.postMessage(
				createFlowSetThemeMessage(
					$currentTheme.scheme,
					flowThemeVars($currentTheme),
				),
				flowMessageTargetOrigin(),
			);
		} catch {
			lastPostedThemeName = null;
		}
	}

	function handleWindowMaximizedChange(maximized: boolean) {
		if (windowMaximized === maximized) return;
		const progress = contentEl
			? contentEl.scrollTop /
				Math.max(1, contentEl.scrollHeight - contentEl.clientHeight)
			: readingProgress;
		windowMaximized = maximized;
		const frame = requestAnimationFrame(() => {
			maximizedScrollFrames.delete(frame);
			if (!contentEl || !$currentFilePath) return;
			contentEl.scrollTop =
				progress * Math.max(0, contentEl.scrollHeight - contentEl.clientHeight);
			if ($focusMode) {
				invalidateFocusMetrics();
				scheduleFocusUpdate(lastFocusedIdx >= 0 ? lastFocusedIdx : undefined);
			}
		});
		maximizedScrollFrames.add(frame);
		if (flowReaderSession) postFlowLayoutMode(maximized);
	}

	function setFontScale(next: number, options?: { fromFlow?: boolean }) {
		const clamped = clampFontScale(next);
		zoomIndicatorText = `字号 ${Math.round(clamped * 100)}%`;
		if (zoomIndicatorTimer) clearTimeout(zoomIndicatorTimer);
		zoomIndicatorTimer = setTimeout(() => {
			zoomIndicatorText = "";
			zoomIndicatorTimer = null;
		}, 1200);
		if (clamped !== $fontScale) $fontScale = clamped;
		// Flow-originated changes already applied the scale; avoid echo loops.
		if (!options?.fromFlow) postFlowFontScale(clamped);
	}

	// Keep the continuous-reader iframe in sync with settings / store updates.
	// A new session/iframe re-sends both messages; between those, each postFlow*
	// dedups on its own last-sent value so unrelated store changes are no-ops.
	$: if (flowReaderSession && flowIframeEl) {
		if (
			flowSyncTarget?.session !== flowReaderSession ||
			flowSyncTarget.iframe !== flowIframeEl
		) {
			flowSyncTarget = { session: flowReaderSession, iframe: flowIframeEl };
			lastPostedFontScale = null;
			lastPostedLayoutWide = null;
			lastPostedThemeName = null;
		}
		postFlowFontScale($fontScale);
		postFlowLayoutMode(windowMaximized);
		postFlowTheme();
	} else {
		flowSyncTarget = null;
	}

	function adjustFontScale(direction: number) {
		setFontScale($fontScale + direction * FONT_SCALE_STEP);
	}

	function currentReaderPreferences(): DurableReaderPreferences {
		return {
			schemaVersion: 1,
			fontScale: clampFontScale($fontScale),
			lineHeight: $readingLineHeight,
			contentWidth: $readingWidth,
			fontFamily: $readingFontFamily,
		};
	}

	async function saveDurableReaderPreferences() {
		if (!readerPreferencesBackendAvailable) return;
		try {
			await invoke("save_reader_preferences", {
				preferences: currentReaderPreferences(),
			});
		} catch (error) {
			console.warn("Unable to save durable reader preferences.", error);
		}
	}

	function scheduleDurableReaderPreferencesSave() {
		if (!readerPreferencesHydrated || !readerPreferencesBackendAvailable) return;
		if (readerPreferencesSaveTimer) clearTimeout(readerPreferencesSaveTimer);
		readerPreferencesSaveTimer = setTimeout(() => {
			readerPreferencesSaveTimer = null;
			void saveDurableReaderPreferences();
		}, 180);
	}

	async function hydrateDurableReaderPreferences() {
		try {
			const loaded = await invoke<DurableReaderPreferencesLoad>("load_reader_preferences");
			readerPreferencesBackendAvailable = true;
			if (loaded.storeExists) {
				const preferences = loaded.preferences;
				$fontScale = clampFontScale(preferences.fontScale);
				if ((READING_LINE_HEIGHTS as readonly number[]).includes(preferences.lineHeight)) {
					$readingLineHeight = preferences.lineHeight;
				}
				if ((READING_WIDTHS as readonly number[]).includes(preferences.contentWidth)) {
					$readingWidth = preferences.contentWidth;
				}
				if (preferences.fontFamily === "sans" || preferences.fontFamily === "serif") {
					$readingFontFamily = preferences.fontFamily;
				}
			}
			readerPreferencesHydrated = true;
			lastReaderPreferencesSignature = `${$fontScale}|${$readingLineHeight}|${$readingWidth}|${$readingFontFamily}`;
			if (!loaded.storeExists) {
				await saveDurableReaderPreferences();
			}
		} catch (error) {
			readerPreferencesHydrated = true;
			console.warn("Unable to load durable reader preferences; using WebView fallback.", error);
		}
	}

	$: {
		const signature = `${$fontScale}|${$readingLineHeight}|${$readingWidth}|${$readingFontFamily}`;
		if (readerPreferencesHydrated && signature !== lastReaderPreferencesSignature) {
			lastReaderPreferencesSignature = signature;
			scheduleDurableReaderPreferencesSave();
		}
	}

	// Any typography change (zoom, line height, width, font) reflows the whole
	// document. Re-anchor on the block under the viewport anchor line — the
	// same viewport-anchor mechanism focus mode uses (raw scroll progress
	// drifts when block heights change) — and re-align focus mode on the
	// same unit.
	let lastTypographySignature: string | null = null;
	let typographyReflowFrame: number | null = null;
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
		if (!contentEl || typographyReflowFrame !== null) return;
		const anchor = captureViewportAnchor();
		const progress = readingProgress;
		typographyReflowFrame = requestAnimationFrame(() => {
			typographyReflowFrame = null;
			if (!contentEl) return;
			if (anchor && anchor.element.isConnected) {
				restoreViewportAnchor(anchor);
			} else {
				contentEl.scrollTop =
					progress * Math.max(0, contentEl.scrollHeight - contentEl.clientHeight);
			}
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

	// Shared 200ms hide delay: gives pointer/keyboard users time to move onto
	// the preview card itself without it vanishing underneath them.
	function cancelFootnoteHide() {
		if (!footnoteHideTimer) return;
		clearTimeout(footnoteHideTimer);
		footnoteHideTimer = null;
	}

	function scheduleFootnoteHide() {
		cancelFootnoteHide();
		footnoteHideTimer = setTimeout(() => {
			footnoteHideTimer = null;
			hideFootnotePreview();
		}, 200);
	}

	// ===== Image lightbox =====
	let lightboxSrc = "";
	let lightboxAlt = "";
	let lightboxScale = 1;
	let lightboxCloseEl: HTMLButtonElement | null = null;
	let lightboxReturnFocus: HTMLElement | null = null;

	function openLightbox(img: HTMLImageElement) {
		lightboxSrc = img.currentSrc || img.src;
		lightboxAlt = img.alt || "";
		lightboxScale = 1;
		// Keyboard path: move focus onto the close button (Esc is handled by the
		// global keydown) and restore it when the lightbox goes away.
		lightboxReturnFocus =
			document.activeElement instanceof HTMLElement ? document.activeElement : null;
		void tick().then(() => lightboxCloseEl?.focus());
	}

	function closeLightbox() {
		lightboxSrc = "";
		lightboxAlt = "";
		lightboxScale = 1;
		const returnTo = lightboxReturnFocus;
		lightboxReturnFocus = null;
		if (returnTo?.isConnected) returnTo.focus();
	}

	function handleLightboxWheel(e: WheelEvent) {
		e.preventDefault();
		const factor = e.deltaY < 0 ? 1.12 : 1 / 1.12;
		lightboxScale = Math.min(6, Math.max(0.2, lightboxScale * factor));
	}

	// true when opened as a single file (not library book chapter) — label only, not chrome pin.
	let isTemporaryReading = false;

	// Unified immersion chrome: window bar + context toolbar share chromeVisible.
	let chromeState: ChromeState = createChromeState("library");
	let chromeHideTimer: ReturnType<typeof setTimeout> | null = null;
	let chromeStackEl: HTMLElement | null = null;
	let flowIframeEl: HTMLIFrameElement | null = null;
	let readingCursorState: ReadingCursorState = createReadingCursorState();
	let readingCursorHideTimer: ReturnType<typeof setTimeout> | null = null;
	let readingCursorMoveFrame: number | null = null;

	function clearChromeHideTimer() {
		if (chromeHideTimer) {
			clearTimeout(chromeHideTimer);
			chromeHideTimer = null;
		}
	}

	function dispatchChrome(event: ChromeEvent) {
		chromeState = reduceChrome(chromeState, event);
		if (chromeState.shouldCancelHide) {
			clearChromeHideTimer();
		}
		if (chromeState.shouldScheduleHide && !chromeHideTimer) {
			chromeHideTimer = setTimeout(() => {
				chromeHideTimer = null;
				dispatchChrome({ type: "apply-hide" });
			}, CHROME_HIDE_DELAY_MS);
		}
	}

	function syncChromeSurface() {
		const surface = deriveChromeSurface({
			flowActive: !!flowReaderSession,
			focusMode: $focusMode,
			fileOpen: !!$currentFilePath,
			workflowOpen: podcastWorkflowOpen || zhihuWorkflowOpen,
		});
		if (surface !== chromeState.surface) {
			dispatchChrome({ type: "enter-surface", surface });
		}
	}

	function noteReadingActivity() {
		if (isImmersiveSurface(chromeState.surface)) {
			dispatchChrome({ type: "reading-activity" });
		}
	}

	// Wheel/scroll bursts would dispatch one chrome event per input event.
	// The reducer already early-returns when nothing changes, but gating the
	// notification itself to one dispatch per frame avoids the call entirely.
	let readingActivityFrame: number | null = null;
	function scheduleReadingActivity() {
		if (readingActivityFrame !== null) return;
		readingActivityFrame = requestAnimationFrame(() => {
			readingActivityFrame = null;
			noteReadingActivity();
		});
	}

	function revealChromeFromTopEdge() {
		dispatchChrome({ type: "top-edge-enter" });
	}

	// Keyboard path to the immersive chrome — the pointer gets the top-edge
	// hotzone; without this a keyboard user had no way to reach the toolbar or
	// window controls once chrome auto-hid (and the inert stack trapped them).
	async function revealChromeForKeyboard() {
		if (isImmersiveSurface(chromeState.surface)) {
			dispatchChrome({ type: "top-edge-enter" });
		}
		await tick();
		const target =
			chromeStackEl?.querySelector<HTMLElement>(".context-bar button") ??
			chromeStackEl?.querySelector<HTMLElement>(
				"button, [href], input, [tabindex]:not([tabindex='-1'])"
			);
		target?.focus();
	}

	function onChromeMouseEnter() {
		dispatchChrome({ type: "cancel-hide" });
	}

	function onChromeMouseLeave() {
		dispatchChrome({ type: "chrome-leave" });
	}

	function onChromeFocusIn() {
		dispatchChrome({ type: "chrome-focus" });
	}

	function onChromeFocusOut(event: FocusEvent) {
		const next = event.relatedTarget as Node | null;
		const stack = event.currentTarget as HTMLElement | null;
		if (stack && next && stack.contains(next)) return;
		dispatchChrome({ type: "chrome-blur" });
	}

	function clearReadingCursorHideTimer() {
		if (readingCursorHideTimer) {
			clearTimeout(readingCursorHideTimer);
			readingCursorHideTimer = null;
		}
	}

	function dispatchReadingCursor(event: ReadingCursorEvent) {
		readingCursorState = reduceReadingCursor(readingCursorState, event);
		if (readingCursorState.shouldCancelHide) {
			clearReadingCursorHideTimer();
		}
		if (readingCursorState.shouldScheduleHide && !readingCursorHideTimer) {
			readingCursorHideTimer = setTimeout(() => {
				readingCursorHideTimer = null;
				dispatchReadingCursor({ type: "apply-hide" });
			}, READING_CURSOR_HIDE_DELAY_MS);
		}
	}

	$: {
		const readingSurfaceActive = !!$currentFilePath && !flowReaderSession;
		if (readingSurfaceActive !== readingCursorState.active) {
			dispatchReadingCursor({ type: readingSurfaceActive ? "enter-reading" : "leave-reading" });
		}
	}

	// The rendered shell is the final authority: bookshelf/workflow/trash
	// surfaces always reserve space for the window bar. This second guard keeps
	// a transient reader state from ever placing the homepage underneath it.
	$: librarySurface = !flowReaderSession && !$currentFilePath;
	$: chromeVisible = librarySurface ? true : chromeState.chromeVisible;
	$: chromeOverlay = !librarySurface && isOverlaySurface(chromeState.surface);
	$: showMarkdownContext =
		!!$currentFilePath && !flowReaderSession;
	$: showFlowContext = !!flowReaderSession;
	$: showTopEdgeHotzone = isImmersiveSurface(chromeState.surface);
	$: {
		// Modal layers (nav guard / workflows / book detail / trash) must own
		// the topmost surface — the Ctrl+F overlay sits at z-1000 and would
		// otherwise cover the non-top-layer dialogs, so yield it on open.
		if (
			navigationGuardOpen ||
			actionConfirm !== null ||
			podcastWorkflowOpen ||
			zhihuWorkflowOpen ||
			selectedBookDetail !== null ||
			trashOpen
		) {
			closeSearchOverlay();
		}
	}
	$: {
		// Keep surface in sync with mode flags without fighting explicit dispatches mid-transition.
		void flowReaderSession;
		void $focusMode;
		void $currentFilePath;
		void podcastWorkflowOpen;
		void zhihuWorkflowOpen;
		syncChromeSurface();
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
	const ARTICLE_LINK_OPEN_DELAY_MS = 240;
	let focusEdgeSpace = 0;
	// Accumulator for Ctrl+wheel / pinch zoom gestures (see handleWheel).
	let ctrlWheelDelta = 0;
	let focusWheelDelta = 0;
	let focusWheelResetTimer: ReturnType<typeof setTimeout> | null = null;
	// Wheel ticks can cross the FOCUS_WHEEL_STEP threshold several times per
	// frame; the accumulated steps are applied once per frame so each move's
	// style/scroll writes batch into one layout pass instead of thrashing.
	let pendingFocusWheelSteps = 0;
	let focusWheelMoveFrame: number | null = null;
	let lastFocusedIdx = -1;

	function isTextInputTarget(target: EventTarget | null) {
		const el = target instanceof HTMLElement ? target : null;
		return !!el?.closest("input, textarea, select, [contenteditable='true']");
	}

	function isMarkdownPath(path: string): boolean {
		const lower = path.toLowerCase();
		return lower.endsWith('.md') || lower.endsWith('.markdown');
	}

	// Read one 0-based line without materializing a full split("\n") array —
	// the edit-save path only needs the lines adjacent to the edited block.
	function sourceLineAt(source: string, lineIndex: number): string | null {
		let start = 0;
		for (let i = 0; i < lineIndex; i += 1) {
			const next = source.indexOf("\n", start);
			if (next === -1) return null;
			start = next + 1;
		}
		const end = source.indexOf("\n", start);
		return source.slice(start, end === -1 ? source.length : end);
	}

	function rejectPendingMarkdownRenders(error: Error) {
		pendingMarkdownRenders.forEach(({ reject }) => reject(error));
		pendingMarkdownRenders.clear();
	}

	// F4: worker renders run FIFO and cannot be cancelled once posted — a
	// stale in-flight render (superseded navigation, obsolete chapter preload)
	// would head-of-line block the next document. Recycle the worker; it is
	// lazily rebuilt by the next renderMarkdownForUi call. Losers rethrow this
	// sentinel instead of falling back to a main-thread render of a document
	// nobody will show.
	const RENDER_SUPERSEDED = new Error("markdown render superseded");

	function dropStaleMarkdownRenders() {
		if (pendingMarkdownRenders.size === 0) return;
		const staleWorker = markdownWorker;
		markdownWorker = null;
		rejectPendingMarkdownRenders(RENDER_SUPERSEDED);
		staleWorker?.terminate();
	}

	function noteMarkdownWorkerTransportFailure() {
		// Runtime transport failures are recoverable — the next render lazily
		// builds a fresh worker. Only a worker that keeps dying (e.g. a script
		// that can never load) trips the permanent fallback so editing does not
		// pay a doomed rebuild on every render.
		markdownWorkerTransportFailures += 1;
		if (markdownWorkerTransportFailures >= MARKDOWN_WORKER_MAX_TRANSPORT_FAILURES) {
			markdownWorkerFailed = true;
		}
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
				noteMarkdownWorkerTransportFailure();
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

	async function renderMarkdownForUi(
		source: string,
		options: { speculative?: boolean } = {},
	): Promise<RenderedMarkdownDocument> {
		let worker = getMarkdownWorker();
		if (worker && !options.speculative && pendingMarkdownRenders.size > 0) {
			// A queued speculative preload must not head-of-line block a
			// user-facing render (open/edit): when every pending job is
			// speculative, recycle the worker so this render starts now.
			let allSpeculative = true;
			for (const pending of pendingMarkdownRenders.values()) {
				if (!pending.speculative) {
					allSpeculative = false;
					break;
				}
			}
			if (allSpeculative) {
				dropStaleMarkdownRenders();
				worker = getMarkdownWorker();
			}
		}
		if (worker) {
			const id = nextRenderRequestId++;
			// A synchronous postMessage throw must not leave a dead pending
			// entry — remove it before rejecting; postFailed keeps the catch
			// below treating it as a transport failure (not a document error).
			let postFailed = false;
			try {
				const rendered = await new Promise<RenderedMarkdownDocument>((resolve, reject) => {
					const timer = setTimeout(() => {
						// Entry stays in the map so the catch below classifies this
						// as a transport failure (orphanedRequest found), not a
						// document error — a wedged worker must be recycled.
						reject(new Error("Markdown 渲染超时"));
					}, MARKDOWN_WORKER_TIMEOUT_MS);
					pendingMarkdownRenders.set(id, {
						speculative: options.speculative === true,
						resolve: (result) => {
							clearTimeout(timer);
							resolve(result);
						},
						reject: (error) => {
							clearTimeout(timer);
							reject(error);
						},
					});
					try {
						worker.postMessage({ id, source });
					} catch (postErr) {
						postFailed = true;
						pendingMarkdownRenders.delete(id);
						clearTimeout(timer);
						reject(postErr instanceof Error ? postErr : new Error(String(postErr)));
					}
				});
				markdownWorkerTransportFailures = 0;
				return rendered;
			} catch (err) {
				// Superseded renders rethrow instead of falling back to the
				// main thread — the caller's load-token check discards the
				// result anyway, so rendering inline only stalls the UI.
				if (err === RENDER_SUPERSEDED) throw err;
				const orphanedRequest = pendingMarkdownRenders.delete(id);
				if (!orphanedRequest && !postFailed && markdownWorker === worker) {
					// Document-level failure: the worker answered {id,error}, so it
					// is healthy and stays alive. The same source would fail the
					// same way on the main thread — propagate instead of retrying.
					throw err;
				}
				// Transport-level failure (postMessage threw / worker error event
				// / teardown): drop this worker and fail every in-flight render so
				// pending preload/navigation promises can never hang. The next
				// render lazily rebuilds a worker unless failures keep recurring.
				if (markdownWorker === worker) {
					noteMarkdownWorkerTransportFailure();
					markdownWorker = null;
				}
				worker.terminate();
				rejectPendingMarkdownRenders(err instanceof Error ? err : new Error(String(err)));
				console.warn("Markdown worker render failed; falling back to main thread.", err);
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
	let domPurifyModule: typeof import("dompurify").default | null = null;
	let nextMermaidId = 1;

	// Mermaid's strict mode already sanitizes internally, but this output is
	// the one DOM sink that bypasses the rehype-sanitize pipeline — a mermaid
	// bypass would become a main-WebView XSS primitive, so the SVG goes
	// through our own DOMPurify pass (svg-only profile) before innerHTML.
	function sanitizeMermaidSvg(svg: string): string {
		return domPurifyModule
			? domPurifyModule.sanitize(svg, {
					USE_PROFILES: { svg: true, svgFilters: true },
				})
			: svg;
	}

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
			if (!domPurifyModule) {
				domPurifyModule = (await import("dompurify")).default;
			}
		} catch (err) {
			console.warn("Mermaid unavailable:", err);
			return;
		}

		mermaidModule.initialize({
			startOnLoad: false,
			theme: getMermaidTheme(),
			securityLevel: "strict"
		});
		let replaced = false;
		// mermaid.render is main-thread CPU work — serial awaits on a
		// diagram-heavy document stall the page; a small pool bounds it.
		const renderOne = async (code: Element) => {
			const pre = code.closest("pre");
			if (!pre || !pre.isConnected) return;
			const src = code.textContent ?? "";
			const renderId = `mmbook-mermaid-${nextMermaidId++}`;
			try {
				const { svg } = await mermaidModule.render(renderId, src);
				if (!pre.isConnected) return;
				const container = document.createElement("div");
				container.className = "mermaid-diagram";
				container.dataset.mermaidSource = src;
				const start = pre.getAttribute("data-source-start");
				const end = pre.getAttribute("data-source-end");
				if (start) container.setAttribute("data-source-start", start);
				if (end) container.setAttribute("data-source-end", end);
				container.innerHTML = sanitizeMermaidSvg(svg);
				pre.replaceWith(container);
				replaced = true;
			} catch (err) {
				console.warn("Mermaid render failed:", err);
				document.getElementById(`d${renderId}`)?.remove();
			}
		};
		const MERMAID_RENDER_CONCURRENCY = 3;
		let cursor = 0;
		await Promise.all(
			Array.from(
				{ length: Math.min(MERMAID_RENDER_CONCURRENCY, codes.length) },
				async () => {
					while (cursor < codes.length) {
						await renderOne(codes[cursor++]);
					}
				},
			),
		);

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
	// The trigger keys on the effective mermaid theme (light/dark), not the
	// theme name: two dark themes share the same diagram colors and must not
	// re-render. The generation counter guards concurrency — a retheme that
	// is still awaiting mermaid.render when the theme flips again must not
	// overwrite containers with stale-theme SVG.
	let mermaidGeneration = 0;
	let lastMermaidIsLight: boolean | null = null;
	$: if (isLightTheme !== lastMermaidIsLight) {
		lastMermaidIsLight = isLightTheme;
		mermaidGeneration += 1;
		void rethemeMermaidDiagrams(mermaidGeneration);
	}

	async function rethemeMermaidDiagrams(generation: number) {
		if (!mermaidModule) return;
		const article = getArticleElement();
		const containers = article
			? (Array.from(article.querySelectorAll(".mermaid-diagram")) as HTMLElement[])
			: [];
		if (containers.length === 0) return;
		mermaidModule.initialize({
			startOnLoad: false,
			theme: getMermaidTheme(),
			securityLevel: "strict"
		});
		for (const container of containers) {
			if (generation !== mermaidGeneration) return;
			const src = container.dataset.mermaidSource ?? "";
			if (!src || !container.isConnected) continue;
			const renderId = `mmbook-mermaid-${nextMermaidId++}`;
			try {
				const { svg } = await mermaidModule.render(renderId, src);
				if (generation !== mermaidGeneration || !container.isConnected) continue;
				container.innerHTML = sanitizeMermaidSvg(svg);
			} catch {
				document.getElementById(`d${renderId}`)?.remove();
			}
		}
	}

	function getExternalUrlToOpen(rawHref: string | null): string | null {
		if (!rawHref || !/^[a-z][a-z0-9+.-]*:/i.test(rawHref)) return null;
		try {
			const url = new URL(rawHref, window.location.href);
			if (["http:", "https:"].includes(url.protocol)) {
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

	// Relative links resolve against the current file's directory. Absolute
	// (drive-letter / leading-slash) targets pass through as-is — the backend
	// command still gates what may actually be read.
	function resolveRelativeArticleLink(
		rawHref: string,
	): { path: string; anchor: string } | null {
		const hashIdx = rawHref.indexOf("#");
		const anchorRaw = hashIdx >= 0 ? rawHref.slice(hashIdx + 1) : "";
		const rel = (hashIdx >= 0 ? rawHref.slice(0, hashIdx) : rawHref).split("?")[0];
		if (!rel) return null;
		let decoded: string;
		try {
			decoded = decodeURIComponent(rel);
		} catch {
			decoded = rel;
		}
		let anchor = "";
		try {
			anchor = decodeURIComponent(anchorRaw);
		} catch {
			anchor = anchorRaw;
		}
		const normalized = decoded.replace(/\\/g, "/");
		if (/^[a-zA-Z]:\//.test(normalized) || normalized.startsWith("/")) {
			return { path: normalized, anchor };
		}
		const base = $currentFilePath;
		if (!base) return null;
		const baseDir = base.replace(/\\/g, "/").replace(/\/[^/]*$/, "");
		const parts = baseDir.split("/");
		for (const seg of normalized.split("/")) {
			if (seg === "" || seg === ".") continue;
			if (seg === "..") {
				if (parts.length <= 1) return null;
				parts.pop();
			} else {
				parts.push(seg);
			}
		}
		return { path: parts.join("/"), anchor };
	}

	async function openRelativeArticleLink(rawHref: string) {
		const resolved = resolveRelativeArticleLink(rawHref);
		if (!resolved || !/\.(md|markdown)$/i.test(resolved.path)) {
			showAppNotice("仅支持打开 Markdown 文件链接");
			return;
		}
		try {
			// Doubles as an existence + allowed-path check via the backend.
			await invoke<number>("get_file_mtime", { path: resolved.path });
		} catch {
			showAppNotice("链接的目标文件不存在或不可读");
			return;
		}
		const opened = await openFile(resolved.path, { bookChapter: !!activeBook });
		if (opened && resolved.anchor) scrollToHeading(resolved.anchor);
	}

	function handleArticleLinkClick(event: MouseEvent, link: HTMLAnchorElement): boolean {
		const rawHref = link.getAttribute("href");

		// In-document anchors go through the same jump logic as the TOC, so focus
		// mode stays anchored and long jumps don't fly.
		if (rawHref && rawHref.startsWith("#")) {
			event.preventDefault();
			event.stopPropagation();
			hideFootnotePreview();
			try {
				scrollToHeading(decodeURIComponent(rawHref.slice(1)));
			} catch {
				scrollToHeading(rawHref.slice(1));
			}
			return true;
		}

		const externalUrl = getExternalUrlToOpen(rawHref);
		if (!externalUrl) {
			// 相对路径与非 http(s) 协议链接绝不能走默认导航——WebView 会去
			// 加载 tauri.localhost 下的地址，把整个应用页面换掉。Markdown
			// 链接按当前文件目录解析后在阅读器内打开，其余提示不支持。
			event.preventDefault();
			event.stopPropagation();
			hideFootnotePreview();
			if (rawHref) void openRelativeArticleLink(rawHref);
			return true;
		}

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
		const nativeTitle = name ? `${name} — 沉浸阅读` : "沉浸阅读";
		void getCurrentWebviewWindow()
			.setTitle(nativeTitle)
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

	// First-run hint: the window X hides to the tray (tasks keep running) — it
	// does not quit. Shown once per install via a localStorage flag.
	function maybeShowTrayHint() {
		try {
			if (localStorage.getItem("mmbook-tray-hint-shown")) return;
			localStorage.setItem("mmbook-tray-hint-shown", "1");
		} catch {
			return;
		}
		showAppNotice("提示：右上角 X 会把窗口最小化到系统托盘（任务继续运行），托盘菜单选「退出」才会真正关闭");
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

	// User-facing failure notice: raw backend codes go to the console, the
	// notice shows the localized description from lib/errors.ts.
	function noticeError(prefix: string, error: unknown) {
		showAppNotice(`${prefix}：${reportError(prefix, error)}`);
	}

	// Close the Ctrl+F overlay so it can never sit above a modal layer.
	function closeSearchOverlay() {
		if (!$searchOpen) return;
		$searchOpen = false;
		$searchQuery = "";
		clearSearchHighlights();
	}

	// Request ids derive from the operation inputs (see $lib/requestId):
	// retrying the same action reproduces the same id, so the backend's
	// idempotent command claim dedupes retries rather than spawning
	// duplicate tasks — including the no-WebCrypto fallback, which used to
	// mint a random id per call and silently disable the dedupe.

	// Overlapping refreshes (task-event bursts interleave with explicit calls)
	// resolved out of order used to make the bookshelf flash a stale list.
	// Only the latest call may land its results.
	let libraryRefreshNonce = 0;
	let libraryLoadedOnce = false;
	async function refreshLibrary() {
		const nonce = ++libraryRefreshNonce;
		// Runs on every task event — only the first load may flip the view to a
		// spinner; later refreshes update in place so the resume card doesn't flash.
		if (!libraryLoadedOnce) libraryLoading = true;
		try {
			// The trash list is only read while its panel is open — the bookshelf
			// badge follows the panel's own refreshes instead.
			const [settings, scan, nextTemporaryItems, nextTrashItems] = await Promise.all([
				invoke<AppSettings>("get_app_settings"),
				invoke<{ books: BookSummary[]; issues: LibraryIssue[]; writable: boolean }>("scan_library"),
				invoke<TemporaryItem[]>("list_temporary_content").catch(() => [] as TemporaryItem[]),
				trashOpen
					? invoke<TrashItem[]>("list_trash").catch(() => [] as TrashItem[])
					: Promise.resolve(null),
			]);
			if (nonce !== libraryRefreshNonce) return;
			appSettings = settings;
			libraryBooks = scan.books;
			libraryIssues = scan.issues.map((issue) => ({
				...issue,
				message: describeError(issue.message),
			}));
			libraryWritable = scan.writable;
			temporaryItems = nextTemporaryItems;
			if (nextTrashItems !== null) trashItems = nextTrashItems;
		} catch (error) {
			if (nonce !== libraryRefreshNonce) return;
			libraryBooks = [];
			libraryIssues = [{ path: appSettings?.libraryRoot ?? "书库", message: reportError("书库扫描", error) }];
			libraryWritable = false;
		} finally {
			if (nonce === libraryRefreshNonce) {
				libraryLoadedOnce = true;
				libraryLoading = false;
			}
		}
	}

	async function importFolderToLibrary() {
		if (importFolderBusy) return;
		const selected = await open({ directory: true, multiple: false, title: "选择 Markdown 文件夹" });
		if (!selected || Array.isArray(selected)) return;
		importFolderBusy = true;
		showAppNotice("正在导入文件夹…");
		try {
			const manifest = await invoke<{ bookId: string }>("import_markdown_folder", { path: selected });
			await refreshLibrary();
			await openLibraryBook(manifest.bookId);
		} catch (error) {
			noticeError("导入失败", error);
		} finally {
			importFolderBusy = false;
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
			noticeError("无法更新书库", error);
		}
	}

	async function removeLibraryBook(bookId: string, title: string, chapterCount: number) {
		const ok = await confirmAction(
			`将《${title}》移出书架？\n\n${chapterCount} 篇内容会移入回收站，可手动恢复，不会立刻永久删除。`,
			"移出书架"
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
			noticeError("移出失败", error);
		}
	}

	async function deleteLibraryBook(bookId: string, title: string, chapterCount: number) {
		const ok = await confirmAction(
			`永久删除《${title}》？\n\n将删除磁盘上的 ${chapterCount} 篇 Markdown 与阅读进度，不可恢复。`,
			"永久删除",
			true
		);
		if (!ok) return;
		const again = await confirmAction("再次确认：此操作不可撤销。", "确认删除", true);
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
			noticeError("删除失败", error);
		}
	}

	async function openBrowserReader(bookId: string) {
		if (flowReaderStarting) return;
		try {
			if ((await requestNavigationGuard("进入连读")) === "cancel") return;
			flowReaderStarting = true;
			showAppNotice("正在启动连读…");
			await flushSaveState();
			if (flowReaderSession) {
				await invoke("close_reader_session", { sessionId: flowReaderSession.sessionId });
			}
			flowReaderSession = await invoke<{ sessionId: string; url: string }>(
				"start_reader_session",
				{ bookId }
			);
		} catch (error) {
			noticeError("无法启动连读", error);
		} finally {
			flowReaderStarting = false;
		}
	}

	async function startPodcastTask(taskId: string) {
		try {
			// Backend returns false when the worker slot is taken and the task
			// stays Queued — "已启动" would lie about a task that is not running.
			const started = await invoke<boolean>("start_podcast_task", { taskId });
			showAppNotice(started ? "播客任务已启动" : "播客任务已排队，等待空闲槽位");
		} catch (error) {
			noticeError("无法启动播客任务", error);
		}
	}

	async function startZhihuTask(taskId: string, expectedRevision: number) {
		try {
			await invoke("start_zhihu_task", { taskId, expectedRevision });
			showAppNotice("知乎任务已启动");
			await refreshAcquisitionSnapshot();
		} catch (error) {
			noticeError("无法启动知乎任务", error);
		}
	}

	async function controlZhihuTask(
		taskId: string,
		action: "pause" | "resume" | "cancel",
		expectedRevision: number
	) {
		// Call sites (task rows) already confirm cancel — a second confirm here
		// made every zhihu cancel ask twice.
		try {
			await invoke("control_zhihu_task", {
				taskId,
				action,
				expectedRevision,
				requestId: await stableRequestId("control_zhihu_task", taskId, action, expectedRevision)
			});
			showAppNotice(action === "pause" ? "知乎任务已暂停" : action === "resume" ? "知乎任务已恢复" : "知乎任务已取消");
			await refreshAcquisitionSnapshot();
		} catch (error) {
			noticeError("知乎任务控制失败", error);
		}
	}

	async function restartPodcastTask(taskId: string, budgetLimitCny?: number) {
		try {
			showAppNotice("正在重试播客任务…");
			const snapshot = await invoke<{
				id: string;
				outcome?: string;
				lifecycleState?: string;
			}>("restart_podcast_task", {
				taskId,
				budgetLimitCny: budgetLimitCny ?? null
			});
			if (snapshot.outcome === "success") {
				showAppNotice("发布成功，文稿已保存到 书库/播客");
			} else {
				// Same task id = in-place resume reusing the checkpoint; a new
				// id means a full re-run on a cloned task.
				showAppNotice(
					snapshot.id === taskId ? "已从上次进度续跑" : "已重新开始转写"
				);
			}
			await refreshAcquisitionSnapshot();
		} catch (error) {
			// Never let a retry error look like a silent crash — surface it clearly.
			showAppNotice(
				String(error).includes("INPUT_MISSING") || String(error).includes("TASK_CONTRACT_MISSING")
					? "无法重试：原音频或任务信息已丢失，请重新添加音频。"
					: `无法重试播客任务：${reportError("重试播客任务", error)}`
			);
			await refreshAcquisitionSnapshot();
		}
	}

	async function openPodcastTaskResult(taskId: string) {
		try {
			const detail = await invoke<BookDetail>("open_task_result", { taskId });
			await refreshLibrary();
			await openLibraryBook(detail.manifest.bookId);
		} catch (error) {
			const description = reportError("打开播客结果", error);
			showAppNotice(
				String(error).includes("Book not found") || String(error).includes("书架中找不到")
					? "无法打开播客：书架中还没有这本书。若转写刚完成，请点「重试」或重新打开。"
					: `无法打开播客结果：${description}`
			);
		}
	}

	async function controlPodcastTask(
		taskId: string,
		action: "pause" | "resume" | "cancel" | "cancel_and_discard",
		expectedRevision: number
	) {
		// TaskRow already confirms discard in its own dialog — a second
		// confirmAction here made every discard ask twice.
		try {
			await invoke("control_podcast_task", {
				taskId,
				action,
				expectedRevision,
				requestId: await stableRequestId("control_podcast_task", taskId, action, expectedRevision)
			});
			showAppNotice(
				action === "pause" ? "任务已暂停"
				: action === "resume" ? "任务已恢复"
				: action === "cancel_and_discard" ? "任务缓存已丢弃"
				: "任务已取消"
			);
		} catch (error) {
			noticeError("任务控制失败", error);
		}
	}

	async function refreshTrash(): Promise<void> {
		trashLoading = true;
		try {
			trashItems = await invoke<TrashItem[]>("list_trash");
		} catch (error) {
			noticeError("无法读取回收站", error);
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
				requestId: await stableRequestId("restore_trash_item", item.trashId, item.revision)
			});
			await Promise.all([refreshTrash(), refreshLibrary()]);
			showAppNotice(`已恢复《${item.title}》`);
		} catch (error) {
			noticeError("恢复失败", error);
		}
	}

	async function permanentlyDeleteTrashItem(item: TrashItem): Promise<void> {
		const confirmed = await confirmAction(
			`永久删除《${item.title}》？\n\n这会删除回收站中的全部正文、资源与阅读状态，无法恢复。`,
			"永久删除",
			true
		);
		if (
			!confirmed ||
			!(await confirmAction("再次确认：只删除这个受管回收站条目。", "确认删除", true))
		)
			return;
		try {
			const result = await invoke<TrashDeleteResult>("permanently_delete_trash_item", {
				trashId: item.trashId,
				expectedRevision: item.revision,
				requestId: await stableRequestId("permanently_delete_trash_item", item.trashId, item.revision)
			});
			await refreshTrash();
			showAppNotice(`已永久删除 ${result.deletedItems} 个项目`);
		} catch (error) {
			noticeError("永久删除失败", error);
		}
	}

	async function closeFlowReaderSession() {
		const session = flowReaderSession;
		flowReaderSession = null;
		if (!session) return;
		try {
			await invoke("close_reader_session", { sessionId: session.sessionId });
		} catch (error) {
			noticeError("无法关闭连读会话", error);
		}
	}

	async function closeFlowReader() {
		if ((await requestNavigationGuard("返回书架")) === "cancel") return;
		await closeFlowReaderSession();
	}

	async function openBookDetails(bookId: string) {
		if ((await requestNavigationGuard("打开书目详情")) === "cancel") return;
		try {
			selectedBookDetail = await invoke<BookDetail>("open_book", { bookId });
		} catch (error) {
			noticeError("无法读取书目详情", error);
		}
	}

	function openBookSource(source: string, sourceId?: string | null) {
		if (source !== "zhihu" || !sourceId) return;
		const url = getExternalUrlToOpen(`https://www.zhihu.com/people/${encodeURIComponent(sourceId)}`);
		if (url) void openUrl(url).catch((error) => noticeError("无法打开来源链接", error));
	}

	// chapterIndex comes from a detail-dialog chapter click: open that chapter
	// from its top; without it, resume at saved progress.
	async function openLibraryBook(bookId: string, chapterIndex?: number) {
		if ((await requestNavigationGuard("切换书目")) === "cancel") return;
		try {
			const detail = await invoke<BookDetail>("open_book", { bookId });
			// F12: an unacked crash shadow may hold progress the backend never
			// confirmed. Adopt it only when its `updated` stamp beats the on-disk
			// record — the flow reader can write the same record via HTTP, so a
			// stale shadow must never regress a newer save.
			const shadow = readProgressShadow(`book:${bookId}`);
			if (shadow && !shadow.acked && shadow.bookProgress) {
				const shadowed = shadow.bookProgress as { updated?: string };
				const diskStamp = typeof detail.progress?.updated === "string" ? detail.progress.updated : "";
				if (typeof shadowed.updated === "string" && shadowed.updated > diskStamp) {
					detail.progress = { ...detail.progress, ...(shadow.bookProgress as typeof detail.progress) };
				}
			}
			const index =
				chapterIndex !== undefined &&
				chapterIndex >= 0 &&
				chapterIndex < detail.manifest.chapters.length
					? chapterIndex
					: resolveChapterIndex(detail.manifest.chapters, detail.progress.current, detail.progress.read);
			if (index < 0) {
				showAppNotice("这本文集还没有可读章节");
				return;
			}
			const chapter = detail.manifest.chapters[index];
			const path = await invoke<string>("get_book_chapter_path", { bookId, chapterId: chapter.id });
			const opened = await openFile(path, {
				bookChapter: true,
				restoreRatio: chapterIndex !== undefined ? 0 : detail.progress.position,
				suppressRecent: true,
			});
			if (!opened) return;
			// Opening from the details dialog dismisses it — a stale detail must
			// not keep the modal drop guard engaged while reading.
			selectedBookDetail = null;
			activeBook = detail;
			activeChapterIndex = index;
			tocItems = chapterTocItems(detail.manifest.chapters);
			updateWindowTitle(`${detail.manifest.title} · ${chapter.title}`);
		} catch (error) {
			noticeError("无法打开文集", error);
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
		const shadowKey = `book:${activeBook.manifest.bookId}`;
		stageProgressShadow(shadowKey, {
			scrollPosition: contentEl?.scrollTop,
			progressRatio: readingProgress,
			bookProgress: progress,
		});
		await invoke("save_book_progress", { bookId: activeBook.manifest.bookId, progress });
		ackProgressShadow(shadowKey);
	}

	function preloadNextBookChapter(): Promise<void> {
		if (!activeBook || activeChapterIndex < 0 || readingProgress < 0.84 || preloadedChapter) {
			return Promise.resolve();
		}
		const index = activeChapterIndex + 1;
		const chapter = activeBook.manifest.chapters[index];
		if (!chapter) return Promise.resolve();
		// Every scroll event past 84% re-enters here; without an in-flight guard
		// each one would repeat the chapter file read + full markdown render
		// while the first preload is still awaiting its IPC calls.
		if (chapterPreloadInFlight) return chapterPreloadInFlight;
		const bookId = activeBook.manifest.bookId;
		chapterPreloadInFlight = (async () => {
			try {
				const path = await invoke<string>("get_book_chapter_path", { bookId, chapterId: chapter.id });
				const result = await invoke<{ content: string; encoding: string }>("read_markdown_file", { path });
				const rendered = await renderMarkdownForUi(result.content, { speculative: true });
				if (activeBook?.manifest.bookId === bookId && activeChapterIndex + 1 === index) {
					preloadedChapter = { bookId, index, path, result, rendered };
				}
			} catch {
				// The normal open path will show a concrete error if the next chapter is unavailable.
			}
		})().finally(() => {
			chapterPreloadInFlight = null;
		});
		return chapterPreloadInFlight;
	}

	type ChapterBoundaryRestore = {
		direction: -1 | 1;
		offsetPx: number;
	};

	function cancelChapterBoundaryRestore() {
		chapterBoundaryRestoreGeneration += 1;
	}

	async function restoreChapterBoundaryAfterLayout(boundary: ChapterBoundaryRestore) {
		if (!contentEl) return;
		const restoreGeneration = ++chapterBoundaryRestoreGeneration;
		const startedAt = performance.now();
		let previousMaxScrollTop = -1;
		let stableFrames = 0;

		// content-visibility and late font/layout work can grow scrollHeight for
		// several frames after the Markdown DOM appears. Stay attached to the
		// chapter seam until the height settles, otherwise "previous end"
		// silently collapses back to scrollTop 0.
		while (performance.now() - startedAt < 500) {
			await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
			if (!contentEl || restoreGeneration !== chapterBoundaryRestoreGeneration) return;

			const maxScrollTop = Math.max(0, contentEl.scrollHeight - contentEl.clientHeight);
			const targetScrollTop = resolveChapterBoundaryScroll(
				boundary.direction,
				maxScrollTop,
				boundary.offsetPx,
			);
			const previousScrollBehavior = contentEl.style.scrollBehavior;
			contentEl.style.scrollBehavior = "auto";
			contentEl.scrollTop = targetScrollTop;
			contentEl.style.scrollBehavior = previousScrollBehavior;
			readingProgress = maxScrollTop > 0 ? targetScrollTop / maxScrollTop : 0;

			const heightStable = Math.abs(maxScrollTop - previousMaxScrollTop) < 1;
			const zeroHeightHadTimeToSettle =
				maxScrollTop > 0 || performance.now() - startedAt >= 160;
			stableFrames = heightStable && zeroHeightHadTimeToSettle ? stableFrames + 1 : 0;
			previousMaxScrollTop = maxScrollTop;
			if (stableFrames >= 3) return;
		}
	}

	async function openBookChapter(
		index: number,
		position = 0,
		completePrevious = false,
		restoreBoundary?: ChapterBoundaryRestore,
	) {
		if ((await requestNavigationGuard("切换章节")) === "cancel") return;
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
				const shadowKey = `book:${activeBook.manifest.bookId}`;
				stageProgressShadow(shadowKey, { bookProgress: progress });
				await invoke("save_book_progress", { bookId: activeBook.manifest.bookId, progress });
				ackProgressShadow(shadowKey);
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
				restoreBoundary,
				suppressRecent: true,
				skipFlush: true,
				preloaded: cached ? { result: cached.result, rendered: cached.rendered } : undefined,
			});
			if (!opened) return;
			activeChapterIndex = index;
			tocItems = chapterTocItems(activeBook.manifest.chapters);
			preloadedChapter = null;
			updateWindowTitle(`${activeBook.manifest.title} · ${chapter.title}`);
			if (restoreBoundary && contentEl) {
				// activeChapterIndex changes the seam below the article. Apply
				// the carried scroll only after that final layout commit, not
				// while the loading/current chapter DOM is still settling.
				await tick();
				await restoreChapterBoundaryAfterLayout(restoreBoundary);
				if ($focusMode) {
					enterFocusMode();
				}
			}
		} finally {
			chapterSwitching = false;
		}
	}

	async function navigateBookChapter(direction: -1 | 1, offsetPx = 0) {
		if (!activeBook) return;
		if (chapterSwitching || chapterNavigationPending) {
			if (inFlightChapterDirection !== null) {
				if (direction === inFlightChapterDirection) {
					// Repeated events from one wheel/key gesture must not cascade
					// through several chapters. They also cancel an older queued
					// reversal because the latest intent follows the active seam.
					queuedChapterNavigation = null;
				} else if (queuedChapterNavigation?.direction === direction) {
					queuedChapterNavigation.offsetPx += Math.max(0, offsetPx);
				} else {
					// A genuine reversal can arrive while the adjacent chapter's
					// height is still settling. Keep the latest intent and apply
					// it as soon as the current seam has finished positioning.
					queuedChapterNavigation = {
						direction,
						offsetPx: Math.max(0, offsetPx),
					};
				}
			}
			return;
		}
		chapterNavigationPending = true;
		inFlightChapterDirection = direction;
		try {
			const targetIndex = activeChapterIndex + direction;
			const target = activeBook.manifest.chapters[targetIndex];
			if (!target) {
				if (direction > 0) await persistActiveBookProgress(true);
				return;
			}
			await openBookChapter(
				targetIndex,
				direction > 0 ? 0 : 1,
				direction > 0,
				{ direction, offsetPx },
			);
		} finally {
			chapterNavigationPending = false;
			inFlightChapterDirection = null;
			const queued = queuedChapterNavigation;
			queuedChapterNavigation = null;
			if (queued && activeBook) {
				void navigateBookChapter(queued.direction, queued.offsetPx);
			}
		}
	}

	function navigateBookChapterFromKey(key: string, direction: -1 | 1, offsetPx = 0) {
		if (key.startsWith("Touch") || key.startsWith("AudioVolume") || key.startsWith("Volume")) {
			void navigateBookChapter(direction, offsetPx);
			return;
		}
		if (!chapterNavigationKeyLatch.tryLatch(key)) return;
		void navigateBookChapter(direction, offsetPx);
	}

	async function returnToBookshelf() {
		if ((await requestNavigationGuard("返回书架")) === "cancel") return;
		await flushSaveState();
		// Invalidate any in-flight openFile/worker render before tearing the
		// surface down — otherwise a slow render can repopulate the reader
		// after the bookshelf is already shown.
		navigationGeneration += 1;
		currentLoadToken = "";
		dropStaleMarkdownRenders();
		resetReaderSurfaceForBookshelf();
		activeBook = null;
		trashOpen = false;
		activeChapterIndex = -1;
		cancelChapterBoundaryRestore();
		inFlightChapterDirection = null;
		queuedChapterNavigation = null;
		preloadedChapter = null;
		isTemporaryReading = false;
		$currentFilePath = "";
		$markdownSource = "";
		$renderedHtml = "";
		fileName = "";
		tocItems = [];
		readingProgress = 0;
		updateWindowTitle("");
		await refreshLibrary();
	}

	function resetReaderSurfaceForBookshelf() {
		clearFocusStyles();
		clearFocusScrollActive();
		chapterNavigationKeyLatch.reset();
		if (focusUpdateFrame !== null) {
			cancelAnimationFrame(focusUpdateFrame);
			focusUpdateFrame = null;
		}
		queuedFocusIndex = undefined;
		clearFocusBlockIndex();
		focusEntryStickyIndices = null;
		focusEntryStickyUntil = 0;
		pendingExitAnchor = null;
		if (focusWheelResetTimer) {
			clearTimeout(focusWheelResetTimer);
			focusWheelResetTimer = null;
		}
		focusWheelDelta = 0;
		$focusMode = false;
		$searchOpen = false;
		$searchQuery = "";
		clearSearchHighlights();
		$tocOpen = false;
		$settingsOpen = false;
		closeLightbox();
		hideFootnotePreview();
		if (contentEl) {
			const previousScrollBehavior = contentEl.style.scrollBehavior;
			contentEl.style.scrollBehavior = "auto";
			contentEl.scrollTop = 0;
			contentEl.style.scrollBehavior = previousScrollBehavior;
		}
	}

	function clearFocusScrollActive() {
		isFocusScrollActive = false;
		focusLockedIndex = null;
		clearProgrammaticFocusScrollLock();
		if (focusScrollEndTimer) {
			clearTimeout(focusScrollEndTimer);
			focusScrollEndTimer = null;
		}
		if (focusWheelMoveFrame !== null) {
			cancelAnimationFrame(focusWheelMoveFrame);
			focusWheelMoveFrame = null;
		}
		pendingFocusWheelSteps = 0;
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
	// Right after open-restore the anchor is remembered so images that finish
	// loading afterwards (and reflow content above it) can be re-anchored
	// instead of silently drifting. Cleared on the first user scroll intent
	// or after a short settle window.
	let openAnchor: ViewportAnchor | null = null;
	let openAnchorDeadline = 0;
	const OPEN_ANCHOR_SETTLE_MS = 4000;

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
		void checkForDesktopUpdate(false);
		const checkInitialFile = () => {
			if ((window as any).__INITIAL_FILE__) {
				openFile((window as any).__INITIAL_FILE__);
				delete (window as any).__INITIAL_FILE__;
			}
		};
		checkInitialFile();
		const initialFileTimer = setTimeout(checkInitialFile, 200);

		const handleKeydown = (e: KeyboardEvent) => {
			// Keyboard scrolling is user scroll intent too.
			if (openAnchor && isReadingActivityKey(e.key)) openAnchor = null;
			// While a modal layer is open it owns the keyboard: global shortcuts
			// (Space, arrows, Ctrl+…) must not act on the surface beneath. The
			// workflow dialogs are native <dialog> and close on Esc by themselves;
			// for our own layers Esc has explicit semantics — the navigation
			// guard resolves "cancel" (never silently discards the edit), the
			// book-detail dialog and trash surface close.
			if (
				navigationGuardOpen ||
				actionConfirm !== null ||
				podcastWorkflowOpen ||
				zhihuWorkflowOpen ||
				selectedBookDetail !== null ||
				trashOpen
			) {
				if (e.key === "Escape") {
					if (navigationGuardOpen) {
						e.preventDefault();
						void chooseNavigationGuard("cancel");
					} else if (actionConfirm !== null) {
						e.preventDefault();
						chooseActionConfirm(false);
					} else if (selectedBookDetail !== null) {
						e.preventDefault();
						selectedBookDetail = null;
					} else if (trashOpen) {
						e.preventDefault();
						trashOpen = false;
					}
				}
				return;
			}

			if (readingCursorState.active) {
				dispatchReadingCursor({ type: "keyboard" });
			}

			if (isModKey(e) && e.key === "o") {
				e.preventDefault();
				openFileDialog();
				return;
			}

			if (isModKey(e) && e.key === "f") {
				e.preventDefault();
				// 搜索只作用于精读界面：没有打开的文章时 Ctrl+F 不弹出
				// 一个永远 0 结果的空搜索层。
				if (!showMarkdownContext) return;
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
			} else if (isImmersiveSurface(chromeState.surface) && !chromeVisible) {
				// Keyboard equivalent of the top-edge hotzone: first Esc summons
				// the chrome (and moves focus into it) rather than being a dead key.
				e.preventDefault();
				void revealChromeForKeyboard();
			} else if (flowReaderSession) {
				e.preventDefault();
				void closeFlowReader();
			} else if ($currentFilePath) {
				// Last ladder step: leave the reading surface for the bookshelf.
				e.preventDefault();
				void returnToBookshelf();
			}
			return;
		}

			// F10 moves keyboard focus to the chrome (window bar / toolbar) —
			// the standard "menu bar" key, and the only way to reach the window
			// controls without a pointer once chrome auto-hides.
			if (e.key === "F10") {
				e.preventDefault();
				void revealChromeForKeyboard();
				return;
			}

			if (isModKey(e) && e.key === "t") {
				e.preventDefault();
				// 目录同样需要已渲染的文章；书架/连读界面下不做任何事。
				if (!showMarkdownContext) return;
				$tocOpen = !$tocOpen;
				return;
			}

			if (e.key === "F11" || (isModKey(e) && e.shiftKey && e.key === "F")) {
				e.preventDefault();
				void toggleFocusMode(!$focusMode);
				return;
			}

			if (
				($currentFilePath || flowReaderSession) &&
				!e.ctrlKey &&
				!e.metaKey &&
				!e.altKey &&
				!isTextInputTarget(e.target) &&
				isReadingActivityKey(e.key)
			) {
				noteReadingActivity();
			}

			const isVolUp = e.key === "AudioVolumeUp" || e.key === "VolumeUp" || (e as any).keyCode === 24;
			const isVolDown = e.key === "AudioVolumeDown" || e.key === "VolumeDown" || (e as any).keyCode === 25;

			if ($currentFilePath && $focusMode && !e.ctrlKey && !e.metaKey && !e.altKey && !isTextInputTarget(e.target)) {
				const focusDirection =
					e.key === "ArrowUp" || e.key === "ArrowLeft" || ($volumeKeyPaging && isVolUp)
						? -1
						: e.key === "ArrowDown" || e.key === "ArrowRight" || ($volumeKeyPaging && isVolDown)
							? 1
							: null;
				if (focusDirection !== null) {
					e.preventDefault();
					const eventKey = isVolUp ? "AudioVolumeUp" : isVolDown ? "AudioVolumeDown" : e.key;
					if (chapterNavigationKeyLatch.isLatched(eventKey)) return;
					const chapterDirection = moveFocus(focusDirection);
					if (chapterDirection !== null) {
						navigateBookChapterFromKey(eventKey, chapterDirection);
					}
					return;
				}
				// Home/End in focus mode: jump to the chapter edges. The scroll
				// event then re-anchors the spotlight on the nearest unit.
				if (e.key === "Home" || e.key === "End") {
					e.preventDefault();
					if (chapterNavigationKeyLatch.isLatched(e.key)) return;
					handleReadingScrollIntent(
						{ direction: e.key === "Home" ? -1 : 1, kind: "edge" },
						e.key,
					);
					return;
				}
			}

			if (
				$currentFilePath &&
				!$focusMode &&
				!editingParagraph &&
				!$searchOpen &&
				!$tocOpen &&
				!$settingsOpen &&
				!lightboxSrc &&
				!e.ctrlKey &&
				!e.metaKey &&
				!e.altKey &&
				!isTextInputTarget(e.target)
			) {
				const eventKey = isVolUp ? "AudioVolumeUp" : isVolDown ? "AudioVolumeDown" : e.key;
				if ((isVolUp || isVolDown) && !$volumeKeyPaging) {
					// volume paging disabled in settings
				} else {
					const intent = readingScrollIntentForKey(eventKey, e.shiftKey);
					if (intent) {
						e.preventDefault();
						if (chapterNavigationKeyLatch.isLatched(eventKey)) return;
						handleReadingScrollIntent(intent, eventKey);
						return;
					}
				}
			}

			// 键盘编辑入口：E / F2 编辑当前阅读位置的段落（专注模式下取
			// 聚光灯所在块），Esc 取消，双击仍然是鼠标路径。
			if (
				$currentFilePath &&
				!flowReaderSession &&
				!editingParagraph &&
				!$searchOpen &&
				!$tocOpen &&
				!$settingsOpen &&
				!lightboxSrc &&
				!e.ctrlKey &&
				!e.metaKey &&
				!e.altKey &&
				!isTextInputTarget(e.target) &&
				(e.key === "e" || e.key === "E" || e.key === "F2")
			) {
				e.preventDefault();
				startEditAtReadingPosition();
				return;
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
			chapterNavigationKeyLatch.release(e.key);
		};

		const handleWindowBlur = () => {
			chapterNavigationKeyLatch.reset();
		};

		// Scroll events fire far more often than once per frame. Everything in
		// the handler reads *current* scroll state, so coalescing to one run per
		// animation frame is identical to running it per event.
		let scrollFrame: number | null = null;
		const handleScroll = () => {
			if (!contentEl) return;
			if (scrollFrame !== null) return;
			scrollFrame = requestAnimationFrame(() => {
				scrollFrame = null;
				if (!contentEl) return;
				if (codeCopyVisible) scheduleCodeCopyHide(0);
				const scrollTop = contentEl.scrollTop;
				const scrollHeight = contentEl.scrollHeight - contentEl.clientHeight;
				readingProgress = scrollHeight > 0 ? scrollTop / scrollHeight : 0;
				if (activeBook && readingProgress >= 0.84) void preloadNextBookChapter();
				// Keep the TOC "current section" highlight tracking while it is
				// open — scrollTop is not a reactive dependency of the $: block.
				if ($tocOpen && !activeBook) {
					tocActiveId = getCurrentHeading()?.id ?? "";
				}
				if ($currentFilePath) noteReadingActivity();

				if ($currentFilePath && $focusMode) {
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
			});
		};

		const handleWheel = (e: WheelEvent) => {
			// User scrolled — stop re-anchoring the just-opened document.
			openAnchor = null;
			if (!$currentFilePath && !flowReaderSession) return;

			// Ctrl+wheel (and touchpad pinch on Windows) zooms the reader font.
			// Pinch gestures emit a burst of tiny deltaY events — accumulate
			// them so one gesture produces one scale step, not ten.
			if (e.ctrlKey) {
				e.preventDefault();
				ctrlWheelDelta += e.deltaY;
				if (Math.abs(ctrlWheelDelta) >= 50) {
					adjustFontScale(ctrlWheelDelta < 0 ? 1 : -1);
					ctrlWheelDelta = 0;
				}
				return;
			}
			ctrlWheelDelta = 0;
			if ($currentFilePath || flowReaderSession) {
				scheduleReadingActivity();
			}
			if (contentEl && activeBook && !$focusMode && Math.abs(e.deltaY) > 0.01) {
				cancelChapterBoundaryRestore();
				const wheelUnit =
					e.deltaMode === WheelEvent.DOM_DELTA_LINE
						? 40
						: e.deltaMode === WheelEvent.DOM_DELTA_PAGE
							? contentEl.clientHeight
							: 1;
				const deltaPx = e.deltaY * wheelUnit;
				const maxScrollTop = Math.max(0, contentEl.scrollHeight - contentEl.clientHeight);
				const requestedTop = contentEl.scrollTop + deltaPx;
				if (deltaPx > 0 && requestedTop > maxScrollTop) {
					e.preventDefault();
					void navigateBookChapter(1, requestedTop - maxScrollTop);
					return;
				}
				if (deltaPx < 0 && requestedTop < 0) {
					e.preventDefault();
					void navigateBookChapter(-1, Math.abs(requestedTop));
					return;
				}
			}
			if (!contentEl || !$currentFilePath || !$focusMode) return;
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
			scheduleFocusWheelStep(stepDirection);
		};

		// Track the revealed originals instead of rescanning the whole article on
		// every selectionchange tick — the event also fires for plain caret moves.
		let selectionRevealedOriginals: Element[] = [];
		const clearPodcastSelectionReveal = () => {
			if (selectionRevealedOriginals.length === 0) return;
			for (const node of selectionRevealedOriginals) {
				node.classList.remove("is-selection-revealed");
			}
			selectionRevealedOriginals = [];
		};

		const handlePodcastSelectionChange = () => {
			const selection = window.getSelection();
			if (!selection || selection.rangeCount === 0 || selection.isCollapsed) {
				clearPodcastSelectionReveal();
				return;
			}
			const getTranslation = (node: Node | null) => {
				const element = node instanceof Element ? node : node?.parentElement;
				return element?.closest("p.podcast-translation") as HTMLElement | null;
			};
			const translation = getTranslation(selection.anchorNode) ?? getTranslation(selection.focusNode);
			if (!translation || !contentEl?.contains(translation)) {
				clearPodcastSelectionReveal();
				return;
			}
			const bilingualId = translation.dataset.bilingualId;
			if (!bilingualId) {
				clearPodcastSelectionReveal();
				return;
			}
			clearPodcastSelectionReveal();
			const originals = contentEl.querySelectorAll(
				`blockquote.podcast-original[data-bilingual-id="${CSS.escape(bilingualId)}"]`,
			);
			originals.forEach((node) => {
				node.classList.add("is-selection-revealed");
			});
			selectionRevealedOriginals = Array.from(originals);
		};

		const togglePodcastOriginal = (original: HTMLElement) => {
			const bilingualId = original.dataset.bilingualId;
			const originals = bilingualId
				? Array.from(contentEl.querySelectorAll("blockquote.podcast-original")).filter(
					(node) => (node as HTMLElement).dataset.bilingualId === bilingualId,
				  )
				: [original];
			const reveal = !original.classList.contains("is-revealed");
			originals.forEach((node) => {
				node.classList.toggle("is-revealed", reveal);
				// blockquote 是 role="button" 的展开控件（见 markdown.ts
				// markPodcastOriginal），同步 aria-expanded 供读屏器报状态。
				// 选中译文触发的 is-selection-revealed 是瞬时视觉提示，
				// 不动 aria-expanded——展开状态只反映用户显式 pin 的操作。
				node.setAttribute("aria-expanded", reveal ? "true" : "false");
			});
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

			const original = target.closest("blockquote.podcast-original") as HTMLElement | null;
			if (original) {
				togglePodcastOriginal(original);
				return;
			}

			if (!$focusMode || editingParagraph) return;
			focusBlockFromInteraction(target);
		};

		// 双语原文块是 tabindex=0 的可聚焦元素：键盘用户用 Enter/Space
		// 展开/收起，与鼠标点击等价。stopPropagation 防止 Space 被全局
		// 阅读快捷键解释成向下翻页。
		const handleContentKeydown = (e: KeyboardEvent) => {
			if (e.key !== "Enter" && e.key !== " ") return;
			const target = e.target as HTMLElement | null;
			if (!target || !contentEl.contains(target)) return;
			// tabindex 在 blockquote 本身上，只有它聚焦时才切换。
			const original = target.closest?.("blockquote.podcast-original") as HTMLElement | null;
			if (!original || original !== target) return;
			e.preventDefault();
			e.stopPropagation();
			togglePodcastOriginal(original);
		};

		const handleDblClick = (e: MouseEvent) => {
			cancelPendingArticleLinkOpen();
			// 移动端双击手势专属用于切换专注模式，不误触发段落就地源码编辑
			if (isMobile) return;
			// 双击选词是正常的文本选择手势——非折叠选区说明用户在选择而
			// 不是想进编辑；此时不动选区、不吞事件，双击只取消链接导航。
			if (window.getSelection()?.isCollapsed === false) return;
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
				// A modal layer owns the surface while it is open — a dropped file
				// must not swap the document out from under it.
				if (
					podcastWorkflowOpen ||
					zhihuWorkflowOpen ||
					navigationGuardOpen ||
					actionConfirm !== null ||
					selectedBookDetail !== null ||
					trashOpen
				) return;
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
		const unlistenOpenFile = listenManaged<string>('open-file', (event) => {
			// openFile will handle canceling any active edit
			openFile(event.payload);
		});
		const unlistenTaskEvents = listenManaged<TaskEvent>("acquisition://task-event", (event) => {
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

		const handleReadingCursorPointerMove = () => {
			if (!readingCursorState.active || readingCursorMoveFrame !== null) return;
			readingCursorMoveFrame = window.requestAnimationFrame(() => {
				readingCursorMoveFrame = null;
				dispatchReadingCursor({ type: "pointer-move" });
			});
		};

		// Same coalescing as pointer-move: the cursor reducer is a store write,
		// so a wheel burst would otherwise invalidate it once per tick.
		let readingCursorWheelFrame: number | null = null;
		const handleReadingCursorWheel = (e: WheelEvent) => {
			if (!readingCursorState.active) return;
			if (Math.abs(e.deltaX) + Math.abs(e.deltaY) + Math.abs(e.deltaZ) < 0.01) return;
			if (readingCursorWheelFrame !== null) return;
			readingCursorWheelFrame = window.requestAnimationFrame(() => {
				readingCursorWheelFrame = null;
				dispatchReadingCursor({ type: "wheel" });
			});
		};

		const handleContentMouseOver = (e: MouseEvent) => {
			const link = getFootnoteLink(e.target);
			if (link) {
				cancelFootnoteHide();
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
				scheduleFootnoteHide();
			}
			if (e.target instanceof HTMLElement && e.target.closest(".article pre")) {
				scheduleCodeCopyHide();
			}
		};

		// 脚注链接可被 Tab 聚焦——预览对键盘用户同样可用（focusin/focusout
		// 与 mouseover/mouseout 走同一套 200ms 隐藏延时）。
		const handleContentFocusIn = (e: FocusEvent) => {
			const link = getFootnoteLink(e.target);
			if (!link) return;
			cancelFootnoteHide();
			showFootnotePreview(link);
		};

		const handleContentFocusOut = (e: FocusEvent) => {
			if (getFootnoteLink(e.target)) scheduleFootnoteHide();
		};

		window.addEventListener("keydown", handleKeydown);
		window.addEventListener("keyup", handleKeyup);
		window.addEventListener("blur", handleWindowBlur);
		window.addEventListener("pointermove", handleReadingCursorPointerMove, { passive: true });
		window.addEventListener("wheel", handleReadingCursorWheel, { passive: true });
		contentEl?.addEventListener("scroll", handleScroll);
		contentEl?.addEventListener("wheel", handleWheel, { passive: false });
		contentEl?.addEventListener("click", handleContentClick);
		contentEl?.addEventListener("keydown", handleContentKeydown);
		document.addEventListener("selectionchange", handlePodcastSelectionChange);
		contentEl?.addEventListener("dblclick", handleDblClick);
		contentEl?.addEventListener("mouseover", handleContentMouseOver);
		contentEl?.addEventListener("mouseout", handleContentMouseOut);
		contentEl?.addEventListener("focusin", handleContentFocusIn);
		contentEl?.addEventListener("focusout", handleContentFocusOut);
		// Images load lazily and reflow the article when they land — capture-phase
		// load events keep the cached heading offsets honest (focus metrics have
		// their own invalidation paths).
		let contentLoadFrame: number | null = null;
		const handleContentLoad = () => {
			invalidateHeadingIndex();
			// An image that just landed reflows the article: focus metrics go
			// stale and content above the viewport pushes the reading position
			// down. Coalesce to one correction per frame.
			if (contentLoadFrame !== null) return;
			contentLoadFrame = requestAnimationFrame(() => {
				contentLoadFrame = null;
				if (openAnchor && Date.now() > openAnchorDeadline) openAnchor = null;
				if (openAnchor) restoreViewportAnchor(openAnchor);
				invalidateFocusMetrics();
				if ($currentFilePath && $focusMode) {
					rebuildFocusMetrics();
					const units = getFocusUnits();
					if (lastFocusedIdx >= 0 && units[lastFocusedIdx]) {
						focusLockedIndex = lastFocusedIdx;
						markFocusScrollActive();
						scrollUnitToFocusPosition(units[lastFocusedIdx], lastFocusedIdx, "auto");
					}
					scheduleFocusUpdate(lastFocusedIdx >= 0 ? lastFocusedIdx : undefined);
				}
			});
		};
		contentEl?.addEventListener("load", handleContentLoad, true);
		const clearOpenAnchor = () => {
			openAnchor = null;
		};
		contentEl?.addEventListener("pointerdown", clearOpenAnchor);
		contentEl?.addEventListener("touchstart", clearOpenAnchor, { passive: true });
		contentEl?.addEventListener("touchstart", handleReaderTouchStart, { passive: true });
		contentEl?.addEventListener("touchend", handleReaderTouchEnd, { passive: true });

		const handlePopState = () => {
			mobileHistoryPushed = false;
			if (lightboxSrc) {
				closeLightbox();
			} else if (editingParagraph) {
				cancelEdit();
			} else if ($searchOpen) {
				$searchOpen = false;
				$searchQuery = "";
				clearSearchHighlights();
			} else if ($tocOpen) {
				$tocOpen = false;
			} else if ($settingsOpen) {
				$settingsOpen = false;
			} else if (trashOpen) {
				trashOpen = false;
			} else if (podcastWorkflowOpen) {
				podcastWorkflowOpen = false;
			} else if (zhihuWorkflowOpen) {
				zhihuWorkflowOpen = false;
			} else if (selectedBookDetail !== null) {
				selectedBookDetail = null;
			} else if (actionConfirm !== null) {
				chooseActionConfirm(false);
			} else if (navigationGuardOpen) {
				void chooseNavigationGuard("cancel");
			} else if ($focusMode) {
				void toggleFocusMode(false);
			} else if (flowReaderSession) {
				void closeFlowReader();
			} else if ($currentFilePath) {
				void returnToBookshelf();
			}
			setTimeout(syncMobileHistory, 60);
		};
		window.addEventListener("popstate", handlePopState);
		// rebuildFocusMetrics is O(units) synchronous layout; a resize drag
		// fires continuously, so coalesce to one run per animation frame.
		let resizeFrame: number | null = null;
		const handleResize = () => {
			if (resizeFrame !== null) return;
			resizeFrame = requestAnimationFrame(() => {
				resizeFrame = null;
				invalidateFocusMetrics();
				if ($currentFilePath && $focusMode) {
					rebuildFocusMetrics();
					scheduleFocusUpdate(lastFocusedIdx >= 0 ? lastFocusedIdx : undefined);
				}
				updateEditOrbitPosition();
			});
		};
		window.addEventListener("resize", handleResize);
		const handleBeforeUnload = () => {
			if ($currentFilePath && contentEl) void flushSaveState();
		};
		window.addEventListener("beforeunload", handleBeforeUnload);

		// Window X / chrome close: save reading state and hide to tray so acquisition
		// tasks keep running. Only tray "退出" ends the process (preserve / cleanup).
		let isClosing = false;
		// One-shot race timeouts below are tracked so teardown can cancel a
		// pending exit flow instead of letting callbacks fire post-destroy.
		const exitTimers = new Set<ReturnType<typeof setTimeout>>();
		const raceWithTimeout = <T>(promise: Promise<T>, ms: number, fallback: T): Promise<T> => {
			let timer: ReturnType<typeof setTimeout> | null = null;
			const timeout = new Promise<T>((resolve) => {
				timer = setTimeout(() => {
					if (timer) exitTimers.delete(timer);
					resolve(fallback);
				}, ms);
			});
			if (timer) exitTimers.add(timer);
			return Promise.race([promise, timeout]).finally(() => {
				if (timer) {
					clearTimeout(timer);
					exitTimers.delete(timer);
				}
			});
		};
		const finishExit = async (mode: "hide" | "preserve" | "cancel_and_discard") => {
			try {
				try {
					await appWindow.hide();
				} catch (hideErr) {
					// Hiding is best-effort: a missing allow-hide capability or an
					// already-gone window must not skip the graceful flush below —
					// quit_app still has to run or only the hard tray fallback exits.
					console.warn("Window hide failed; continuing exit cleanup.", hideErr);
				}
				if (editingParagraph) {
					await raceWithTimeout(finishEdit(), 1200, false);
				}
				await raceWithTimeout(flushSaveState(), 500, undefined);
				// Exit path skips the nav-guard (already confirmed "退出应用")
				// and bounds the close so a wedged session can't stall shutdown.
				await raceWithTimeout(closeFlowReaderSession(), 1500, undefined);
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
		const requestExit = async (mode: "hide" | "preserve" | "cancel_and_discard") => {
			if (isClosing) return;
			if (mode !== "hide") {
				if ((await requestNavigationGuard("退出应用")) === "cancel") {
					// The tray armed a hard-exit fallback before we could ask —
					// disarm it or the app dies anyway seconds later.
					await invoke("cancel_exit_fallback").catch(() => {});
					return;
				}
			} else if (editingParagraph || $currentFilePath) {
				// Soft hide: still flush edits, but do not treat as app exit.
			}
			isClosing = true;
			void finishExit(mode);
		};
		const unlistenExit = listenManaged<{ mode?: string }>("request-app-exit", (event) => {
			const mode = event.payload?.mode === "cancel_and_discard" ? "cancel_and_discard" : "preserve";
			void requestExit(mode);
		});
		const unlistenClose = appWindow.onCloseRequested((event) => {
			if (isClosing) return;
			event.preventDefault();
			void requestExit("hide");
		});

		void refreshLibrary();
		// F6: one-shot notice when control.db was quarantined + rebuilt this
		// launch — otherwise an empty task list looks like silent data loss.
		void invoke<boolean>("take_control_db_recovery_notice")
			.then((recovered) => {
				if (recovered) {
					showAppNotice("检测到任务数据库损坏：已隔离损坏文件并重建空库，历史任务记录不可恢复；书目与阅读进度不受影响");
				}
			})
			.catch(() => {});

		const handleFlowReaderMessage = (event: MessageEvent) => {
			if (!flowReaderSession || !flowIframeEl) return;
			if (event.source !== flowIframeEl.contentWindow) return;
			// Local reader sessions are served from 127.0.0.1/localhost only —
			// opaque "null" origins are rejected by isAllowedFlowMessageOrigin.
			if (!isAllowedFlowMessageOrigin(event.origin)) return;
			// Reader scripts finished binding their listener — re-send both
			// sync messages in case an earlier post arrived before that.
			if (isFlowReaderReadyMessage(event.data)) {
				lastPostedFontScale = null;
				lastPostedLayoutWide = null;
				lastPostedThemeName = null;
				postFlowFontScale($fontScale);
				postFlowLayoutMode(windowMaximized);
				postFlowTheme();
				return;
			}
			if (isFlowReadingActivityMessage(event.data)) {
				noteReadingActivity();
				return;
			}
			if (isFlowFontScaleChangeMessage(event.data)) {
				setFontScale(event.data.scale, { fromFlow: true });
				return;
			}
			// Keys the iframe didn't consume (Esc with nothing open inside,
			// F10) are forwarded here — same behavior as a real window keydown.
			if (isFlowKeyDownMessage(event.data)) {
				if (event.data.key === 'F10') {
					void revealChromeForKeyboard();
				} else if (event.data.key === 'Escape') {
					void closeFlowReader();
				}
			}
		};
		window.addEventListener("message", handleFlowReaderMessage);

		void loadRecentFiles().then((files) => {
			recentFiles = files;
		});
		void hydrateDurableReaderPreferences();
		maybeShowTrayHint();

		// Prewarm the markdown worker off the critical path: the first render
		// then only pays postMessage latency, not worker script load/eval.
		let workerPrewarmIdleId: number | null = null;
		let workerPrewarmTimer: ReturnType<typeof setTimeout> | null = null;
		if (typeof requestIdleCallback === "function") {
			workerPrewarmIdleId = requestIdleCallback(() => {
				workerPrewarmIdleId = null;
				getMarkdownWorker();
			}, { timeout: 3000 });
		} else {
			workerPrewarmTimer = setTimeout(() => {
				workerPrewarmTimer = null;
				getMarkdownWorker();
			}, 0);
		}

		// 外部变更自动重载：每 2s 轮询当前文件 mtime；
		// 内容与内存一致（如刚由本应用保存）或正在编辑时不打断。
		// 窗口失焦时降频到 1/5(约 10s)。
		let watchedPath = "";
		let watchedMtime = 0;
		let pollTick = 0;
		let fileWatchInFlight = false;
		let fileMissingNotified = false;
		const fileWatchTimer = window.setInterval(async () => {
			pollTick += 1;
			if (!document.hasFocus() && pollTick % 5 !== 0) return;
			if (fileWatchInFlight) return;
			// 连读会话期间不要触发 openFile —— 它会把精读面盖到 iframe 之上。
			if (flowReaderSession) return;
			fileWatchInFlight = true;
			const path = $currentFilePath ?? "";
			const navigationAtStart: NavigationSnapshot = {
				generation: navigationGeneration,
				path,
			};
			try {
				if (!path) {
					watchedPath = "";
					return;
				}
				const mtime = await invoke<number>("get_file_mtime", { path });
				// File readable again — allow a future disappearance notice.
				fileMissingNotified = false;
				if (!isCurrentNavigation(navigationAtStart, {
					generation: navigationGeneration,
					path: $currentFilePath ?? "",
				})) return;
				if (watchedPath !== path) {
					watchedPath = path;
					watchedMtime = mtime;
					return;
				}
				if (mtime <= watchedMtime) return;
				if (editingParagraph) return;
				const result = await invoke<{ content: string; encoding: string }>(
					"read_markdown_file",
					{ path },
				);
				if (!isCurrentNavigation(navigationAtStart, {
					generation: navigationGeneration,
					path: $currentFilePath ?? "",
				})) return;
				if (result.content === $markdownSource) return;
				const scrollTop = contentEl?.scrollTop ?? 0;
				const bookId = activeBook?.manifest.bookId;
				const opened = await openFile(path, activeBook ? {
					bookChapter: true,
					restoreRatio: readingProgress,
					suppressRecent: true,
					skipFlush: true,
					expectedNavigationGeneration: navigationAtStart.generation,
				} : { expectedNavigationGeneration: navigationAtStart.generation });
				if (!opened) return;
				watchedMtime = mtime;
				if (bookId && activeBook?.manifest.bookId === bookId) {
					tocItems = chapterTocItems(activeBook.manifest.chapters);
				}
				if (contentEl) contentEl.scrollTop = scrollTop;
			} catch {
				// A moved/deleted file must surface — otherwise the reader keeps
				// showing a stale copy forever. Notify once per disappearance.
				if (
					path &&
					!fileMissingNotified &&
					isCurrentNavigation(navigationAtStart, {
						generation: navigationGeneration,
						path: $currentFilePath ?? "",
					})
				) {
					fileMissingNotified = true;
					showAppNotice("文件已被移动或删除，当前显示的是旧内容");
				}
			} finally {
				fileWatchInFlight = false;
			}
		}, 2000);

		return () => {
			window.clearInterval(fileWatchTimer);
			if (taskPollTimer !== null) {
				clearInterval(taskPollTimer);
				taskPollTimer = null;
			}
			clearTimeout(initialFileTimer);
			if (workerPrewarmIdleId !== null) cancelIdleCallback(workerPrewarmIdleId);
			if (workerPrewarmTimer) clearTimeout(workerPrewarmTimer);
			exitTimers.forEach((timer) => clearTimeout(timer));
			exitTimers.clear();
			maximizedScrollFrames.forEach((frame) => cancelAnimationFrame(frame));
			maximizedScrollFrames.clear();
			focusReleaseTimers.forEach((timer) => clearTimeout(timer));
			focusReleaseTimers.clear();
			if (focusInteractionFrame !== null) {
				cancelAnimationFrame(focusInteractionFrame);
				focusInteractionFrame = null;
			}
			if (editOrbitFrame !== null) {
				cancelAnimationFrame(editOrbitFrame);
				editOrbitFrame = null;
			}
			cancelChapterBoundaryRestore();
			if (libraryRefreshTimer) {
				clearTimeout(libraryRefreshTimer);
				libraryRefreshTimer = null;
			}
			unlistenDrop.then(fn => fn());
			unlistenOpenFile();
			unlistenTaskEvents();
			unlistenExit();
			unlistenClose.then(fn => fn());
			window.removeEventListener("focus", refreshTasksOnResume);
			document.removeEventListener("visibilitychange", refreshTasksOnResume);
			window.removeEventListener("keydown", handleKeydown);
			window.removeEventListener("keyup", handleKeyup);
			window.removeEventListener("blur", handleWindowBlur);
			window.removeEventListener("pointermove", handleReadingCursorPointerMove);
			window.removeEventListener("wheel", handleReadingCursorWheel);
			window.removeEventListener("message", handleFlowReaderMessage);
			contentEl?.removeEventListener("scroll", handleScroll);
			contentEl?.removeEventListener("wheel", handleWheel);
			contentEl?.removeEventListener("click", handleContentClick);
			contentEl?.removeEventListener("keydown", handleContentKeydown);
			document.removeEventListener("selectionchange", handlePodcastSelectionChange);
			contentEl?.removeEventListener("dblclick", handleDblClick);
			contentEl?.removeEventListener("mouseover", handleContentMouseOver);
			contentEl?.removeEventListener("mouseout", handleContentMouseOut);
			contentEl?.removeEventListener("focusin", handleContentFocusIn);
			contentEl?.removeEventListener("focusout", handleContentFocusOut);
			contentEl?.removeEventListener("load", handleContentLoad, true);
			contentEl?.removeEventListener("pointerdown", clearOpenAnchor);
			contentEl?.removeEventListener("touchstart", clearOpenAnchor);
			contentEl?.removeEventListener("touchstart", handleReaderTouchStart);
			contentEl?.removeEventListener("touchend", handleReaderTouchEnd);
			window.removeEventListener("popstate", handlePopState);
			if (contentLoadFrame !== null) {
				cancelAnimationFrame(contentLoadFrame);
				contentLoadFrame = null;
			}
			if (footnoteHideTimer) {
				clearTimeout(footnoteHideTimer);
			}
			if (codeCopyHideTimer) {
				clearTimeout(codeCopyHideTimer);
			}
			window.removeEventListener("resize", handleResize);
			window.removeEventListener("beforeunload", handleBeforeUnload);
			if (scrollFrame !== null) {
				cancelAnimationFrame(scrollFrame);
				scrollFrame = null;
			}
			if (resizeFrame !== null) {
				cancelAnimationFrame(resizeFrame);
				resizeFrame = null;
			}
			if (readingCursorWheelFrame !== null) {
				cancelAnimationFrame(readingCursorWheelFrame);
				readingCursorWheelFrame = null;
			}
			if (focusWheelMoveFrame !== null) {
				cancelAnimationFrame(focusWheelMoveFrame);
				focusWheelMoveFrame = null;
				pendingFocusWheelSteps = 0;
			}
			if (readingActivityFrame !== null) {
				cancelAnimationFrame(readingActivityFrame);
				readingActivityFrame = null;
			}
			if (focusWheelResetTimer) {
				clearTimeout(focusWheelResetTimer);
			}
			if (searchDebounceTimer) {
				clearTimeout(searchDebounceTimer);
			}
			if (saveStateTimer) {
				clearTimeout(saveStateTimer);
			}
			clearChromeHideTimer();
			if (readingCursorMoveFrame !== null) {
				cancelAnimationFrame(readingCursorMoveFrame);
				readingCursorMoveFrame = null;
			}
			clearReadingCursorHideTimer();
			if (zoomIndicatorTimer) {
				clearTimeout(zoomIndicatorTimer);
			}
			if (typographyReflowFrame !== null) {
				cancelAnimationFrame(typographyReflowFrame);
				typographyReflowFrame = null;
			}
			if (readerPreferencesSaveTimer) {
				clearTimeout(readerPreferencesSaveTimer);
				readerPreferencesSaveTimer = null;
				void saveDurableReaderPreferences();
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

	let fileDialogOpen = false;
	async function openFileDialog() {
		// Re-entry (repeat Ctrl+O / double activation) must not stack dialogs.
		if (fileDialogOpen) return;
		fileDialogOpen = true;
		try {
			const selected = await open({
				multiple: false,
				filters: [{ name: "Markdown", extensions: ["md", "markdown"] }],
			});
			if (selected) {
				openFile(selected as string);
			}
		} finally {
			fileDialogOpen = false;
		}
	}

	type OpenFileOptions = {
		suppressFailureNotice?: boolean;
		bookChapter?: boolean;
		expectedNavigationGeneration?: number;
		restoreRatio?: number;
		restoreBoundary?: ChapterBoundaryRestore;
		suppressRecent?: boolean;
		skipFlush?: boolean;
		preloaded?: {
			result: { content: string; encoding: string };
			rendered: RenderedMarkdownDocument;
		};
	};

	async function openFile(path: string, options: OpenFileOptions = {}): Promise<boolean> {
		if (
			options.expectedNavigationGeneration !== undefined &&
			options.expectedNavigationGeneration !== navigationGeneration
		) return false;
		// Bump the generation only after the guard resolves — bumping before the
		// await meant "cancel" still invalidated any in-flight load, killing it
		// silently mid-flight (its token check fails, no UI feedback).
		if ((await requestNavigationGuard("打开另一篇 Markdown")) === "cancel") return false;
		const loadGeneration = ++navigationGeneration;

		// Opening a Markdown file always replaces the 连读 surface: an OS
		// open-file event or drag-and-drop can arrive while a flow session is
		// still active — leaving it running would both hide the new document
		// behind the iframe and leak the backend reader session.
		if (flowReaderSession) {
			const session = flowReaderSession;
			flowReaderSession = null;
			void invoke("close_reader_session", { sessionId: session.sessionId }).catch((err) => {
				console.warn("Failed to close flow reader session:", err);
			});
		}

		if ($currentFilePath && !options.skipFlush) {
			await flushSaveState();
		}
		if (navigationGeneration !== loadGeneration) return false;
		if (!options.bookChapter) {
			activeBook = null;
			activeChapterIndex = -1;
			preloadedChapter = null;
			isTemporaryReading = true;
		} else {
			isTemporaryReading = false;
		}

		// Race condition protection: invalidate previous loads
		const loadToken = `${loadGeneration}:${path}`;
		currentLoadToken = loadToken;
		// F4: drop any stale in-flight worker render (superseded navigation,
		// obsolete chapter preload) so this document does not queue behind it.
		dropStaleMarkdownRenders();

		// Keep the current chapter visible while an adjacent chapter is prepared.
		// The DOM swaps only when the next Markdown is ready, avoiding a spinner
		// flash in the middle of what should feel like continuous scrolling.
		const showLoadingState = !options.bookChapter || !$currentFilePath;
		if (showLoadingState) {
			$isLoading = true;
		}
		fileError = "";
		const nextFileName = path.split(/[\\/]/).pop() || "";
		let loadSucceeded = false;
		let scrollRestore:
			| { kind: "ratio"; value: number }
			| { kind: "absolute"; value: number }
			| null = options.restoreBoundary
				? null
				: options.restoreRatio !== undefined
					? { kind: "ratio", value: options.restoreRatio }
					: null;

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
			$markdownSource = result.content;
			fileEncoding = result.encoding;
			$renderedHtml = resolveRenderedHtmlAssets(rendered.html, path);
			tocItems = rendered.toc;
			frontMatterEntries = rendered.frontMatter;
			void tick().then(() => renderMermaidBlocks());

			// Reset focus state when file changes
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

			if (options.restoreRatio === undefined && !options.restoreBoundary) {
				const shadow = readProgressShadow(`file:${path}`);
				try {
					const state: { scroll_position: number; bookmarks: number[]; progress?: number } =
						await invoke("load_reading_state", { path });
					if (currentLoadToken !== loadToken) return false;
					// F12: an unacked shadow is newer than the last state the
					// backend confirmed — prefer it over the on-disk record.
					const effective = shadow && !shadow.acked
						? {
							scroll_position: shadow.scrollPosition ?? state.scroll_position,
							progress: shadow.progressRatio ?? state.progress,
						}
						: state;
					scrollRestore = typeof effective.progress === "number" && effective.progress > 0
						? { kind: "ratio", value: effective.progress }
						: { kind: "absolute", value: effective.scroll_position };
				} catch {
					// No saved state — the crash shadow is better than top-of-file.
					if (shadow && (shadow.progressRatio || shadow.scrollPosition)) {
						scrollRestore = shadow.progressRatio && shadow.progressRatio > 0
							? { kind: "ratio", value: shadow.progressRatio }
							: { kind: "absolute", value: shadow.scrollPosition ?? 0 };
					}
				}
			}

			loadSucceeded = true;
			if (!options.suppressRecent) recordRecentFile(path, nextFileName);
			maybeShowFirstOpenHint();

		} catch (err) {
			if (currentLoadToken !== loadToken) return false;
			console.error("Failed to open file:", err);
			fileError = `无法打开文件：${describeError(err)}`;
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
					if (contentEl && scrollRestore) {
						const maxScrollTop = Math.max(0, contentEl.scrollHeight - contentEl.clientHeight);
						const targetScrollTop =
							scrollRestore.kind === "ratio"
								? Math.max(0, Math.min(1, scrollRestore.value)) * maxScrollTop
								: Math.max(0, Math.min(maxScrollTop, scrollRestore.value));
						const previousScrollBehavior = contentEl.style.scrollBehavior;
						contentEl.style.scrollBehavior = "auto";
						contentEl.scrollTop = targetScrollTop;
						contentEl.style.scrollBehavior = previousScrollBehavior;
						readingProgress = maxScrollTop > 0 ? targetScrollTop / maxScrollTop : 0;
					}
					// Images that finish loading after this point reflow content
					// above the restored position — keep the anchor block so the
					// load handler can re-anchor until the user scrolls.
					openAnchor = captureViewportAnchor();
					openAnchorDeadline = Date.now() + OPEN_ANCHOR_SETTLE_MS;
					if ($searchQuery.trim()) {
						performSearch();
					} else if (!$focusMode) {
						clearFocusBlockIndex();
					}
					if ($focusMode && !options.restoreBoundary) {
						await tick();
						if (currentLoadToken === loadToken) {
							enterFocusMode();
						}
					} else if ($autoFocusMode) {
						$focusMode = true;
						await tick();
						if (currentLoadToken === loadToken && !options.restoreBoundary) {
							enterFocusMode();
						}
					}
				}
			}
		}
		return loadSucceeded && currentLoadToken === loadToken;
	}

	// Keyboard entry for paragraph editing (E / F2 / toolbar button): picks the
	// block under the reading position — the focused unit in focus mode, else
	// the unit nearest the viewport anchor line.
	function startEditAtReadingPosition() {
		if (editingParagraph || !$currentFilePath) return;
		const units = getFocusUnits();
		if (units.length === 0) return;
		const idx = lastFocusedIdx >= 0 ? lastFocusedIdx : getClosestFocusIndex(units);
		const unit = units[idx];
		if (!unit) return;
		// Table rows and grouped members carry no source range themselves —
		// fall back to the enclosing block (same rule as the dblclick path).
		const target =
			unit.find((member) => member.dataset.sourceStart !== undefined) ??
			(unit[0]?.closest("[data-source-start]") as HTMLElement | null);
		if (target instanceof HTMLElement) startEdit(target);
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
		el.addEventListener("input", scheduleEditOrbitUpdate);

		if ($focusMode) {
			focusBlockFromInteraction(el);
		}

		const isDark = !$currentTheme.name.includes('light');
		if ($focusMode && isDark) {
			updateEditOrbitPosition();
		}
	}

	// Keystroke input events fire far more often than once per frame — batch the
	// rect read + style write so each frame pays at most one layout pass.
	let editOrbitFrame: number | null = null;
	function scheduleEditOrbitUpdate() {
		if (editOrbitFrame !== null) return;
		editOrbitFrame = requestAnimationFrame(() => {
			editOrbitFrame = null;
			updateEditOrbitPosition();
		});
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
			// While the navigation guard is open Esc belongs to it — reverting
			// the edit here would bypass the guard's explicit choice.
			if (navigationGuardOpen) {
				void chooseNavigationGuard("cancel");
				return;
			}
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
		if (editOrbitFrame !== null) {
			cancelAnimationFrame(editOrbitFrame);
			editOrbitFrame = null;
		}
	}

	function teardownEdit(el: HTMLElement) {
		editingParagraph = null;
		originalMarkdownBlock = "";
		el.removeEventListener("blur", finishEdit);
		el.removeEventListener("keydown", handleEditKeydown);
		el.removeEventListener("input", scheduleEditOrbitUpdate);
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

	// Promise<boolean> confirm used by destructive actions; a second call while
	// one is open resolves false rather than stacking dialogs.
	function confirmAction(
		message: string,
		confirmLabel: string,
		danger = false
	): Promise<boolean> {
		if (actionConfirm) return Promise.resolve(false);
		return new Promise((resolve) => {
			actionConfirm = { message, confirmLabel, danger, resolve };
		});
	}

	function chooseActionConfirm(ok: boolean) {
		const pending = actionConfirm;
		actionConfirm = null;
		pending?.resolve(ok);
	}

	function requestNavigationGuard(reason: string): Promise<NavigationGuardChoice> {
		if (!editingParagraph) return Promise.resolve("save");
		if (navigationGuardResolve) return Promise.resolve("cancel");
		navigationGuardReason = reason;
		navigationGuardOpen = true;
		return new Promise((resolve) => {
			navigationGuardResolve = resolve;
		});
	}

	async function chooseNavigationGuard(choice: NavigationGuardChoice) {
		const resolve = navigationGuardResolve;
		if (!resolve) return;
		navigationGuardResolve = null;
		navigationGuardOpen = false;
		if (choice === "save") {
			resolve((await finishEdit()) ? "save" : "cancel");
			return;
		}
		if (choice === "discard") cancelEdit();
		resolve(choice);
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
		// could merge with a neighbour when the full document is parsed. Only
		// the two boundary lines are needed — read them without splitting the
		// whole source into a per-line array.
		const newLines = newBlockSource.split("\n");
		const beforeIdx = sourceStart - 2;
		if (beforeIdx >= 0 && (sourceLineAt($markdownSource, beforeIdx) ?? "").trim() !== "") return false;
		const afterIdx = sourceStart - 1 + newLines.length;
		const afterLine = sourceLineAt($markdownSource, afterIdx);
		if (afterLine !== null && afterLine.trim() !== "") return false;

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
				fileError = `保存失败：${describeError(err)}`;
				$markdownSource = oldMarkdownSource;
				// Keep the draft: the block stays in edit mode with the typed text
				// intact — Enter/blur retries the save, Esc still discards it.
				if (el.isConnected) {
					el.addEventListener("blur", finishEdit, { once: true });
					if (document.activeElement !== el) {
						el.focus();
						placeCaretAtEnd(el);
					}
				}
				showAppNotice("保存失败，已保留草稿——继续编辑后可重试，或按 Esc 放弃");
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

	// F12: crash shadow — every progress write is mirrored to localStorage
	// *before* the IPC call, tagged `acked` only after the backend save
	// resolves. A crash or wedged backend between the two leaves an unacked
	// shadow that the next open prefers over the stale on-disk state, so a
	// process kill can no longer silently drop the last reading position.
	type ProgressShadow = {
		savedAt: string;
		acked: boolean;
		scrollPosition?: number;
		progressRatio?: number;
		bookProgress?: unknown;
	};
	const PROGRESS_SHADOW_KEY = "ir-progress-shadow";
	const PROGRESS_SHADOW_LIMIT = 40;

	function readProgressShadows(): Record<string, ProgressShadow> {
		try {
			const raw = localStorage.getItem(PROGRESS_SHADOW_KEY);
			if (!raw) return {};
			const parsed = JSON.parse(raw);
			return parsed && typeof parsed === "object" ? parsed : {};
		} catch {
			return {};
		}
	}

	function writeProgressShadows(shadows: Record<string, ProgressShadow>) {
		try {
			const entries = Object.entries(shadows);
			if (entries.length > PROGRESS_SHADOW_LIMIT) {
				entries.sort((a, b) => b[1].savedAt.localeCompare(a[1].savedAt));
				shadows = Object.fromEntries(entries.slice(0, PROGRESS_SHADOW_LIMIT));
			}
			localStorage.setItem(PROGRESS_SHADOW_KEY, JSON.stringify(shadows));
		} catch {
			// Quota/full storage: the shadow is a bonus layer, never a blocker.
		}
	}

	function stageProgressShadow(key: string, shadow: Omit<ProgressShadow, "savedAt" | "acked">) {
		const shadows = readProgressShadows();
		shadows[key] = { ...shadow, savedAt: new Date().toISOString(), acked: false };
		writeProgressShadows(shadows);
	}

	function ackProgressShadow(key: string) {
		const shadows = readProgressShadows();
		const entry = shadows[key];
		if (!entry || entry.acked) return;
		shadows[key] = { ...entry, acked: true };
		writeProgressShadows(shadows);
	}

	function readProgressShadow(key: string): ProgressShadow | null {
		return readProgressShadows()[key] ?? null;
	}

	// saveState fires every few seconds while scrolling — surface a failure once
	// (not per tick) so a dead backend can't silently drop reading progress.
	let saveStateFailureNotified = false;
	async function saveState() {
		if (!$currentFilePath || !contentEl) return;
		const shadowKey = activeBook
			? `book:${activeBook.manifest.bookId}`
			: `file:${$currentFilePath}`;
		try {
			if (activeBook) {
				await persistActiveBookProgress();
			} else {
				stageProgressShadow(shadowKey, {
					scrollPosition: contentEl.scrollTop,
					progressRatio: readingProgress,
				});
				await invoke("save_reading_state", {
					path: $currentFilePath,
					state: {
						scroll_position: contentEl.scrollTop,
						bookmarks: [],
						progress: readingProgress,
					},
				});
			}
			ackProgressShadow(shadowKey);
			saveStateFailureNotified = false;
		} catch (e) {
			console.error("Failed to save state:", e);
			if (!saveStateFailureNotified) {
				saveStateFailureNotified = true;
				showAppNotice("阅读进度保存失败，退出前请确认进度已记录");
			}
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
			// Semantic English originals under Chinese translations: not focus targets.
			if (
				child.tagName === "BLOCKQUOTE" &&
				(child.classList.contains("podcast-original") || child.getAttribute("lang") === "en")
			) {
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

	// Wrap a long paragraph's sentences into <span class="focus-seg"> groups of
	// roughly FOCUS_SEGMENT_GROUP_LINES lines each. Text nodes are split at
	// sentence starts; inline elements (links, code…) stay whole.
	//
	// Layout work is strictly two-phase: every DOM mutation happens before a
	// single batched measurement pass. Interleaving getClientRects reads with
	// these inserts would force a synchronous reflow per sentence.
	// Text nodes inside inline code / KaTeX spans can contain `!`/`?`/`。`
	// that are not prose terminators (e.g. `n!`, formulas). Splitting there
	// produces ghost sentence boundaries the DOM can't even honor — a
	// boundary inside an inline element is never split — so the grouping
	// math skews. Mask those subtrees with same-length placeholders so
	// offsets still line up with textContent.
	function maskedSegmentationText(root: HTMLElement): string {
		const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
		let text = "";
		let node: Node | null;
		while ((node = walker.nextNode())) {
			const data = (node as Text).data;
			const parent = (node as Text).parentElement;
			text += parent?.closest("code, .katex") ? "x".repeat(data.length) : data;
		}
		return text;
	}

	function segmentLongParagraph(p: HTMLElement, lineHeight: number): HTMLElement[] {
		if (p.dataset.focusSegmented === "true") {
			return Array.from(p.querySelectorAll(":scope > .focus-seg")) as HTMLElement[];
		}

		const text = maskedSegmentationText(p);
		const sentences = splitSentences(text);
		if (sentences.length < 2) return [];

		// Phase 1 (writes only): split text nodes at sentence starts.
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

		// Phase 2 (reads only): measure every sentence span once. An inline
		// group wrapping sentences s..e has a run height of
		// bottom(lastRect(sentence e)) - top(firstRect(sentence s)) — the same
		// value a post-wrap getClientRects on the group would report — so
		// grouping can be computed without any further DOM writes.
		const sentenceMetrics = sentenceSpans.map((span) => {
			const rects = span.getClientRects();
			return rects.length === 0
				? { top: 0, bottom: 0 }
				: { top: rects[0].top, bottom: rects[rects.length - 1].bottom };
		});

		// Merge sentences into groups of ~GROUP_LINES lines, measured for real.
		const targetHeight = lineHeight * FOCUS_SEGMENT_GROUP_LINES;
		const groupRanges: [number, number][] = []; // [firstSentenceIdx, endIdxExclusive)
		let groupStart = 0;
		for (let i = 1; i < sentenceSpans.length; i++) {
			if (sentenceMetrics[i - 1].bottom - sentenceMetrics[groupStart].top >= targetHeight) {
				groupRanges.push([groupStart, i]);
				groupStart = i;
			}
		}
		groupRanges.push([groupStart, sentenceSpans.length]);

		// A tiny tail group reads as an orphan line — fold it into the previous.
		if (groupRanges.length >= 2) {
			const [tailStart, tailEnd] = groupRanges[groupRanges.length - 1];
			const tailHeight =
				sentenceMetrics[tailEnd - 1].bottom - sentenceMetrics[tailStart].top;
			if (tailHeight < lineHeight * FOCUS_SEGMENT_MIN_TAIL_LINES) {
				groupRanges[groupRanges.length - 2][1] = tailEnd;
				groupRanges.pop();
			}
		}

		// Phase 3 (writes only): wrap each sentence range in its group span.
		const groups: HTMLElement[] = [];
		for (const [start, end] of groupRanges) {
			const group = document.createElement("span");
			group.className = "focus-seg";
			p.insertBefore(group, sentenceSpans[start]);
			for (let i = start; i < end; i++) {
				group.appendChild(sentenceSpans[i]);
			}
			groups.push(group);
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

		// Measure every atom before any segmentation write: the loop below
		// wraps long paragraphs in span pairs, so reading offsetHeight on
		// demand forced one synchronous reflow per long paragraph. Heights are
		// position-independent, so batching the reads is behavior-identical.
		const atomHeights = atoms.map((atom) => atom.offsetHeight);

		for (let atomIndex = 0; atomIndex < atoms.length; atomIndex++) {
			const atom = atoms[atomIndex];
			const height = atomHeights[atomIndex];

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
		invalidateHeadingIndex();
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

		// Both updates are idempotent — collapse repeat calls into one frame.
		if (focusInteractionFrame === null) {
			focusInteractionFrame = requestAnimationFrame(() => {
				focusInteractionFrame = null;
				updateSpotlightPosition();
				if (editingParagraph && isEditingInDarkFocus) {
					updateEditOrbitPosition();
				}
			});
		}
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

	function scrollUnitToFocusPosition(
		unit: FocusUnit,
		focusIndex: number,
		forceBehavior?: ScrollBehavior,
	) {
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
			behavior:
				forceBehavior ??
				getJumpBehavior(Math.abs(nextScrollTop - contentEl.scrollTop)),
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
			const timer = setTimeout(() => {
				focusReleaseTimers.delete(timer);
				const index = Number(block.dataset.focusIndex ?? "-1");
				if (block.dataset.focusBlock === "true" && styledFocusIndices.has(index)) return;
				clearBlockFocusStyle(block);
			}, FOCUS_RELEASE_FADE_MS);
			focusReleaseTimers.add(timer);
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
		const signature = `${hitIdx}|${currentMatchIndex}|${searchMatchBlocks.size}|${$currentTheme.name}`;
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

			if (dist === 1) {
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

	function moveFocus(direction: -1 | 1): -1 | 1 | null {
		const units = getFocusUnits();
		if (units.length === 0) return null;

		const baseIdx = lastFocusedIdx >= 0 ? lastFocusedIdx : getClosestFocusIndex(units);
		const resolution = resolveFocusStep(baseIdx, units.length, direction);
		if (resolution.type === "chapter") {
			return resolution.direction;
		}
		const nextIdx = resolution.index;
		focusLockedIndex = nextIdx;
		markFocusScrollActive();
		updateFocusParagraph(nextIdx);
		scrollUnitToFocusPosition(units[nextIdx], nextIdx);
		updateSpotlightPosition();
		return null;
	}

	// Queue one focus step per accumulated wheel threshold crossing and flush
	// them inside a single animation frame — running moveFocus per input event
	// rewrites the blur window and issues a scrollTo each time.
	function scheduleFocusWheelStep(direction: -1 | 1) {
		pendingFocusWheelSteps += direction;
		if (focusWheelMoveFrame !== null) return;
		focusWheelMoveFrame = requestAnimationFrame(() => {
			focusWheelMoveFrame = null;
			const steps = pendingFocusWheelSteps;
			pendingFocusWheelSteps = 0;
			if (!$focusMode || steps === 0) return;
			const direction = steps > 0 ? 1 : -1;
			for (let i = Math.abs(steps); i > 0; i -= 1) {
				const chapterDirection = moveFocus(direction);
				if (chapterDirection !== null) {
					void navigateBookChapter(chapterDirection);
					break;
				}
			}
		});
	}

	function handleReadingScrollIntent(intent: ReadingScrollIntent, key: string) {
		if (!contentEl) return;
		cancelChapterBoundaryRestore();
		const resolution = resolveReadingScroll(intent, {
			scrollTop: contentEl.scrollTop,
			scrollHeight: contentEl.scrollHeight,
			clientHeight: contentEl.clientHeight,
		});
		if (resolution.type === "chapter") {
			navigateBookChapterFromKey(key, resolution.direction, resolution.offsetPx);
			return;
		}
		// Locked behavior: long jumps (e.g. Home/End across a long chapter)
		// cut instantly; only short distances smooth-scroll.
		contentEl.scrollTo({
			top: resolution.top,
			behavior: getJumpBehavior(Math.abs(resolution.top - contentEl.scrollTop)),
		});
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
		if ((await requestNavigationGuard("退出沉浸模式")) === "cancel") return false;
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
			while (idx !== -1 && blockMatches.length < MAX_SEARCH_MATCHES) {
				blockMatches.push({ block, occurrence });
				occurrence += 1;
				idx = text.indexOf(query, idx + query.length);
			}
			if (blockMatches.length >= MAX_SEARCH_MATCHES) break;
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

		const article = getArticleElement();
		if (!article) return;

		searchMatches = [];
		// Focus mode needs unit membership (data-focus-block/index) to
		// re-anchor the spotlight on the match's unit — those units are
		// already built and cached while focus is on. Outside focus mode a
		// search is a pure text scan: iterate the block atoms directly so
		// each keystroke doesn't pay a full focus-index pass (attribute
		// writes + per-unit rect reads) for nothing.
		const blocks = $focusMode ? getFocusUnits().flat() : collectFocusAtoms(article);
		for (const block of blocks) {
			collectMatchesInBlock(block, query, searchMatches);
			if (searchMatches.length >= MAX_SEARCH_MATCHES) break;
		}
		if (searchMatches.length >= MAX_SEARCH_MATCHES) {
			showAppNotice(`匹配过多，仅列出前 ${MAX_SEARCH_MATCHES} 条`);
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
		// In focus mode the enclosing unit's metrics were just measured —
		// reuse them instead of a rect read per matched block. data-focus-index
		// only exists on focus-mode unit members; anything else falls back to
		// a direct rect read.
		const metrics =
			$focusMode && focusMetricsValid && focusBlockMetrics.length === focusUnits.length
				? focusBlockMetrics
				: null;
		const positions: number[] = [];
		searchMatchBlocks.forEach((block) => {
			const index = metrics ? Number(block.dataset.focusIndex) : NaN;
			const metric = metrics && Number.isInteger(index) ? metrics[index] : null;
			if (metric) {
				positions.push(metric.top / total);
			} else {
				const rect = block.getBoundingClientRect();
				positions.push((scrollTop + rect.top - contentRect.top) / total);
			}
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
		if (searchMatches.length === 0) {
			// Enter/next on an empty result set used to be a dead key — say so.
			if ($searchQuery.trim()) {
				showAppNotice(`没有找到「${$searchQuery.trim()}」`);
			}
			return;
		}
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

	function prefersReducedMotion(): boolean {
		return (
			typeof window !== "undefined" &&
			window.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true
		);
	}

	function getJumpBehavior(distancePx: number): ScrollBehavior {
		if (prefersReducedMotion()) return "instant";
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

	// Heading positions in scroll-content coordinates
	// (scrollTop + rect.top - contentRect.top at measure time, same convention
	// as focusBlockMetrics). Rebuilt lazily — invalidateFocusMetrics() drops
	// the cache on every content/layout change; late-loading images invalidate
	// via a capture-phase load listener on contentEl.
	let headingIndex: { item: TocItem; top: number }[] | null = null;
	// The array the index was measured against — a swapped tocItems (new
	// render, edit, chapter change) always re-measures even if a call site
	// forgot to invalidate.
	let headingIndexItems: TocItem[] | null = null;

	function invalidateHeadingIndex() {
		headingIndex = null;
		headingIndexItems = null;
	}

	function getHeadingIndex() {
		if (!contentEl || tocItems.length === 0) return null;
		if (headingIndex && headingIndexItems === tocItems) return headingIndex;
		const contentRect = contentEl.getBoundingClientRect();
		const scrollTop = contentEl.scrollTop;
		const entries: { item: TocItem; top: number }[] = [];
		for (const item of tocItems) {
			const el = contentEl.querySelector(`#${CSS.escape(item.id)}`);
			if (!el) continue;
			entries.push({
				item,
				top: scrollTop + el.getBoundingClientRect().top - contentRect.top,
			});
		}
		headingIndex = entries;
		headingIndexItems = tocItems;
		return headingIndex;
	}

	function getCurrentHeading(): TocItem | null {
		if (activeBook) return tocItems[activeChapterIndex] ?? null;
		const index = getHeadingIndex();
		if (!contentEl || !index || index.length === 0) return null;
		// rect.top <= contentRect.top + clientHeight*0.35 expressed in scroll
		// coordinates: heading top <= scrollTop + clientHeight*0.35.
		const threshold = contentEl.scrollTop + contentEl.clientHeight * 0.35;
		// Entries follow document order, so their tops are non-decreasing —
		// binary search for the last heading at/above the threshold.
		let low = 0;
		let high = index.length - 1;
		let current = -1;
		while (low <= high) {
			const mid = (low + high) >> 1;
			if (index[mid].top <= threshold) {
				current = mid;
				low = mid + 1;
			} else {
				high = mid - 1;
			}
		}
		return current >= 0 ? index[current].item : null;
	}

	// ===== Bottom status line: chapter · progress · time left =====
	let statusLineVisible = false;
	let statusLineTimer: ReturnType<typeof setTimeout> | null = null;
	let statusChapterText = "";
	let lastStatusChapterUpdate = 0;

	// Mixed-language reading speed: ~400 CJK chars/min + ~230 latin words/min.
	// exec-loop counting keeps this O(1) memory — String.match would build a
	// per-CJK-char array over the whole document on every open and save.
	const CJK_CHAR_RE = /[一-鿿぀-ヿ가-힯]/g;
	const LATIN_WORD_RE = /[A-Za-z0-9]+/g;
	function countMatches(re: RegExp, text: string): number {
		re.lastIndex = 0;
		let count = 0;
		while (re.exec(text) !== null) count += 1;
		return count;
	}
	function estimateReadingMinutes(text: string): number {
		return countMatches(CJK_CHAR_RE, text) / 400 + countMatches(LATIN_WORD_RE, text) / 230;
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
	class:is-light-theme={isLightTheme}
	class:editing-in-focus={isEditingInDarkFocus}
	class:reading-cursor-hidden={readingCursorState.hidden}
	class:chrome-overlay={chromeOverlay}
	class:library-surface={librarySurface}
	class:layout-wide={windowMaximized}
>
	<WindowResizeHandles />

	<!-- Reading progress indicator (hidden during continuous reading) -->
	{#if $currentFilePath && !flowReaderSession}
		<div
			class="progress-line"
			role="progressbar"
			aria-label="本章阅读进度"
			aria-valuemin={0}
			aria-valuemax={100}
			aria-valuenow={Math.round(readingProgress * 100)}
			style="width: {readingProgress * 100}%"
		></div>
	{/if}

	<!-- 10px top-edge hotzone: only way to re-show immersive chrome -->
	{#if showTopEdgeHotzone}
		<div
			class="topbar-hover-zone"
			style="height: {CHROME_TOP_EDGE_PX}px"
			role="presentation"
			on:mouseenter={revealChromeFromTopEdge}
		></div>
	{/if}

	<!-- Unified chrome: custom window bar + context toolbar share chromeVisible -->
	<div
		class="chrome-stack"
		bind:this={chromeStackEl}
		class:hidden={!chromeVisible}
		class:overlay={chromeOverlay}
		aria-hidden={!chromeVisible}
		inert={!chromeVisible || undefined}
		on:mouseenter={onChromeMouseEnter}
		on:mouseleave={onChromeMouseLeave}
		on:focusin={onChromeFocusIn}
		on:focusout={onChromeFocusOut}
	>
		<WindowChrome
			visible={true}
			overlay={false}
			onMaximizedChange={handleWindowMaximizedChange}
		/>

		{#if showMarkdownContext}
			<header class="topbar context-bar" role="toolbar" aria-label="阅读工具栏">
				<div class="topbar-left">
					<BackButton label="返回书架" onClick={() => void returnToBookshelf()} />
					{#if fileName}
						<span class="filename">{isTemporaryReading ? `临时 · ${fileName}` : fileName}</span>
					{/if}
				</div>
				<div class="topbar-right">
					<button
						class="icon-btn"
						class:active={$focusMode}
						on:click={() => void toggleFocusMode(!$focusMode)}
						title="专注模式 (F11 / Esc)"
						aria-label="专注模式"
						aria-pressed={$focusMode}
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
						on:click={startEditAtReadingPosition}
						title="编辑当前段落 (E)"
						aria-label="编辑当前段落"
					>
						<svg
							width="18"
							height="18"
							viewBox="0 0 24 24"
							fill="none"
							stroke="currentColor"
							stroke-width="1.5"
						>
							<path d="M12 20h9" />
							<path d="M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4Z" />
						</svg>
					</button>
					<button
						class="icon-btn"
						class:active={$tocOpen}
						on:click={() => ($tocOpen = !$tocOpen)}
						title="目录 ({modLabel}T)"
						aria-label="目录"
						aria-expanded={$tocOpen}
						aria-controls="toc-panel"
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
						aria-label="搜索"
						aria-expanded={$searchOpen}
						aria-controls="search-panel"
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
					<button
						class="icon-btn"
						class:active={$settingsOpen}
						on:click={() => ($settingsOpen = !$settingsOpen)}
						title="设置 ({modLabel},)"
						aria-label="设置"
						aria-expanded={$settingsOpen}
						aria-controls="settings-panel"
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
					<button
						class="icon-btn"
						class:active={firstOpenHintVisible}
						on:click={() => (firstOpenHintVisible = !firstOpenHintVisible)}
						title="快捷键"
						aria-label="快捷键"
						aria-pressed={firstOpenHintVisible}
					>
						<svg
							width="18"
							height="18"
							viewBox="0 0 24 24"
							fill="none"
							stroke="currentColor"
							stroke-width="1.5"
						>
							<circle cx="12" cy="12" r="9" />
							<path d="M9.2 9a2.8 2.8 0 0 1 5.5.7c0 1.8-2.7 2.2-2.7 3.8" />
							<path d="M12 17h.01" stroke-linecap="round" />
						</svg>
					</button>
				</div>
			</header>
		{:else if showFlowContext}
			<header class="flow-context-bar context-bar" role="toolbar" aria-label="连续阅读">
				<div class="flow-context-left">
					<BackButton label="返回书架" onClick={() => void closeFlowReader()} />
					<div>
						<strong>连续阅读</strong>
						<span>所有章节留在沉浸阅读窗口内</span>
					</div>
				</div>
			</header>
		{/if}
	</div>

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

	{#if navigationGuardOpen}
		<!-- WorkflowDialogShell gives real modal semantics (showModal, Esc →
		     cancel, focus restore) — a bare <dialog open> has none of those. -->
		<WorkflowDialogShell
			titleId="navigation-guard-title"
			descriptionId="navigation-guard-desc"
			title="尚有未保存的编辑"
			description={`继续${navigationGuardReason}前，选择如何处理当前段落。`}
			maxWidth="480px"
			onClose={() => void chooseNavigationGuard("cancel")}
		>
			<div slot="footer" class="navigation-guard-actions">
				<button type="button" class="wf-primary" on:click={() => void chooseNavigationGuard("save")}>保存并继续</button>
				<button type="button" class="wf-secondary" on:click={() => void chooseNavigationGuard("discard")}>放弃并继续</button>
				<button type="button" class="wf-quiet" on:click={() => void chooseNavigationGuard("cancel")}>取消导航</button>
			</div>
		</WorkflowDialogShell>
	{/if}

	{#if actionConfirm}
		<WorkflowDialogShell
			titleId="action-confirm-title"
			descriptionId="action-confirm-desc"
			title="确认操作"
			description={actionConfirm.message}
			maxWidth="480px"
			onClose={() => chooseActionConfirm(false)}
		>
			<div slot="footer" class="navigation-guard-actions">
				<button
					type="button"
					class={actionConfirm.danger ? "wf-primary wf-danger" : "wf-primary"}
					on:click={() => chooseActionConfirm(true)}
				>{actionConfirm.confirmLabel}</button>
				<button type="button" class="wf-quiet" on:click={() => chooseActionConfirm(false)}>取消</button>
			</div>
		</WorkflowDialogShell>
	{/if}

	<!-- Main content -->
	<ReaderWorkspace flowActive={!!flowReaderSession} bind:element={contentEl}>
		{#if flowReaderSession}
			<section class="flow-reader-workspace" aria-label="连续阅读">
				<iframe
					bind:this={flowIframeEl}
					title="连续阅读器"
					src={flowReaderSession.url}
					sandbox="allow-same-origin allow-scripts"
					on:load={() => {
						// Frame (re)loaded: force all sync messages to re-send.
						lastPostedFontScale = null;
						lastPostedLayoutWide = null;
						lastPostedThemeName = null;
						postFlowFontScale($fontScale);
						postFlowLayoutMode(windowMaximized);
						postFlowTheme();
					}}
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
				events={taskEventLog}
				selectedBookDetail={selectedBookDetail}
				trashCount={trashItems.length}
				loading={libraryLoading || importFolderBusy}
				writable={libraryWritable}
				libraryRoot={appSettings?.libraryRoot ?? ""}
				onOpenBook={(bookId, chapterIndex) => void openLibraryBook(bookId, chapterIndex)}
				onOpenDetails={(bookId) => void openBookDetails(bookId)}
				onOpenSource={(source, sourceId) => openBookSource(source, sourceId)}
				onCloseDetails={() => (selectedBookDetail = null)}
				onFlowBook={(bookId) => void openBrowserReader(bookId)}
				onRefresh={() => void refreshLibrary()}
				onImport={() => void importFolderToLibrary()}
				onOpenFile={() => void openFileDialog()}
				onOpenTemporary={(path) => void openFile(path)}
				onOpenZhihuWorkflow={() => (zhihuWorkflowOpen = true)}
				onOpenPodcastWorkflow={() => (podcastWorkflowOpen = true)}
				onStartTask={(taskId) => void startPodcastTask(taskId)}
				onStartZhihuTask={(taskId, revision) => void startZhihuTask(taskId, revision)}
				onOpenTaskResult={(taskId) => void openPodcastTaskResult(taskId)}
				onRestartTask={(taskId) => void restartPodcastTask(taskId)}
				onApproveBudget={(taskId, limit) => void restartPodcastTask(taskId, limit)}
				onControlTask={(taskId, action, revision) => void controlPodcastTask(taskId, action, revision)}
				onControlZhihuTask={(taskId, action, revision) => void controlZhihuTask(taskId, action, revision)}
				onChooseLibrary={() => void chooseLibraryRoot()}
				onOpenTrash={openTrashPanel}
				onOpenSettings={() => ($settingsOpen = !$settingsOpen)}
				onRemoveBook={(bookId, title, chapterCount) =>
					void removeLibraryBook(bookId, title, chapterCount)}
				onDeleteBook={(bookId, title, chapterCount) =>
					void deleteLibraryBook(bookId, title, chapterCount)}
			/>
			{#if podcastWorkflowOpen}
				<PodcastWorkflow
					tasks={acquisitionTasks}
					onClose={() => (podcastWorkflowOpen = false)}
					onRefreshTasks={() => void refreshAcquisitionSnapshot()}
					onStartTask={(taskId) => void startPodcastTask(taskId)}
					onOpenResult={(taskId) => void openPodcastTaskResult(taskId)}
				/>
			{/if}
			{#if zhihuWorkflowOpen}
				<ZhihuWorkflow
					tasks={acquisitionTasks}
					onClose={() => (zhihuWorkflowOpen = false)}
					onRefreshTasks={() => void refreshAcquisitionSnapshot()}
					onStartTask={(taskId, revision) => void startZhihuTask(taskId, revision)}
					onControlTask={(taskId, action, revision) => void controlZhihuTask(taskId, action, revision)}
				/>
			{/if}
		{/if}
		{/if}
	</ReaderWorkspace>

	<!-- Spotlight overlays for focus mode. The dust/beam layer is a permanent
	     GPU cost while focus mode is on (80 particles + masked, blurred beams) —
	     the fan noise in focus mode comes from here. Locked visuals, kept
	     intentionally (P3-10). -->
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
			<span>{modLabel}T 目录 · {modLabel}F 搜索 · F11 专注 · E 编辑段落 · {modLabel}滚轮 字号</span>
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
		<div class="zoom-indicator app-notice" role="status" aria-live="polite">{appNoticeText}</div>
	{/if}

	<!-- Search hit tick marks along the right edge: pointer-only markers —
	     keyboard users navigate hits with Enter / Shift+Enter instead, so the
	     9×3px ticks stay out of the tab order and the a11y tree. -->
	{#if searchTickPositions.length > 0}
		<div class="search-ticks" aria-hidden="true">
			{#each searchTickPositions as pos, i (i)}
				<!-- svelte-ignore a11y-click-events-have-key-events -->
				<!-- svelte-ignore a11y-no-static-element-interactions -->
				<span
					class="search-tick"
					style="top: {pos * 100}%"
					on:click={() => jumpToSearchTick(pos)}
				></span>
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

	<!-- Footnote hover/focus preview — hoverable so readers can move onto it -->
	{#if footnotePreviewHtml}
		<div
			class="footnote-preview"
			role="tooltip"
			style={footnotePreviewStyle}
			on:mouseenter={cancelFootnoteHide}
			on:mouseleave={scheduleFootnoteHide}
		>
			{@html footnotePreviewHtml}
		</div>
	{/if}

	<!-- Image lightbox -->
	{#if lightboxSrc}
		<!-- svelte-ignore a11y-click-events-have-key-events -->
		<!-- svelte-ignore a11y-no-static-element-interactions -->
		<div
			class="lightbox-overlay"
			role="dialog"
			aria-modal="true"
			aria-label="图片预览"
			tabindex="-1"
			on:click={closeLightbox}
			on:wheel={handleLightboxWheel}
		>
			<img
				class="lightbox-image"
				src={lightboxSrc}
				alt={lightboxAlt}
				style="transform: scale({lightboxScale})"
			/>
			<button
				type="button"
				class="lightbox-close"
				aria-label="关闭图片预览"
				title="关闭 (Esc)"
				bind:this={lightboxCloseEl}
				on:click={closeLightbox}
			>
				×
			</button>
		</div>
	{/if}

	<!-- Bottom status line: appears while scrolling; never during continuous reading -->
	{#if $currentFilePath && !$focusMode && !editingParagraph && !flowReaderSession}
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

	<!-- Mobile Focus Mode Navigation HUD (OPPO Find X9 & mobile devices) -->
	{#if $focusMode && isMobile && $currentFilePath && !flowReaderSession}
		<div class="mobile-focus-bar" role="toolbar" aria-label="手机专注模式导航">
			<button class="mobile-focus-btn" on:click={() => moveFocus(-1)} aria-label="上一句">
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
					<path d="m18 15-6-6-6 6"/>
				</svg>
				<span>上一句</span>
			</button>
			<div class="mobile-focus-counter" aria-live="polite">
				{lastFocusedIdx >= 0 ? lastFocusedIdx + 1 : 1} / {focusUnits.length || 1}
			</div>
			<button class="mobile-focus-btn" on:click={() => moveFocus(1)} aria-label="下一句">
				<span>下一句</span>
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
					<path d="m6 9 6 6 6-6"/>
				</svg>
			</button>
			<button class="mobile-focus-btn mobile-focus-exit" on:click={() => void toggleFocusMode(false)} aria-label="退出专注模式">
				退出
			</button>
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

	.app.reading-cursor-hidden,
	.app.reading-cursor-hidden :global(*) {
		cursor: none !important;
	}

	/* ========== Unified chrome stack ========== */
	.chrome-stack {
		display: flex;
		flex-direction: column;
		flex: none;
		z-index: 55;
		background: color-mix(in srgb, var(--bg) 94%, var(--bg-secondary) 6%);
		border-bottom: 0;
		box-shadow: none;
		transition:
			transform 200ms cubic-bezier(0.2, 0.8, 0.2, 1),
			opacity 200ms cubic-bezier(0.2, 0.8, 0.2, 1);
		will-change: transform, opacity;
	}

	.chrome-stack.overlay {
		position: fixed;
		top: 0;
		left: 0;
		right: 0;
	}

	/* Homepage invariant: even if an old overlay class survives a reader
	   transition, the visible bookshelf still starts below the window bar. */
	.app.library-surface .chrome-stack {
		position: relative;
		top: auto;
		left: auto;
		right: auto;
	}

	.chrome-stack.hidden {
		transform: translateY(-100%);
		opacity: 0;
		pointer-events: none;
		visibility: hidden;
	}

	@media (prefers-reduced-motion: reduce) {
		.chrome-stack {
			transition: none;
		}
	}

	/* ========== Progress line ========== */
	.progress-line {
		position: fixed;
		top: 0;
		left: 0;
		height: 2px;
		background: var(--link);
		opacity: 0.3;
		z-index: 80;
		/* No width transition — scroll fires continuously and a transition
		   would lag behind the thumb anyway. */
		pointer-events: none; /* pure indicator — never cover window controls */
	}

	/* ========== Context toolbar under window chrome ========== */
	.topbar,
	.context-bar {
		display: flex;
		align-items: center;
		justify-content: space-between;
		height: 40px;
		padding: 0 16px;
		border-bottom: 1px solid color-mix(in srgb, var(--hr) 92%, var(--text-secondary) 8%);
		user-select: none;
		background: transparent;
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
	}

	.book-seam small {
		color: var(--text-faded);
		font-size: 11px;
		letter-spacing: 0.12em;
	}

	/* Invisible strip along the top edge; hovering summons chrome (does not drag).
	   Must sit BELOW .chrome-stack (z-55): when chrome is visible the window
	   controls own the top edge (a higher strip made their top 10px unclickable
	   and auto-hid chrome under the pointer); when chrome hides it is
	   pointer-events:none anyway, so the strip still catches the hover. */
	.topbar-hover-zone {
		position: fixed;
		top: 0;
		left: 0;
		right: 0;
		z-index: 50;
	}

	.topbar-left,
	.topbar-right {
		display: flex;
		align-items: center;
		gap: 4px;
		min-width: 0;
		overflow: visible;
	}

	.flow-context-left {
		display: flex;
		align-items: center;
		gap: 12px;
		min-width: 0;
	}
	.flow-context-bar {
		gap: 12px;
	}

	.flow-context-bar > div {
		display: flex;
		align-items: baseline;
		gap: 10px;
		min-width: 0;
	}

	.flow-context-bar strong {
		font-size: 13px;
		font-weight: 600;
	}

	.flow-context-bar span {
		font-size: 11px;
		color: var(--text-secondary);
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
		background: linear-gradient(135deg, color-mix(in srgb, var(--text) 10%, transparent) 0%, transparent 50%, color-mix(in srgb, var(--text) 4%, transparent) 100%);
		opacity: 0; transition: opacity 0.3s ease;
		pointer-events: none;
	}
	.icon-btn:hover {
		background: var(--bg-secondary);
		border-color: var(--hr);
		color: var(--text);
		transform: translateY(-2px);
		box-shadow:
			0 3px 10px color-mix(in srgb, var(--bg-secondary) 80%, transparent),
			0 1px 2px color-mix(in srgb, var(--text) 6%, transparent),
			inset 0 1px 1px color-mix(in srgb, var(--text) 8%, transparent);
		backdrop-filter: blur(8px);
		-webkit-backdrop-filter: blur(8px);
	}
	.icon-btn:hover::after {
		opacity: 1;
	}
	.icon-btn:active {
		transform: translateY(0);
		box-shadow:
			0 1px 3px color-mix(in srgb, var(--bg-secondary) 80%, transparent),
			0 1px 1px color-mix(in srgb, var(--text) 4%, transparent);
	}
	.icon-btn.active {
		color: var(--link);
		background: var(--bg-secondary);
		border-color: var(--hr);
	}
	.icon-btn:focus-visible {
		outline: 2px solid var(--link);
		outline-offset: 2px;
	}

	/* ========== Main content ========== */
	/* Flow chrome lives in the overlay stack; iframe fills the workspace. */
	.flow-reader-workspace {
		height: 100%;
		background: var(--bg);
	}
	.flow-reader-workspace iframe {
		width: 100%;
		height: 100%;
		border: 0;
		background: var(--bg);
		display: block;
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
		/* CJK 排版：break-word 仍能断超长 URL/代码，但不像 anywhere 那样把
		   句号逗号挤到行首；line-break: strict 启用中文避头尾规则。 */
		overflow-wrap: break-word;
		line-break: strict;
		hanging-punctuation: allow-end;
	}
	/* Maximized / fullscreen keeps the user's chosen column width
	   (--article-max-width) — the earlier fixed 1120px cap ignored it. */

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
		border: 1px solid color-mix(in srgb, var(--danger) 45%, transparent);
		border-radius: 8px;
		background: var(--bg-secondary);
		color: var(--danger);
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
		/* 负字距会让 CJK 字形在大字号标题下笔画粘连，正文标题不收紧。 */
		letter-spacing: 0;
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
	:global(.article h4) {
		font-size: 1.1em;
		font-weight: 600;
		color: var(--heading);
		margin: 1.2em 0 0.4em;
	}
	:global(.article h5) {
		font-size: 1em;
		font-weight: 600;
		color: var(--heading);
		margin: 1.1em 0 0.35em;
	}
	:global(.article h6) {
		font-size: 0.92em;
		font-weight: 600;
		color: var(--text-secondary);
		letter-spacing: 0.02em;
		margin: 1em 0 0.3em;
	}

	:global(.article p) {
		margin: 0.8em 0;
		color: var(--text);
	}
	:global(.article p.podcast-translation) {
		margin: 0.34em 0;
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

	/* remark-rehype 脚注区标题（「脚注」）保留在无障碍树中但不占版面。 */
	:global(.article .sr-only) {
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

	/* 裸露元素兜底：sanitize 放行但上方没有专门规则的标签。 */
	:global(.article del) {
		color: var(--text-faded);
	}
	:global(.article ins) {
		text-decoration: underline;
		text-decoration-style: dotted;
		text-underline-offset: 0.2em;
	}
	:global(.article sub),
	:global(.article sup) {
		font-size: 0.75em;
		line-height: 0;
	}
	:global(.article kbd) {
		font-family: "Cascadia Code", "Fira Code", "JetBrains Mono", "Consolas",
			monospace;
		font-size: 0.85em;
		padding: 0.1em 0.4em;
		border: 1px solid var(--hr);
		border-bottom-width: 2px;
		border-radius: 4px;
		background: var(--bg-secondary);
	}
	:global(.article mark) {
		background: var(--search-highlight);
		color: inherit;
		border-radius: 2px;
		padding: 1px 0;
	}
	:global(.article figure) {
		margin: 1.5em 0;
	}
	:global(.article figcaption) {
		margin-top: 0.5em;
		text-align: center;
		font-size: 0.85em;
		color: var(--text-secondary);
	}
	:global(.article details) {
		margin: 0.8em 0;
		padding: 0.6em 1em;
		border: 1px solid var(--hr);
		border-radius: 8px;
		background: var(--bg-secondary);
	}
	:global(.article summary) {
		cursor: pointer;
		font-weight: 600;
	}
	:global(.article abbr[title]) {
		text-decoration: underline dotted;
		text-underline-offset: 0.2em;
		cursor: help;
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

	/* Podcast English original: soft veil via text-shadow, no filter:blur() */
	:global(.article blockquote.podcast-original) {
		margin: 0.35em 0 1.1em;
		padding: 0.35em 0 0.35em 0.9em;
		border-left: 2px solid color-mix(in srgb, var(--line, var(--hr)) 80%, transparent);
		background: transparent;
		color: var(--text-secondary);
		opacity: 0.82;
		filter: blur(4.5px);
		text-shadow: 0 0 2px color-mix(in srgb, var(--text-secondary) 45%, transparent);
		cursor: pointer;
		transition: color 0.2s ease, filter 0.2s ease, opacity 0.2s ease, text-shadow 0.2s ease;
	}
	:global(.article blockquote.podcast-original:hover),
	:global(.article blockquote.podcast-original:focus),
	:global(.article blockquote.podcast-original:focus-within),
	:global(.article blockquote.podcast-original.is-revealed),
	:global(.article blockquote.podcast-original.is-selection-revealed) {
		color: var(--text-secondary);
		text-shadow: none;
		filter: none;
		opacity: 1;
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
	/* Dual-theme Shiki output carries light inline colors + --shiki-dark*
	 * vars; switch to the dark vars when the active theme maps to a dark
	 * Shiki theme (stamped on <html data-shiki> by applyTheme). */
	:global(html[data-shiki='github-dark'] .article .shiki),
	:global(html[data-shiki='github-dark'] .article .shiki span) {
		color: var(--shiki-dark) !important;
		background-color: var(--shiki-dark-bg) !important;
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
	/* Expand the tap target past the visual pill without changing its look. */
	.code-copy-btn::before {
		content: '';
		position: absolute;
		inset: -8px;
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
		/* 长脚注在预览内滚动而不是撑出视口（预览可悬停，不会一闪即逝）。 */
		max-height: min(320px, 60vh);
		overflow-y: auto;
		box-shadow: 0 8px 32px rgba(0, 0, 0, 0.18);
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
	.lightbox-close {
		position: fixed;
		top: 18px;
		right: 18px;
		width: 36px;
		height: 36px;
		border: 1px solid var(--hr);
		border-radius: 50%;
		background: var(--bg-secondary);
		color: var(--text-secondary);
		font-size: 18px;
		line-height: 1;
		cursor: pointer;
	}
	.lightbox-close:hover {
		color: var(--text);
	}
	.lightbox-close:focus-visible {
		outline: 2px solid var(--link);
		outline-offset: 2px;
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
		position: relative;
	}
	/* Expand the tap target to ~32px without changing the 20px visual. */
	.first-hint-close::before {
		content: '';
		position: absolute;
		inset: -6px;
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
	.focus-mode :global(.article pre) {
		background: transparent;
		border-color: transparent;
	}
	.focus-mode :global(.article blockquote.podcast-original:not(.is-revealed):not(.is-selection-revealed)) {
		visibility: hidden;
		opacity: 0;
		filter: none;
		pointer-events: none;
	}
	.focus-mode :global(.article blockquote.podcast-original.is-revealed),
	.focus-mode :global(.article blockquote.podcast-original.is-selection-revealed) {
		visibility: visible;
		pointer-events: auto;
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
	.navigation-guard-actions {
		display: flex;
		justify-content: flex-end;
		gap: 8px;
		flex-wrap: wrap;
	}
	/* Danger variant of the shell's primary action (confirm-delete flows).
	   The fill is the theme's --danger; --on-accent keeps the label readable
	   whether the danger tone is a light fill (dark themes) or a deep red
	   (light themes). */
	:global(.wf-primary.wf-danger) {
		border-color: var(--danger);
		background: var(--danger);
		color: var(--on-accent);
	}
	:global(.wf-primary.wf-danger:hover) {
		border-color: color-mix(in srgb, var(--danger) 82%, var(--text));
		background: color-mix(in srgb, var(--danger) 82%, var(--text));
		color: var(--on-accent);
	}

	/* Reduced motion: drop non-essential animation/transitions on UI chrome.
	   The Focus Mode particle/spotlight layer is intentionally excluded — its
	   visuals are locked behavior (see P3-10). */
	@media (prefers-reduced-motion: reduce) {
		.article,
		.search-tick,
		.code-copy-btn,
		.footnote-preview,
		.lightbox-overlay,
		.lightbox-image,
		.zoom-indicator,
		.first-hint,
		.edit-hint,
		.status-line-pill,
		.loading-dot,
		.icon-btn,
		.progress-line {
			animation: none;
			transition: none;
		}
		:global(.editing) {
			animation: none;
			box-shadow: 0 0 0 1.5px var(--selection);
		}
	}

	/* ===== Mobile Adaptation (e.g. OPPO Find X9 & iOS/Android devices) ===== */
	@media (max-width: 768px) {
		.app {
			padding-top: env(safe-area-inset-top, 0px);
			padding-bottom: env(safe-area-inset-bottom, 0px);
			padding-left: env(safe-area-inset-left, 0px);
			padding-right: env(safe-area-inset-right, 0px);
		}

		.context-bar {
			padding-left: max(14px, env(safe-area-inset-left, 0px));
			padding-right: max(14px, env(safe-area-inset-right, 0px));
			height: 48px;
		}

		.topbar-left .filename {
			max-width: 150px;
			overflow: hidden;
			text-overflow: ellipsis;
			white-space: nowrap;
			font-size: 13.5px;
		}

		.article {
			--article-padding-x: max(16px, env(safe-area-inset-left, 0px));
			--article-padding-top: calc(16px + env(safe-area-inset-top, 0px));
			--article-padding-bottom: calc(88px + env(safe-area-inset-bottom, 0px));
			max-width: 100% !important;
			padding-left: var(--article-padding-x);
			padding-right: var(--article-padding-x);
			padding-top: var(--article-padding-top);
			padding-bottom: var(--article-padding-bottom);
		}

		:global(.article pre),
		:global(.article table),
		:global(.article .mermaid) {
			max-width: 100%;
			overflow-x: auto;
			-webkit-overflow-scrolling: touch;
		}

		.icon-btn {
			width: 38px;
			height: 38px;
			padding: 8px;
		}

		.status-line-pill {
			bottom: calc(14px + env(safe-area-inset-bottom, 0px));
			font-size: 11.5px;
			padding: 4px 12px;
		}

		.mobile-focus-bar {
			position: fixed;
			bottom: calc(18px + env(safe-area-inset-bottom, 0px));
			left: 50%;
			transform: translateX(-50%);
			z-index: 95;
			display: flex;
			align-items: center;
			gap: 8px;
			padding: 6px 14px;
			border-radius: 999px;
			background: color-mix(in srgb, var(--bg-secondary) 90%, transparent);
			backdrop-filter: blur(16px);
			-webkit-backdrop-filter: blur(16px);
			border: 1px solid var(--hr);
			box-shadow: 0 8px 32px rgba(0, 0, 0, 0.28);
			animation: fadeIn 0.2s ease;
		}

		.mobile-focus-btn {
			display: flex;
			align-items: center;
			gap: 4px;
			border: none;
			background: transparent;
			color: var(--text);
			font-size: 13px;
			font-weight: 500;
			padding: 6px 10px;
			border-radius: 999px;
			cursor: pointer;
			touch-action: manipulation;
		}

		.mobile-focus-btn:active {
			background: color-mix(in srgb, var(--text) 14%, transparent);
		}

		.mobile-focus-counter {
			font-size: 12px;
			color: var(--text-secondary);
			padding: 0 6px;
			font-variant-numeric: tabular-nums;
			font-weight: 500;
		}

		.mobile-focus-exit {
			color: var(--danger);
			font-size: 12px;
			margin-left: 4px;
		}
	}
</style>










