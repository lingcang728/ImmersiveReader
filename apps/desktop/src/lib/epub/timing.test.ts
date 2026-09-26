import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { debounced, throttled } from "./timing";

describe("throttled", () => {
	beforeEach(() => vi.useFakeTimers());
	afterEach(() => vi.useRealTimers());

	it("runs the first call immediately and coalesces the rest", () => {
		const fn = vi.fn();
		const t = throttled(fn, 500);
		t("a");
		t("b");
		t("c");
		expect(fn).toHaveBeenCalledTimes(1);
		vi.advanceTimersByTime(600);
		expect(fn).toHaveBeenCalledTimes(2);
		// trailing call uses the latest arguments
		expect(fn).toHaveBeenLastCalledWith("c");
	});

	it("cancel() drops a pending trailing call", () => {
		const fn = vi.fn();
		const t = throttled(fn, 500);
		t("a");
		t("b");
		t.cancel();
		vi.advanceTimersByTime(1000);
		expect(fn).toHaveBeenCalledTimes(1);
	});

	it("flush() forces the pending call now", () => {
		const fn = vi.fn();
		const t = throttled(fn, 500);
		t("a");
		t("b");
		t.flush();
		expect(fn).toHaveBeenCalledTimes(2);
		expect(fn).toHaveBeenLastCalledWith("b");
		vi.advanceTimersByTime(1000);
		expect(fn).toHaveBeenCalledTimes(2);
	});
});

describe("debounced", () => {
	beforeEach(() => vi.useFakeTimers());
	afterEach(() => vi.useRealTimers());

	it("fires once after quiet period with latest args", () => {
		const fn = vi.fn();
		const d = debounced(fn, 300);
		d("a");
		vi.advanceTimersByTime(200);
		d("b");
		vi.advanceTimersByTime(200);
		expect(fn).not.toHaveBeenCalled();
		vi.advanceTimersByTime(150);
		expect(fn).toHaveBeenCalledTimes(1);
		expect(fn).toHaveBeenLastCalledWith("b");
	});

	it("flush() emits immediately, cancel() drops", () => {
		const fn = vi.fn();
		const d = debounced(fn, 300);
		d("x");
		d.flush();
		expect(fn).toHaveBeenCalledTimes(1);
		d("y");
		d.cancel();
		vi.advanceTimersByTime(1000);
		expect(fn).toHaveBeenCalledTimes(1);
	});
});
