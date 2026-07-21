import { describe, expect, it } from 'vitest';
import { taskEventPublishesToLibrary } from './libraryRefresh';
import type { TaskEvent, TaskSnapshot } from './sync';

function event(snapshot: Partial<TaskSnapshot>): TaskEvent {
	return {
		schemaVersion: 1,
		taskId: 'task-1',
		sequence: 2,
		revision: 2,
		type: 'engine_completed',
		createdAt: '2026-07-22T00:00:00Z',
		snapshot: {
			id: 'task-1',
			kind: 'zhihu',
			revision: 2,
			lastSequence: 2,
			lifecycleState: 'terminal',
			outcome: 'success',
			requiredAction: 'none',
			progress: { mode: 'determinate', percent: 100 },
			engineStage: 'content',
			engineStatus: 'success',
			recoverable: false,
			canPause: false,
			canResume: false,
			canRetry: false,
			canCancel: false,
			cacheLeaseBytes: 0,
			createdAt: '2026-07-22T00:00:00Z',
			updatedAt: '2026-07-22T00:00:00Z',
			...snapshot,
		},
	};
}

describe('taskEventPublishesToLibrary', () => {
	it('refreshes for complete and partial Zhihu publications', () => {
		expect(taskEventPublishesToLibrary(event({ outcome: 'success' }))).toBe(true);
		expect(taskEventPublishesToLibrary(event({ outcome: 'partial_success' }))).toBe(true);
	});

	it('does not refresh before the Zhihu publish is terminal', () => {
		expect(taskEventPublishesToLibrary(event({ lifecycleState: 'running', outcome: 'none' }))).toBe(false);
		expect(taskEventPublishesToLibrary(event({ outcome: 'failed' }))).toBe(false);
		expect(taskEventPublishesToLibrary(event({ kind: 'podcast' }))).toBe(false);
	});
});
