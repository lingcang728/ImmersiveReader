/**
 * Pure immersion chrome state machine.
 * Single chromeVisible drives both the custom window bar and the context toolbar.
 */

export type ChromeSurface = 'library' | 'markdown' | 'focus' | 'flow' | 'workflow';

export type ChromeEvent =
	| { type: 'enter-surface'; surface: ChromeSurface }
	| { type: 'reading-activity' }
	| { type: 'top-edge-enter' }
	| { type: 'chrome-leave' }
	| { type: 'chrome-focus' }
	| { type: 'chrome-blur' }
	| { type: 'apply-hide' }
	| { type: 'cancel-hide' };

export interface ChromeState {
	surface: ChromeSurface;
	chromeVisible: boolean;
	focusedInChrome: boolean;
	/** One-shot: host should start the leave-delay timer. */
	shouldScheduleHide: boolean;
	/** One-shot: host should clear any pending hide timer. */
	shouldCancelHide: boolean;
}

export const CHROME_HEIGHT_PX = 36;
export const CHROME_TOP_EDGE_PX = 10;
export const CHROME_HIDE_DELAY_MS = 350;
export const CHROME_ANIMATION_MS = 200;

export function isAlwaysVisibleSurface(surface: ChromeSurface): boolean {
	return surface === 'library' || surface === 'workflow';
}

export function isImmersiveSurface(surface: ChromeSurface): boolean {
	return surface === 'markdown' || surface === 'focus' || surface === 'flow';
}

export function isOverlaySurface(surface: ChromeSurface): boolean {
	return isImmersiveSurface(surface);
}

export function initialChromeVisible(surface: ChromeSurface): boolean {
	if (isAlwaysVisibleSurface(surface)) return true;
	if (surface === 'focus' || surface === 'flow') return false;
	return true; // markdown starts with chrome shown
}

export function createChromeState(surface: ChromeSurface = 'library'): ChromeState {
	return {
		surface,
		chromeVisible: initialChromeVisible(surface),
		focusedInChrome: false,
		shouldScheduleHide: false,
		shouldCancelHide: false
	};
}

function clearOneShots(state: ChromeState): ChromeState {
	return {
		...state,
		shouldScheduleHide: false,
		shouldCancelHide: false
	};
}

export function reduceChrome(state: ChromeState, event: ChromeEvent): ChromeState {
	const base = clearOneShots(state);

	switch (event.type) {
		case 'enter-surface': {
			const surface = event.surface;
			return {
				...base,
				surface,
				chromeVisible: initialChromeVisible(surface),
				focusedInChrome: false,
				shouldCancelHide: true
			};
		}

		case 'reading-activity': {
			// Scroll up/down, wheel, arrow keys, PageUp/PageDown — hide immediately.
			// Never use scroll direction to reveal chrome.
			if (!isImmersiveSurface(base.surface)) return base;
			return {
				...base,
				chromeVisible: false,
				shouldCancelHide: true
			};
		}

		case 'top-edge-enter': {
			if (!isImmersiveSurface(base.surface)) return base;
			return {
				...base,
				chromeVisible: true,
				shouldCancelHide: true
			};
		}

		case 'chrome-leave': {
			if (!isImmersiveSurface(base.surface)) return base;
			if (!base.chromeVisible) return base;
			if (base.focusedInChrome) return base;
			return {
				...base,
				shouldScheduleHide: true
			};
		}

		case 'chrome-focus': {
			return {
				...base,
				focusedInChrome: true,
				shouldCancelHide: true
			};
		}

		case 'chrome-blur': {
			const next = { ...base, focusedInChrome: false };
			if (isImmersiveSurface(next.surface) && next.chromeVisible) {
				return { ...next, shouldScheduleHide: true };
			}
			return next;
		}

		case 'apply-hide': {
			if (!isImmersiveSurface(base.surface)) return base;
			if (base.focusedInChrome) return base;
			return {
				...base,
				chromeVisible: false
			};
		}

		case 'cancel-hide': {
			return {
				...base,
				shouldCancelHide: true
			};
		}

		default:
			return base;
	}
}

/** Derive surface from app mode flags (pure helper for hosts / tests). */
export function deriveChromeSurface(input: {
	flowActive: boolean;
	focusMode: boolean;
	fileOpen: boolean;
	workflowOpen?: boolean;
}): ChromeSurface {
	if (input.flowActive) return 'flow';
	if (input.focusMode) return 'focus';
	if (input.fileOpen) return 'markdown';
	if (input.workflowOpen) return 'workflow';
	return 'library';
}

/** Keyboard keys that count as immersive reading activity. */
export function isReadingActivityKey(key: string): boolean {
	return (
		key === 'ArrowDown' ||
		key === 'ArrowUp' ||
		key === 'ArrowLeft' ||
		key === 'ArrowRight' ||
		key === 'PageDown' ||
		key === 'PageUp' ||
		key === 'Home' ||
		key === 'End' ||
		key === ' '
	);
}

export const FLOW_READING_MESSAGE_SOURCE = 'immersive-reader-flow' as const;
export const FLOW_READING_MESSAGE_VERSION = 1 as const;

export type FlowReadingActivityMessage = {
	source: typeof FLOW_READING_MESSAGE_SOURCE;
	version: typeof FLOW_READING_MESSAGE_VERSION;
	type: 'reading-activity';
};

export function createFlowReadingActivityMessage(): FlowReadingActivityMessage {
	return {
		source: FLOW_READING_MESSAGE_SOURCE,
		version: FLOW_READING_MESSAGE_VERSION,
		type: 'reading-activity'
	};
}

export function isFlowReadingActivityMessage(
	data: unknown
): data is FlowReadingActivityMessage {
	if (typeof data !== 'object' || data === null) return false;
	const record = data as Record<string, unknown>;
	return (
		record.source === FLOW_READING_MESSAGE_SOURCE &&
		record.version === FLOW_READING_MESSAGE_VERSION &&
		record.type === 'reading-activity'
	);
}
