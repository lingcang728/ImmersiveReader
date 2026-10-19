<!--
	Android-first mobile shell.

	Owns boot, the shelf ⇄ reader ⇄ settings view state, the Android back
	stack (history.pushState/popstate), toasts, the import sheet and the
	pending-open intent drain. It deliberately imports no desktop page or
	component: the shelf/settings surfaces live in this folder and the reader
	is a lazily imported full-screen overlay.

	Boot rule (docs/mobile-redesign.md §5): the first frame renders the shelf
	skeleton immediately and only three IPC calls start concurrently —
	platform capabilities, library scan and pending-open drain.

	Back-stack contract with the reader:
	  - the shell pushes one history entry per shell layer (reader / settings /
	    search / import sheet / trash confirm);
	  - on popstate the shell asks the reader to consume the back gesture via a
	    `ir:android-back` window event; the reader closes its own sheet (keeping
	    the entry re-pushed) or calls `onExit` (the entry stays popped).
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import type { Component } from 'svelte';
	import './mobile.css';
	import * as api from '$lib/mobile/api';
	import type { ImportStatus } from '$lib/mobile/api';
	import { currentTheme } from '$lib/stores/app';
	import { volumeKeyPaging } from '$lib/platform/device';
	import {
		fallbackPlatformCapabilities,
		normalizePlatformCapabilities,
		type PlatformCapabilities
	} from '$lib/platform/capabilities';
	import Shelf from './Shelf.svelte';
	import SettingsScreen from './SettingsScreen.svelte';
	import ImportSheet from './ImportSheet.svelte';
	import Toast from './components/Toast.svelte';
	import Sheet from './components/Sheet.svelte';
	import { importToast, pickImportedBook } from './importState';
	import { appendToast, removeToast, toastDuration, type ToastItem, type ToastKind } from './ui';
	import type { BookSummary, LibraryIssue, MobileView, ReaderTarget } from './contracts';

	interface ReaderProps {
		target: ReaderTarget;
		onExit: () => void;
	}

	type ReaderComponent = Component<ReaderProps>;

	// ----- view state ---------------------------------------------------
	let view = $state<MobileView>('shelf');
	let readerTarget = $state<ReaderTarget | null>(null);
	let searchOpen = $state(false);
	let importOpen = $state(false);
	let importInitialUris = $state<string[] | null>(null);
	let deleteTarget = $state<BookSummary | null>(null);
	let deleting = $state(false);

	const shelfInert = $derived(
		view !== 'shelf' || importOpen || deleteTarget !== null
	);

	// ----- shelf snapshot ----------------------------------------------
	let books = $state<BookSummary[]>([]);
	let issues = $state<LibraryIssue[]>([]);
	let libraryWritable = $state(true);
	let shelfLoading = $state(true);
	let bootError = $state<string | null>(null);
	let caps = $state<PlatformCapabilities>(fallbackPlatformCapabilities());

	// ----- toasts -------------------------------------------------------
	let toasts = $state<ToastItem[]>([]);
	let toastSeq = 0;
	const toastTimers = new Map<number, ReturnType<typeof setTimeout>>();

	function showToast(text: string, kind: ToastKind = 'info') {
		const message = text.trim();
		if (!message) return;
		toastSeq += 1;
		const id = toastSeq;
		toasts = appendToast(toasts, { id, text: message, kind });
		const timer = setTimeout(() => {
			toastTimers.delete(id);
			toasts = removeToast(toasts, id);
		}, toastDuration(kind));
		toastTimers.set(id, timer);
	}

	// ----- library / caps / intents ------------------------------------
	let libraryRefreshNonce = 0;
    async function refreshLibrary(silent = false) {
        const nonce = ++libraryRefreshNonce;
		if (!silent) shelfLoading = true;
		try {
			const snapshot = await api.scanLibrary();
            if (nonce !== libraryRefreshNonce) return;
			books = snapshot.books;
			issues = snapshot.issues;
			libraryWritable = snapshot.writable;
			bootError = null;
		} catch (error) {
			const message = api.errorText(error);
			console.warn('[mobile] library scan failed', error);
			if (!silent) bootError = message;
		} finally {
            if (nonce === libraryRefreshNonce) shelfLoading = false;
        }
    }

	async function loadCapabilities() {
		try {
			const raw = await api.getRawPlatformCapabilities();
			caps = normalizePlatformCapabilities(raw, fallbackPlatformCapabilities());
		} catch (error) {
			console.warn('[mobile] platform capabilities unavailable', error);
		}
	}

	let drainingIntents = false;

	async function drainIntents() {
		if (drainingIntents) return;
		drainingIntents = true;
		try {
			const uris = await api.takePendingOpenUris();
			if (uris.length > 0) openImportSheet(uris);
		} catch (error) {
			console.warn('[mobile] pending open intents unavailable', error);
		} finally {
			drainingIntents = false;
		}
	}

	let booted = false;

	async function boot() {
		if (booted) return;
        booted = true;
		// First frame already rendered the skeleton; exactly three concurrent
		// IPC calls are allowed here (docs §5).
		const scanTask = refreshLibrary();
		const capsTask = loadCapabilities();
		const intentTask = drainIntents();
		await Promise.allSettled([scanTask, capsTask, intentTask]);
	}

	// ----- reader (lazy chunk) -----------------------------------------
	let ReaderScreen = $state<ReaderComponent | null>(null);
	let readerLoad: Promise<boolean> | null = null;

	function ensureReader(silent = false): Promise<boolean> {
		if (ReaderScreen !== null) return Promise.resolve(true);
		if (readerLoad !== null) return readerLoad;
		readerLoad = import('./reader/ReaderScreen.svelte')
			.then((module) => {
				ReaderScreen = module.default;
				return true;
			})
			.catch((error) => {
				if (!silent) console.error('[mobile] reader chunk failed to load', error);
				return false;
			})
			.finally(() => {
				readerLoad = null;
			});
		return readerLoad;
	}

	let warmedReader = false;

	function warmReaderOnce() {
		if (warmedReader) return;
		warmedReader = true;
		// First touch anywhere is the earliest reliable "about to interact"
		// signal; loading the chunk now hides the first-open latency without
		// paying for it during the critical boot path.
		void ensureReader(true);
	}

	async function openReader(target: ReaderTarget, replace?: ShellLayer) {
		if (view === 'reader') return;
		readerTarget = target;
		view = 'reader';
		if (replace) replaceLayer(replace, 'reader');
		else pushLayer('reader');
		if (ReaderScreen !== null) return;
		const loaded = await ensureReader();
		if (loaded) return;
		if (view !== 'reader') return;
		view = 'shelf';
		readerTarget = null;
		removeLayer('reader');
		showToast('阅读器加载失败，请重试', 'error');
	}

	function handleReaderExit() {
		if (view !== 'reader') return;
		view = 'shelf';
		readerTarget = null;
		removeLayer('reader');
		// The reader persists progress/chapters on exit; refresh quietly so the
		// continue-reading card reflects it without blanking the shelf.
		void refreshLibrary(true);
	}

	function openBook(book: BookSummary) {
		void openReader({ kind: 'book', bookId: book.bookId, title: book.title });
	}

	// ----- settings / search / sheets ----------------------------------
	function openSettings() {
		if (view !== 'shelf') return;
		view = 'settings';
		pushLayer('settings');
	}

	function closeSettings() {
        void refreshLibrary(true);
		if (view !== 'settings') return;
		view = 'shelf';
		removeLayer('settings');
	}

	function toggleSearch() {
		if (view !== 'shelf') return;
		if (searchOpen) {
			searchOpen = false;
			removeLayer('search');
			return;
		}
		if (importOpen || deleteTarget !== null) return;
		searchOpen = true;
		pushLayer('search');
	}

	function closeSearch() {
		if (!searchOpen) return;
		searchOpen = false;
		removeLayer('search');
	}

	function openImportSheet(uris: string[] | null = null) {
		if (importOpen) {
			// A second share while an import is running is intentionally ignored:
			// replacing the live operation would orphan the backend worker.
			if (uris && uris.length > 0) showToast('已有导入正在进行', 'info');
			return;
		}
		importInitialUris = uris;
		importOpen = true;
		pushLayer('import');
	}

	function closeImportSheet() {
		if (!importOpen) return;
        importOpen = false;
        importInitialUris = null;
        void refreshLibrary(true);
        removeLayer('import');
	}

	function requestDelete(book: BookSummary) {
		if (deleteTarget !== null) return;
		deleteTarget = book;
		pushLayer('delete');
	}

	function closeDeleteConfirm() {
		if (deleteTarget === null) return;
		deleteTarget = null;
		deleting = false;
		removeLayer('delete');
	}

	async function confirmDelete() {
		const book = deleteTarget;
		if (book === null || deleting) return;
		deleting = true;
		try {
			await api.removeBookToTrash(book.bookId);
			deleteTarget = null;
			removeLayer('delete');
			showToast(`《${book.title}》已移入回收站`, 'success');
			await refreshLibrary(true);
		} catch (error) {
			showToast(api.errorText(error), 'error');
			deleting = false;
		}
	}

	async function handleImportCompleted(status: ImportStatus) {
		const beforeIds = new Set(books.map((book) => book.bookId));
		// Keep the history layer until the follow-up view is decided: opening
		// the imported book replaces the sheet layer (same depth, no pop/push
		// race), while a plain completion pops it.
		importOpen = false;
		importInitialUris = null;
		await refreshLibrary(true);
		const imported = pickImportedBook(status, beforeIds, books);
		const toast = importToast(status, imported);
		if (imported) {
			showToast(toast.text, toast.kind);
			await openReader({ kind: 'book', bookId: imported.bookId, title: imported.title }, 'import');
			return;
		}
		removeLayer('import');
		showToast(toast.text, toast.kind);
	}

	// ----- Android back stack ------------------------------------------
	//
	// One history entry per open shell layer, pushed/popped in the order the
	// layers were opened (the stack below), so layered combinations — reader
	// above search, import sheet above reader — unwind last-in-first-out.
	// `irMobileDepth` in the history state lets a programmatic pop be told
	// apart from a user back gesture after the fact.
	type ShellLayer = 'reader' | 'settings' | 'search' | 'import' | 'delete';

	const layerStack: ShellLayer[] = [];
	let historyDepth = 0;
	let expectedPops = 0;

	function syncHistory() {
		if (typeof history === 'undefined') return;
		const target = layerStack.length;
		if (target === historyDepth) return;
		if (target > historyDepth) {
			for (let depth = historyDepth + 1; depth <= target; depth += 1) {
				try {
					history.pushState({ irMobileDepth: depth }, '');
				} catch {
					historyDepth = depth - 1;
					return;
				}
			}
			historyDepth = target;
			return;
		}
		const popCount = historyDepth - target;
		historyDepth = target;
		expectedPops += popCount;
		try {
			history.go(-popCount);
		} catch {
			expectedPops = Math.max(0, expectedPops - popCount);
		}
	}

	function pushLayer(layer: ShellLayer) {
		if (layerStack[layerStack.length - 1] === layer) return;
		layerStack.push(layer);
		syncHistory();
	}

	function removeLayer(layer: ShellLayer) {
		const index = layerStack.lastIndexOf(layer);
		if (index < 0) return;
		layerStack.splice(index, 1);
		syncHistory();
	}

	/** Swap one layer for another without popping/pushing history. */
	function replaceLayer(from: ShellLayer, to: ShellLayer) {
		const index = layerStack.lastIndexOf(from);
		if (index >= 0) layerStack.splice(index, 1);
		layerStack.push(to);
		syncHistory();
	}

	function depthFromState(state: unknown): number | null {
		if (typeof state !== 'object' || state === null) return null;
		const value = (state as { irMobileDepth?: unknown }).irMobileDepth;
		if (typeof value !== 'number' || !Number.isFinite(value)) return null;
		return Math.max(0, Math.trunc(value));
	}

	function consumeReaderBack() {
		if (ReaderScreen === null) {
			handleReaderExit();
			return;
		}
		// The reader closes its top sheet, or exits when nothing is open.
		window.dispatchEvent(new CustomEvent('ir:android-back'));
	}

	function handlePopState(event: PopStateEvent) {
		const reported = depthFromState(event.state);
		if (expectedPops > 0) {
			expectedPops -= 1;
			if (reported !== null) historyDepth = Math.min(historyDepth, reported);
			return;
		}
		historyDepth = reported !== null ? reported : Math.max(0, historyDepth - 1);
		const top = layerStack.pop() ?? null;
		if (top === null) {
			syncHistory();
			return;
		}
		switch (top) {
			case 'reader':
				consumeReaderBack();
				// A reader that only closed its own sheet keeps its shell layer.
				if (view === 'reader') layerStack.push('reader');
				break;
			case 'settings':
				view = 'shelf';
				break;
			case 'search':
				searchOpen = false;
				break;
			case 'import':
				// Unmounting ImportSheet cancels a running operation.
				importOpen = false;
				importInitialUris = null;
				break;
			case 'delete':
				deleteTarget = null;
				deleting = false;
				break;
		}
		syncHistory();
	}

	// ----- volume-key capture ------------------------------------------
	// Default capture state is off; only an explicit transition (reader open)
	// issues the IPC, so boot stays at exactly three concurrent calls.
	let volumeCaptureActive: boolean | null = false;
	let immersiveActive: boolean | null = null;
	let readerCaptureAllowed = $state(true);
	let appVisible = $state(true);
	let textSelected = $state(false);

	function updateVolumeCapture() {
		const wanted = view === 'reader' && caps.volumeKeyBridge && $volumeKeyPaging && readerCaptureAllowed && appVisible && !textSelected;
		if (wanted === volumeCaptureActive) return;
		volumeCaptureActive = wanted;
		void api.setVolumeKeyCapture(wanted).catch((error) => {
			volumeCaptureActive = null;
			console.warn('[mobile] volume-key capture toggle failed', error);
		});
	}

	$effect(() => {
		// Re-evaluated on view/capability/volume-store changes.
		void view;
		void caps;
		void $volumeKeyPaging;
		updateVolumeCapture();
	});

	$effect(() => {
		if (caps.platform !== 'android') return;
		const wanted = view === 'reader';
		if (wanted === immersiveActive) return;
		immersiveActive = wanted;
		void api.setReaderImmersive(wanted).catch((error) => {
			immersiveActive = null;
			console.warn('[mobile] status-bar toggle failed', error);
		});
	});

	// ----- lifecycle ----------------------------------------------------
	function teardown() {
		if (immersiveActive) void api.setReaderImmersive(false).catch(() => {});
		for (const timer of toastTimers.values()) clearTimeout(timer);
		toastTimers.clear();
		if (volumeCaptureActive) {
			volumeCaptureActive = null;
			void api.setVolumeKeyCapture(false).catch(() => {});
		}
	}

	onMount(() => {
		void boot();
		const onReaderCapture = (event: Event) => { readerCaptureAllowed = (event as CustomEvent<{ enabled: boolean }>).detail.enabled; };
		const onSelection = () => { textSelected = Boolean(window.getSelection()?.toString()); };
		window.addEventListener('ir:reader-capture', onReaderCapture);
		document.addEventListener('selectionchange', onSelection);
		const onVisibilityChange = () => {
			appVisible = document.visibilityState === 'visible';
			if (document.visibilityState !== 'visible') return;
			// App resumed: a share/VIEW intent may have queued while backgrounded.
			void drainIntents();
		};
		document.addEventListener('visibilitychange', onVisibilityChange);
		return () => {
			window.removeEventListener('ir:reader-capture', onReaderCapture);
			document.removeEventListener('selectionchange', onSelection);
			document.removeEventListener('visibilitychange', onVisibilityChange);
			teardown();
		};
	});
</script>

<svelte:window onpopstate={handlePopState} />

<main class="mobile-root" data-testid="mobile-root" data-theme={$currentTheme.name}>
	<div class="shelf-layer" inert={shelfInert}>
		<Shelf
			{books}
			{issues}
			loading={shelfLoading}
			writable={libraryWritable}
			{bootError}
			{searchOpen}
			onOpenBook={openBook}
			onOpenSettings={openSettings}
			onImport={() => openImportSheet(null)}
			onRequestDelete={requestDelete}
			onRetryScan={() => void refreshLibrary()}
			onToggleSearch={toggleSearch}
			onCloseSearch={closeSearch}
			onWarmReader={warmReaderOnce}
		/>
	</div>

	{#if view === 'settings'}
		<SettingsScreen {caps} onBack={closeSettings} onToast={showToast} />
	{/if}

	{#if view === 'reader'}
		{#if ReaderScreen && readerTarget}
			{@const ActiveReader = ReaderScreen}
			{@const activeTarget = readerTarget}
			<ActiveReader target={activeTarget} onExit={handleReaderExit} />
		{:else}
			<div class="reader-boot" aria-busy="true">
				<p class="reader-boot-text">正在打开…</p>
			</div>
		{/if}
	{/if}

	{#if importOpen}
		<ImportSheet
			open
			initialUris={importInitialUris}
			onClose={closeImportSheet}
			onCompleted={(status) => void handleImportCompleted(status)}
			onToast={showToast}
		/>
	{/if}

	{#if deleteTarget}
		<Sheet open title="移入回收站" testid="delete-sheet" onClose={closeDeleteConfirm}>
			<p class="delete-copy">
				确定将《{deleteTarget.title}》移入回收站？之后可以从回收站恢复。
			</p>
			<div class="delete-actions">
				<button type="button" class="outline-btn" onclick={closeDeleteConfirm}>取消</button>
				<button
					type="button"
					class="danger-btn"
					disabled={deleting}
					onclick={() => void confirmDelete()}
				>
					{deleting ? '正在移入…' : '移入回收站'}
				</button>
			</div>
		</Sheet>
	{/if}

	<Toast {toasts} />
</main>

<style>
	.shelf-layer {
		position: absolute;
		inset: 0;
		display: flex;
		flex-direction: column;
	}

	.reader-boot {
		position: fixed;
		inset: 0;
		z-index: 40;
		display: flex;
		align-items: center;
		justify-content: center;
		background: var(--bg);
		color: var(--text-secondary);
	}

	.reader-boot-text {
		margin: 0;
		font-size: 14px;
	}

	.delete-copy {
		margin: 0;
		color: var(--text);
		font-size: 15px;
		line-height: 1.7;
	}

	.delete-actions {
		display: flex;
		gap: 8px;
		margin-top: 20px;
	}

	.delete-actions button {
		flex: 1;
		min-height: 48px;
		padding: 0 16px;
		border-radius: var(--ir-radius-md);
		font-size: 15px;
	}

	.outline-btn {
		border: 1px solid var(--hr);
		background: transparent;
		color: var(--text);
	}

	.outline-btn:active {
		background: var(--bg-secondary);
	}

	.danger-btn {
		border: 1px solid color-mix(in srgb, var(--danger) 48%, var(--hr));
		background: transparent;
		color: var(--danger);
		font-weight: 500;
	}

	.danger-btn:active {
		background: color-mix(in srgb, var(--danger) 10%, transparent);
	}

	.danger-btn:disabled {
		opacity: 0.6;
	}
</style>
