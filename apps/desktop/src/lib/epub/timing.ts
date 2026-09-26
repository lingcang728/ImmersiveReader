// Small timer helpers — DOM-free so they can run under vitest.

/**
 * Trailing-edge throttle: calls at most once per `wait`, the final call
 * receiving the latest arguments. The returned function exposes `cancel()`
 * and `flush()` so callers can force a pending emit (e.g. on chapter change
 * or unmount) instead of losing the trailing update.
 */
export function throttled<Args extends unknown[]>(
	fn: (...args: Args) => void,
	wait: number,
): ((...args: Args) => void) & { cancel: () => void; flush: () => void } {
	let timer: ReturnType<typeof setTimeout> | null = null;
	let lastArgs: Args | null = null;
	let lastRun = 0;

	const run = () => {
		timer = null;
		lastRun = Date.now();
		const args = lastArgs;
		lastArgs = null;
		if (args) fn(...args);
	};

	const wrapped = (...args: Args) => {
		lastArgs = args;
		const elapsed = Date.now() - lastRun;
		if (elapsed >= wait && timer === null) {
			run();
		} else if (timer === null) {
			timer = setTimeout(run, wait - elapsed);
		}
	};

	wrapped.cancel = () => {
		if (timer !== null) clearTimeout(timer);
		timer = null;
		lastArgs = null;
	};

	wrapped.flush = () => {
		if (timer !== null) {
			clearTimeout(timer);
			run();
		} else if (lastArgs) {
			run();
		}
	};

	return wrapped;
}

/** Classic trailing debounce with `cancel()`/`flush()`. */
export function debounced<Args extends unknown[]>(
	fn: (...args: Args) => void,
	wait: number,
): ((...args: Args) => void) & { cancel: () => void; flush: () => void } {
	let timer: ReturnType<typeof setTimeout> | null = null;
	let lastArgs: Args | null = null;

	const wrapped = (...args: Args) => {
		lastArgs = args;
		if (timer !== null) clearTimeout(timer);
		timer = setTimeout(() => {
			timer = null;
			const callArgs = lastArgs;
			lastArgs = null;
			if (callArgs) fn(...callArgs);
		}, wait);
	};

	wrapped.cancel = () => {
		if (timer !== null) clearTimeout(timer);
		timer = null;
		lastArgs = null;
	};

	wrapped.flush = () => {
		if (timer === null) {
			lastArgs = null;
			return;
		}
		clearTimeout(timer);
		timer = null;
		const args = lastArgs;
		lastArgs = null;
		if (args) fn(...args);
	};

	return wrapped;
}
