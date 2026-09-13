<script lang="ts">
	import { onDestroy } from 'svelte';
	import type { TrashItem } from '$lib/trash/types';
	import BackButton from './BackButton.svelte';

	export let items: readonly TrashItem[] = [];
	export let loading = false;
	export let onClose: () => void;
	export let onRefresh: () => void;
	export let onRestore: (item: TrashItem) => void;
	export let onDelete: (item: TrashItem) => void;

	// P2-39: the parent handlers are fire-and-forget (the invoke promise is
	// discarded upstream), so we cannot await them. Track a per-row pending
	// flag that clears when the trash list refreshes (success removes the row)
	// or after a bounded safety timeout (failure leaves the row in place).
	const PENDING_TIMEOUT_MS = 15000;
	let pendingIds: ReadonlySet<string> = new Set();
	const pendingTimers = new Map<string, ReturnType<typeof setTimeout>>();
	let lastItems = items;

	function markPending(trashId: string) {
		pendingIds = new Set(pendingIds).add(trashId);
		clearTimeout(pendingTimers.get(trashId));
		pendingTimers.set(
			trashId,
			setTimeout(() => clearPending(trashId), PENDING_TIMEOUT_MS)
		);
	}

	function clearPending(trashId: string) {
		if (!pendingIds.has(trashId)) return;
		const next = new Set(pendingIds);
		next.delete(trashId);
		pendingIds = next;
		clearTimeout(pendingTimers.get(trashId));
		pendingTimers.delete(trashId);
	}

	// Any list refresh settles in-flight row actions.
	$: if (items !== lastItems) {
		lastItems = items;
		for (const id of [...pendingIds]) clearPending(id);
	}

	function runItemAction(item: TrashItem, action: (item: TrashItem) => void) {
		if (loading || pendingIds.has(item.trashId)) return;
		markPending(item.trashId);
		action(item);
	}

	onDestroy(() => {
		for (const timer of pendingTimers.values()) clearTimeout(timer);
		pendingTimers.clear();
	});

	const deletedFormatter = new Intl.DateTimeFormat('zh-CN', {
		year: 'numeric',
		month: 'short',
		day: 'numeric',
		hour: '2-digit',
		minute: '2-digit'
	});

	function deletedLabel(value: string): string {
		const date = new Date(value);
		if (Number.isNaN(date.getTime())) return '删除时间未知';
		return deletedFormatter.format(date);
	}
</script>

<section class="trash-workspace" aria-label="回收站">
	<header>
		<div class="header-left">
			<BackButton label="返回书架" onClick={onClose} />
			<div>
				<span>书库</span>
				<h1>回收站</h1>
			</div>
		</div>
		<button type="button" class="refresh" on:click={onRefresh} disabled={loading}>刷新</button>
	</header>

	<div class="trash-body">
		<div class="trash-intro">
			<p>移出书架的内容保留在 Library\.trash；恢复不会覆盖同名目录。</p>
			<span>{items.length} 项受管书目</span>
		</div>

		{#if loading}
			<div class="trash-state" aria-live="polite">正在读取回收站…</div>
		{:else if items.length === 0}
			<div class="trash-state empty">
				<strong>回收站是空的</strong>
				<p>旧版留下但没有 trash-entry.json 的目录不会在这里出现，也不会被自动删除。</p>
			</div>
		{:else}
			<div class="trash-list">
				{#each items as item (item.trashId)}
					<article>
						<div class="trash-copy">
							<span class="book-id">{item.bookId}</span>
							<h2>{item.title}</h2>
							<p>{item.originalRelativePath}</p>
						</div>
						<div class="trash-meta">
							<time datetime={item.deletedAt}>{deletedLabel(item.deletedAt)}</time>
							<span>revision {item.revision}</span>
						</div>
						<div class="trash-actions">
							<button
								type="button"
								class="restore"
								disabled={loading || pendingIds.has(item.trashId)}
								on:click={() => runItemAction(item, onRestore)}>恢复</button
							>
							<button
								type="button"
								class="delete"
								disabled={loading || pendingIds.has(item.trashId)}
								on:click={() => runItemAction(item, onDelete)}>永久删除…</button
							>
						</div>
					</article>
				{/each}
			</div>
		{/if}
	</div>
</section>

<style>
	.trash-workspace {
		position: absolute;
		inset: 0;
		display: grid;
		grid-template-rows: 60px minmax(0, 1fr);
		background: var(--bg);
		color: var(--text);
		z-index: 3;
	}
	header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 0 36px;
		border-bottom: 1px solid var(--hr);
		background: color-mix(in srgb, var(--bg) 94%, var(--link) 6%);
	}
	.header-left {
		display: flex;
		align-items: center;
		gap: 13px;
		min-width: 0;
	}
	.header-left > div {
		display: flex;
		align-items: baseline;
		gap: 8px;
	}
	header span {
		font-size: 11px;
		color: var(--text-faded);
	}
	h1 {
		margin: 0;
		font-size: 16px;
		font-weight: 650;
	}
	button {
		font: inherit;
		cursor: pointer;
	}
	.refresh {
		border: 1px solid var(--hr);
		border-radius: 999px;
		background: var(--bg-secondary);
		color: var(--text-secondary);
		padding: 7px 14px;
	}
	.trash-body {
		overflow: auto;
		padding: 30px 36px 44px;
	}
	.trash-intro {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
		gap: 20px;
		padding-bottom: 16px;
		border-bottom: 1px solid var(--hr);
	}
	.trash-intro p {
		margin: 0;
		font-size: 12px;
		color: var(--text-secondary);
	}
	.trash-intro span {
		font-size: 11px;
		color: var(--text-faded);
	}
	.trash-list article {
		display: grid;
		grid-template-columns: minmax(0, 1fr) auto auto;
		align-items: center;
		gap: 28px;
		min-height: 88px;
		border-bottom: 1px solid var(--hr);
	}
	.trash-copy {
		min-width: 0;
	}
	.book-id {
		font: 10px ui-monospace, "Cascadia Mono", monospace;
		color: var(--link);
	}
	h2 {
		margin: 5px 0 3px;
		font-size: 15px;
		font-weight: 620;
	}
	.trash-copy p {
		margin: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		font-size: 11px;
		color: var(--text-faded);
	}
	.trash-meta {
		display: flex;
		flex-direction: column;
		gap: 4px;
		font-size: 10px;
		color: var(--text-faded);
		text-align: right;
	}
	.trash-actions {
		display: flex;
		gap: 8px;
	}
	.trash-actions button {
		border-radius: 8px;
		padding: 7px 11px;
		font-size: 12px;
	}
	.trash-actions button:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}
	.restore {
		border: 1px solid color-mix(in srgb, var(--link) 55%, var(--hr));
		background: color-mix(in srgb, var(--link) 13%, transparent);
		color: var(--link);
	}
	.delete {
		border: 1px solid rgba(196, 100, 90, 0.4);
		background: transparent;
		color: #d4a099;
	}
	.trash-state {
		padding: 72px 0;
		color: var(--text-secondary);
		text-align: center;
	}
	.trash-state strong {
		display: block;
		margin-bottom: 8px;
		color: var(--heading);
	}
	.trash-state p {
		margin: 0 auto;
		max-width: 58ch;
		font-size: 12px;
		line-height: 1.6;
	}
	@media (max-width: 760px) {
		header,
		.trash-body {
			padding-left: 16px;
			padding-right: 16px;
		}
		.trash-list article {
			grid-template-columns: 1fr;
			gap: 10px;
			padding: 16px 0;
		}
		.trash-meta {
			text-align: left;
		}
	}
</style>
