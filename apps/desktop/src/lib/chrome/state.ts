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
	// Scroll/wheel events hit this reducer at pointer rate; when nothing is
	// pending the state object must come back as-is so stores keyed on
	// reference identity don't invalidate per event.
	if (!state.shouldScheduleHide && !state.shouldCancelHide) return state;
	return {
		...state,
		shouldScheduleHide: false,
		shouldCancelHide: false
	};
}

// Invariant relied on by the early exits below: a pending hide timer implies
// a visible chrome. shouldScheduleHide is only emitted while chromeVisible is
// true (chrome-leave / chrome-blur), and the host consumes that one-shot —
// the timer is cancelled by shouldCancelHide or fires apply-hide, both of
// which keep "hidden ⇒ no pending hide" true afterwards. So when a branch can
// prove chromeVisible === false (or focusedInChrome === true, which blocks
// scheduling entirely), dropping a redundant shouldCancelHide/shouldScheduleHide
// emission cannot strand a live timer.
export function reduceChrome(state: ChromeState, event: ChromeEvent): ChromeState {
	const base = clearOneShots(state);

	switch (event.type) {
		case 'enter-surface': {
			const surface = event.surface;
			if (
				surface === base.surface &&
				base.chromeVisible === initialChromeVisible(surface) &&
				!base.focusedInChrome
			) {
				// Same surface, same derived visibility — the only remaining
				// effect would be a redundant cancel one-shot.
				return { ...base, shouldCancelHide: true };
			}
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
			// Already hidden: nothing changes, and no hide timer can be pending.
			if (!base.chromeVisible) return base;
			return {
				...base,
				chromeVisible: false,
				shouldCancelHide: true
			};
		}

		case 'top-edge-enter': {
			if (!isImmersiveSurface(base.surface)) return base;
			// Already visible with focus inside chrome: no hide timer can be
			// pending (focus blocks scheduling), so the reveal is a no-op.
			if (base.chromeVisible && base.focusedInChrome) return base;
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
			// Already focused: no hide timer can be pending, so the cancel
			// one-shot would be redundant.
			if (base.focusedInChrome) return base;
			return {
				...base,
				focusedInChrome: true,
				shouldCancelHide: true
			};
		}

		case 'chrome-blur': {
			if (isImmersiveSurface(base.surface) && base.chromeVisible) {
				return { ...base, focusedInChrome: false, shouldScheduleHide: true };
			}
			if (!base.focusedInChrome) return base;
			return { ...base, focusedInChrome: false };
		}

		case 'apply-hide': {
			if (!isImmersiveSurface(base.surface)) return base;
			if (base.focusedInChrome) return base;
			if (!base.chromeVisible) return base;
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
	// Focus is a reader mode, never a standalone surface. If a stale focus
	// flag survives while the file is closing, the visible bookshelf must not
	// inherit the reader's fixed/overlay chrome.
	if (input.focusMode && input.fileOpen) return 'focus';
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
		key === ' ' ||
		key === 'AudioVolumeDown' ||
		key === 'AudioVolumeUp' ||
		key === 'VolumeDown' ||
		key === 'VolumeUp'
	);
}

export const FLOW_READING_MESSAGE_SOURCE = 'immersive-reader-flow' as const;
export const FLOW_READING_MESSAGE_VERSION = 1 as const;

export type FlowReadingActivityMessage = {
	source: typeof FLOW_READING_MESSAGE_SOURCE;
	version: typeof FLOW_READING_MESSAGE_VERSION;
	type: 'reading-activity';
};

/** Parent → iframe: apply the shared reader font scale. */
export type FlowSetFontScaleMessage = {
	source: typeof FLOW_READING_MESSAGE_SOURCE;
	version: typeof FLOW_READING_MESSAGE_VERSION;
	type: 'set-font-scale';
	scale: number;
};

/** iframe → parent: report a user-driven scale change for persistence. */
export type FlowFontScaleChangeMessage = {
	source: typeof FLOW_READING_MESSAGE_SOURCE;
	version: typeof FLOW_READING_MESSAGE_VERSION;
	type: 'font-scale-change';
	scale: number;
};

/** iframe → parent: a key the reader did not consume — Escape with nothing
 * open inside, or F10 — so the shell's global shortcuts keep working while
 * the iframe holds keyboard focus. Only these keys are ever forwarded. */
export type FlowKeyDownMessage = {
	source: typeof FLOW_READING_MESSAGE_SOURCE;
	version: typeof FLOW_READING_MESSAGE_VERSION;
	type: 'key-down';
	key: 'Escape' | 'F10';
};

/** Parent → iframe: wide layout when the shell is maximized/fullscreen. */
export type FlowSetLayoutModeMessage = {
	source: typeof FLOW_READING_MESSAGE_SOURCE;
	version: typeof FLOW_READING_MESSAGE_VERSION;
	type: 'set-layout-mode';
	wide: boolean;
	contentMaxWidth: number;
};

/** iframe → parent: reader scripts finished binding the message listener —
 * the shell re-sends font scale / layout mode so no early post is lost. */
export type FlowReaderReadyMessage = {
	source: typeof FLOW_READING_MESSAGE_SOURCE;
	version: typeof FLOW_READING_MESSAGE_VERSION;
	type: 'reader-ready';
};

/** Parent → iframe: apply the active app theme (vars already translated to
 * the reader template's token names by theme/themes.ts flowThemeVars). */
export type FlowSetThemeMessage = {
	source: typeof FLOW_READING_MESSAGE_SOURCE;
	version: typeof FLOW_READING_MESSAGE_VERSION;
	type: 'set-theme';
	scheme: 'light' | 'dark';
	vars: Record<string, string>;
};

export type FlowBridgeMessage =
	| FlowReadingActivityMessage
	| FlowSetFontScaleMessage
	| FlowFontScaleChangeMessage
	| FlowSetLayoutModeMessage
	| FlowKeyDownMessage
	| FlowReaderReadyMessage
	| FlowSetThemeMessage;

/** Wide column cap when the desktop window is maximized or fullscreen. */
export const WIDE_LAYOUT_MAX_WIDTH_PX = 1120;

export function createFlowReadingActivityMessage(): FlowReadingActivityMessage {
	return {
		source: FLOW_READING_MESSAGE_SOURCE,
		version: FLOW_READING_MESSAGE_VERSION,
		type: 'reading-activity'
	};
}

export function createFlowSetFontScaleMessage(scale: number): FlowSetFontScaleMessage {
	return {
		source: FLOW_READING_MESSAGE_SOURCE,
		version: FLOW_READING_MESSAGE_VERSION,
		type: 'set-font-scale',
		scale
	};
}

export function createFlowFontScaleChangeMessage(scale: number): FlowFontScaleChangeMessage {
	return {
		source: FLOW_READING_MESSAGE_SOURCE,
		version: FLOW_READING_MESSAGE_VERSION,
		type: 'font-scale-change',
		scale
	};
}

export function createFlowSetLayoutModeMessage(
	wide: boolean,
	contentMaxWidth: number = WIDE_LAYOUT_MAX_WIDTH_PX
): FlowSetLayoutModeMessage {
	return {
		source: FLOW_READING_MESSAGE_SOURCE,
		version: FLOW_READING_MESSAGE_VERSION,
		type: 'set-layout-mode',
		wide,
		contentMaxWidth: Math.max(480, Math.round(contentMaxWidth))
	};
}

export function createFlowSetThemeMessage(
	scheme: 'light' | 'dark',
	vars: Record<string, string>
): FlowSetThemeMessage {
	return {
		source: FLOW_READING_MESSAGE_SOURCE,
		version: FLOW_READING_MESSAGE_VERSION,
		type: 'set-theme',
		scheme,
		vars
	};
}

function isFlowEnvelope(data: unknown): data is Record<string, unknown> {
	if (typeof data !== 'object' || data === null) return false;
	const record = data as Record<string, unknown>;
	return (
		record.source === FLOW_READING_MESSAGE_SOURCE &&
		record.version === FLOW_READING_MESSAGE_VERSION &&
		typeof record.type === 'string'
	);
}

export function isFlowReadingActivityMessage(
	data: unknown
): data is FlowReadingActivityMessage {
	return isFlowEnvelope(data) && data.type === 'reading-activity';
}

export function isFlowSetFontScaleMessage(data: unknown): data is FlowSetFontScaleMessage {
	if (!isFlowEnvelope(data) || data.type !== 'set-font-scale') return false;
	return typeof data.scale === 'number' && Number.isFinite(data.scale);
}

export function isFlowFontScaleChangeMessage(
	data: unknown
): data is FlowFontScaleChangeMessage {
	if (!isFlowEnvelope(data) || data.type !== 'font-scale-change') return false;
	return typeof data.scale === 'number' && Number.isFinite(data.scale);
}

export function isFlowSetLayoutModeMessage(data: unknown): data is FlowSetLayoutModeMessage {
	if (!isFlowEnvelope(data) || data.type !== 'set-layout-mode') return false;
	return (
		typeof data.wide === 'boolean' &&
		typeof data.contentMaxWidth === 'number' &&
		Number.isFinite(data.contentMaxWidth)
	);
}

export function isFlowKeyDownMessage(data: unknown): data is FlowKeyDownMessage {
	if (!isFlowEnvelope(data) || data.type !== 'key-down') return false;
	// Whitelist — the iframe must not be able to inject arbitrary keys.
	return data.key === 'Escape' || data.key === 'F10';
}

export function isFlowReaderReadyMessage(data: unknown): data is FlowReaderReadyMessage {
	return isFlowEnvelope(data) && data.type === 'reader-ready';
}

/** Accept only local reader origins for the flow iframe message bridge.
 *  'null' (file:// / opaque origins) is never legitimate here — the bridge
 *  iframe is always served over http://127.0.0.1 by the reader server. */
export function isAllowedFlowMessageOrigin(origin: string): boolean {
	try {
		const host = new URL(origin).hostname;
		return host === '127.0.0.1' || host === 'localhost';
	} catch {
		return false;
	}
}
