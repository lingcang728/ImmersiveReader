<script lang="ts">
	// EPUB 目录浮层 — 桌面居中面板 / 手机底部抽屉。渲染 publication.nav 的
	// 嵌套树；当前章节高亮。
	import { onMount, tick } from "svelte";
	import { cycleFocusWithin } from "$lib/a11y/focusTrap";
	import type { EpubNavItem } from "$lib/epub/types";

	export let items: EpubNavItem[] = [];
	export let activeChapterId = "";
	export let isMobile = false;
	export let onJump: (chapterId: string) => void = () => {};
	export let onClose: () => void = () => {};

	let panelEl: HTMLElement | null = null;
	let listEl: HTMLElement | null = null;
	let restoreFocusEl: HTMLElement | null = null;

	function flatten(items: EpubNavItem[], depth = 0): { item: EpubNavItem; depth: number }[] {
		const out: { item: EpubNavItem; depth: number }[] = [];
		for (const item of items) {
			out.push({ item, depth });
			if (item.children?.length) out.push(...flatten(item.children, depth + 1));
		}
		return out;
	}

	$: flat = flatten(items);

	// 挂载即打开：把当前章节滚进视野；卸载时焦点还给触发按钮。
	onMount(() => {
		restoreFocusEl =
			typeof document !== "undefined" && document.activeElement instanceof HTMLElement
				? document.activeElement
				: null;
		void tick().then(() => {
			listEl?.querySelector(".current")?.scrollIntoView({ block: "center" });
		});
		return () => {
			restoreFocusEl?.focus();
			restoreFocusEl = null;
		};
	});

	function jump(chapterId: string) {
		onJump(chapterId);
		onClose();
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

</script>

<!-- svelte-ignore a11y-click-events-have-key-events -->
<!-- svelte-ignore a11y-no-static-element-interactions -->
<div class="epub-sheet-overlay" class:mobile={isMobile} on:click={onClose}>
	<div
		class="epub-sheet epub-toc"
		role="dialog"
		aria-modal="true"
		aria-label="目录"
		tabindex="-1"
		bind:this={panelEl}
		on:click|stopPropagation
		on:keydown={handleKeydown}
	>
		<div class="epub-sheet-header">
			<span class="epub-sheet-title">目录</span>
			<button type="button" class="epub-sheet-close" on:click={onClose} aria-label="关闭目录">
				<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true">
					<path d="M6 6l12 12M18 6L6 18" />
				</svg>
			</button>
		</div>
		<div class="epub-sheet-body" bind:this={listEl}>
			{#each flat as { item, depth } (item.chapterId + depth + item.title)}
				<button
					type="button"
					class="epub-toc-item"
					class:current={item.chapterId === activeChapterId}
					style="padding-left: {12 + depth * 18}px"
					on:click={() => jump(item.chapterId)}
				>
					<span class="epub-toc-text">{item.title}</span>
					{#if item.chapterId === activeChapterId}
						<span class="epub-toc-dot" aria-hidden="true"></span>
					{/if}
				</button>
			{:else}
				<div class="epub-sheet-empty">本书没有目录</div>
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
		padding: 0 16px;
		border-bottom: 1px solid var(--hr);
		flex-shrink: 0;
	}
	.epub-sheet-title {
		font-size: 14px;
		font-weight: 600;
		color: var(--text);
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
	.epub-toc-item {
		display: flex;
		align-items: center;
		gap: 8px;
		width: 100%;
		text-align: left;
		border: none;
		background: transparent;
		color: var(--text-secondary);
		font-size: 14px;
		padding: 8px 12px;
		border-radius: 8px;
		cursor: pointer;
		line-height: 1.45;
	}
	.epub-toc-item:hover {
		background: var(--bg);
		color: var(--text);
	}
	.epub-toc-item.current {
		color: var(--link);
		font-weight: 600;
	}
	.epub-toc-item:focus-visible {
		outline: 2px solid var(--link);
		outline-offset: -2px;
	}
	.epub-toc-text {
		flex: 1;
		overflow: hidden;
		text-overflow: ellipsis;
		display: -webkit-box;
		-webkit-line-clamp: 2;
		line-clamp: 2;
		-webkit-box-orient: vertical;
	}
	.epub-toc-dot {
		width: 5px;
		height: 5px;
		border-radius: 50%;
		background: var(--link);
		flex-shrink: 0;
	}
	.epub-sheet-empty {
		padding: 24px;
		text-align: center;
		color: var(--text-faded);
		font-size: 13px;
	}
	.mobile .epub-toc-item {
		min-height: 44px;
		font-size: 15px;
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
