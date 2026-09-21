<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import type { TaskSnapshot } from '$lib/tasks/sync';
	import { displayTaskPercent, taskDisplayTitle } from '$lib/tasks/queueList';
	import WorkflowDialogShell from './WorkflowDialogShell.svelte';

	export let task: TaskSnapshot;
	export let onStartTask: (taskId: string) => void;
	export let onStartZhihuTask: (taskId: string, revision: number) => void;
	export let onOpenTaskResult: (taskId: string) => void;
	export let onRestartTask: (taskId: string) => void;
	export let onApproveBudget: (taskId: string, budgetLimitCny: number) => void;
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
	onMount(() => {
		documentHidden = document.hidden;
		const handleVisibilityChange = () => {
			documentHidden = document.hidden;
		};
		document.addEventListener('visibilitychange', handleVisibilityChange);
		return () => document.removeEventListener('visibilitychange', handleVisibilityChange);
	});

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
		// 05-F5: high-value codes that used to fall through to the generic
		// "处理失败，可点击重试" — several are deterministic and a retry would
		// just burn the same failure again.
		if (code === 'UPSTREAM_UNAUTHORIZED') return 'DeepSeek 密钥无效或已过期，请重新配置';
		if (code === 'INSUFFICIENT_DISK' || code === 'LOCAL_IO') {
			return '磁盘空间不足或写入失败，请清理后重试';
		}
		if (code === 'MODEL_LOAD_FAILED') return '语音模型加载失败，请检查运行环境后重试';
		if (code === 'MODEL_INCOMPATIBLE' || code === 'PIPELINE_INCOMPATIBLE' || code === 'CONFIG_INCOMPATIBLE') {
			return '任务与当前引擎版本不兼容，请重新创建任务';
		}
		if (code === 'ENGINE_PROTOCOL_MISMATCH' || code === 'ENGINE_UNAVAILABLE') {
			return '转写引擎异常，请重启应用后重试';
		}
		if (code === 'ENGINE_BUSY') return '转写引擎忙，请稍后重试';
		if (code === 'INVALID_TASK_SPEC' || code === 'PROMPT_BUDGET_EXCEEDED') {
			return '任务参数无效，请重新创建任务';
		}
		if (code === 'LOCAL_NETWORK' || code === 'LOCAL_TIMEOUT' || code === 'UPSTREAM_TIMEOUT' || code === 'UPSTREAM_UNAVAILABLE') {
			return '网络连接异常，请检查网络后重试';
		}
		if (code === 'TRANSCRIPTION_FAILED') return '转写失败，可重试';
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
		| { kind: 'discard' }
		| { kind: 'approveBudget' }
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
		// Terminal approve_budget tasks are deliberately can_retry=false: a plain
		// retry must not silently spend past the approved ceiling. The dedicated
		// action asks for a new limit first.
		if (task.requiredAction === 'approve_budget' && task.kind === 'podcast') {
			return { action: { kind: 'approveBudget' }, label: '提高预算重试' };
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
		// 05-F4: failed/interrupted podcast tasks keep their cache lease (input
		// copy + 16kHz WAV + chunks, ≈230MB/h) forever because the lease is only
		// released at publish — offer the wired-but-unused cancel_and_discard
		// path. A user-cancelled row still holding its lease is offered too;
		// an already-discarded row (recoverable=false, canRetry=false) is not.
		if (
			task.kind === 'podcast' &&
			task.lifecycleState === 'terminal' &&
			(task.outcome === 'failed' ||
				task.outcome === 'interrupted' ||
				(task.outcome === 'cancelled' && (task.recoverable || task.canRetry)))
		) {
			return { action: { kind: 'discard' }, label: '丢弃缓存' };
		}
		return null;
	}

	// P2-39: the parent handlers are fire-and-forget (invoke promises are
	// discarded upstream), so a successful command is detected by the next
	// task snapshot — revision or lifecycle changes clear the busy flag; a
	// bounded timeout covers the failure path where nothing changes.
	const ACTION_TIMEOUT_MS = 10000;
	let actionBusy = false;
	let actionStamp = '';
	let actionTimer: ReturnType<typeof setTimeout> | undefined;

	$: if (actionBusy && `${task.revision}:${task.lifecycleState}` !== actionStamp) {
		actionBusy = false;
		if (actionTimer) {
			clearTimeout(actionTimer);
			actionTimer = undefined;
		}
	}

	onDestroy(() => {
		if (actionTimer) clearTimeout(actionTimer);
	});

	// P3-12: themed confirm instead of window.confirm — the cancel only
	// dispatches after the user confirms inside the dialog.
	let pendingCancelAction: Action = null;
	let pendingDiscardAction: Action = null;
	let pendingBudgetAction = false;
	let budgetInput = '';

	function runAction(action: Action) {
		if (!action) return;
		// 'open' is read-only navigation — no duplicate-submission risk, and a
		// successful open does not change the task snapshot (would pin busy).
		if (action.kind === 'open') {
			onOpenTaskResult(task.id);
			return;
		}
		if (actionBusy) return;
		if (action.kind === 'cancel') {
			pendingCancelAction = action;
			return;
		}
		if (action.kind === 'discard') {
			pendingDiscardAction = action;
			return;
		}
		if (action.kind === 'approveBudget') {
			budgetInput = '';
			pendingBudgetAction = true;
			return;
		}
		dispatchAction(action);
	}

	function confirmCancelAction() {
		const action = pendingCancelAction;
		pendingCancelAction = null;
		dispatchAction(action);
	}

	function confirmDiscardAction() {
		const action = pendingDiscardAction;
		pendingDiscardAction = null;
		dispatchAction(action);
	}

	// 05-F7: the worker's BUDGET_CONFIRMATION_REQUIRED fatal carries the
	// verified estimate floor ("…below the verified estimate 12.34 CNY").
	// Surface it and reject lower inputs — otherwise a retry below the floor
	// dies again at spec validation before doing any work.
	$: budgetFloorCny = (() => {
		const match = /verified estimate\s+([0-9]+(?:\.[0-9]+)?)/i.exec(task.errorMessage || '');
		return match ? Number.parseFloat(match[1]) : null;
	})();
	$: parsedBudgetLimit = Number.parseFloat(budgetInput);
	$: budgetValid =
		Number.isFinite(parsedBudgetLimit) &&
		parsedBudgetLimit > 0 &&
		(budgetFloorCny === null || parsedBudgetLimit + 1e-9 >= budgetFloorCny);

	function confirmBudgetAction() {
		if (!budgetValid) return;
		const limit = parsedBudgetLimit;
		pendingBudgetAction = false;
		armBusy();
		onApproveBudget(task.id, limit);
	}

	function armBusy() {
		actionBusy = true;
		actionStamp = `${task.revision}:${task.lifecycleState}`;
		actionTimer = setTimeout(() => {
			actionBusy = false;
			actionTimer = undefined;
		}, ACTION_TIMEOUT_MS);
	}

	function dispatchAction(action: Action) {
		if (!action) return;
		armBusy();
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
		if (action.kind === 'resume' || action.kind === 'relogin') {
			if (task.kind === 'podcast') onControlTask(task.id, 'resume', task.revision);
			else onControlZhihuTask(task.id, 'resume', task.revision);
			return;
		}
		// 'reconnect' (interrupted/crashed) and 'retry' (terminal failure) are the
		// same underlying operation: the worker/sidecar run is already dead, so
		// resume is meaningless — re-run via the task restart path.
		if (action.kind === 'retry' || action.kind === 'reconnect') {
			if (task.kind === 'podcast') onRestartTask(task.id);
			else onStartZhihuTask(task.id, task.revision);
			return;
		}
		if (action.kind === 'cancel') {
			if (task.kind === 'podcast') onControlTask(task.id, 'cancel', task.revision);
			else onControlZhihuTask(task.id, 'cancel', task.revision);
			return;
		}
		if (action.kind === 'discard') {
			onControlTask(task.id, 'cancel_and_discard', task.revision);
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
		role="progressbar"
		aria-label={percent !== null ? `进度 ${Math.round(percent)}%` : '进行中'}
		aria-valuemin={0}
		aria-valuemax={100}
		aria-valuenow={percent !== null ? Math.round(percent) : undefined}
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
		<output class="task-pct" aria-live="off">{Math.round(percent)}%</output>
	{:else if isActive(task)}
		<output class="task-pct task-ellipsis" aria-live="off">…</output>
	{:else}
		<output class="task-pct task-idle" aria-live="off">—</output>
	{/if}

	<div class="task-actions">
		{#if primary}
			<button
				type="button"
				class="task-btn primary"
				disabled={actionBusy}
				on:click={() => runAction(primary.action)}
			>
				{primary.label}
			</button>
		{/if}
		{#if secondary}
			<button
				type="button"
				class="task-btn danger"
				disabled={actionBusy}
				on:click={() => runAction(secondary.action)}
			>
				{secondary.label}
			</button>
		{/if}
	</div>

	{#if task.lifecycleState === 'terminal' && (task.outcome === 'failed' || task.outcome === 'interrupted') && friendlyError}
		<p class="task-hint">{friendlyError}</p>
	{/if}

	{#if pendingCancelAction}
		<WorkflowDialogShell
			titleId={`task-cancel-title-${task.id}`}
			descriptionId={`task-cancel-desc-${task.id}`}
			title="取消任务"
			description="确定取消该任务？"
			maxWidth="420px"
			onClose={() => (pendingCancelAction = null)}
		>
			<div slot="footer" class="task-confirm-actions">
				<button type="button" class="wf-quiet" on:click={() => (pendingCancelAction = null)}
					>暂不取消</button
				>
				<button type="button" class="wf-primary" on:click={confirmCancelAction}>取消任务</button>
			</div>
		</WorkflowDialogShell>
	{/if}

	{#if pendingDiscardAction}
		<WorkflowDialogShell
			titleId={`task-discard-title-${task.id}`}
			descriptionId={`task-discard-desc-${task.id}`}
			title="丢弃任务缓存"
			description="将删除该任务的输入副本与中间产物缓存；之后重试需要重新转写。确定丢弃？"
			maxWidth="420px"
			onClose={() => (pendingDiscardAction = null)}
		>
			<div slot="footer" class="task-confirm-actions">
				<button type="button" class="wf-quiet" on:click={() => (pendingDiscardAction = null)}
					>暂不丢弃</button
				>
				<button type="button" class="wf-primary" on:click={confirmDiscardAction}>丢弃缓存</button>
			</div>
		</WorkflowDialogShell>
	{/if}

	{#if pendingBudgetAction}
		<WorkflowDialogShell
			titleId={`task-budget-title-${task.id}`}
			descriptionId={`task-budget-desc-${task.id}`}
			title="提高预算重试"
			description={budgetFloorCny !== null
				? `该任务因超出 API 预算上限而停止。核验预估下限约 ¥${budgetFloorCny.toFixed(2)}，低于该值会再次失败。输入新的单任务预算上限（元）后将重新转写。`
				: '该任务因超出 API 预算上限而停止。输入新的单任务预算上限（元）后将重新转写。'}
			maxWidth="420px"
			onClose={() => (pendingBudgetAction = false)}
		>
			<label class="task-budget-field">
				<span
					>新的预算上限（元）{#if budgetFloorCny !== null}，不低于 ¥{budgetFloorCny.toFixed(
						2
					)}{/if}</span
				>
				<input
					type="number"
					min="0.01"
					step="0.1"
					placeholder="例如 2.0"
					bind:value={budgetInput}
				/>
			</label>
			<div slot="footer" class="task-confirm-actions">
				<button type="button" class="wf-quiet" on:click={() => (pendingBudgetAction = false)}
					>暂不重试</button
				>
				<button
					type="button"
					class="wf-primary"
					disabled={!budgetValid}
					on:click={confirmBudgetAction}>确认并重试</button
				>
			</div>
		</WorkflowDialogShell>
	{/if}
</article>

<style>
	.task-confirm-actions {
		display: flex;
		justify-content: flex-end;
		gap: 8px;
	}

	.task-budget-field {
		display: flex;
		flex-direction: column;
		gap: 6px;
		font-size: 13px;
	}

	.task-budget-field input {
		padding: 8px 10px;
		border: 1px solid var(--line, rgba(0, 0, 0, 0.18));
		border-radius: 8px;
		font-size: 14px;
		background: var(--bg, #fff);
		color: var(--text, #111);
	}
</style>
