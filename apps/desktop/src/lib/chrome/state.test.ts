import { afterEach, describe, expect, it, vi } from 'vitest';
import {
	createChromeState,
	createFlowReadingActivityMessage,
	deriveChromeSurface,
	isFlowReadingActivityMessage,
	isReadingActivityKey,
	reduceChrome
} from './state';
import type { ChromeEvent, ChromeState } from './state';

function apply(state: ChromeState, events: ChromeEvent[]): ChromeState {
	return events.reduce((current, event) => reduceChrome(current, event), state);
}

function enterSurface(surface: ChromeState['surface']): ChromeEvent {
	return { type: 'enter-surface', surface };
}

function readingActivity(): ChromeEvent {
	return { type: 'reading-activity' };
}

function topEdgeEnter(): ChromeEvent {
	return { type: 'top-edge-enter' };
}

function chromeLeave(): ChromeEvent {
	return { type: 'chrome-leave' };
}

function chromeFocus(): ChromeEvent {
	return { type: 'chrome-focus' };
}

function chromeBlur(): ChromeEvent {
	return { type: 'chrome-blur' };
}

function applyHide(): ChromeEvent {
	return { type: 'apply-hide' };
}

describe('chrome state machine', () => {
	it('keeps library and workflow chrome always visible', () => {
		let state = createChromeState('library');
		expect(state.chromeVisible).toBe(true);
		state = apply(state, [readingActivity(), topEdgeEnter()]);
		expect(state.chromeVisible).toBe(true);

		state = apply(state, [enterSurface('workflow')]);
		expect(state.chromeVisible).toBe(true);
		state = apply(state, [readingActivity()]);
		expect(state.chromeVisible).toBe(true);
	});

	it('does not pin chrome for temporary/double-click markdown opens', () => {
		// open as markdown (includes double-click / isTemporaryReading paths)
		let state = createChromeState('markdown');
		expect(state.chromeVisible).toBe(true);

		// reading activity hides — temporary files share the same machine
		state = apply(state, [readingActivity()]);
		expect(state.chromeVisible).toBe(false);

		// only top-edge re-shows
		state = apply(state, [readingActivity()]);
		expect(state.chromeVisible).toBe(false);
		state = apply(state, [topEdgeEnter()]);
		expect(state.chromeVisible).toBe(true);
	});

	it('never reveals chrome on scroll/keyboard activity (including scroll-up)', () => {
		let state = createChromeState('markdown');
		state = apply(state, [readingActivity()]);
		expect(state.chromeVisible).toBe(false);

		// Repeated activity stays hidden — no scroll-up reveal path exists
		for (let i = 0; i < 5; i++) {
			state = apply(state, [readingActivity()]);
			expect(state.chromeVisible).toBe(false);
		}
	});

	it('only top-edge events reveal immersive chrome', () => {
		let state = createChromeState('markdown');
		state = apply(state, [readingActivity()]);
		expect(state.chromeVisible).toBe(false);

		state = apply(state, [chromeLeave()]);
		expect(state.chromeVisible).toBe(false);

		state = apply(state, [topEdgeEnter()]);
		expect(state.chromeVisible).toBe(true);
	});

	it('hides chrome immediately on enter for focus and flow', () => {
		let state = createChromeState('library');
		state = apply(state, [enterSurface('focus')]);
		expect(state.surface).toBe('focus');
		expect(state.chromeVisible).toBe(false);

		state = apply(state, [enterSurface('flow')]);
		expect(state.surface).toBe('flow');
		expect(state.chromeVisible).toBe(false);

		// top edge temporary reveal
		state = apply(state, [topEdgeEnter()]);
		expect(state.chromeVisible).toBe(true);
		state = apply(state, [readingActivity()]);
		expect(state.chromeVisible).toBe(false);
	});

	it('restores correct surface visibility on exit', () => {
		let state = createChromeState('focus');
		expect(state.chromeVisible).toBe(false);

		// exit focus → markdown (file still open)
		state = apply(state, [enterSurface('markdown')]);
		expect(state.chromeVisible).toBe(true);

		// exit flow → library
		state = apply(state, [enterSurface('flow')]);
		expect(state.chromeVisible).toBe(false);
		state = apply(state, [enterSurface('library')]);
		expect(state.chromeVisible).toBe(true);
	});

	it('schedules hide on chrome leave but not when focused inside chrome', () => {
		let state = createChromeState('markdown');
		expect(state.chromeVisible).toBe(true);

		state = apply(state, [chromeLeave()]);
		expect(state.shouldScheduleHide).toBe(true);
		expect(state.chromeVisible).toBe(true);

		state = apply(state, [chromeFocus()]);
		expect(state.focusedInChrome).toBe(true);
		expect(state.shouldCancelHide).toBe(true);

		state = apply(state, [chromeLeave()]);
		expect(state.shouldScheduleHide).toBe(false);

		state = apply(state, [applyHide()]);
		// still focused — refuse hide
		expect(state.chromeVisible).toBe(true);

		state = apply(state, [chromeBlur()]);
		expect(state.shouldScheduleHide).toBe(true);
		state = apply(state, [applyHide()]);
		expect(state.chromeVisible).toBe(false);
	});

	it('cancels pending hide when reading activity fires (no duplicate timer semantics)', () => {
		let state = createChromeState('markdown');
		state = apply(state, [chromeLeave()]);
		expect(state.shouldScheduleHide).toBe(true);

		// activity immediately hides and cancels pending timer
		state = apply(state, [readingActivity()]);
		expect(state.chromeVisible).toBe(false);
		expect(state.shouldCancelHide).toBe(true);
		expect(state.shouldScheduleHide).toBe(false);

		// high-frequency activity does not re-schedule hide timers
		for (let i = 0; i < 20; i++) {
			state = apply(state, [readingActivity()]);
			expect(state.shouldScheduleHide).toBe(false);
			expect(state.chromeVisible).toBe(false);
		}
	});

	it('deriveChromeSurface prefers flow over focus over markdown', () => {
		expect(
			deriveChromeSurface({ flowActive: true, focusMode: true, fileOpen: true })
		).toBe('flow');
		expect(
			deriveChromeSurface({ flowActive: false, focusMode: true, fileOpen: true })
		).toBe('focus');
		expect(
			deriveChromeSurface({ flowActive: false, focusMode: false, fileOpen: true })
		).toBe('markdown');
		expect(
			deriveChromeSurface({
				flowActive: false,
				focusMode: false,
				fileOpen: false,
				workflowOpen: true
			})
		).toBe('workflow');
		expect(
			deriveChromeSurface({ flowActive: false, focusMode: false, fileOpen: false })
		).toBe('library');
	});

	it('classifies reading activity keys', () => {
		expect(isReadingActivityKey('ArrowDown')).toBe(true);
		expect(isReadingActivityKey('PageUp')).toBe(true);
		expect(isReadingActivityKey(' ')).toBe(true);
		expect(isReadingActivityKey('Escape')).toBe(false);
		expect(isReadingActivityKey('a')).toBe(false);
	});

	it('validates flow reading-activity messages', () => {
		const msg = createFlowReadingActivityMessage();
		expect(msg).toEqual({
			source: 'immersive-reader-flow',
			version: 1,
			type: 'reading-activity'
		});
		expect(isFlowReadingActivityMessage(msg)).toBe(true);
		expect(
			isFlowReadingActivityMessage({
				source: 'other',
				version: 1,
				type: 'reading-activity'
			})
		).toBe(false);
		expect(
			isFlowReadingActivityMessage({
				source: 'immersive-reader-flow',
				version: 2,
				type: 'reading-activity'
			})
		).toBe(false);
		expect(isFlowReadingActivityMessage(null)).toBe(false);
	});
});

describe('chrome hide timer host pattern', () => {
	afterEach(() => {
		vi.useRealTimers();
	});

	it('does not create duplicate hide timers when host cancels on each reading-activity', () => {
		vi.useFakeTimers();
		let state = createChromeState('markdown');
		let hideTimer: ReturnType<typeof setTimeout> | null = null;
		let hideCount = 0;

		const dispatch = (event: ChromeEvent) => {
			state = reduceChrome(state, event);
			if (state.shouldCancelHide && hideTimer) {
				clearTimeout(hideTimer);
				hideTimer = null;
			}
			if (state.shouldScheduleHide && !hideTimer) {
				hideTimer = setTimeout(() => {
					hideTimer = null;
					hideCount += 1;
					state = reduceChrome(state, applyHide());
				}, 350);
			}
		};

		dispatch(topEdgeEnter());
		dispatch(chromeLeave());
		expect(hideTimer).not.toBeNull();

		// burst of reading activity should cancel and not stack timers
		for (let i = 0; i < 30; i++) {
			dispatch(readingActivity());
		}
		expect(hideTimer).toBeNull();
		expect(state.chromeVisible).toBe(false);

		vi.advanceTimersByTime(1000);
		expect(hideCount).toBe(0);
	});
});
