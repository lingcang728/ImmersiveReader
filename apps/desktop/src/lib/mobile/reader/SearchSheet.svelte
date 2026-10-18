<!--
	Search sheet — one input, two modes:
	  - markdown: in-chapter search (match count + prev/next, highlights live
	    in the article via `--search-highlight`)
	  - epub: full-book `search_book` results (snippet rows with <b> marks,
	    tap jumps to the locator)
-->
<script lang="ts">
	import { tick } from 'svelte';
	import SheetBase from './SheetBase.svelte';
	import type { SearchResultView } from './types';

	interface Props {
		open: boolean;
		mode: 'markdown' | 'epub';
		query: string;
		busy: boolean;
		statusText: string;
		results: SearchResultView[];
		activeIndex: number;
		truncated?: boolean;
		onQueryChange: (query: string) => void;
		onPrev: () => void;
		onNext: () => void;
		onSelect: (index: number) => void;
		onClose: () => void;
	}

	let {
		open,
		mode,
		query,
		busy,
		statusText,
		results,
		activeIndex,
		truncated = false,
		onQueryChange,
		onPrev,
		onNext,
		onSelect,
		onClose
	}: Props = $props();

	let inputEl = $state<HTMLInputElement | null>(null);

	$effect(() => {
		const isOpen = open;
		if (!isOpen) return;
		void tick().then(() => {
			inputEl?.focus({ preventScroll: true });
		});
	});

	function handleKeydown(event: KeyboardEvent) {
		if (event.key === 'Enter') {
			event.preventDefault();
			if (event.shiftKey) onPrev();
			else onNext();
		}
	}
</script>

<SheetBase {open} testid="search-sheet" title="搜索" height="full" {onClose}>
	<div class="search-head">
		<input
			bind:this={inputEl}
			class="search-input"
			type="search"
			placeholder={mode === 'epub' ? '搜索全书' : '搜索本章'}
			value={query}
			oninput={(event) => onQueryChange((event.currentTarget as HTMLInputElement).value)}
			onkeydown={handleKeydown}
			aria-label={mode === 'epub' ? '搜索全书' : '搜索本章'}
			autocomplete="off"
			autocapitalize="off"
			spellcheck="false"
		/>
		{#if mode === 'markdown'}
			<div class="search-nav">
				<button type="button" class="search-nav-btn" onclick={onPrev} aria-label="上一个匹配" disabled={busy}>上一个</button>
				<span class="search-status" data-testid="search-status">{statusText}</span>
				<button type="button" class="search-nav-btn" onclick={onNext} aria-label="下一个匹配" disabled={busy}>下一个</button>
			</div>
		{:else}
			<div class="search-nav">
				<span class="search-status" data-testid="search-status">{busy ? '搜索中…' : statusText}</span>
				<button type="button" class="search-nav-btn" onclick={onPrev} aria-label="上一条结果" disabled={results.length === 0}>上一条</button>
				<button type="button" class="search-nav-btn" onclick={onNext} aria-label="下一条结果" disabled={results.length === 0}>下一条</button>
			</div>
		{/if}
	</div>

	{#if mode === 'epub'}
		<div class="search-results">
			{#each results as result, index (result.id + ':' + index)}
				<button
					type="button"
					class="search-result"
					class:active={index === activeIndex}
					data-testid="search-result-{index}"
					onclick={() => onSelect(index)}
				>
					<span class="search-result-title">{result.title}</span>
					{#if result.snippetHtml}
						<span class="search-result-snippet">{@html result.snippetHtml}</span>
					{/if}
				</button>
			{/each}
			{#if results.length === 0 && !busy && query.trim()}
				<p class="search-empty">没有找到匹配内容</p>
			{/if}
		</div>
	{:else}
		{#if truncated}
			<p class="search-hint">匹配过多，仅高亮前 300 处</p>
		{/if}
		{#if !query.trim()}
			<p class="search-hint">输入关键词，在当前章节中查找</p>
		{:else if !busy && statusText.startsWith('没有')}
			<p class="search-empty">没有找到匹配内容</p>
		{/if}
	{/if}
</SheetBase>

<style>
	.search-head {
		display: flex;
		flex-direction: column;
		gap: 10px;
		padding-bottom: 12px;
		position: sticky;
		top: 0;
		background: var(--bg);
		z-index: 1;
	}

	.search-input {
		width: 100%;
		height: 44px;
		padding: 0 14px;
		border: 1px solid var(--hr);
		border-radius: 12px;
		background: var(--bg-secondary);
		color: var(--text);
		font-size: 16px;
		outline: none;
	}

	.search-input:focus {
		border-color: var(--link);
		box-shadow: 0 0 0 2px color-mix(in srgb, var(--link) 22%, transparent);
	}

	.search-nav {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
	}

	.search-nav-btn {
		min-height: 44px;
		padding: 0 16px;
		border: 1px solid var(--hr);
		border-radius: 12px;
		background: transparent;
		color: var(--link);
		font-size: 14px;
	}

	.search-nav-btn:disabled {
		color: var(--text-faded);
		opacity: 0.6;
	}

	.search-status {
		flex: 1;
		text-align: center;
		font-size: 12.5px;
		color: var(--text-secondary);
		font-variant-numeric: tabular-nums;
	}

	.search-results {
		display: flex;
		flex-direction: column;
		gap: 6px;
	}

	.search-result {
		display: flex;
		flex-direction: column;
		gap: 4px;
		width: 100%;
		padding: 10px 12px;
		border: 1px solid var(--hr);
		border-radius: 12px;
		background: transparent;
		color: var(--text);
		text-align: left;
		font-size: 14px;
		line-height: 1.55;
	}

	.search-result.active {
		border-color: var(--link);
		background: var(--bg-secondary);
	}

	.search-result-title {
		font-size: 12.5px;
		color: var(--link);
	}

	.search-result-snippet :global(b) {
		color: var(--text);
		background: var(--search-highlight);
		border-radius: 2px;
		font-weight: 600;
	}

	.search-result-snippet {
		color: var(--text-secondary);
		overflow: hidden;
		display: -webkit-box;
		-webkit-line-clamp: 3;
		line-clamp: 3;
		-webkit-box-orient: vertical;
	}

	.search-empty,
	.search-hint {
		margin: 28px 0;
		text-align: center;
		color: var(--text-faded);
		font-size: 14px;
	}
</style>
