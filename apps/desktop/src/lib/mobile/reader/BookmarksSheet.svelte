<!--
	EPUB bookmark sheet: list, jump, delete, and "bookmark this position".
	Markdown chapters have no bookmark surface (desktop parity: bookmarks are
	an EPUB reader feature).
-->
<script lang="ts">
	import SheetBase from './SheetBase.svelte';
	import type { Bookmark } from '$lib/epub/types';

	interface Props {
		open: boolean;
		bookmarks: Bookmark[];
		loading: boolean;
		canAdd: boolean;
		onAdd: () => void;
		onJump: (bookmark: Bookmark) => void;
		onRemove: (bookmarkId: string) => void;
		onClose: () => void;
	}

	let { open, bookmarks, loading, canAdd, onAdd, onJump, onRemove, onClose }: Props = $props();

	function formatTime(value: string): string {
		const date = new Date(value);
		if (Number.isNaN(date.getTime())) return '';
		const month = `${date.getMonth() + 1}`.padStart(2, '0');
		const day = `${date.getDate()}`.padStart(2, '0');
		const hours = `${date.getHours()}`.padStart(2, '0');
		const minutes = `${date.getMinutes()}`.padStart(2, '0');
		return `${month}-${day} ${hours}:${minutes}`;
	}
</script>

<SheetBase {open} testid="bookmarks-sheet" title="书签" height="80" {onClose}>
	{#if loading}
		<p class="bookmarks-hint">加载中…</p>
	{:else if bookmarks.length === 0}
		<p class="bookmarks-hint">还没有书签。阅读时点击「添加书签」保存当前位置。</p>
	{:else}
		<div class="bookmarks-list">
			{#each bookmarks as bookmark (bookmark.bookmarkId)}
				<div class="bookmark-row">
					<button type="button" class="bookmark-jump" onclick={() => onJump(bookmark)}>
						<span class="bookmark-label">{bookmark.label || '未命名位置'}</span>
						<span class="bookmark-meta">{formatTime(bookmark.createdAt)}</span>
					</button>
					<button
						type="button"
						class="bookmark-remove"
						aria-label="删除书签"
						onclick={() => onRemove(bookmark.bookmarkId)}
					>
						<svg viewBox="0 0 24 24" aria-hidden="true">
							<path d="M5 7h14M10 7V5h4v2M8 7l1 13h6l1-13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
						</svg>
					</button>
				</div>
			{/each}
		</div>
	{/if}
	<button type="button" class="bookmark-add" disabled={!canAdd} onclick={onAdd}>
		添加当前页书签
	</button>
</SheetBase>

<style>
	.bookmarks-list {
		display: flex;
		flex-direction: column;
		gap: 4px;
	}

	.bookmark-row {
		display: flex;
		align-items: stretch;
		gap: 4px;
		border: 1px solid var(--hr);
		border-radius: 12px;
		overflow: hidden;
	}

	.bookmark-jump {
		flex: 1;
		display: flex;
		flex-direction: column;
		gap: 2px;
		min-height: 52px;
		padding: 8px 12px;
		border: 0;
		background: transparent;
		color: var(--text);
		text-align: left;
		font-size: 14px;
		line-height: 1.45;
	}

	.bookmark-label {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.bookmark-meta {
		font-size: 11.5px;
		color: var(--text-faded);
	}

	.bookmark-remove {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 48px;
		border: 0;
		border-left: 1px solid var(--hr);
		background: transparent;
		color: var(--text-faded);
	}

	.bookmark-remove:active {
		color: var(--danger);
		background: var(--bg-secondary);
	}

	.bookmark-remove svg {
		width: 18px;
		height: 18px;
	}

	.bookmark-add {
		width: 100%;
		min-height: 44px;
		margin-top: 14px;
		border: 1px solid var(--link);
		border-radius: 12px;
		background: transparent;
		color: var(--link);
		font-size: 14.5px;
	}

	.bookmark-add:disabled {
		border-color: var(--hr);
		color: var(--text-faded);
	}

	.bookmarks-hint {
		margin: 20px 0;
		text-align: center;
		color: var(--text-faded);
		font-size: 14px;
		line-height: 1.7;
	}
</style>
