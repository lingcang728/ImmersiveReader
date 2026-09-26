<script lang="ts">
	// EPUB 书签浮层。列表数据由父级持有（书签与当前 locator 强相关），本
	// 组件只负责渲染与回传操作。
	import { onMount } from "svelte";
	import { cycleFocusWithin } from "$lib/a11y/focusTrap";
	import type { Bookmark, ReaderLocator } from "$lib/epub/types";

	export let bookmarks: Bookmark[] = [];
	export let loading = false;
	export let canAdd = false;
	export let isMobile = false;
	export let onAdd: () => void = () => {};
	export let onJump: (locator: ReaderLocator) => void = () => {};
	export let onRemove: (bookmarkId: string) => void = () => {};
	export let onClose: () => void = () => {};

	let panelEl: HTMLElement | null = null;
	let restoreFocusEl: HTMLElement | null = null;

	function formatTime(iso: string): string {
		const date = new Date(iso);
		if (Number.isNaN(date.getTime())) return "";
		const pad = (n: number) => String(n).padStart(2, "0");
		return `${date.getMonth() + 1}/${date.getDate()} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
	}

	function handleKeydown(event: KeyboardEvent) {
		if (event.key === "Escape") {
			event.preventDefault();
			event.stopPropagation();
			onClose();
			return;
		}
		if (panelEl && cycleFocusWithin(panelEl, event)) return;
	}

	onMount(() => {
		restoreFocusEl =
			document.activeElement instanceof HTMLElement ? document.activeElement : null;
		return () => {
			restoreFocusEl?.focus();
			restoreFocusEl = null;
		};
	});
</script>

<!-- svelte-ignore a11y-click-events-have-key-events -->
<!-- svelte-ignore a11y-no-static-element-interactions -->
<div class="epub-sheet-overlay" class:mobile={isMobile} on:click={onClose}>
	<div
		class="epub-sheet"
		role="dialog"
		aria-modal="true"
		aria-label="书签"
		tabindex="-1"
		bind:this={panelEl}
		on:click|stopPropagation
		on:keydown={handleKeydown}
	>
		<div class="epub-sheet-header">
			<span class="epub-sheet-title">书签</span>
			<div class="epub-sheet-actions">
				<button
					type="button"
					class="epub-add-btn"
					disabled={!canAdd}
					on:click={onAdd}
				>
					收藏当前位置
				</button>
				<button type="button" class="epub-sheet-close" on:click={onClose} aria-label="关闭书签">
					<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true">
						<path d="M6 6l12 12M18 6L6 18" />
					</svg>
				</button>
			</div>
		</div>
		<div class="epub-sheet-body">
			{#if loading}
				<div class="epub-sheet-empty">加载中…</div>
			{:else if bookmarks.length === 0}
				<div class="epub-sheet-empty">还没有书签 — 点击「收藏当前位置」标记阅读进度</div>
			{/if}
			{#each bookmarks as bookmark (bookmark.bookmarkId)}
				<div class="epub-bm-row">
					<button
						type="button"
						class="epub-bm-jump"
						on:click={() => {
							onJump(bookmark.locator);
							onClose();
						}}
					>
						<span class="epub-bm-label">{bookmark.label}</span>
						{#if formatTime(bookmark.createdAt)}
							<span class="epub-bm-time">{formatTime(bookmark.createdAt)}</span>
						{/if}
					</button>
					<button
						type="button"
						class="epub-bm-del"
						on:click={() => onRemove(bookmark.bookmarkId)}
						aria-label="删除书签"
					>
						删除
					</button>
				</div>
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
		padding-top: 14vh;
		animation: epubFadeIn 0.15s ease;
	}
	.epub-sheet-overlay.mobile {
		padding-top: 0;
		align-items: flex-end;
	}
	.epub-sheet {
		width: 560px;
		max-width: 92vw;
		max-height: 62vh;
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
		max-height: 78vh;
		border-radius: 20px 20px 0 0;
		animation: epubSlideUp 0.25s cubic-bezier(0.2, 0.8, 0.2, 1);
		padding-bottom: max(12px, env(safe-area-inset-bottom, 0px));
	}

	.epub-sheet-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		height: 48px;
		padding: 0 12px 0 16px;
		border-bottom: 1px solid var(--hr);
		flex-shrink: 0;
	}
	.epub-sheet-title {
		font-size: 14px;
		font-weight: 600;
		color: var(--text);
	}
	.epub-sheet-actions {
		display: flex;
		align-items: center;
		gap: 8px;
	}
	.epub-add-btn {
		border: 1px solid var(--hr);
		background: transparent;
		color: var(--link);
		font-size: 12.5px;
		padding: 6px 12px;
		border-radius: 8px;
		cursor: pointer;
		white-space: nowrap;
	}
	.epub-add-btn:hover:not(:disabled) {
		background: var(--bg);
	}
	.epub-add-btn:disabled {
		opacity: 0.45;
		cursor: default;
	}
	.epub-add-btn:focus-visible {
		outline: 2px solid var(--link);
		outline-offset: -2px;
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
	.epub-bm-row {
		display: flex;
		align-items: stretch;
		gap: 4px;
		border-radius: 8px;
	}
	.epub-bm-row:hover {
		background: var(--bg);
	}
	.epub-bm-jump {
		flex: 1;
		display: flex;
		flex-direction: column;
		gap: 2px;
		min-width: 0;
		text-align: left;
		border: none;
		background: transparent;
		padding: 10px 12px;
		cursor: pointer;
		border-radius: 8px;
	}
	.epub-bm-jump:focus-visible {
		outline: 2px solid var(--link);
		outline-offset: -2px;
	}
	.epub-bm-label {
		font-size: 13.5px;
		color: var(--text);
		line-height: 1.45;
		overflow: hidden;
		display: -webkit-box;
		-webkit-line-clamp: 2;
		line-clamp: 2;
		-webkit-box-orient: vertical;
	}
	.epub-bm-time {
		font-size: 11.5px;
		color: var(--text-faded);
	}
	.epub-bm-del {
		border: none;
		background: transparent;
		color: var(--text-faded);
		font-size: 12px;
		padding: 0 12px;
		cursor: pointer;
		border-radius: 8px;
		flex-shrink: 0;
	}
	.epub-bm-del:hover {
		color: var(--text);
	}
	.epub-bm-del:focus-visible {
		outline: 2px solid var(--link);
		outline-offset: -2px;
	}
	.mobile .epub-bm-jump {
		padding: 12px;
	}
	.mobile .epub-bm-label {
		font-size: 14.5px;
	}
	.mobile .epub-bm-del {
		min-width: 48px;
		font-size: 13px;
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
