import { describe, expect, it } from 'vitest';
import type { TaskSnapshot } from './sync';
import {
	buildTaskQueueSignature,
	canDismissTaskQueue,
	isAllTasksTerminal,
	readDismissedTaskQueueSignature,
	shouldShowTaskQueue,
	TASK_QUEUE_DISMISSED_STORAGE_KEY,
	writeDismissedTaskQueueSignature
} from './queueVisibility';

function task(
	partial: Partial<TaskSnapshot> & Pick<TaskSnapshot, 'id' | 'lifecycleState' | 'outcome'>
): Pick<TaskSnapshot, 'id' | 'revision' | 'lifecycleState' | 'outcome'> {
	return {
		id: partial.id,
		revision: partial.revision ?? 1,
		lifecycleState: partial.lifecycleState,
		outcome: partial.outcome
	};
}

describe('task queue visibility', () => {
	it('builds a stable sorted signature from id/revision/lifecycle/outcome', () => {
		const signature = buildTaskQueueSignature([
			task({ id: 'b', revision: 2, lifecycleState: 'terminal', outcome: 'success' }),
			task({ id: 'a', revision: 1, lifecycleState: 'terminal', outcome: 'failed' })
		]);
		expect(signature).toBe(
			JSON.stringify([
				{ id: 'a', revision: 1, lifecycleState: 'terminal', outcome: 'failed' },
				{ id: 'b', revision: 2, lifecycleState: 'terminal', outcome: 'success' }
			])
		);
	});

	it('allows dismiss only when every task is terminal', () => {
		expect(
			canDismissTaskQueue([
				task({ id: 'a', lifecycleState: 'terminal', outcome: 'success' }),
				task({ id: 'b', lifecycleState: 'terminal', outcome: 'failed' })
			])
		).toBe(true);
		expect(
			canDismissTaskQueue([
				task({ id: 'a', lifecycleState: 'terminal', outcome: 'success' }),
				task({ id: 'b', lifecycleState: 'running', outcome: 'none' })
			])
		).toBe(false);
		expect(canDismissTaskQueue([])).toBe(false);
		expect(isAllTasksTerminal([])).toBe(false);
	});

	it('hides a dismissed terminal set and reappears when the set changes', () => {
		const terminal = [
			task({ id: 'a', revision: 1, lifecycleState: 'terminal', outcome: 'success' })
		];
		const signature = buildTaskQueueSignature(terminal);
		expect(shouldShowTaskQueue(terminal, null)).toBe(true);
		expect(shouldShowTaskQueue(terminal, signature)).toBe(false);

		// Same logical set after restart keeps the dismissed signature.
		expect(shouldShowTaskQueue(terminal, signature)).toBe(false);

		// New task forces re-open even if still terminal-only.
		const withNew = [
			...terminal,
			task({ id: 'b', revision: 1, lifecycleState: 'terminal', outcome: 'cancelled' })
		];
		expect(shouldShowTaskQueue(withNew, signature)).toBe(true);

		// Revision change forces re-open.
		const revised = [task({ id: 'a', revision: 2, lifecycleState: 'terminal', outcome: 'success' })];
		expect(shouldShowTaskQueue(revised, signature)).toBe(true);
	});

	it('never hides while any task is non-terminal', () => {
		const mixed = [
			task({ id: 'a', revision: 1, lifecycleState: 'terminal', outcome: 'success' }),
			task({ id: 'b', revision: 1, lifecycleState: 'queued', outcome: 'none' })
		];
		const signature = buildTaskQueueSignature(mixed);
		expect(shouldShowTaskQueue(mixed, signature)).toBe(true);
		expect(canDismissTaskQueue(mixed)).toBe(false);
	});

	it('persists dismissed signature through a storage stub (restart simulation)', () => {
		const store = new Map<string, string>();
		const storage = {
			getItem: (key: string) => store.get(key) ?? null,
			setItem: (key: string, value: string) => {
				store.set(key, value);
			}
		};
		const terminal = [
			task({ id: 'done-1', revision: 3, lifecycleState: 'terminal', outcome: 'success' })
		];
		const signature = buildTaskQueueSignature(terminal);
		writeDismissedTaskQueueSignature(signature, storage);
		expect(store.get(TASK_QUEUE_DISMISSED_STORAGE_KEY)).toBe(signature);
		expect(readDismissedTaskQueueSignature(storage)).toBe(signature);
		expect(shouldShowTaskQueue(terminal, readDismissedTaskQueueSignature(storage))).toBe(false);
	});
});
