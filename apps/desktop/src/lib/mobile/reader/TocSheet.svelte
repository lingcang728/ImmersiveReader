<!--
	Table-of-contents sheet.

	Flat chapter lists (Markdown books) and multi-level EPUB nav share this
	view; `level` drives the indentation. Selection closes the sheet via the
	parent callback.
-->
<script lang="ts">
	import { tick } from 'svelte';
	import SheetBase from './SheetBase.svelte';
	import type { TocEntryView } from './types';

	interface Props {
		open: boolean;
		items: TocEntryView[];
		activeId: string;
		onSelect: (id: string) => void;
		onClose: () => void;
	}

	let { open, items, activeId, onSelect, onClose }: Props = $props();

	let listEl = $state<HTMLElement | null>(null);

	function indentFor(level: number): number {
		return 12 + Math.max(0, Math.min(3, level - 1)) * 16;
	}

	$effect(() => {
		const isOpen = open;
		const current = activeId;
		if (!isOpen) return;
		void tick().then(() => {
			if (current !== activeId) return;
			const active = listEl?.querySelector<HTMLElement>('.toc-item.active');
			active?.scrollIntoView({ block: 'center' });
		});
	});
</script>

<SheetBase {open} testid="toc-sheet" title="目录" height="80" {onClose}>
	<div class="toc-list" bind:this={listEl}>
		{#each items as item, index (item.id + ':' + index)}
			<button
				type="button"
				class="toc-item"
				class:active={item.id === activeId}
				data-testid="toc-item-{index}"
				style:padding-left="{indentFor(item.level)}px"
				onclick={() => onSelect(item.id)}
			>
				<span class="toc-label">{item.label}</span>
				{#if item.id === activeId}
					<span class="toc-badge">当前</span>
				{/if}
			</button>
		{/each}
		{#if items.length === 0}
			<p class="toc-empty">暂无目录</p>
		{/if}
	</div>
</SheetBase>

<style>
	.toc-list {
		display: flex;
		flex-direction: column;
		gap: 2px;
		padding-bottom: 8px;
	}

	.toc-item {
		display: flex;
		align-items: center;
		gap: 10px;
		width: 100%;
		min-height: 48px;
		padding: 8px 14px;
		border: 0;
		border-radius: 10px;
		background: transparent;
		color: var(--text);
		text-align: left;
		font-size: 15px;
		line-height: 1.4;
		-webkit-tap-highlight-color: transparent;
	}

	.toc-item:active {
		background: var(--bg-secondary);
	}

	.toc-item.active {
		color: var(--link);
		background: var(--bg-secondary);
		font-weight: 600;
	}

	.toc-label {
		flex: 1;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.toc-badge {
		flex: 0 0 auto;
		font-size: 11.5px;
		font-weight: 400;
		color: var(--link);
		border: 1px solid color-mix(in srgb, var(--link) 45%, transparent);
		border-radius: 999px;
		padding: 1px 8px;
	}

	.toc-empty {
		margin: 24px 0;
		text-align: center;
		color: var(--text-faded);
		font-size: 14px;
	}
</style>
