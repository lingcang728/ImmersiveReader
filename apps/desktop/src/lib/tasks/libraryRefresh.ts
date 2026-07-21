import type { TaskEvent } from './sync';

export function taskEventPublishesToLibrary(event: TaskEvent): boolean {
	const snapshot = event.snapshot;
	return snapshot.kind === 'zhihu'
		&& snapshot.lifecycleState === 'terminal'
		&& (snapshot.outcome === 'success' || snapshot.outcome === 'partial_success');
}
