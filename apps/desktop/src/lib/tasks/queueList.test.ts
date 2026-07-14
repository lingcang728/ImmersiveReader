import { describe, expect, it } from 'vitest';
import type { TaskSnapshot } from './sync';
import { displayTaskPercent, partitionTaskQueue, taskDisplayTitle } from './queueList';

function task(
	partial: Partial<TaskSnapshot> &
		Pick<TaskSnapshot, 'id' | 'lifecycleState' | 'outcome' | 'createdAt' | 'updatedAt'>
): TaskSnapshot {
	return {
		id: partial.id,
		kind: partial.kind ?? 'podcast',
		revision: partial.revision ?? 1,
		lastSequence: partial.lastSequence ?? 1,
		lifecycleState: partial.lifecycleState,
		outcome: partial.outcome,
		requiredAction: 'none',
		progress: partial.progress ?? { mode: 'indeterminate' },
		engineStage: partial.engineStage ?? 'queued',
		engineStatus: 'working',
		recoverable: true,
		canPause: false,
		canResume: false,
		canRetry: false,
		canCancel: true,
		displayName: partial.displayName,
		cacheLeaseBytes: 0,
		createdAt: partial.createdAt,
		updatedAt: partial.updatedAt
	};
}

describe('partitionTaskQueue', () => {
	it('shows only active work by default and hides older terminals', () => {
		const now = Date.parse('2026-07-14T12:00:00Z');
		const tasks = [
			task({
				id: 'old',
				lifecycleState: 'terminal',
				outcome: 'success',
				createdAt: '2026-07-13T10:00:00Z',
				updatedAt: '2026-07-13T11:00:00Z'
			}),
			task({
				id: 'run',
				lifecycleState: 'running',
				outcome: 'none',
				createdAt: '2026-07-14T11:50:00Z',
				updatedAt: '2026-07-14T11:55:00Z',
				engineStage: 'chunking'
			})
		];
		const { primary, historyCount } = partitionTaskQueue(tasks, false, now);
		expect(primary.map((item) => item.id)).toEqual(['run']);
		expect(historyCount).toBe(1);
	});

	it('expands to show full history', () => {
		const tasks = [
			task({
				id: 'a',
				lifecycleState: 'terminal',
				outcome: 'success',
				createdAt: '2026-07-13T10:00:00Z',
				updatedAt: '2026-07-13T11:00:00Z'
			}),
			task({
				id: 'b',
				lifecycleState: 'terminal',
				outcome: 'success',
				createdAt: '2026-07-14T10:00:00Z',
				updatedAt: '2026-07-14T10:30:00Z'
			})
		];
		const { primary, historyCount } = partitionTaskQueue(tasks, true, Date.parse('2026-07-14T12:00:00Z'));
		expect(primary).toHaveLength(2);
		expect(historyCount).toBe(0);
	});

	it('includes terminals completed in the same active session', () => {
		const now = Date.parse('2026-07-14T12:00:00Z');
		const tasks = [
			task({
				id: 'done',
				lifecycleState: 'terminal',
				outcome: 'success',
				createdAt: '2026-07-14T11:40:00Z',
				updatedAt: '2026-07-14T11:52:00Z'
			}),
			task({
				id: 'run',
				lifecycleState: 'running',
				outcome: 'none',
				createdAt: '2026-07-14T11:45:00Z',
				updatedAt: '2026-07-14T11:55:00Z'
			})
		];
		const { primary, historyCount } = partitionTaskQueue(tasks, false, now);
		expect(primary.map((item) => item.id).sort()).toEqual(['done', 'run']);
		expect(historyCount).toBe(0);
	});
});

describe('task display helpers', () => {
	it('prefers displayName for titles', () => {
		expect(
			taskDisplayTitle(
				task({
					id: '1',
					lifecycleState: 'running',
					outcome: 'none',
					createdAt: '2026-07-14T00:00:00Z',
					updatedAt: '2026-07-14T00:00:00Z',
					displayName: '商业就是这样 349'
				})
			)
		).toBe('商业就是这样 349');
	});

	it('reads determinate percent without inventing 100 for active work', () => {
		const running = task({
			id: '1',
			lifecycleState: 'running',
			outcome: 'none',
			createdAt: '2026-07-14T00:00:00Z',
			updatedAt: '2026-07-14T00:00:00Z',
			progress: { mode: 'determinate', percent: 26 }
		});
		expect(displayTaskPercent(running)).toBe(26);
		const indeterminate = task({
			id: '2',
			lifecycleState: 'running',
			outcome: 'none',
			createdAt: '2026-07-14T00:00:00Z',
			updatedAt: '2026-07-14T00:00:00Z',
			progress: { mode: 'indeterminate' }
		});
		expect(displayTaskPercent(indeterminate)).toBeNull();
	});
});
