<script lang="ts">
	import { onMount } from 'svelte';
	import { invoke } from '@tauri-apps/api/core';
	import type { TaskSnapshot } from '$lib/tasks/sync';
	import WorkflowDialogShell from './WorkflowDialogShell.svelte';

	type ItemTypes = 'answers' | 'articles' | 'all';
	type SortBy = 'time' | 'vote';

	interface LoginStatus {
		loggedIn: boolean;
	}

	interface CreateRequest {
		peopleId: string;
		itemTypes: ItemTypes;
		topN: number | null;
		sortBy: SortBy;
	}

	export let tasks: readonly TaskSnapshot[] = [];
	export let onClose: () => void;
	export let onRefreshTasks: () => void;
	export let onStartTask: (taskId: string, revision: number) => void;
	export let onControlTask: (
		taskId: string,
		action: 'pause' | 'resume' | 'cancel',
		revision: number
	) => void;

	let peopleId = '';
	let itemTypes: ItemTypes = 'all';
	let sortBy: SortBy = 'time';
	let topN: number | '' = '';
	let loginStatus: LoginStatus | null = null;
	let createdTaskId = '';
	let busy = false;
	let errorText = '';
	let noticeText = '';

	$: createdTask = createdTaskId
		? (tasks.find((task) => task.id === createdTaskId) ?? null)
		: null;

	function taskStateLabel(task: TaskSnapshot): string {
		if (task.lifecycleState === 'queued') return '等待开始';
		if (task.lifecycleState === 'starting') return '正在启动';
		if (task.lifecycleState === 'running') return '抓取中';
		if (task.lifecycleState === 'paused') return '已暂停';
		if (task.lifecycleState === 'terminal') {
			if (task.outcome === 'success') return '已完成';
			if (task.outcome === 'partial_success') return '部分完成';
			if (task.outcome === 'cancelled') return '已取消';
			return '失败';
		}
		return task.progress.label ?? '处理中';
	}

	function progressLabel(task: TaskSnapshot): string {
		if (task.progress.percent === undefined) return task.progress.label ?? '等待进度';
		return String(Math.round(task.progress.percent)) + '% · ' + (task.progress.label ?? '');
	}

	async function refreshLoginStatus() {
		try {
			loginStatus = await invoke<LoginStatus>('get_zhihu_login_status');
			errorText = '';
		} catch (error) {
			loginStatus = null;
			errorText = '无法读取登录状态：' + String(error);
		}
	}

	async function startLogin() {
		try {
			await invoke('start_zhihu_login');
			noticeText = '已打开受管知乎登录流程；完成后点击刷新。';
			errorText = '';
		} catch (error) {
			errorText = '无法启动登录流程：' + String(error);
		}
	}

	async function createTask() {
		const normalized = peopleId.trim();
		if (!normalized) {
			errorText = '请输入知乎答主 ID。';
			return;
		}
		if (topN !== '' && (!Number.isInteger(Number(topN)) || Number(topN) < 1 || Number(topN) > 5000)) {
			errorText = 'Top N 必须是 1–5000 的整数。';
			return;
		}
		busy = true;
		errorText = '';
		noticeText = '';
		const request: CreateRequest = {
			peopleId: normalized,
			itemTypes,
			topN: topN === '' ? null : Number(topN),
			sortBy
		};
		try {
			const snapshot = await invoke<TaskSnapshot>('create_zhihu_task', { request });
			createdTaskId = snapshot.id;
			noticeText = '任务已加入统一队列；可从这里或书架任务栏开始。';
			onRefreshTasks();
		} catch (error) {
			errorText = '创建任务失败：' + String(error);
		} finally {
			busy = false;
		}
	}

	onMount(() => {
		void refreshLoginStatus();
	});
</script>

<WorkflowDialogShell
	titleId="zhihu-title"
	descriptionId="zhihu-description"
	eyebrow="ZHIHU WORKFLOW"
	title="归档知乎"
	description="任务、登录态和抓取进度都留在沉浸阅读的统一队列中。"
	maxWidth="650px"
	{onClose}
>
	<section class="login-card wf-card" aria-label="知乎登录状态">
		<div>
			<span class="wf-label">登录状态</span>
			<strong
				class:wf-status-ok={loginStatus?.loggedIn === true}
				class:wf-status-warn={loginStatus?.loggedIn === false}
				aria-live="polite"
			>
				{loginStatus === null ? '检查中…' : loginStatus.loggedIn ? '已登录' : '需要登录'}
			</strong>
			{#if loginStatus && !loginStatus.loggedIn}
				<span class="status-hint">请先完成知乎登录后再开始抓取</span>
			{/if}
		</div>
		<div class="login-actions">
			<button type="button" class="wf-quiet" on:click={() => void refreshLoginStatus()}>刷新</button>
			<button type="button" class="wf-quiet" on:click={() => void startLogin()}>开始登录</button>
		</div>
	</section>

	<label class="wf-field">
		<span class="wf-label">知乎答主 ID</span>
		<input bind:value={peopleId} placeholder="例如 xiao-xue-shi-46-24" autocomplete="off" />
	</label>

	<fieldset class="choice-field">
		<legend class="wf-label">内容类型</legend>
		<label><input type="radio" bind:group={itemTypes} value="all" />回答 + 文章</label>
		<label><input type="radio" bind:group={itemTypes} value="answers" />仅回答</label>
		<label><input type="radio" bind:group={itemTypes} value="articles" />仅文章</label>
	</fieldset>

	<div class="options-grid">
		<label class="wf-field">
			<span class="wf-label">排序</span>
			<select bind:value={sortBy}>
				<option value="time">发布时间（新到旧）</option>
				<option value="vote">点赞数（高到低）</option>
			</select>
		</label>
		<label class="wf-field">
			<span class="wf-label">Top N（留空为全部）</span>
			<input type="number" min="1" max="5000" step="1" bind:value={topN} placeholder="例如 5" />
		</label>
	</div>

	{#if createdTask}
		<section class="task-card wf-card" aria-label="知乎任务结果">
			<div class="task-title">
				<strong>{taskStateLabel(createdTask)}</strong>
				<span>{createdTask.id}</span>
			</div>
			<p>{progressLabel(createdTask)}</p>
			{#if createdTask.lifecycleState === 'queued'}
				<button
					type="button"
					class="wf-primary"
					on:click={() => onStartTask(createdTask.id, createdTask.revision)}>开始抓取</button
				>
			{:else if createdTask.canPause}
				<button
					type="button"
					class="wf-secondary"
					on:click={() => onControlTask(createdTask.id, 'pause', createdTask.revision)}>暂停</button
				>
			{:else if createdTask.canResume}
				<button
					type="button"
					class="wf-secondary"
					on:click={() => onControlTask(createdTask.id, 'resume', createdTask.revision)}>恢复</button
				>
			{:else if createdTask.canCancel}
				<button
					type="button"
					class="wf-secondary"
					on:click={() => onControlTask(createdTask.id, 'cancel', createdTask.revision)}>取消</button
				>
			{/if}
			{#if createdTask.lifecycleState === 'terminal'}
				<div class="result-card wf-card">
					<strong>结果页</strong>
					<span>{createdTask.progress.label ?? '任务已结束'}</span>
					{#if createdTask.errorMessage}<small class="wf-msg-error">{createdTask.errorMessage}</small>{/if}
				</div>
			{/if}
		</section>
	{/if}

	{#if errorText}<p class="wf-msg-error" role="alert">{errorText}</p>{/if}
	{#if noticeText}<p class="wf-msg-success" role="status">{noticeText}</p>{/if}

	<div slot="footer" class="footer-actions">
		<button type="button" class="wf-quiet" on:click={onClose}>稍后处理</button>
		<button type="button" class="wf-primary" disabled={busy} on:click={() => void createTask()}
			>{busy ? '创建中…' : '加入任务队列'}</button
		>
	</div>
</WorkflowDialogShell>

<style>
	.login-card,
	.task-title,
	.login-actions {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
	}

	.login-card {
		align-items: flex-start;
	}

	.login-card strong {
		display: block;
		margin-top: 4px;
		font-size: 14px;
	}

	.status-hint {
		display: block;
		margin-top: 4px;
		font-size: 11px;
		color: var(--wf-muted, var(--text-secondary));
	}

	.login-actions {
		flex: none;
	}

	.choice-field {
		display: flex;
		flex-wrap: wrap;
		gap: 14px;
		border: 1px solid var(--wf-border, var(--hr));
		border-radius: 10px;
		padding: 10px 12px;
		color: var(--wf-body, var(--text));
		font-size: 13px;
	}

	.choice-field legend {
		padding: 0 5px;
	}

	.choice-field label {
		display: flex;
		align-items: center;
		gap: 6px;
		color: var(--wf-body, var(--text));
	}

	.options-grid {
		display: grid;
		grid-template-columns: 1fr 1fr;
		gap: 14px;
	}

	.task-card {
		display: grid;
		gap: 10px;
	}

	.task-title span {
		max-width: 260px;
		overflow: hidden;
		color: var(--wf-muted, var(--text-secondary));
		font-size: 11px;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.task-card p,
	.result-card span {
		margin: 0;
		color: var(--wf-muted, var(--text-secondary));
		font-size: 12px;
		line-height: 1.5;
	}

	.result-card {
		display: grid;
		gap: 4px;
	}

	.result-card strong {
		color: var(--wf-accent, var(--link));
	}

	.footer-actions {
		display: flex;
		gap: 8px;
	}

	@media (max-width: 620px) {
		.options-grid {
			grid-template-columns: 1fr;
		}
	}
</style>
