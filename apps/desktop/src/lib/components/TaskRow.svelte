<script lang="ts">
	import type { TaskSnapshot } from '$lib/tasks/sync';

	export let task: TaskSnapshot;
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

	$: percent = taskProgress(task);
	$: stageLabel = taskStageLabel(task);
	$: stateLabel = taskStateLabel(task);
	$: heartbeatLabel = formatHeartbeat(task.lastHeartbeatAt);
	$: progressLabel = formatProgress(task, percent);
	$: primary = primaryAction(task);
	$: secondary = secondaryAction(task);

	function taskKindLabel(kind: TaskSnapshot['kind']): string {
		return kind === 'podcast' ? '播客' : '知乎';
	}

	function taskStateLabel(task: TaskSnapshot): string {
		if (task.requiredAction === 'login') return '等待登录';
		if (task.requiredAction === 'captcha') return '等待验证码';
		if (task.requiredAction === 'configure_secret') return '需要配置密钥';
		if (task.requiredAction === 'free_disk_space') return '磁盘空间不足';
		if (task.requiredAction === 'approve_budget') return '等待预算确认';
		if (task.lifecycleState === 'terminal') {
			if (task.outcome === 'success') return '已完成';
			if (task.outcome === 'partial_success') return '部分完成';
			if (task.outcome === 'cancelled') return '已取消';
			if (task.outcome === 'interrupted') return '已中断';
			if (task.errorCode === 'INPUT_COPY_FAILED') return '输入副本失败';
			return '失败';
		}
		if (task.lifecycleState === 'paused') return '已暂停';
		if (task.lifecycleState === 'pausing') return '正在暂停';
		if (task.lifecycleState === 'queued') return '等待开始';
		if (task.lifecycleState === 'starting') return '正在启动';
		if (task.lifecycleState === 'stopping') return '正在停止';
		return task.progress.label ?? '处理中';
	}

	function taskStageLabel(task: TaskSnapshot): string {
		const stage = (task.engineStage || '').toLowerCase();
		const map: Record<string, string> = {
			queued: '排队',
			index: '索引列表',
			content: '抓取正文',
			transcribe: '语音转写',
			chunking: '音频切块',
			normalize: '标准化',
			translate: '翻译',
			publish: '发布到书库',
			copy: '复制输入',
			input_copy: '复制输入',
			input_copy_failed: '输入准备失败',
			completed: '已完成',
			failed: '失败',
			crashed: '异常退出',
			cancelled: '已取消',
			recovery_check: '恢复检查'
		};
		if (map[stage]) return map[stage];
		if (task.progress.label) return task.progress.label;
		return stage || '准备中';
	}

	function taskProgress(task: TaskSnapshot): number | null {
		if (task.progress.mode !== 'determinate' || task.progress.percent === undefined) return null;
		return Math.max(0, Math.min(100, task.progress.percent));
	}

	function formatProgress(task: TaskSnapshot, percent: number | null): string {
		const completed = task.progress.completedUnits;
		const total = task.progress.totalUnits;
		const source = task.progress.sourceTotalUnits;
		const unit = task.progress.unit ?? '';
		const parts: string[] = [];
		if (source != null && source > 0) {
			parts.push(`主页/API ${source}${unit}`);
		}
		if (completed != null && total != null && total > 0) {
			if (task.kind === 'zhihu' && task.engineStage === 'index') {
				parts.push(`已发现 ${completed}${unit}`);
			} else if (task.kind === 'zhihu') {
				parts.push(`已归档 ${completed}/${total}${unit}`);
			} else {
				parts.push(`${completed}/${total}${unit}`);
			}
		} else if (completed != null) {
			parts.push(`已发现 ${completed}${unit}`);
		} else if (percent !== null) {
			parts.push(`${Math.round(percent)}%`);
		}
		if (parts.length === 0) return '…';
		return parts.join(' · ');
	}

	function formatHeartbeat(value?: string | null): string {
		if (!value) return '尚无心跳';
		const date = new Date(value);
		if (Number.isNaN(date.getTime())) return '心跳未知';
		const seconds = Math.max(0, Math.round((Date.now() - date.getTime()) / 1000));
		if (seconds < 5) return '刚刚心跳';
		if (seconds < 60) return `${seconds}s 前心跳`;
		const minutes = Math.floor(seconds / 60);
		if (minutes < 60) return `${minutes} 分钟前心跳`;
		return new Intl.DateTimeFormat('zh-CN', {
			hour: '2-digit',
			minute: '2-digit'
		}).format(date);
	}

	type Action =
		| { kind: 'start' }
		| { kind: 'pause' }
		| { kind: 'resume' }
		| { kind: 'retry' }
		| { kind: 'reconnect' }
		| { kind: 'relogin' }
		| { kind: 'open' }
		| { kind: 'cancel' }
		| null;

	function primaryAction(task: TaskSnapshot): { action: Action; label: string } | null {
		if (task.lifecycleState === 'queued') {
			return { action: { kind: 'start' }, label: '开始' };
		}
		if (task.canPause) {
			return { action: { kind: 'pause' }, label: '暂停' };
		}
		if (task.canResume) {
			return { action: { kind: 'resume' }, label: '继续' };
		}
		if (task.requiredAction === 'login') {
			return { action: { kind: 'relogin' }, label: '重新登录' };
		}
		if (task.errorCode === 'ENGINE_CRASHED' || task.outcome === 'interrupted') {
			return { action: { kind: 'reconnect' }, label: '重新连接' };
		}
		if (task.lifecycleState === 'terminal' && task.outcome === 'success' && task.kind === 'podcast') {
			return { action: { kind: 'open' }, label: '打开结果' };
		}
		if (task.canRetry) {
			return { action: { kind: 'retry' }, label: '重试' };
		}
		return null;
	}

	function secondaryAction(task: TaskSnapshot): { action: Action; label: string } | null {
		if (task.canCancel) {
			return { action: { kind: 'cancel' }, label: '取消' };
		}
		return null;
	}

	function runAction(action: Action) {
		if (!action) return;
		if (action.kind === 'start') {
			if (task.kind === 'podcast') onStartTask(task.id);
			else onStartZhihuTask(task.id, task.revision);
			return;
		}
		if (action.kind === 'pause') {
			if (task.kind === 'podcast') onControlTask(task.id, 'pause', task.revision);
			else onControlZhihuTask(task.id, 'pause', task.revision);
			return;
		}
		if (action.kind === 'resume' || action.kind === 'reconnect' || action.kind === 'relogin') {
			if (task.kind === 'podcast') onControlTask(task.id, 'resume', task.revision);
			else onControlZhihuTask(task.id, 'resume', task.revision);
			return;
		}
		if (action.kind === 'retry') {
			if (task.kind === 'podcast') onRestartTask(task.id);
			else onStartZhihuTask(task.id, task.revision);
			return;
		}
		if (action.kind === 'open') {
			onOpenTaskResult(task.id);
			return;
		}
		if (action.kind === 'cancel') {
			const ok = window.confirm('确定取消该任务？进行中的进度将按检查点保留。');
			if (!ok) return;
			if (task.kind === 'podcast') onControlTask(task.id, 'cancel', task.revision);
			else onControlZhihuTask(task.id, 'cancel', task.revision);
		}
	}

	function isRunning(task: TaskSnapshot): boolean {
		return (
			task.lifecycleState === 'running' ||
			task.lifecycleState === 'starting' ||
			task.lifecycleState === 'pausing' ||
			task.lifecycleState === 'stopping'
		);
	}
</script>

<article
	class="task-row"
	class:running={isRunning(task)}
	class:error={task.lifecycleState === 'terminal' &&
		(task.outcome === 'failed' || task.outcome === 'interrupted')}
	class:success={task.lifecycleState === 'terminal' && task.outcome === 'success'}
	aria-label={`${taskKindLabel(task.kind)}任务 ${stateLabel}`}
>
	<span class="task-kind" class:zhihu={task.kind === 'zhihu'}>{taskKindLabel(task.kind)}</span>

	<div class="task-copy">
		<strong>{stateLabel}</strong>
		<small>
			<span>{stageLabel}</span>
			<span aria-hidden="true">·</span>
			<span>{heartbeatLabel}</span>
			{#if progressLabel !== '…'}
				<span aria-hidden="true">·</span>
				<span>{progressLabel}</span>
			{/if}
		</small>
	</div>

	<div class="task-meter" aria-hidden={percent === null}>
		{#if percent !== null}
			<span class="task-progress" aria-label={`进度 ${Math.round(percent)}%`}>
				<i style={`transform:scaleX(${percent / 100})`}></i>
			</span>
			<output>{Math.round(percent)}%</output>
		{:else if isRunning(task)}
			<span class="task-pulse" aria-hidden="true"></span>
		{:else}
			<span class="task-idle">—</span>
		{/if}
	</div>

	<div class="task-actions">
		{#if primary}
			<button
				type="button"
				class="task-btn primary"
				on:click={() => runAction(primary.action)}
			>
				{primary.label}
			</button>
		{/if}
		{#if secondary}
			<button
				type="button"
				class="task-btn danger"
				on:click={() => runAction(secondary.action)}
			>
				{secondary.label}
			</button>
		{/if}
	</div>

	{#if task.errorCode || task.errorMessage}
		<details class="task-detail">
			<summary>技术详情</summary>
			<p>
				{#if task.errorCode}<code>{task.errorCode}</code>{/if}
				{#if task.errorMessage}<span>{task.errorMessage}</span>{/if}
			</p>
			<p class="task-detail-meta">
				状态码 {task.engineStatus || '—'} · 阶段 {task.engineStage || '—'} · revision {task.revision}
			</p>
		</details>
	{/if}
</article>
