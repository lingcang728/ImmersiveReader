<script lang="ts">
	// EPUB 全书搜索浮层。调用后端 search_book；命中列表点击 → 父级导航到
	// 对应章节并定位。
	import { onMount, tick } from "svelte";
	import { cycleFocusWithin } from "$lib/a11y/focusTrap";
	import { searchBook } from "$lib/epub/ipc";
	import { reportEpubError } from "$lib/epub/errors";
	import { sanitizeSearchSnippet } from "$lib/epub/sanitize";
	import { debounced } from "$lib/epub/timing";
	import type { SearchHit } from "$lib/epub/types";

	export let bookId: string;
	export let isMobile = false;
	export let onJump: (hit: SearchHit) => void = () => {};
	export let onClose: () => void = () => {};

	let inputEl: HTMLInputElement | null = null;
	let panelEl: HTMLElement | null = null;
	let restoreFocusEl: HTMLElement | null = null;

	let query = "";
	let hits: SearchHit[] = [];
	let searching = false;
	let searched = false;
	let errorMsg = "";
	let searchSeq = 0;

	async function runSearch() {
		const q = query.trim();
		const seq = ++searchSeq;
		if (!q) {
			hits = [];
			searched = false;
			searching = false;
			errorMsg = "";
			return;
		}
		searching = true;
		errorMsg = "";
		try {
			const result = await searchBook(bookId, q, 50);
			if (seq !== searchSeq) return; // a newer query already in flight
			hits = result;
			searched = true;
		} catch (error) {
			if (seq !== searchSeq) return;
			errorMsg = reportEpubError("epub-search", error);
			hits = [];
			searched = true;
		} finally {
			if (seq === searchSeq) searching = false;
		}
	}

	const debouncedSearch = debounced(() => void runSearch(), 300);

	function handleInput() {
		debouncedSearch();
	}

	function jump(hit: SearchHit) {
		onJump(hit);
		onClose();
	}

	function handleKeydown(event: KeyboardEvent) {
		if (event.key === "Escape") {
			event.preventDefault();
			event.stopPropagation();
			onClose();
			return;
		}
		if (event.key === "Enter") {
			event.preventDefault();
			debouncedSearch.cancel();
			void runSearch();
			return;
		}
		if (panelEl && cycleFocusWithin(panelEl, event)) return;
	}

	onMount(() => {
		restoreFocusEl =
			document.activeElement instanceof HTMLElement ? document.activeElement : null;
		void tick().then(() => inputEl?.focus());
		return () => {
			debouncedSearch.cancel();
			searchSeq += 1; // invalidate in-flight results
			restoreFocusEl?.focus();
			restoreFocusEl = null;
		};
	});
</script>

<!-- svelte-ignore a11y-click-events-have-key-events -->
<!-- svelte-ignore a11y-no-static-element-interactions -->
<div class="epub-sheet-overlay" class:mobile={isMobile} on:click={onClose}>
	<div
		class="epub-sheet epub-search"
		role="dialog"
		aria-modal="true"
		aria-label="全书搜索"
		tabindex="-1"
		bind:this={panelEl}
		on:click|stopPropagation
		on:keydown={handleKeydown}
	>
		<div class="epub-search-row">
			<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="var(--text-secondary)" stroke-width="2" aria-hidden="true">
				<circle cx="11" cy="11" r="7" />
				<path d="M21 21l-4.35-4.35" />
			</svg>
			<input
				bind:this={inputEl}
				bind:value={query}
				on:input={handleInput}
				placeholder="搜索全书…"
				aria-label="搜索全书"
				class="epub-search-input"
			/>
			{#if searching}
				<span class="epub-search-status">搜索中…</span>
			{:else if hits.length > 0}
				<span class="epub-search-status" aria-live="polite">{hits.length} 条</span>
			{/if}
			<button type="button" class="epub-sheet-close" on:click={onClose} aria-label="关闭搜索">
				<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true">
					<path d="M6 6l12 12M18 6L6 18" />
				</svg>
			</button>
		</div>
		<div class="epub-sheet-body epub-search-results">
			{#if errorMsg}
				<div class="epub-sheet-empty">{errorMsg}</div>
			{:else if searched && hits.length === 0 && query.trim()}
				<div class="epub-sheet-empty">没有找到「{query.trim()}」</div>
			{:else if !searched && !searching}
				<div class="epub-sheet-empty">输入关键词，搜索本书全部章节</div>
			{/if}
			{#each hits as hit (hit.chapterId + hit.snippet)}
				<button type="button" class="epub-hit" on:click={() => jump(hit)}>
					<span class="epub-hit-title">{hit.title || hit.chapterId}</span>
					<!-- 后端返回的 <b> 高亮标记，sanitizeSearchSnippet 白名单放行 -->
					<span class="epub-hit-snippet">{@html sanitizeSearchSnippet(hit.snippet)}</span>
				</button>
			{/each}
		</div>
	</div>
</div>

<style>
	.epub-sheet-overlay {
		position: fixed;
		inset: 0;
		z-index: 1000;
		background: rgba(0, 0, 0, 0.25);
		display: flex;
		justify-content: center;
		align-items: flex-start;
		padding-top: 12vh;
		animation: epubFadeIn 0.15s ease;
	}
	.epub-sheet-overlay.mobile {
		padding-top: 0;
		align-items: flex-end;
	}
	.epub-sheet {
		width: 600px;
		max-width: 92vw;
		max-height: 66vh;
		display: flex;
		flex-direction: column;
		background: var(--bg-secondary);
		border: 1px solid var(--hr);
		border-radius: 16px;
		box-shadow:
			0 16px 48px rgba(0, 0, 0, 0.3),
			inset 0 1px 1px color-mix(in srgb, var(--text) 10%, transparent);
		overflow: hidden;
		animation: epubScaleIn 0.25s cubic-bezier(0.2, 0.8, 0.2, 1);
	}
	.mobile .epub-sheet {
		width: 100vw;
		max-width: 100vw;
		max-height: 82vh;
		border-radius: 20px 20px 0 0;
		animation: epubSlideUp 0.25s cubic-bezier(0.2, 0.8, 0.2, 1);
		padding-bottom: max(12px, env(safe-area-inset-bottom, 0px));
	}

	.epub-search-row {
		display: flex;
		align-items: center;
		gap: 12px;
		height: 54px;
		padding: 0 16px;
		border-bottom: 1px solid var(--hr);
		flex-shrink: 0;
	}
	.epub-search-input {
		flex: 1;
		border: none;
		background: none;
		color: var(--text);
		font-size: 16px;
		outline: none;
		min-width: 0;
	}
	.epub-search-input::placeholder {
		color: var(--text-faded);
	}
	.epub-search-input:focus-visible {
		outline: 2px solid var(--link);
		outline-offset: 4px;
		border-radius: 4px;
	}
	.epub-search-status {
		font-size: 12px;
		color: var(--text-faded);
		white-space: nowrap;
	}
	.epub-sheet-close {
		display: grid;
		place-items: center;
		width: 32px;
		height: 32px;
		border: none;
		border-radius: 8px;
		background: transparent;
		color: var(--text-secondary);
		cursor: pointer;
		flex-shrink: 0;
	}
	.epub-sheet-close:hover {
		background: var(--bg);
		color: var(--text);
	}
	.epub-sheet-close:focus-visible {
		outline: 2px solid var(--link);
		outline-offset: -2px;
	}

	.epub-sheet-body {
		overflow-y: auto;
		padding: 8px;
	}
	.epub-sheet-empty {
		padding: 24px;
		text-align: center;
		color: var(--text-faded);
		font-size: 13px;
	}
	.epub-hit {
		display: flex;
		flex-direction: column;
		gap: 4px;
		width: 100%;
		text-align: left;
		border: none;
		background: transparent;
		padding: 10px 12px;
		border-radius: 8px;
		cursor: pointer;
	}
	.epub-hit:hover {
		background: var(--bg);
	}
	.epub-hit:focus-visible {
		outline: 2px solid var(--link);
		outline-offset: -2px;
	}
	.epub-hit-title {
		font-size: 13px;
		font-weight: 600;
		color: var(--link);
	}
	.epub-hit-snippet {
		font-size: 13px;
		color: var(--text-secondary);
		line-height: 1.55;
		overflow-wrap: break-word;
	}
	.epub-hit-snippet :global(b) {
		color: var(--link);
		font-weight: 600;
	}
	.mobile .epub-hit {
		padding: 12px;
	}
	.mobile .epub-hit-title {
		font-size: 14px;
	}
	.mobile .epub-hit-snippet {
		font-size: 14px;
	}

	@keyframes epubFadeIn {
		from {
			opacity: 0;
		}
		to {
			opacity: 1;
		}
	}
	@keyframes epubScaleIn {
		from {
			opacity: 0;
			transform: scale(0.96);
		}
		to {
			opacity: 1;
			transform: scale(1);
		}
	}
	@keyframes epubSlideUp {
		from {
			transform: translateY(100%);
		}
		to {
			transform: translateY(0);
		}
	}
</style>
