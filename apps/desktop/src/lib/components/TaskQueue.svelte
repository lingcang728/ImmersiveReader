<script lang="ts">
	import type { TaskEvent, TaskSnapshot } from '$lib/tasks/sync';
	import TaskRow from './TaskRow.svelte';

	export let tasks: readonly TaskSnapshot[] = [];
	export let events: readonly TaskEvent[] = [];
	export let recoverableBytes = 0;
	export let maxVisible = 8;
	export let onStartTask: (taskId: string) => void;
	export let onStartZhihuTask: (taskId: string, revision: number) => void;
	export let onOpenTaskResult: (taskId: string) => void;
	export let onRestartTask: (taskId: string) => void;
	export let onControlTask: (
		taskId: string,
		action: 'pause' | 'resume' | 'cancel' | 'cancel_and_discard',
		revision: number
	) => void;
	export let onControlZhihuTask: (
		taskId: string,
		action: 'pause' | 'resume' | 'cancel',
		revision: number
	) => void;

	function formatBytes(bytes: number): string {
		if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
		if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
		return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
	}

	function eventTime(value: string): string {
		const date = new Date(value);
		if (Number.isNaN(date.getTime())) return '--:--';
		return new Intl.DateTimeFormat('zh-CN', {
			hour: '2-digit',
			minute: '2-digit',
			second: '2-digit'
		}).format(date);
	}

	$: visible = tasks.slice(0, maxVisible);
	$: hidden = Math.max(0, tasks.length - visible.length);
</script>

{#if tasks.length > 0}
	<section class="task-queue" aria-label="统一任务队列" aria-live="polite">
		<div class="task-queue-shell">
			<header class="task-queue-header">
				<div>
					<strong>任务队列</strong>
					<span>{tasks.length} 项 · 可恢复材料 {formatBytes(recoverableBytes)}</span>
				</div>
				<span class="task-source">由沉浸阅读统一管理</span>
			</header>

			<div class="task-list">
				{#each visible as task (task.id)}
					<TaskRow
						{task}
						{onStartTask}
						{onStartZhihuTask}
						{onOpenTaskResult}
						{onRestartTask}
						{onControlTask}
						{onControlZhihuTask}
					/>
				{/each}
			</div>

			{#if hidden > 0}
				<p class="task-more">还有 {hidden} 项任务未展开显示</p>
			{/if}

			{#if events.length > 0}
				<details class="task-events">
					<summary>近期事件（{events.length}）</summary>
					<div class="task-event-list">
						{#each events.slice(0, 8) as event (event.taskId + ':' + event.sequence)}
							<div class="task-event-row">
								<time>{eventTime(event.createdAt)}</time>
								<span>{event.snapshot.engineStage}</span>
							</div>
						{/each}
					</div>
				</details>
			{/if}
		</div>
	</section>
{/if}
