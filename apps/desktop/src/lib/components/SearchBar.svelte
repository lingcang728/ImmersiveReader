<script lang="ts">
	import { tick } from "svelte";
	import { searchOpen, searchQuery } from "$lib/stores/app";

	export let matchCount = 0;
	export let currentIndex = 0;
	export let onInput: () => void;
	export let onNavigate: (direction: 1 | -1) => void;
	export let onClose: () => void;

	let inputEl: HTMLInputElement;

	// 打开即聚焦（顶栏按钮与 mod+F 快捷键只负责切换 store）
	$: if ($searchOpen) {
		void tick().then(() => inputEl?.focus());
	}

	function close() {
		$searchOpen = false;
		$searchQuery = "";
		onClose();
	}
</script>

{#if $searchOpen}
	<!-- svelte-ignore a11y-click-events-have-key-events -->
	<!-- svelte-ignore a11y-no-static-element-interactions -->
	<div class="search-overlay" class:has-results={matchCount > 0} on:click={close}>
		<div class="mac-search-bar" on:click|stopPropagation>
			<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="var(--text-secondary)" stroke-width="2">
				<circle cx="11" cy="11" r="7" />
				<path d="M21 21l-4.35-4.35" />
			</svg>
			<input
				bind:this={inputEl}
				bind:value={$searchQuery}
				on:input={onInput}
				placeholder="搜索..."
				class="search-input"
			/>
			{#if matchCount > 0}
				<span class="search-count">{currentIndex + 1} / {matchCount}</span>
			{/if}
			<button class="search-nav" on:click={() => onNavigate(-1)} title="上一条结果">
				<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M18 15l-6-6-6 6" /></svg>
			</button>
			<button class="search-nav" on:click={() => onNavigate(1)} title="下一条结果">
				<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M6 9l6 6 6-6" /></svg>
			</button>
		</div>
	</div>
{/if}

<style>
	.search-overlay {
		position: fixed;
		inset: 0;
		z-index: 1000;
		background: rgba(0, 0, 0, 0.25);
		backdrop-filter: blur(12px);
		-webkit-backdrop-filter: blur(12px);
		display: flex;
		justify-content: center;
		padding-top: 20vh;
		animation: fadeIn 0.15s ease;
	}
	.search-overlay.has-results {
		background: transparent;
		backdrop-filter: none;
		-webkit-backdrop-filter: none;
	}
	:global(.is-light-theme) .search-overlay {
		background: rgba(180, 180, 180, 0.1);
	}
	:global(.is-light-theme) .search-overlay.has-results {
		background: transparent;
	}

	.mac-search-bar {
		width: 640px;
		max-width: 90vw;
		height: 64px;
		background: var(--bg-secondary);
		border: 1px solid var(--hr);
		border-radius: 16px;
		box-shadow: 0 16px 48px rgba(0, 0, 0, 0.3), inset 0 1px 1px rgba(255, 255, 255, 0.1);
		display: flex;
		align-items: center;
		padding: 0 20px;
		gap: 16px;
		animation: scaleIn 0.25s cubic-bezier(0.2, 0.8, 0.2, 1);
	}
	:global(.is-light-theme) .mac-search-bar {
		box-shadow: 0 16px 48px rgba(0, 0, 0, 0.15), inset 0 1px 1px rgba(255, 255, 255, 0.8);
		background: rgba(255, 255, 255, 0.8);
	}

	.search-input {
		flex: 1;
		border: none;
		background: none;
		color: var(--text);
		font-size: 20px;
		outline: none;
	}
	.search-input::placeholder {
		color: var(--text-faded);
	}
	.search-count {
		font-size: 14px;
		color: var(--text-faded);
		white-space: nowrap;
		margin-right: 8px;
	}
	.search-nav {
		display: flex;
		align-items: center;
		justify-content: center;
		width: 32px;
		height: 32px;
		border: 1px solid transparent;
		background: transparent;
		color: var(--text-secondary);
		cursor: pointer;
		border-radius: 8px;
		transition: all 0.3s cubic-bezier(0.2, 0.8, 0.2, 1);
		position: relative;
		overflow: hidden;
	}
	.search-nav::after {
		content: ''; position: absolute; inset: 0;
		background: linear-gradient(135deg, rgba(255, 255, 255, 0.4) 0%, rgba(255, 255, 255, 0) 50%, rgba(255, 255, 255, 0.1) 100%);
		opacity: 0; transition: opacity 0.3s ease;
		pointer-events: none;
	}
	.search-nav:hover {
		background: var(--bg);
		border-color: var(--hr);
		color: var(--text);
		transform: translateY(-1px);
		box-shadow: 0 4px 12px rgba(0, 0, 0, 0.1);
	}
	.search-nav:hover::after {
		opacity: 1;
	}
	.search-nav:active {
		transform: translateY(0);
		box-shadow: 0 1px 2px rgba(0, 0, 0, 0.05);
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
