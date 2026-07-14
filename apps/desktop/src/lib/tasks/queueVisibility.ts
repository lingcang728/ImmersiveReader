import type { LifecycleState, TaskOutcome, TaskSnapshot } from './sync';

export const TASK_QUEUE_DISMISSED_STORAGE_KEY = 'immersive-reader-task-queue-dismissed-v1';

export type TaskQueueSignaturePart = {
	readonly id: string;
	readonly revision: number;
	readonly lifecycleState: LifecycleState;
	readonly outcome: TaskOutcome;
};

/** Sorted signature of the current terminal task set for dismiss persistence. */
export function buildTaskQueueSignature(
	tasks: readonly Pick<TaskSnapshot, 'id' | 'revision' | 'lifecycleState' | 'outcome'>[]
): string {
	const parts: TaskQueueSignaturePart[] = [...tasks]
		.map((task) => ({
			id: task.id,
			revision: task.revision,
			lifecycleState: task.lifecycleState,
			outcome: task.outcome
		}))
		.sort((left, right) => {
			const byId = left.id.localeCompare(right.id);
			if (byId !== 0) return byId;
			return left.revision - right.revision;
		});
	return JSON.stringify(parts);
}

export function isAllTasksTerminal(
	tasks: readonly Pick<TaskSnapshot, 'lifecycleState'>[]
): boolean {
	return tasks.length > 0 && tasks.every((task) => task.lifecycleState === 'terminal');
}

/** Close button is only available when every task has reached a terminal state. */
export function canDismissTaskQueue(
	tasks: readonly Pick<TaskSnapshot, 'lifecycleState'>[]
): boolean {
	return isAllTasksTerminal(tasks);
}

/**
 * Whether the queue shell should be visible.
 * Active / non-terminal work always forces the queue open; a dismissed
 * terminal signature stays hidden until the set changes.
 */
export function shouldShowTaskQueue(
	tasks: readonly Pick<TaskSnapshot, 'id' | 'revision' | 'lifecycleState' | 'outcome'>[],
	dismissedSignature: string | null | undefined
): boolean {
	if (tasks.length === 0) return false;
	if (!isAllTasksTerminal(tasks)) return true;
	const signature = buildTaskQueueSignature(tasks);
	return signature !== (dismissedSignature ?? null);
}

export function readDismissedTaskQueueSignature(
	storage: Pick<Storage, 'getItem'> | null | undefined = typeof localStorage !== 'undefined'
		? localStorage
		: null
): string | null {
	if (!storage) return null;
	try {
		const raw = storage.getItem(TASK_QUEUE_DISMISSED_STORAGE_KEY);
		return raw && raw.length > 0 ? raw : null;
	} catch {
		return null;
	}
}

export function writeDismissedTaskQueueSignature(
	signature: string,
	storage: Pick<Storage, 'setItem'> | null | undefined = typeof localStorage !== 'undefined'
		? localStorage
		: null
): void {
	if (!storage) return;
	try {
		storage.setItem(TASK_QUEUE_DISMISSED_STORAGE_KEY, signature);
	} catch {
		// Quota / private mode — dismiss is best-effort only.
	}
}
