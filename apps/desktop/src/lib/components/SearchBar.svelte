<script lang="ts">
	import { tick } from "svelte";
	import { searchOpen, searchQuery } from "$lib/stores/app";
	import { cycleFocusWithin } from "$lib/a11y/focusTrap";

	export let matchCount = 0;
	export let currentIndex = 0;
	export let onInput: () => void;
	export let onNavigate: (direction: 1 | -1) => void;
	export let onClose: () => void;

	let inputEl: HTMLInputElement;
	let barEl: HTMLElement;
	let wasOpen = false;
	let restoreFocusEl: HTMLElement | null = null;

	// 打开即聚焦（顶栏按钮与 mod+F 快捷键只负责切换 store）；关闭时把焦点
	// 还给触发控件，键盘用户不会丢回 <body>。
	$: if ($searchOpen !== wasOpen) {
		wasOpen = $searchOpen;
		if ($searchOpen) {
			restoreFocusEl =
				document.activeElement instanceof HTMLElement
					? document.activeElement
					: null;
			void tick().then(() => inputEl?.focus());
		} else {
			restoreFocusEl?.focus();
			restoreFocusEl = null;
		}
	}

	function close() {
		$searchOpen = false;
		$searchQuery = "";
		onClose();
	}

	function handleKeydown(event: KeyboardEvent) {
		if (event.key === "Escape") {
			event.preventDefault();
			event.stopPropagation();
			close();
			return;
		}
		if (barEl) cycleFocusWithin(barEl, event);
	}
</script>

{#if $searchOpen}
	<!-- svelte-ignore a11y-click-events-have-key-events -->
	<!-- svelte-ignore a11y-no-static-element-interactions -->
	<div class="search-overlay" class:has-results={matchCount > 0} on:click={close}>
		<div
			id="search-panel"
			class="mac-search-bar"
			role="dialog"
			tabindex="-1"
			aria-modal="true"
			aria-label="搜索正文"
			bind:this={barEl}
			on:click|stopPropagation
			on:keydown={handleKeydown}
		>
			<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="var(--text-secondary)" stroke-width="2" aria-hidden="true">
				<circle cx="11" cy="11" r="7" />
				<path d="M21 21l-4.35-4.35" />
			</svg>
			<input
				bind:this={inputEl}
				bind:value={$searchQuery}
				on:input={onInput}
				placeholder="搜索..."
				aria-label="搜索正文"
				class="search-input"
			/>
			{#if matchCount > 0}
				<span class="search-count">{currentIndex + 1} / {matchCount}</span>
			{/if}
			<span class="visually-hidden" role="status" aria-live="polite">
				{$searchQuery.trim()
					? matchCount > 0
						? `共 ${matchCount} 条结果，当前第 ${currentIndex + 1} 条`
						: "无匹配结果"
					: ""}
			</span>
			<button class="search-nav" on:click={() => onNavigate(-1)} title="上一条结果" aria-label="上一条结果">
				<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="M18 15l-6-6-6 6" /></svg>
			</button>
			<button class="search-nav" on:click={() => onNavigate(1)} title="下一条结果" aria-label="下一条结果">
				<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="M6 9l6 6 6-6" /></svg>
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
		/* 顶部高光从 --text 派生而非固定白色：暗主题下 --text 是浅色，效果一致；
		   暖色/非常规主题也不会残留纯白边缘。 */
		box-shadow: 0 16px 48px rgba(0, 0, 0, 0.3), inset 0 1px 1px color-mix(in srgb, var(--text) 10%, transparent);
		display: flex;
		align-items: center;
		padding: 0 20px;
		gap: 16px;
		animation: scaleIn 0.25s cubic-bezier(0.2, 0.8, 0.2, 1);
	}
	:global(.is-light-theme) .mac-search-bar {
		/* 浅色纸面上的白色内高光本来就不可见，直接省略。 */
		box-shadow: 0 16px 48px rgba(0, 0, 0, 0.15);
		/* 浮层底色跟随主题纸面而非纯白——暮光等暖色主题下不突兀。 */
		background: color-mix(in srgb, var(--bg) 88%, transparent);
	}

	.search-input {
		flex: 1;
		border: none;
		background: none;
		color: var(--text);
		font-size: 20px;
		outline: none;
	}
	.search-input:focus-visible {
		outline: 2px solid var(--link);
		outline-offset: 4px;
		border-radius: 4px;
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
		/* 与 .icon-btn::after 同一套主题派生 sheen——纯白渐变在暗主题下是奶雾。 */
		background: linear-gradient(135deg, color-mix(in srgb, var(--text) 10%, transparent) 0%, transparent 50%, color-mix(in srgb, var(--text) 4%, transparent) 100%);
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
	.search-nav:focus-visible {
		outline: 2px solid var(--link);
		outline-offset: 2px;
	}
	.search-nav:active {
		transform: translateY(0);
		box-shadow: 0 1px 2px rgba(0, 0, 0, 0.05);
	}

	.visually-hidden {
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

	@keyframes fadeIn {
		from { opacity: 0; }
		to { opacity: 1; }
	}
	@keyframes scaleIn {
		from { opacity: 0; transform: scale(0.95); }
		to { opacity: 1; transform: scale(1); }
	}
</style>
