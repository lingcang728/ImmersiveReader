<script lang="ts">
	import type { TaskSnapshot } from '$lib/tasks/sync';
	import { displayTaskPercent, taskDisplayTitle } from '$lib/tasks/queueList';

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

	$: name = taskDisplayTitle(task);
	$: status = userStatus(task);
	$: subtitle = userSubtitle(task);
	$: percent = displayTaskPercent(task);
	$: flowing = isActive(task) && (percent === null || percent < 100);
	$: primary = primaryAction(task);
	$: secondary = secondaryAction(task);
	$: friendlyError = friendlyErrorText(task);
	let documentHidden = false;
	if (typeof document !== 'undefined') {
		documentHidden = document.hidden;
		document.addEventListener('visibilitychange', () => {
			documentHidden = document.hidden;
		});
	}

	$: preferReducedMotion =
		typeof window !== 'undefined' &&
		window.matchMedia?.('(prefers-reduced-motion: reduce)')?.matches === true;
	// Wave only for running, visible, non-queued work (not paused/terminal/hidden).
	$: waveEnabled =
		flowing &&
		!preferReducedMotion &&
		!documentHidden &&
		task.lifecycleState !== 'queued' &&
		task.lifecycleState !== 'paused' &&
		task.lifecycleState !== 'terminal';

	function taskKindLabel(kind: TaskSnapshot['kind']): string {
		return kind === 'podcast' ? '播客' : '知乎';
	}

	function isActive(task: TaskSnapshot): boolean {
		return (
			task.lifecycleState === 'running' ||
			task.lifecycleState === 'starting' ||
			task.lifecycleState === 'pausing' ||
			task.lifecycleState === 'stopping' ||
			task.lifecycleState === 'queued'
		);
	}

	function userStatus(task: TaskSnapshot): string {
		if (task.requiredAction === 'login') return '需要登录';
		if (task.requiredAction === 'captcha') return '需要验证';
		if (task.requiredAction === 'configure_secret') return '需要配置密钥';
		if (task.requiredAction === 'free_disk_space') return '磁盘空间不足';
		if (task.requiredAction === 'approve_budget') return '需要确认预算';
		if (task.lifecycleState === 'terminal') {
			if (task.outcome === 'success') return '已完成';
			if (task.outcome === 'partial_success') return '部分完成';
			if (task.outcome === 'cancelled') return '已取消';
			if (task.outcome === 'interrupted') return '已中断';
			return '失败';
		}
		if (task.lifecycleState === 'paused') return '已暂停';
		if (task.lifecycleState === 'pausing') return '正在暂停';
		if (task.lifecycleState === 'queued') {
			if (task.engineStage === 'input_copy') return '正在准备';
			return '等待开始';
		}
		if (task.lifecycleState === 'starting') return '正在启动';
		return stageTitle(task.engineStage);
	}

	function stageTitle(stage: string): string {
		const s = (stage || '').toLowerCase();
		const map: Record<string, string> = {
			queued: '排队中',
			input_copy: '准备音频',
			launching: '启动中',
			load_model: '加载模型',
			normalizing: '处理音频',
			chunking: '切分音频',
			transcribing: '语音转写',
			transcribe: '语音转写',
			translating: '翻译中',
			translate: '翻译中',
			polishing: '润色文稿',
			postprocess: '后处理',
			writing_output: '生成文稿',
			publish: '发布中',
			index: '索引列表',
			content: '抓取正文',
			working: '处理中',
			completed: '即将完成'
		};
		return map[s] || '处理中';
	}

	function userSubtitle(task: TaskSnapshot): string {
		if (task.lifecycleState === 'terminal') {
			if (task.outcome === 'success') {
				return task.kind === 'podcast' ? '已保存到 书库/播客' : '归档完成';
			}
			if (task.outcome === 'partial_success') return '部分条目已完成';
			if (task.outcome === 'cancelled') return '已取消';
			if (task.outcome === 'interrupted') return '意外中断，可重试';
			return friendlyErrorText(task) || '处理未完成';
		}
		if (task.kind === 'zhihu' && task.progress.completedUnits != null && task.progress.totalUnits) {
			if (task.engineStage === 'index') {
				return `已发现 ${task.progress.completedUnits} 篇`;
			}
			return `已归档 ${task.progress.completedUnits}/${task.progress.totalUnits} 篇`;
		}
		return status;
	}

	function friendlyErrorText(task: TaskSnapshot): string {
		const code = (task.errorCode || '').toUpperCase();
		const msg = (task.errorMessage || '').toLowerCase();
		if (code === 'PUBLISH_FAILED' || msg.includes('publish_failed') || msg.includes('no markdown')) {
			return '文稿生成后发布失败，请重试';
		}
		if (code === 'BUDGET_CONFIRMATION_REQUIRED' || msg.includes('budget exceeds')) {
			return '翻译预算不足，请提高预算后重试';
		}
		if (code === 'INPUT_COPY_FAILED' || code === 'INPUT_CHANGED') {
			return '音频文件无效或已变更，请重新选择';
		}
		if (code === 'SECRET_MISSING' || task.requiredAction === 'configure_secret') {
			return '请先配置 DeepSeek 密钥';
		}
		if (code === 'ENGINE_CRASHED' || task.outcome === 'interrupted') {
			return '任务意外中断，可重试继续';
		}
		if (code === 'RATE_LIMITED') return '请求过于频繁，请稍后重试';
		if (code === 'LOGIN_REQUIRED') return '需要重新登录知乎';
		if (code === 'CAPTCHA_REQUIRED') return '需要完成人机验证';
		if (task.lifecycleState === 'terminal' && task.outcome === 'failed') {
			return '处理失败，可点击重试';
		}
		return '';
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
		if (task.lifecycleState === 'queued' && task.engineStage !== 'input_copy') {
			return { action: { kind: 'start' }, label: '开始' };
		}
		if (task.canPause && isActive(task) && task.lifecycleState !== 'queued') {
			return { action: { kind: 'pause' }, label: '暂停' };
		}
		if (task.canResume) {
			return { action: { kind: 'resume' }, label: '继续' };
		}
		if (task.requiredAction === 'login') {
			return { action: { kind: 'relogin' }, label: '去登录' };
		}
		if (task.errorCode === 'ENGINE_CRASHED' || task.outcome === 'interrupted') {
			return { action: { kind: 'reconnect' }, label: '重试' };
		}
		if (task.lifecycleState === 'terminal' && task.outcome === 'success' && task.kind === 'podcast') {
			return { action: { kind: 'open' }, label: '打开' };
		}
		if (task.canRetry) {
			return { action: { kind: 'retry' }, label: '重试' };
		}
		return null;
	}

	function secondaryAction(task: TaskSnapshot): { action: Action; label: string } | null {
		if (task.canCancel && task.lifecycleState !== 'terminal') {
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
			if (!window.confirm('确定取消该任务？')) return;
			if (task.kind === 'podcast') onControlTask(task.id, 'cancel', task.revision);
			else onControlZhihuTask(task.id, 'cancel', task.revision);
		}
	}
</script>

<article
	class="task-row"
	class:running={isActive(task) && task.lifecycleState !== 'queued'}
	class:error={task.lifecycleState === 'terminal' &&
		(task.outcome === 'failed' || task.outcome === 'interrupted')}
	class:success={task.lifecycleState === 'terminal' && task.outcome === 'success'}
	aria-label={`${taskKindLabel(task.kind)} ${name} ${status}`}
>
	<span class="task-kind" class:zhihu={task.kind === 'zhihu'}>{taskKindLabel(task.kind)}</span>

	<div class="task-copy">
		<strong class="task-title" title={name}>{name}</strong>
		<small class="task-sub">{subtitle}</small>
	</div>

	<div
		class="task-progress"
		class:flowing
		class:determinate={percent !== null}
		class:wave-on={waveEnabled}
		aria-label={percent !== null ? `进度 ${Math.round(percent)}%` : '进行中'}
	>
		{#if percent !== null}
			<span class="task-fill-clip" style={`transform:scaleX(${percent / 100})`}>
				<i class="task-fill"></i>
				{#if waveEnabled}
					<svg class="task-wave" viewBox="0 0 120 8" preserveAspectRatio="none" aria-hidden="true">
						<path
							d="M0 4 Q 15 0 30 4 T 60 4 T 90 4 T 120 4 V8 H0 Z"
							fill="currentColor"
						/>
					</svg>
				{/if}
			</span>
		{:else if waveEnabled}
			<svg class="task-wave full" viewBox="0 0 120 8" preserveAspectRatio="none" aria-hidden="true">
				<path d="M0 4 Q 15 0 30 4 T 60 4 T 90 4 T 120 4 V8 H0 Z" fill="currentColor" />
			</svg>
		{:else if flowing}
			<i class="task-flow" aria-hidden="true"></i>
		{/if}
	</div>

	{#if percent !== null}
		<output class="task-pct">{Math.round(percent)}%</output>
	{:else if isActive(task)}
		<output class="task-pct task-ellipsis">…</output>
	{:else}
		<output class="task-pct task-idle">—</output>
	{/if}

	<div class="task-actions">
		{#if primary}
			<button type="button" class="task-btn primary" on:click={() => runAction(primary.action)}>
				{primary.label}
			</button>
		{/if}
		{#if secondary}
			<button type="button" class="task-btn danger" on:click={() => runAction(secondary.action)}>
				{secondary.label}
			</button>
		{/if}
	</div>

	{#if task.lifecycleState === 'terminal' && (task.outcome === 'failed' || task.outcome === 'interrupted') && friendlyError}
		<p class="task-hint">{friendlyError}</p>
	{/if}
</article>
