<script lang="ts">
	import { tick } from "svelte";
	import { tocOpen } from "$lib/stores/app";
	import type { TocItem } from "$lib/render/markdown";
	import { cycleFocusWithin } from "$lib/a11y/focusTrap";

	export let items: TocItem[] = [];
	export let activeId: string = "";
	export let onJump: (id: string) => void;

	let inputEl: HTMLInputElement | null = null;
	let listEl: HTMLElement | null = null;
	let paletteEl: HTMLElement | null = null;
	let query = "";
	let selectedIndex = 0;
	let wasOpen = false;
	let restoreFocusEl: HTMLElement | null = null;

	// P2-5: cap the rendered rows — TopN books can produce ~5000 headings.
	// A ~200-row sliding window follows the selection; the remainder shows
	// as paging hints instead of mounted DOM.
	const TOC_WINDOW = 200;
	let windowStart = 0;

	$: filtered = query.trim()
		? items.filter((item) =>
				item.text.toLowerCase().includes(query.trim().toLowerCase()),
			)
		: items;

	// On open: clear filter, select current section, focus the input. On
	// close: hand focus back to the control that opened the palette.
	$: if ($tocOpen !== wasOpen) {
		wasOpen = $tocOpen;
		if ($tocOpen) {
			restoreFocusEl =
				document.activeElement instanceof HTMLElement
					? document.activeElement
					: null;
			query = "";
			const activeIdx = items.findIndex((item) => item.id === activeId);
			selectedIndex = activeIdx >= 0 ? activeIdx : 0;
			windowStart = Math.max(
				0,
				Math.min(selectedIndex - 40, items.length - TOC_WINDOW),
			);
			void tick().then(() => {
				inputEl?.focus();
				scrollSelectedIntoView();
			});
		} else {
			restoreFocusEl?.focus();
			restoreFocusEl = null;
		}
	}

	$: if (selectedIndex >= filtered.length) {
		selectedIndex = Math.max(0, filtered.length - 1);
	}

	// Keep the sliding render window containing the selection — re-runs only
	// when the filtered list or the selection changes.
	$: syncTocWindow(filtered.length, selectedIndex);

	$: visibleItems = filtered.slice(windowStart, windowStart + TOC_WINDOW);
	$: hiddenBefore = Math.min(windowStart, filtered.length);
	$: hiddenAfter = Math.max(
		0,
		filtered.length - windowStart - visibleItems.length,
	);

	function syncTocWindow(listLength: number, selection: number) {
		const maxStart = Math.max(0, listLength - TOC_WINDOW);
		if (selection < windowStart) {
			windowStart = selection;
		} else if (selection >= windowStart + TOC_WINDOW) {
			windowStart = selection - TOC_WINDOW + 1;
		}
		windowStart = Math.min(Math.max(windowStart, 0), maxStart);
	}

	function pageWindow(direction: 1 | -1) {
		const maxStart = Math.max(0, filtered.length - TOC_WINDOW);
		windowStart = Math.min(
			Math.max(0, windowStart + direction * TOC_WINDOW),
			maxStart,
		);
		// Anchor the selection inside the new window so it cannot snap back.
		selectedIndex =
			direction > 0
				? windowStart
				: Math.min(windowStart + TOC_WINDOW, filtered.length) - 1;
		scrollSelectedIntoView();
	}

	function scrollSelectedIntoView() {
		void tick().then(() => {
			listEl?.querySelector(".selected")?.scrollIntoView({ block: "nearest" });
		});
	}

	function jump(id: string) {
		$tocOpen = false;
		onJump(id);
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === "Escape") {
			e.preventDefault();
			e.stopPropagation();
			$tocOpen = false;
			return;
		}
		if (paletteEl && cycleFocusWithin(paletteEl, e)) return;
		if (e.key === "ArrowDown") {
			e.preventDefault();
			selectedIndex = Math.min(selectedIndex + 1, filtered.length - 1);
			scrollSelectedIntoView();
		} else if (e.key === "ArrowUp") {
			e.preventDefault();
			selectedIndex = Math.max(selectedIndex - 1, 0);
			scrollSelectedIntoView();
		} else if (e.key === "PageDown") {
			e.preventDefault();
			pageWindow(1);
		} else if (e.key === "PageUp") {
			e.preventDefault();
			pageWindow(-1);
		} else if (e.key === "Enter") {
			e.preventDefault();
			const item = filtered[selectedIndex];
			if (item) jump(item.id);
		}
	}
</script>

{#if $tocOpen && items.length > 0}
	<!-- svelte-ignore a11y-click-events-have-key-events -->
	<!-- svelte-ignore a11y-no-static-element-interactions -->
	<div
		class="toc-overlay"
		on:click={() => ($tocOpen = false)}
		role="presentation"
	>
		<!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
		<div
			id="toc-panel"
			class="toc-palette"
			role="dialog"
			tabindex="-1"
			aria-modal="true"
			aria-label="目录"
			bind:this={paletteEl}
			on:click|stopPropagation
			on:keydown={handleKeydown}
		>
			<div class="toc-input-row">
				<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="var(--text-secondary)" stroke-width="2" aria-hidden="true">
					<path d="M4 6h16M4 12h12M4 18h8" />
				</svg>
				<input
					bind:this={inputEl}
					bind:value={query}
					placeholder="跳转到标题..."
					aria-label="跳转到标题"
					aria-controls="toc-list"
					aria-activedescendant={filtered[selectedIndex]
						? `toc-option-${selectedIndex}`
						: undefined}
					class="toc-input"
				/>
				<span class="toc-count" aria-live="polite">{filtered.length}</span>
			</div>
			<div class="toc-list" id="toc-list" role="listbox" aria-label="章节标题" bind:this={listEl}>
				{#if hiddenBefore > 0}
					<button
						type="button"
						class="toc-overflow"
						on:click={() => pageWindow(-1)}
					>
						还有 {hiddenBefore} 条 · 上一页
					</button>
				{/if}
				{#each visibleItems as item, i (item.id)}
					{@const realIndex = windowStart + i}
					<button
						id="toc-option-{realIndex}"
						class="toc-item toc-level-{item.level}"
						class:selected={realIndex === selectedIndex}
						class:current={item.id === activeId}
						role="option"
						tabindex="-1"
						aria-selected={realIndex === selectedIndex}
						aria-current={item.id === activeId ? "true" : undefined}
						on:click={() => jump(item.id)}
						on:mouseenter={() => (selectedIndex = realIndex)}
					>
						<span class="toc-text">{item.text}</span>
						{#if item.id === activeId}
							<span class="toc-current-dot" aria-hidden="true"></span>
						{/if}
					</button>
				{:else}
					<div class="toc-empty">无匹配标题</div>
				{/each}
				{#if hiddenAfter > 0}
					<button
						type="button"
						class="toc-overflow"
						on:click={() => pageWindow(1)}
					>
						还有 {hiddenAfter} 条 · 下一页
					</button>
				{/if}
			</div>
		</div>
	</div>
{/if}

<style>
	.toc-overlay {
		position: fixed;
		inset: 0;
		z-index: 1000;
		background: rgba(0, 0, 0, 0.25);
		display: flex;
		justify-content: center;
		align-items: flex-start;
		padding-top: 16vh;
		animation: fadeIn 0.15s ease;
	}
	:global(.is-light-theme) .toc-overlay {
		background: rgba(180, 180, 180, 0.1);
	}

	.toc-palette {
		width: 640px;
		max-width: 90vw;
		max-height: 60vh;
		display: flex;
		flex-direction: column;
		background: var(--bg-secondary);
		border: 1px solid var(--hr);
		border-radius: 16px;
		/* 顶部高光从 --text 派生而非固定白色（与 SearchBar 同一处理）。 */
		box-shadow: 0 16px 48px rgba(0, 0, 0, 0.3), inset 0 1px 1px color-mix(in srgb, var(--text) 10%, transparent);
		overflow: hidden;
		animation: scaleIn 0.25s cubic-bezier(0.2, 0.8, 0.2, 1);
	}
	:global(.is-light-theme) .toc-palette {
		box-shadow: 0 16px 48px rgba(0, 0, 0, 0.15);
		/* 浮层底色跟随主题纸面而非纯白——暮光等暖色主题下不突兀。 */
		background: color-mix(in srgb, var(--bg) 92%, transparent);
	}

	.toc-input-row {
		display: flex;
		align-items: center;
		gap: 14px;
		height: 56px;
		padding: 0 20px;
		border-bottom: 1px solid var(--hr);
		flex-shrink: 0;
	}
	.toc-input {
		flex: 1;
		border: none;
		background: none;
		color: var(--text);
		font-size: 17px;
		outline: none;
	}
	.toc-input:focus-visible {
		outline: 2px solid var(--link);
		outline-offset: 4px;
		border-radius: 4px;
	}
	.toc-input::placeholder {
		color: var(--text-faded);
	}
	.toc-count {
		font-size: 13px;
		color: var(--text-faded);
	}

	.toc-list {
		overflow-y: auto;
		padding: 8px;
	}
	.toc-item {
		display: flex;
		align-items: center;
		gap: 8px;
		width: 100%;
		text-align: left;
		border: none;
		background: transparent;
		color: var(--text-secondary);
		font-size: 13.5px;
		padding: 7px 10px;
		border-radius: 8px;
		cursor: pointer;
		line-height: 1.4;
	}
	.toc-item.selected {
		background: var(--bg);
		color: var(--text);
	}
	.toc-item.current {
		color: var(--link);
	}
	.toc-item:focus-visible {
		outline: 2px solid var(--link);
		outline-offset: -2px;
	}
	.toc-text {
		flex: 1;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.toc-current-dot {
		width: 5px;
		height: 5px;
		border-radius: 50%;
		background: var(--link);
		flex-shrink: 0;
	}

	.toc-level-1 { padding-left: 10px; font-weight: 600; }
	.toc-level-2 { padding-left: 26px; }
	.toc-level-3 { padding-left: 42px; font-size: 12.5px; }
	.toc-level-4 { padding-left: 58px; font-size: 12.5px; }
	.toc-level-5 { padding-left: 74px; font-size: 12.5px; }
	.toc-level-6 { padding-left: 90px; font-size: 12.5px; }

	.toc-empty {
		padding: 20px;
		text-align: center;
		color: var(--text-faded);
		font-size: 13px;
	}

	.toc-overflow {
		display: block;
		width: 100%;
		border: none;
		background: transparent;
		color: var(--text-faded);
		font-size: 12px;
		padding: 8px 10px;
		text-align: center;
		cursor: pointer;
		border-radius: 8px;
	}
	.toc-overflow:hover {
		background: var(--bg);
		color: var(--text-secondary);
	}
	.toc-overflow:focus-visible {
		outline: 2px solid var(--link);
		outline-offset: -2px;
	}

	@keyframes fadeIn {
		from { opacity: 0; }
		to { opacity: 1; }
	}
	@keyframes scaleIn {
		from { opacity: 0; transform: scale(0.95); }
		to { opacity: 1; transform: scale(1); }
	}
	@keyframes slideUp {
		from { transform: translateY(100%); }
		to { transform: translateY(0); }
	}

	@media (max-width: 768px) {
		.toc-overlay {
			padding-top: 0;
			align-items: flex-end;
		}
		.toc-palette {
			width: 100vw;
			max-width: 100vw;
			max-height: 80vh;
			border-radius: 20px 20px 0 0;
			animation: slideUp 0.25s cubic-bezier(0.2, 0.8, 0.2, 1);
			padding-bottom: max(16px, env(safe-area-inset-bottom));
		}
		.toc-input-row {
			height: 52px;
			padding: 0 16px;
		}
		.toc-item {
			min-height: 44px;
			padding: 10px 14px;
			font-size: 15px;
		}
	}

	:global(.is-mobile) .toc-overlay {
		padding-top: 0;
		align-items: flex-end;
	}
	:global(.is-mobile) .toc-palette {
		width: 100vw;
		max-width: 100vw;
		max-height: 80vh;
		border-radius: 20px 20px 0 0;
		animation: slideUp 0.25s cubic-bezier(0.2, 0.8, 0.2, 1);
		padding-bottom: max(16px, env(safe-area-inset-bottom));
	}
	:global(.is-mobile) .toc-input-row {
		height: 52px;
		padding: 0 16px;
	}
	:global(.is-mobile) .toc-item {
		min-height: 44px;
		padding: 10px 14px;
		font-size: 15px;
	}
</style>
