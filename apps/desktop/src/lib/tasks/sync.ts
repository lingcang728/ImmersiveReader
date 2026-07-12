export type TaskKind = "podcast" | "zhihu";
export type LifecycleState =
	| "queued"
	| "starting"
	| "running"
	| "pausing"
	| "paused"
	| "stopping"
	| "terminal";
export type TaskOutcome =
	| "none"
	| "success"
	| "partial_success"
	| "failed"
	| "cancelled"
	| "interrupted";
export type RequiredAction =
	| "none"
	| "login"
	| "captcha"
	| "configure_secret"
	| "free_disk_space"
	| "approve_budget";

export type TaskProgress = {
	readonly mode: "indeterminate" | "determinate";
	readonly percent?: number;
	readonly completedUnits?: number;
	readonly totalUnits?: number;
	readonly label?: string;
};

export type TaskSnapshot = {
	readonly id: string;
	readonly kind: TaskKind;
	readonly revision: number;
	readonly lastSequence: number;
	readonly lifecycleState: LifecycleState;
	readonly outcome: TaskOutcome;
	readonly requiredAction: RequiredAction;
	readonly progress: TaskProgress;
	readonly errorCode?: string;
	readonly errorMessage?: string;
	readonly engineStage: string;
	readonly engineStatus: string;
	readonly recoverable: boolean;
	readonly canPause: boolean;
	readonly canResume: boolean;
	readonly canRetry: boolean;
	readonly canCancel: boolean;
	readonly bookId?: string;
	readonly sourceId?: string;
	readonly cacheLeaseBytes: number;
	readonly createdAt: string;
	readonly updatedAt: string;
};

export type TaskEvent = {
	readonly schemaVersion: number;
	readonly taskId: string;
	readonly sequence: number;
	readonly revision: number;
	readonly type: string;
	readonly snapshot: TaskSnapshot;
	readonly createdAt: string;
};

export type AcquisitionSnapshot = {
	readonly tasks: readonly TaskSnapshot[];
	readonly recoverableCacheBytes: number;
	readonly generatedAt: string;
};

export type TaskSyncState = {
	readonly tasks: ReadonlyMap<string, TaskSnapshot>;
};

export type TaskEventResult =
	| { readonly kind: "applied"; readonly state: TaskSyncState }
	| { readonly kind: "ignored"; readonly state: TaskSyncState }
	| { readonly kind: "refresh"; readonly state: TaskSyncState };

export function snapshotState(tasks: readonly TaskSnapshot[]): TaskSyncState {
	return { tasks: new Map(tasks.map((task) => [task.id, task])) };
}

export function taskList(state: TaskSyncState): readonly TaskSnapshot[] {
	return [...state.tasks.values()].sort((left, right) =>
		right.updatedAt.localeCompare(left.updatedAt)
	);
}

export function applyTaskEvent(state: TaskSyncState, event: TaskEvent): TaskEventResult {
	if (
		event.schemaVersion !== 1 ||
		event.taskId !== event.snapshot.id ||
		event.sequence !== event.snapshot.lastSequence ||
		event.revision !== event.snapshot.revision
	) {
		return { kind: "refresh", state };
	}
	const current = state.tasks.get(event.taskId);
	if (!current) {
		if (event.sequence !== 1 || event.revision !== 1) {
			return { kind: "refresh", state };
		}
		const tasks = new Map(state.tasks);
		tasks.set(event.taskId, event.snapshot);
		return { kind: "applied", state: { tasks } };
	}
	if (event.sequence <= current.lastSequence || event.revision <= current.revision) {
		return { kind: "ignored", state };
	}
	if (event.sequence !== current.lastSequence + 1) {
		return { kind: "refresh", state };
	}
	const tasks = new Map(state.tasks);
	tasks.set(event.taskId, event.snapshot);
	return { kind: "applied", state: { tasks } };
}
