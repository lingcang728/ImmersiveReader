<script lang="ts">
	import { onMount } from 'svelte';
	import { invoke } from '@tauri-apps/api/core';
	import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
	import { open } from '@tauri-apps/plugin-dialog';
	import type { TaskSnapshot } from '$lib/tasks/sync';

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
	export let onFallback: () => void;

	let selectedPaths: string[] = [];
	let translate = false;
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
				options: { translate, maxApiCostCny: Number(maxApiCostCny) || 0 }
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
			noticeText = result.tasks.length > 0 ? '任务已加入队列；可逐项开始。' : '已复用书架中的相同播客。';
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

<div class="podcast-modal" role="presentation" on:click|self={onClose}>
	<dialog open class="podcast-panel" aria-labelledby="podcast-title">
		<header class="podcast-header">
			<div>
				<span class="eyebrow">PODCAST WORKFLOW</span>
				<h1 id="podcast-title">转写播客</h1>
				<p>音频先进入受管缓存，预检通过后再加入统一任务队列。</p>
			</div>
			<button type="button" class="close-button" aria-label="关闭" on:click={onClose}>×</button>
		</header>

		<div class="podcast-body">
			<div
				class:active={dragActive}
				class="drop-zone"
				role="button"
				tabindex="0"
				on:click={() => void chooseFiles()}
				on:keydown={(event) => {
					if (event.key === 'Enter' || event.key === ' ') void chooseFiles();
				}}
			>
				<strong>{selectedPaths.length > 0 ? '继续添加音频' : '拖放音频文件到这里'}</strong>
				<span>支持 MP3、M4A、WAV；也可以点击选择文件</span>
			</div>

			{#if selectedPaths.length > 0}
				<div class="file-list" aria-label="待预检音频">
					{#each selectedPaths as path (path)}
						<div class="file-row">
							<span>{fileName(path)}</span>
							<button type="button" aria-label={`移除 ${fileName(path)}`} on:click={() => removePath(path)}>移除</button>
						</div>
					{/each}
				</div>
			{/if}

			<div class="options-grid">
				<label class="check-row"><input type="checkbox" bind:checked={translate} on:change={resetPreview} />生成中文翻译</label>
				<label class="field-row">
					<span>本次预算上限（元）</span>
					<input type="number" min="0" step="0.01" bind:value={maxApiCostCny} on:input={resetPreview} />
				</label>
			</div>

			<fieldset class="duplicate-field">
				<legend>重复播客处理</legend>
				<label><input type="radio" bind:group={duplicatePolicy} value="reuse_existing" on:change={resetPreview} />复用书架已有版本</label>
				<label><input type="radio" bind:group={duplicatePolicy} value="new_revision" on:change={resetPreview} />创建新的 revision</label>
			</fieldset>

			{#if preview}
				<section class="preview-card" aria-label="播客预检结果">
					<div class="preview-title"><strong>预检通过</strong><span>{preview.files.length} 个文件 · {formatDuration(preview.files.reduce((sum, file) => sum + file.durationSeconds, 0))}</span></div>
					<div class="metric-grid">
						<div><span>预计缓存</span><strong>{formatBytes(preview.budget.estimatedDiskBytes)}</strong></div>
						<div><span>可用空间</span><strong>{formatBytes(preview.budget.availableDiskBytes)}</strong></div>
						<div><span>API 费用上限</span><strong>¥{preview.budget.estimatedApiCostUpperCny.toFixed(2)}</strong></div>
					</div>
					{#if preview.files.some((file) => file.duplicateBookId)}
						<p class="warning-copy">检测到已有同源内容；当前策略：{duplicatePolicy === 'reuse_existing' ? '复用已有版本' : '创建新 revision'}。</p>
					{/if}
					{#if approvalRequired}
						<label class="approval-row"><input type="checkbox" bind:checked={budgetConfirmed} />我确认本次最高可能产生 ¥{preview.budget.estimatedApiCostUpperCny.toFixed(2)} 的费用</label>
					{/if}
				</section>
			{/if}

			{#if errorText}<p class="message error" role="alert">{errorText}</p>{/if}
			{#if noticeText}<p class="message success" role="status">{noticeText}</p>{/if}

			{#if createdTasks.length > 0}
				<section class="created-card" aria-label="新建播客任务">
					<strong>任务队列</strong>
					{#each createdTasks as task (task.id)}
						<div class="created-row">
							<span>{task.id.slice(0, 8)} · {taskStateLabel(task)}</span>
							{#if task.lifecycleState === 'queued'}<button type="button" on:click={() => onStartTask(task.id)}>开始</button>{/if}
							{#if task.lifecycleState === 'terminal' && task.outcome === 'success'}<button type="button" on:click={() => onOpenResult(task.id)}>打开结果</button>{/if}
						</div>
					{/each}
				</section>
			{/if}
			{#if existingBooks.length > 0}<p class="existing-copy">已复用 {existingBooks.length} 个书架条目。</p>{/if}
		</div>

		<footer class="podcast-footer">
			<button type="button" class="quiet-button" on:click={onFallback}>打开旧版 Podcast 工具</button>
			<div class="footer-actions">
				<button type="button" class="quiet-button" on:click={onClose}>稍后处理</button>
				<button type="button" class="secondary-button" disabled={busy || selectedPaths.length === 0} on:click={() => void runPreview()}>{busy && !preview ? '预检中…' : '运行预检'}</button>
				<button type="button" class="primary-button" disabled={!canAdd} on:click={() => void addTasks()}>{busy && preview ? '加入中…' : approvalRequired && !budgetConfirmed ? '确认预算后加入' : '加入任务队列'}</button>
			</div>
		</footer>
	</dialog>
</div>

<style>
	.podcast-modal { position: fixed; inset: 0; z-index: 40; display: grid; place-items: center; padding: 24px; background: rgba(5, 12, 24, 0.68); backdrop-filter: blur(8px); color: var(--text); }
	.podcast-panel { width: min(720px, 100%); max-height: min(760px, 92vh); overflow: auto; border: 1px solid color-mix(in srgb, var(--link) 45%, var(--hr)); border-radius: 20px; background: var(--bg-secondary); box-shadow: 0 28px 90px rgba(0, 0, 0, 0.45); }
	.podcast-header { display: flex; justify-content: space-between; gap: 20px; padding: 28px 30px 22px; border-bottom: 1px solid var(--hr); }
	.eyebrow { color: var(--link); font-size: 10px; letter-spacing: .16em; }
	h1 { margin: 8px 0 6px; font-size: 26px; }
	.podcast-header p { margin: 0; color: var(--text-secondary); font-size: 13px; }
	.close-button { width: 34px; height: 34px; border: 1px solid var(--hr); border-radius: 50%; background: transparent; color: var(--text-secondary); font-size: 22px; cursor: pointer; }
	.podcast-body { display: grid; gap: 16px; padding: 24px 30px; }
	.drop-zone { display: grid; gap: 7px; place-items: center; min-height: 112px; border: 1px dashed color-mix(in srgb, var(--link) 58%, var(--hr)); border-radius: 14px; background: color-mix(in srgb, var(--link) 8%, transparent); cursor: pointer; text-align: center; }
	.drop-zone.active, .drop-zone:hover { border-color: var(--link); background: color-mix(in srgb, var(--link) 16%, transparent); }
	.drop-zone span { color: var(--text-secondary); font-size: 12px; }
	.file-list, .preview-card, .created-card { display: grid; gap: 8px; padding: 12px 14px; border: 1px solid var(--hr); border-radius: 12px; background: color-mix(in srgb, var(--bg) 45%, transparent); }
	.file-row, .created-row, .preview-title { display: flex; align-items: center; justify-content: space-between; gap: 12px; font-size: 13px; }
	.file-row span, .created-row span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
	.file-row button, .created-row button { flex: none; border: 0; background: transparent; color: var(--link); cursor: pointer; font: inherit; }
	.options-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 14px; }
	.check-row, .field-row, .duplicate-field, .approval-row { color: var(--text-secondary); font-size: 13px; }
	.check-row, .approval-row { display: flex; align-items: center; gap: 8px; }
	.field-row { display: grid; gap: 6px; }
	.field-row input { min-width: 0; border: 1px solid var(--hr); border-radius: 8px; padding: 8px 10px; background: var(--bg); color: var(--text); }
	.duplicate-field { display: flex; flex-wrap: wrap; gap: 14px; border: 1px solid var(--hr); border-radius: 10px; padding: 10px 12px; }
	.duplicate-field legend { padding: 0 5px; color: var(--text-faded); font-size: 11px; }
	.duplicate-field label { display: flex; align-items: center; gap: 6px; }
	.metric-grid { display: grid; grid-template-columns: repeat(3, 1fr); gap: 8px; margin-top: 12px; }
	.metric-grid div { display: grid; gap: 4px; padding: 9px; border-radius: 9px; background: color-mix(in srgb, var(--link) 8%, transparent); }
	.metric-grid span { color: var(--text-faded); font-size: 11px; }
	.metric-grid strong { font-size: 14px; }
	.warning-copy, .existing-copy, .message { margin: 0; font-size: 12px; line-height: 1.5; }
	.warning-copy { color: #d6a84f; }
	.approval-row { margin-top: 10px; color: var(--text); }
	.message.error { color: #ef8b8b; }
	.message.success { color: #82d3a4; }
	.podcast-footer { display: flex; justify-content: space-between; gap: 12px; padding: 18px 30px 24px; border-top: 1px solid var(--hr); }
	.footer-actions { display: flex; gap: 8px; }
	.quiet-button, .secondary-button, .primary-button { border: 1px solid var(--hr); border-radius: 9px; padding: 9px 13px; cursor: pointer; font: inherit; font-size: 12px; }
	.quiet-button { background: transparent; color: var(--text-secondary); }
	.secondary-button { background: var(--bg); color: var(--text); }
	.primary-button { border-color: var(--link); background: var(--link); color: white; }
	.quiet-button:hover, .secondary-button:hover { border-color: var(--link); color: var(--text); }
	.primary-button:disabled, .secondary-button:disabled { cursor: not-allowed; opacity: .45; }
	@media (max-width: 620px) { .options-grid, .metric-grid { grid-template-columns: 1fr; } .podcast-header, .podcast-body, .podcast-footer { padding-left: 18px; padding-right: 18px; } .podcast-footer { flex-direction: column; } .footer-actions { justify-content: flex-end; } }
</style>
