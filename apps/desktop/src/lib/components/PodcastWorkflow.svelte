<script lang="ts">
	import { onMount } from 'svelte';
	import { invoke } from '@tauri-apps/api/core';
	import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
	import { open } from '@tauri-apps/plugin-dialog';
	import type { TaskSnapshot } from '$lib/tasks/sync';
	import WorkflowDialogShell from './WorkflowDialogShell.svelte';

	type DuplicatePolicy = 'reuse_existing' | 'new_revision';

	interface PodcastFilePreview {
		path: string;
		fileName: string;
		bytes: number;
		durationSeconds: number;
		duplicateBookId?: string;
	}

	interface PodcastBudgetPreview {
		estimatedDiskBytes: number;
		estimatedTranslationTokens: number;
		estimatedApiCostUpperCny: number;
		availableDiskBytes: number;
		estimateVersion: string;
		confirmationRequired: boolean;
	}

	interface PodcastFilesPreview {
		previewId: string;
		files: PodcastFilePreview[];
		budget: PodcastBudgetPreview;
	}

	interface PodcastBudgetApproval {
		estimatedDiskBytes: number;
		estimatedApiCostUpperCny: number;
	}

	interface PodcastAddResult {
		tasks: TaskSnapshot[];
		existingBooks: string[];
	}

	export let tasks: readonly TaskSnapshot[] = [];
	export let onClose: () => void;
	export let onRefreshTasks: () => void;
	export let onStartTask: (taskId: string) => void;
	export let onOpenResult: (taskId: string) => void;

	let selectedPaths: string[] = [];
	let translate = true;
	let polish = true;
	let maxApiCostCny = 0;
	let duplicatePolicy: DuplicatePolicy = 'reuse_existing';
	let preview: PodcastFilesPreview | null = null;
	let budgetConfirmed = false;
	let createdTaskIds: string[] = [];
	let existingBooks: string[] = [];
	let busy = false;
	let errorText = '';
	let noticeText = '';
	let dragActive = false;

	$: approvalRequired = preview?.budget.confirmationRequired ?? false;
	$: canAdd = preview !== null && (!approvalRequired || budgetConfirmed) && !busy;
	$: createdTasks = createdTaskIds
		.map((taskId) => tasks.find((task) => task.id === taskId))
		.filter((task): task is TaskSnapshot => task !== undefined);

	function resetPreview() {
		preview = null;
		budgetConfirmed = false;
		createdTaskIds = [];
		existingBooks = [];
		noticeText = '';
	}

	function setPaths(paths: string[]) {
		const supported = paths.filter((path) => /\.(mp3|m4a|wav)$/i.test(path));
		selectedPaths = [...new Set(supported)];
		resetPreview();
		if (paths.length > 0 && supported.length === 0) {
			errorText = '只支持 MP3、M4A 和 WAV 音频文件。';
		} else {
			errorText = '';
		}
	}

	function removePath(path: string) {
		setPaths(selectedPaths.filter((item) => item !== path));
	}

	function fileName(path: string): string {
		return path.split(/[\\/]/).pop() ?? path;
	}

	function formatBytes(bytes: number): string {
		if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
		if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
		return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
	}

	function formatDuration(seconds: number): string {
		const total = Math.max(0, Math.round(seconds));
		const hours = Math.floor(total / 3600);
		const minutes = Math.floor((total % 3600) / 60);
		const rest = total % 60;
		return hours > 0
			? `${hours} 小时 ${minutes} 分钟`
			: `${minutes} 分 ${rest.toString().padStart(2, '0')} 秒`;
	}

	function taskStateLabel(task: TaskSnapshot): string {
		if (task.lifecycleState === 'queued') return '等待开始';
		if (task.lifecycleState === 'paused') return '已暂停';
		if (task.lifecycleState === 'terminal') {
			return task.outcome === 'success' ? '已完成' : task.outcome === 'cancelled' ? '已取消' : '失败';
		}
		return task.progress.label ?? '正在处理';
	}

	async function chooseFiles() {
		const chosen = await open({
			title: '选择播客音频',
			multiple: true,
			filters: [{ name: '音频', extensions: ['mp3', 'm4a', 'wav'] }]
		});
		if (!chosen) return;
		setPaths(Array.isArray(chosen) ? chosen : [chosen]);
	}

	async function runPreview() {
		if (selectedPaths.length === 0) {
			errorText = '请先拖入或选择音频文件。';
			return;
		}
		busy = true;
		errorText = '';
		noticeText = '';
		try {
			preview = await invoke<PodcastFilesPreview>('preview_podcast_files', {
				paths: selectedPaths,
				options: { translate, polish, maxApiCostCny: Number(maxApiCostCny) || 0 }
			});
			budgetConfirmed = false;
		} catch (error) {
			errorText = `预检失败：${String(error)}`;
			preview = null;
		} finally {
			busy = false;
		}
	}

	async function addTasks() {
		if (!preview || !canAdd) return;
		busy = true;
		errorText = '';
		noticeText = '';
		const approval: PodcastBudgetApproval | null = approvalRequired
			? {
					estimatedDiskBytes: preview.budget.estimatedDiskBytes,
					estimatedApiCostUpperCny: preview.budget.estimatedApiCostUpperCny
				}
			: null;
		try {
			const result = await invoke<PodcastAddResult>('add_podcast_files', {
				previewId: preview.previewId,
				duplicatePolicy,
				budgetApproval: approval,
				requestId: crypto.randomUUID()
			});
			createdTaskIds = result.tasks.map((task) => task.id);
			existingBooks = result.existingBooks;
			noticeText =
				result.tasks.length > 0 ? '任务已加入队列；可逐项开始。' : '已复用书架中的相同播客。';
			onRefreshTasks();
		} catch (error) {
			errorText = `创建任务失败：${String(error)}`;
		} finally {
			busy = false;
		}
	}

	onMount(() => {
		let unlisten: (() => void) | undefined;
		void getCurrentWebviewWindow()
			.onDragDropEvent((event) => {
				if (event.payload.type === 'over' || event.payload.type === 'enter') {
					dragActive = true;
				} else if (event.payload.type === 'leave') {
					dragActive = false;
				} else if (event.payload.type === 'drop') {
					dragActive = false;
					setPaths(event.payload.paths);
				}
			})
			.then((cleanup) => {
				unlisten = cleanup;
			});
		return () => unlisten?.();
	});
</script>

<WorkflowDialogShell
	titleId="podcast-title"
	descriptionId="podcast-description"
	eyebrow="PODCAST WORKFLOW"
	title="转写播客"
	description="音频先进入受管缓存，预检通过后再加入统一任务队列。"
	maxWidth="720px"
	{onClose}
>
	<div
		class:active={dragActive}
		class="drop-zone"
		role="button"
		tabindex="0"
		aria-label={selectedPaths.length > 0 ? '继续添加音频文件' : '拖放或选择音频文件'}
		on:click={() => void chooseFiles()}
		on:keydown={(event) => {
			if (event.key === 'Enter' || event.key === ' ') void chooseFiles();
		}}
	>
		<span class="drop-icon" aria-hidden="true">
			<svg width="28" height="28" viewBox="0 0 28 28" fill="none">
				<path
					d="M8 18v2.5A2.5 2.5 0 0 0 10.5 23h7A2.5 2.5 0 0 0 20 20.5V18"
					stroke="currentColor"
					stroke-width="1.5"
					stroke-linecap="round"
				/>
				<path
					d="M14 5v12M14 5l-4 4M14 5l4 4"
					stroke="currentColor"
					stroke-width="1.5"
					stroke-linecap="round"
					stroke-linejoin="round"
				/>
				<path
					d="M6 12.5c0-1.2.7-2.2 1.7-2.6A5 5 0 0 1 17.2 8a3.8 3.8 0 0 1 4.3 3.7"
					stroke="currentColor"
					stroke-width="1.35"
					stroke-linecap="round"
					opacity="0.55"
				/>
			</svg>
		</span>
		<strong class="drop-title"
			>{selectedPaths.length > 0 ? '继续添加音频' : '拖放音频文件到这里'}</strong
		>
		<span class="drop-hint">支持 MP3、M4A、WAV；也可以点击选择文件</span>
	</div>

	{#if selectedPaths.length > 0}
		<div class="file-list wf-card" aria-label="待预检音频">
			{#each selectedPaths as path (path)}
				<div class="file-row">
					<span class="file-name">
						<svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true">
							<path
								d="M4 2.5h4.2L11 5.3V11a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1v-7.5a1 1 0 0 1 1-1z"
								fill="none"
								stroke="currentColor"
								stroke-width="1.1"
							/>
							<path d="M8 2.5V5h2.8" fill="none" stroke="currentColor" stroke-width="1.1" />
						</svg>
						{fileName(path)}
					</span>
					<button
						type="button"
						class="remove-btn"
						aria-label={`移除 ${fileName(path)}`}
						on:click={() => removePath(path)}>移除</button
					>
				</div>
			{/each}
		</div>
	{/if}

	<div class="options-grid">
		<label class="check-row">
			<input type="checkbox" bind:checked={translate} on:change={resetPreview} />
			<span>生成中文译文</span>
		</label>
		<label class="check-row">
			<input type="checkbox" bind:checked={polish} on:change={resetPreview} />
			<span>润色文稿</span>
		</label>
		<label class="wf-field">
			<span class="wf-label">本次预算上限（元）</span>
			<input type="number" min="0" step="0.01" bind:value={maxApiCostCny} on:input={resetPreview} />
		</label>
	</div>

	<fieldset class="duplicate-field">
		<legend class="wf-label">重复播客处理</legend>
		<label
			><input
				type="radio"
				bind:group={duplicatePolicy}
				value="reuse_existing"
				on:change={resetPreview}
			/>复用书架已有版本</label
		>
		<label
			><input
				type="radio"
				bind:group={duplicatePolicy}
				value="new_revision"
				on:change={resetPreview}
			/>创建新的 revision</label
		>
	</fieldset>

	{#if preview}
		<section class="preview-card wf-card" aria-label="播客预检结果">
			<div class="preview-title">
				<strong>预检通过</strong>
				<span
					>{preview.files.length} 个文件 · {formatDuration(
						preview.files.reduce((sum, file) => sum + file.durationSeconds, 0)
					)}</span
				>
			</div>
			<div class="metric-grid">
				<div>
					<span class="wf-label">预计缓存</span>
					<strong>{formatBytes(preview.budget.estimatedDiskBytes)}</strong>
				</div>
				<div>
					<span class="wf-label">可用空间</span>
					<strong>{formatBytes(preview.budget.availableDiskBytes)}</strong>
				</div>
				<div>
					<span class="wf-label">API 费用上限</span>
					<strong>¥{preview.budget.estimatedApiCostUpperCny.toFixed(2)}</strong>
				</div>
			</div>
			{#if preview.files.some((file) => file.duplicateBookId)}
				<p class="warning-copy">
					检测到已有同源内容；当前策略：{duplicatePolicy === 'reuse_existing'
						? '复用已有版本'
						: '创建新 revision'}。
				</p>
			{/if}
			{#if approvalRequired}
				<label class="approval-row">
					<input type="checkbox" bind:checked={budgetConfirmed} />
					我确认本次最高可能产生 ¥{preview.budget.estimatedApiCostUpperCny.toFixed(2)} 的费用
				</label>
			{/if}
		</section>
	{/if}

	{#if errorText}<p class="wf-msg-error" role="alert">{errorText}</p>{/if}
	{#if noticeText}<p class="wf-msg-success" role="status">{noticeText}</p>{/if}

	{#if createdTasks.length > 0}
		<section class="created-card wf-card" aria-label="新建播客任务">
			<strong class="section-title">任务队列</strong>
			{#each createdTasks as task (task.id)}
				<div class="created-row">
					<span>{task.id.slice(0, 8)} · {taskStateLabel(task)}</span>
					{#if task.lifecycleState === 'queued'}
						<button type="button" class="link-btn" on:click={() => onStartTask(task.id)}>开始</button>
					{/if}
					{#if task.lifecycleState === 'terminal' && task.outcome === 'success'}
						<button type="button" class="link-btn" on:click={() => onOpenResult(task.id)}
							>打开结果</button
						>
					{/if}
				</div>
			{/each}
		</section>
	{/if}
	{#if existingBooks.length > 0}
		<p class="existing-copy">已复用 {existingBooks.length} 个书架条目。</p>
	{/if}

	<div slot="footer" class="footer-actions">
		<button type="button" class="wf-quiet" on:click={onClose}>稍后处理</button>
		<button
			type="button"
			class="wf-secondary"
			disabled={busy || selectedPaths.length === 0}
			on:click={() => void runPreview()}>{busy && !preview ? '预检中…' : '运行预检'}</button
		>
		<button type="button" class="wf-primary" disabled={!canAdd} on:click={() => void addTasks()}
			>{busy && preview
				? '加入中…'
				: approvalRequired && !budgetConfirmed
					? '确认预算后加入'
					: '加入任务队列'}</button
		>
	</div>
</WorkflowDialogShell>

<style>
	.drop-zone {
		display: grid;
		gap: 8px;
		place-items: center;
		min-height: 120px;
		padding: 18px 16px;
		border: 1px dashed color-mix(in srgb, var(--wf-accent, var(--link)) 58%, var(--wf-border, var(--hr)));
		border-radius: 14px;
		background:
			linear-gradient(
				180deg,
				color-mix(in srgb, var(--wf-accent, var(--link)) 12%, transparent),
				color-mix(in srgb, var(--wf-panel-raised, var(--bg-secondary)) 80%, transparent)
			);
		cursor: pointer;
		text-align: center;
		color: var(--wf-body, var(--text));
		transition:
			border-color 160ms ease,
			background 160ms ease,
			transform 120ms ease,
			box-shadow 160ms ease;
	}

	.drop-zone:hover,
	.drop-zone:focus-visible,
	.drop-zone.active {
		border-color: var(--wf-accent, var(--link));
		background:
			linear-gradient(
				180deg,
				color-mix(in srgb, var(--wf-accent, var(--link)) 18%, transparent),
				color-mix(in srgb, var(--wf-panel-raised, var(--bg-secondary)) 70%, transparent)
			);
		box-shadow: 0 0 0 1px color-mix(in srgb, var(--wf-accent, var(--link)) 28%, transparent);
		transform: translateY(-1px);
		outline: none;
	}

	.drop-zone:active {
		transform: translateY(0);
	}

	.drop-icon {
		display: grid;
		place-items: center;
		width: 44px;
		height: 44px;
		border-radius: 12px;
		color: var(--wf-accent, var(--link));
		background: color-mix(in srgb, var(--wf-accent, var(--link)) 14%, transparent);
		border: 1px solid color-mix(in srgb, var(--wf-accent, var(--link)) 30%, transparent);
	}

	.drop-title {
		font-size: 15px;
		font-weight: 650;
		color: var(--wf-title, var(--heading, var(--text)));
	}

	.drop-hint {
		color: var(--wf-muted, var(--text-secondary));
		font-size: 12px;
		line-height: 1.45;
	}

	.file-list,
	.preview-card,
	.created-card {
		display: grid;
		gap: 8px;
	}

	.file-row,
	.created-row,
	.preview-title {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
		font-size: 13px;
		color: var(--wf-body, var(--text));
	}

	.file-name {
		display: flex;
		align-items: center;
		gap: 8px;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		color: var(--wf-title, var(--text));
	}

	.file-name svg {
		flex: none;
		color: var(--wf-accent, var(--link));
	}

	.remove-btn,
	.link-btn {
		flex: none;
		border: 0;
		background: transparent;
		color: var(--wf-accent, var(--link));
		cursor: pointer;
		font: inherit;
		font-size: 12px;
	}

	.remove-btn:hover,
	.link-btn:hover {
		color: var(--wf-accent-hover, var(--link-hover));
		text-decoration: underline;
	}

	.options-grid {
		display: grid;
		grid-template-columns: 1fr 1fr;
		gap: 14px;
		align-items: end;
	}

	.check-row,
	.duplicate-field,
	.approval-row {
		color: var(--wf-body, var(--text));
		font-size: 13px;
	}

	.check-row,
	.approval-row {
		display: flex;
		align-items: center;
		gap: 8px;
	}

	.duplicate-field {
		display: flex;
		flex-wrap: wrap;
		gap: 14px;
		border: 1px solid var(--wf-border, var(--hr));
		border-radius: 10px;
		padding: 10px 12px;
	}

	.duplicate-field legend {
		padding: 0 5px;
	}

	.duplicate-field label {
		display: flex;
		align-items: center;
		gap: 6px;
	}

	.metric-grid {
		display: grid;
		grid-template-columns: repeat(3, 1fr);
		gap: 8px;
		margin-top: 4px;
	}

	.metric-grid div {
		display: grid;
		gap: 4px;
		padding: 9px;
		border-radius: 9px;
		background: color-mix(in srgb, var(--wf-accent, var(--link)) 10%, transparent);
		border: 1px solid color-mix(in srgb, var(--wf-accent, var(--link)) 18%, transparent);
	}

	.metric-grid strong {
		font-size: 14px;
		color: var(--wf-title, var(--text));
	}

	.warning-copy,
	.existing-copy {
		margin: 0;
		font-size: 12px;
		line-height: 1.5;
		color: var(--wf-muted, var(--text-secondary));
	}

	.warning-copy {
		color: #c9922e;
	}

	.approval-row {
		margin-top: 6px;
		color: var(--wf-title, var(--text));
	}

	.section-title {
		color: var(--wf-title, var(--text));
		font-size: 13px;
	}

	.footer-actions {
		display: flex;
		gap: 8px;
		flex-wrap: wrap;
		justify-content: flex-end;
	}

	@media (max-width: 620px) {
		.options-grid,
		.metric-grid {
			grid-template-columns: 1fr;
		}
	}

	@media (prefers-reduced-motion: reduce) {
		.drop-zone {
			transition: none;
		}
	}
</style>
