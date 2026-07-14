import type { TaskSnapshot } from './sync';

const RECENT_TERMINAL_MS = 2 * 60 * 60 * 1000;

function isTerminal(task: Pick<TaskSnapshot, 'lifecycleState'>): boolean {
	return task.lifecycleState === 'terminal';
}

function parseTime(value: string): number {
	const ms = Date.parse(value);
	return Number.isFinite(ms) ? ms : 0;
}

/**
 * Default: active work + terminals from this session.
 * Expanded: full list (backend already prunes >7 days).
 */
export function partitionTaskQueue(
	tasks: readonly TaskSnapshot[],
	expanded: boolean,
	nowMs: number = Date.now()
): {
	readonly primary: readonly TaskSnapshot[];
	readonly historyCount: number;
} {
	if (tasks.length === 0) {
		return { primary: [], historyCount: 0 };
	}
	if (expanded) {
		return { primary: tasks, historyCount: 0 };
	}

	const active = tasks.filter((task) => !isTerminal(task));
	const terminal = tasks.filter((task) => isTerminal(task));

	if (active.length > 0) {
		const sessionStart = Math.min(...active.map((task) => parseTime(task.createdAt)));
		// Include terminals that finished during/after this active batch.
		const sessionTerminal = terminal.filter(
			(task) => parseTime(task.updatedAt) >= sessionStart - 60_000
		);
		const primary = [...active, ...sessionTerminal].sort((left, right) =>
			right.updatedAt.localeCompare(left.updatedAt)
		);
		return {
			primary,
			historyCount: Math.max(0, terminal.length - sessionTerminal.length)
		};
	}

	// No active work: only keep terminals from the last couple of hours.
	const recent = terminal.filter((task) => nowMs - parseTime(task.updatedAt) <= RECENT_TERMINAL_MS);
	return {
		primary: recent,
		historyCount: Math.max(0, terminal.length - recent.length)
	};
}

/** Prefer a real overall percent; ignore indeterminate / missing. */
export function displayTaskPercent(task: TaskSnapshot): number | null {
	if (task.lifecycleState === 'terminal' && task.outcome === 'success') return 100;
	if (task.lifecycleState === 'terminal' && task.outcome === 'failed') {
		const p = task.progress.percent;
		return p !== undefined && p !== null ? Math.max(0, Math.min(100, p)) : null;
	}
	if (task.progress.mode === 'determinate' && task.progress.percent !== undefined) {
		return Math.max(0, Math.min(100, task.progress.percent));
	}
	return null;
}

export function taskDisplayTitle(task: TaskSnapshot): string {
	const name = (task.displayName || '').trim();
	if (name) return name;
	return task.kind === 'podcast' ? '播客任务' : '知乎任务';
}
