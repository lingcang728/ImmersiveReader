<!--
	Import bottom sheet.

	Owns the SAF picker → `begin_import` → `get_import_status` polling loop
	(400ms) so the shell only has to open the sheet and react to completion.
	Two entry modes:
	  - manual: user taps the shelf FAB, then "选择文件" opens the picker;
	  - intent: the shell passes `initialUris` drained from an Android
	    VIEW/SEND intent and the import starts as soon as the sheet opens.

	Issues are surfaced per file; a completed import with issues keeps the
	sheet open until the user acknowledges it.
-->
<script lang="ts">
	import { onDestroy } from 'svelte';
	import * as api from '$lib/mobile/api';
	import type { ImportStatus } from '$lib/mobile/api';
	import Sheet from './components/Sheet.svelte';
	import Icon from './components/Icon.svelte';
	import {
		IMPORT_POLL_INTERVAL_MS,
		importStatusView,
		visibleImportIssues
	} from './importState';
	import type { ToastKind } from './ui';

	let {
		open,
		initialUris = null,
		onClose,
		onCompleted,
		onToast
	}: {
		open: boolean;
		initialUris?: string[] | null;
		onClose: () => void;
		onCompleted: (status: ImportStatus) => void;
		onToast: (text: string, kind?: ToastKind) => void;
	} = $props();

	type Phase = 'idle' | 'running' | 'completed' | 'failed' | 'cancelled';

	let phase = $state<Phase>('idle');
	let status = $state<ImportStatus | null>(null);
	let operationId = $state<string | null>(null);
	let startError = $state('');
	let cancelling = $state(false);
	let lastUris = $state<string[] | null>(null);

	let pollTimer: ReturnType<typeof setTimeout> | null = null;
	let pollFailures = 0;
	let disposed = false;
	let handoffDone = false;
	let openedFor: string | null = null;

	const view = $derived(importStatusView(status));
	const issueList = $derived(status?.issues ?? []);
	const issueView = $derived(visibleImportIssues(issueList, 5));
	const sheetTitle = $derived(
		phase === 'completed'
			? '导入完成'
			: phase === 'failed'
				? '导入失败'
				: phase === 'cancelled'
					? '已取消导入'
					: phase === 'running'
						? '正在导入'
						: '打开书籍'
	);
	const closeLabel = $derived(phase === 'running' ? '取消导入' : '关闭');

	// ----- lifecycle --------------------------------------------------
	function stopPolling() {
		if (pollTimer === null) return;
		clearTimeout(pollTimer);
		pollTimer = null;
	}

	function resetSession() {
		stopPolling();
		phase = 'idle';
		status = null;
		operationId = null;
		startError = '';
		cancelling = false;
		lastUris = null;
		pollFailures = 0;
		handoffDone = false;
	}

	$effect(() => {
		if (!open) {
			openedFor = null;
			return;
		}
		const intentKey =
			initialUris && initialUris.length > 0 ? `intent:${initialUris.join('\n')}` : 'manual';
		if (openedFor === intentKey) return;
		openedFor = intentKey;
		resetSession();
		if (initialUris && initialUris.length > 0) void startImport(initialUris);
	});

	onDestroy(() => {
		disposed = true;
		stopPolling();
		const opId = operationId;
		if (opId !== null && phase === 'running') {
			// Best effort: the backend worker keeps running otherwise.
			void api.cancelImport(opId).catch(() => {});
		}
	});

	// ----- import pipeline ---------------------------------------------
	async function pickAndImport() {
		if (phase === 'running') return;
		startError = '';
		try {
			const uris = await api.pickBooks();
			if (disposed) return;
			if (uris.length === 0) return;
			await startImport(uris);
		} catch (error) {
			if (disposed) return;
			startError = api.errorText(error);
			phase = 'failed';
		}
	}

	async function startImport(uris: string[]) {
		if (disposed) return;
		stopPolling();
		lastUris = [...uris];
		status = null;
		startError = '';
		operationId = null;
		cancelling = false;
		handoffDone = false;
		pollFailures = 0;
		phase = 'running';
		try {
			const begun = await api.beginUriImport(uris);
			if (disposed) return;
			operationId = begun.operationId;
			schedulePoll(0);
		} catch (error) {
			if (disposed) return;
			startError = api.errorText(error);
			phase = 'failed';
		}
	}

	function schedulePoll(delay: number = IMPORT_POLL_INTERVAL_MS) {
		stopPolling();
		if (disposed) return;
		pollTimer = setTimeout(() => {
			pollTimer = null;
			void pollOnce();
		}, delay);
	}

	async function pollOnce() {
		const opId = operationId;
		if (opId === null || disposed) return;
		try {
			const next = await api.getImportStatus(opId);
			if (disposed) return;
			pollFailures = 0;
			status = next;
			if (api.isTerminalImportState(next.state)) {
				handleTerminal(next);
				return;
			}
			schedulePoll();
		} catch (error) {
			if (disposed) return;
			pollFailures += 1;
			if (pollFailures >= 3) {
				startError = api.errorText(error);
				phase = 'failed';
				return;
			}
			schedulePoll(1000);
		}
	}

	function handleTerminal(next: ImportStatus) {
		stopPolling();
		status = next;
		if (next.state === 'completed') {
			phase = 'completed';
			// Clean success: hand the result to the shell immediately. With
			// issues the sheet stays open so the user sees what was skipped.
			if (next.issues.length === 0 && (next.books?.length ?? 0) <= 1) handOff(next);
			return;
		}
		if (next.state === 'cancelled') {
			phase = 'cancelled';
			return;
		}
		phase = 'failed';
	}

	function handOff(next: ImportStatus) {
		if (handoffDone) return;
		handoffDone = true;
		onCompleted(next);
	}

	async function handleCancel() {
		const opId = operationId;
		if (opId === null || cancelling) return;
		cancelling = true;
		try {
			await api.cancelImport(opId);
		} catch (error) {
			onToast(api.errorText(error), 'error');
		}
		if (disposed) return;
		cancelling = false;
		// Keep polling: the worker observes the cancel flag between files.
		if (phase === 'running') schedulePoll(IMPORT_POLL_INTERVAL_MS);
	}

    function handleClose() {
        if (phase === 'running') { void handleCancel(); return; }
        onClose();
	}

	function finishCompleted() {
		const finalStatus = status;
		if (!finalStatus) {
			onClose();
			return;
		}
		handOff(finalStatus);
	}

	function retry() {
		if (lastUris && lastUris.length > 0) {
			void startImport(lastUris);
			return;
		}
		resetSession();
	}
</script>

{#snippet issueBlock()}
	{#if issueView.shown.length > 0}
		<div class="issue-block">
			<p class="issue-title">{issueList.length} 个文件未导入</p>
			<ul class="issue-list">
				{#each issueView.shown as issue, index (`${issue.path ?? 'issue'}-${index}`)}
					<li class="issue-item">
						<span class="issue-message">{issue.message}</span>
						{#if issue.path}
							<span class="issue-path">{issue.path}</span>
						{/if}
					</li>
				{/each}
			</ul>
			{#if issueView.hiddenCount > 0}
				<p class="issue-more">另有 {issueView.hiddenCount} 个文件未导入</p>
			{/if}
		</div>
	{/if}
{/snippet}

<Sheet
	open={open}
	title={sheetTitle}
	testid="import-sheet"
	closeLabel={closeLabel}
	onClose={handleClose}
>
	{#if phase === 'idle'}
		<div class="import-intro">
			<span class="intro-icon" aria-hidden="true"><Icon name="folder" size={28} /></span>
			<p class="import-copy">选择 EPUB 或 Markdown，自动保存副本到书库并记忆进度。</p>
			<ul class="import-hints">
				<li>支持 EPUB（无 DRM）与 .md / .markdown / .txt</li>
				<li>文件只在本机读取，不会上传</li>
			</ul>
			{#if startError}
				<p class="error-copy" role="alert">{startError}</p>
			{/if}
			<button
				type="button"
				class="primary-btn"
				data-testid="import-pick"
				onclick={() => void pickAndImport()}
			>
				<Icon name="file" size={20} />
				<span>选择文件</span>
			</button>
		</div>
	{:else if phase === 'running'}
		<div class="import-progress">
			<div class="progress-head">
				<span class="phase-label">{view.phaseLabel || '正在开始导入'}</span>
				<span class="files-label">{view.filesLabel || '准备中…'}</span>
			</div>
			<div
				class="progress-track"
				class:indeterminate={view.percent === null}
				role="progressbar"
				aria-label="导入进度"
				aria-valuemin={0}
				aria-valuemax={100}
				aria-valuenow={view.percent ?? undefined}
				aria-valuetext={view.filesLabel || '准备中'}
			>
				{#if view.percent === null}
					<span class="progress-indeterminate"></span>
				{:else}
					<span class="progress-fill" style={`width: ${view.percent}%`}></span>
				{/if}
			</div>
			{#if view.currentFileLabel}
				<p class="current-file" title={view.currentFileLabel}>{view.currentFileLabel}</p>
			{/if}
			{@render issueBlock()}
			<button
				type="button"
				class="outline-btn"
				data-testid="import-cancel"
				disabled={cancelling}
				onclick={() => void handleCancel()}
			>
				{cancelling ? '正在取消…' : '取消导入'}
			</button>
		</div>
	{:else if phase === 'completed'}
		<div class="import-final">
            <p class="import-copy">打开完成 · {status?.books?.length ?? 1} 本书目</p>
            {#each status?.books ?? [] as book (book.bookId)}<p class="current-file">✓ {book.title}</p>{/each}
			{@render issueBlock()}
            {#if issueList.length > 0}<button type="button" class="outline-btn" onclick={retry}>重试</button>{/if}
            <button type="button" class="primary-btn" onclick={finishCompleted}>完成</button>
		</div>
	{:else if phase === 'cancelled'}
		<div class="import-final">
			<p class="import-copy">已取消本次导入，已读取的文件不会重复。</p>
			<div class="final-actions">
				<button type="button" class="outline-btn" onclick={handleClose}>关闭</button>
				<button type="button" class="primary-btn" onclick={retry}>重新导入</button>
			</div>
		</div>
	{:else}
		<div class="import-final">
			<p class="error-copy" role="alert">{startError || view.errorText || '导入失败，请重试'}</p>
			{@render issueBlock()}
			<div class="final-actions">
				<button type="button" class="outline-btn" onclick={handleClose}>关闭</button>
				<button type="button" class="primary-btn" onclick={retry}>重试</button>
			</div>
		</div>
	{/if}
</Sheet>

<style>
	.import-intro,
	.import-progress,
	.import-final {
		display: flex;
		flex-direction: column;
		gap: 12px;
		padding: 4px 0 8px;
	}

	.intro-icon {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		align-self: flex-start;
		width: 52px;
		height: 52px;
		border-radius: 50%;
		background: var(--bg-secondary);
		color: var(--text-secondary);
	}

	.import-copy {
		margin: 0;
		color: var(--text);
		font-size: 15px;
		line-height: 1.6;
	}

	.import-hints {
		margin: 0;
		padding-left: 18px;
		color: var(--text-secondary);
		font-size: 13px;
		line-height: 1.7;
	}

	.progress-head {
		display: flex;
		gap: 8px;
		align-items: baseline;
		justify-content: space-between;
	}

	.phase-label {
		color: var(--text);
		font-size: 15px;
		font-weight: 500;
	}

	.files-label {
		color: var(--text-secondary);
		font-size: 13px;
		font-variant-numeric: tabular-nums;
	}

	.progress-track {
		position: relative;
		height: 6px;
		overflow: hidden;
		border-radius: 999px;
		background: var(--bg-secondary);
	}

	.progress-fill {
		display: block;
		height: 100%;
		border-radius: 999px;
		background: var(--link);
		transition: width var(--ir-duration) var(--ir-ease);
	}

	.progress-indeterminate {
		position: absolute;
		inset: 0 auto 0 0;
		width: 40%;
		border-radius: 999px;
		background: var(--link);
		opacity: 0.7;
		animation: indeterminate 1.2s var(--ir-ease) infinite;
	}

	@keyframes indeterminate {
		from {
			transform: translateX(-100%);
		}

		to {
			transform: translateX(250%);
		}
	}

	.current-file {
		margin: 0;
		overflow: hidden;
		color: var(--text-faded);
		font-size: 12px;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.issue-block {
		padding: 10px 12px;
		border: 1px solid color-mix(in srgb, var(--danger) 32%, var(--hr));
		border-radius: var(--ir-radius-md);
		background: var(--bg-secondary);
	}

	.issue-title {
		margin: 0 0 6px;
		color: var(--danger);
		font-size: 13px;
		font-weight: 500;
	}

	.issue-list {
		margin: 0;
		padding: 0;
		list-style: none;
	}

	.issue-item {
		padding: 4px 0;
		font-size: 12px;
		line-height: 1.5;
	}

	.issue-message {
		display: block;
		color: var(--text-secondary);
	}

	.issue-path {
		display: block;
		overflow: hidden;
		color: var(--text-faded);
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.issue-more {
		margin: 6px 0 0;
		color: var(--text-faded);
		font-size: 12px;
	}

	.error-copy {
		margin: 0;
		color: var(--danger);
		font-size: 14px;
		line-height: 1.6;
		overflow-wrap: anywhere;
	}

	.primary-btn {
		display: inline-flex;
		gap: 8px;
		align-items: center;
		justify-content: center;
		width: 100%;
		min-height: 48px;
		padding: 0 20px;
		border: 0;
		border-radius: var(--ir-radius-md);
		background: var(--link);
		color: var(--on-accent);
		font-size: 15px;
		font-weight: 500;
	}

	.primary-btn:active {
		opacity: 0.88;
	}

	.outline-btn {
		width: 100%;
		min-height: 48px;
		padding: 0 20px;
		border: 1px solid var(--hr);
		border-radius: var(--ir-radius-md);
		background: transparent;
		color: var(--text);
		font-size: 15px;
	}

	.outline-btn:active {
		background: var(--bg-secondary);
	}

	.outline-btn:disabled {
		opacity: 0.6;
	}

	.final-actions {
		display: flex;
		gap: 8px;
	}

	.final-actions > :global(*) {
		flex: 1;
	}
</style>
