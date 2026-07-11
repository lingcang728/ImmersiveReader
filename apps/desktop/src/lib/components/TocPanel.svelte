<script lang="ts">
	import { tick } from "svelte";
	import { tocOpen } from "$lib/stores/app";
	import type { TocItem } from "$lib/render/markdown";

	export let items: TocItem[] = [];
	export let activeId: string = "";
	export let onJump: (id: string) => void;

	let inputEl: HTMLInputElement | null = null;
	let listEl: HTMLElement | null = null;
	let query = "";
	let selectedIndex = 0;
	let wasOpen = false;

	$: filtered = query.trim()
		? items.filter((item) =>
				item.text.toLowerCase().includes(query.trim().toLowerCase()),
			)
		: items;

	// On open: clear filter, select current section, focus the input.
	$: if ($tocOpen !== wasOpen) {
		wasOpen = $tocOpen;
		if ($tocOpen) {
			query = "";
			const activeIdx = items.findIndex((item) => item.id === activeId);
			selectedIndex = activeIdx >= 0 ? activeIdx : 0;
			void tick().then(() => {
				inputEl?.focus();
				scrollSelectedIntoView();
			});
		}
	}

	$: if (selectedIndex >= filtered.length) {
		selectedIndex = Math.max(0, filtered.length - 1);
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
		if (e.key === "ArrowDown") {
			e.preventDefault();
			selectedIndex = Math.min(selectedIndex + 1, filtered.length - 1);
			scrollSelectedIntoView();
		} else if (e.key === "ArrowUp") {
			e.preventDefault();
			selectedIndex = Math.max(selectedIndex - 1, 0);
			scrollSelectedIntoView();
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
		<div class="toc-palette" on:click|stopPropagation role="presentation">
			<div class="toc-input-row">
				<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="var(--text-secondary)" stroke-width="2">
					<path d="M4 6h16M4 12h12M4 18h8" />
				</svg>
				<input
					bind:this={inputEl}
					bind:value={query}
					on:keydown={handleKeydown}
					placeholder="跳转到标题..."
					class="toc-input"
				/>
				<span class="toc-count">{filtered.length}</span>
			</div>
			<div class="toc-list" bind:this={listEl}>
				{#each filtered as item, i (item.id)}
					<button
						class="toc-item toc-level-{item.level}"
						class:selected={i === selectedIndex}
						class:current={item.id === activeId}
						on:click={() => jump(item.id)}
						on:mousemove={() => (selectedIndex = i)}
					>
						<span class="toc-text">{item.text}</span>
						{#if item.id === activeId}
							<span class="toc-current-dot"></span>
						{/if}
					</button>
				{:else}
					<div class="toc-empty">无匹配标题</div>
				{/each}
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
		box-shadow: 0 16px 48px rgba(0, 0, 0, 0.3), inset 0 1px 1px rgba(255, 255, 255, 0.1);
		overflow: hidden;
		animation: scaleIn 0.25s cubic-bezier(0.2, 0.8, 0.2, 1);
	}
	:global(.is-light-theme) .toc-palette {
		box-shadow: 0 16px 48px rgba(0, 0, 0, 0.15), inset 0 1px 1px rgba(255, 255, 255, 0.8);
		background: rgba(255, 255, 255, 0.92);
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

	@keyframes fadeIn {
		from { opacity: 0; }
		to { opacity: 1; }
	}
	@keyframes scaleIn {
		from { opacity: 0; transform: scale(0.95); }
		to { opacity: 1; transform: scale(1); }
	}
</style>
