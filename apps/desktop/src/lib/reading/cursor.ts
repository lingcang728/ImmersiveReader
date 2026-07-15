export const READING_CURSOR_HIDE_DELAY_MS = 1500;

export type ReadingCursorEvent =
	| { type: "enter-reading" }
	| { type: "leave-reading" }
	| { type: "pointer-move" }
	| { type: "keyboard" }
	| { type: "wheel" }
	| { type: "apply-hide" };

export interface ReadingCursorState {
	active: boolean;
	hidden: boolean;
	shouldScheduleHide: boolean;
	shouldCancelHide: boolean;
}

export function createReadingCursorState(active = false): ReadingCursorState {
	return {
		active,
		hidden: false,
		shouldScheduleHide: active,
		shouldCancelHide: false
	};
}

function clearOneShots(state: ReadingCursorState): ReadingCursorState {
	return {
		...state,
		shouldScheduleHide: false,
		shouldCancelHide: false
	};
}

export function reduceReadingCursor(
	state: ReadingCursorState,
	event: ReadingCursorEvent
): ReadingCursorState {
	const base = clearOneShots(state);

	switch (event.type) {
		case "enter-reading":
			return {
				...base,
				active: true,
				hidden: false,
				shouldScheduleHide: true,
				shouldCancelHide: true
			};

		case "leave-reading":
			return {
				...base,
				active: false,
				hidden: false,
				shouldCancelHide: true
			};

		case "pointer-move":
			if (!base.active) return base;
			return {
				...base,
				hidden: false,
				shouldScheduleHide: true,
				shouldCancelHide: true
			};

		case "keyboard":
		case "wheel":
			if (!base.active) return base;
			return {
				...base,
				hidden: true,
				shouldCancelHide: true
			};

		case "apply-hide":
			if (!base.active) return base;
			return {
				...base,
				hidden: true
			};

		default:
			return base;
	}
}
