import { describe, expect, it } from "vitest";
import { applyTaskEvent, snapshotState, type TaskEvent, type TaskSnapshot } from "./sync";

function task(id: string, revision: number, lastSequence: number): TaskSnapshot {
	return {
		id,
		kind: "podcast",
		revision,
		lastSequence,
		lifecycleState: "running",
		outcome: "none",
		requiredAction: "none",
		progress: { mode: "indeterminate" },
		engineStage: "transcribe",
		engineStatus: "working",
		recoverable: true,
		canPause: true,
		canResume: false,
		canRetry: false,
		canCancel: true,
		cacheLeaseBytes: 42,
		createdAt: "2026-07-12T00:00:00Z",
		updatedAt: "2026-07-12T00:00:00Z"
	};
}

function event(snapshot: TaskSnapshot): TaskEvent {
	return {
		schemaVersion: 1,
		taskId: snapshot.id,
		sequence: snapshot.lastSequence,
		revision: snapshot.revision,
		type: "progress",
		snapshot,
		createdAt: snapshot.updatedAt
	};
}

describe("task event synchronization", () => {
	it("applies the next persisted event", () => {
		const state = snapshotState([task("task-1", 1, 1)]);

		const result = applyTaskEvent(state, event(task("task-1", 2, 2)));

		expect(result.kind).toBe("applied");
		expect(result.state.tasks.get("task-1")?.revision).toBe(2);
	});

	it("requests a snapshot when a sequence gap appears", () => {
		const state = snapshotState([task("task-1", 1, 1)]);

		const result = applyTaskEvent(state, event(task("task-1", 3, 3)));

		expect(result.kind).toBe("refresh");
		expect(result.state).toBe(state);
	});

	it("ignores a duplicated or stale event", () => {
		const state = snapshotState([task("task-1", 2, 2)]);

		const result = applyTaskEvent(state, event(task("task-1", 2, 2)));

		expect(result.kind).toBe("ignored");
		expect(result.state).toBe(state);
	});
});
