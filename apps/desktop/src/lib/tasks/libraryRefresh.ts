import type { TaskEvent } from './sync';

// 05-F17: publish is part of the podcast pipeline — a terminal success/partial
// means the book already landed, so the shelf must refresh right away instead
// of waiting for open_task_result's manual refresh or the next restart.
export function taskEventPublishesToLibrary(event: TaskEvent): boolean {
	const snapshot = event.snapshot;
	return snapshot.lifecycleState === 'terminal'
		&& (snapshot.outcome === 'success' || snapshot.outcome === 'partial_success');
}
