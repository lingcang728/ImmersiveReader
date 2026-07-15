import { describe, expect, it } from "vitest";
import {
	createReadingCursorState,
	reduceReadingCursor,
	READING_CURSOR_HIDE_DELAY_MS
} from "./cursor";

describe("reading cursor state", () => {
	it("starts a 1.5 second idle hide when reading begins", () => {
		expect(READING_CURSOR_HIDE_DELAY_MS).toBe(1500);

		const entered = reduceReadingCursor(createReadingCursorState(), {
			type: "enter-reading"
		});

		expect(entered.active).toBe(true);
		expect(entered.hidden).toBe(false);
		expect(entered.shouldScheduleHide).toBe(true);
		expect(entered.shouldCancelHide).toBe(true);

		const hidden = reduceReadingCursor(entered, { type: "apply-hide" });
		expect(hidden.hidden).toBe(true);
	});

	it("reveals on pointer movement and restarts the idle hide", () => {
		let state = reduceReadingCursor(createReadingCursorState(), { type: "enter-reading" });
		state = reduceReadingCursor(state, { type: "apply-hide" });
		state = reduceReadingCursor(state, { type: "pointer-move" });

		expect(state.hidden).toBe(false);
		expect(state.shouldScheduleHide).toBe(true);
		expect(state.shouldCancelHide).toBe(true);
	});

	it.each(["keyboard", "wheel"] as const)(
		"hides immediately on %s activity",
		(type) => {
			let state = reduceReadingCursor(createReadingCursorState(), { type: "enter-reading" });
			state = reduceReadingCursor(state, { type: "apply-hide" });
			state = reduceReadingCursor(state, { type });

			expect(state.hidden).toBe(true);
			expect(state.shouldScheduleHide).toBe(false);
			expect(state.shouldCancelHide).toBe(true);
		}
	);

	it("shows the cursor and cancels timers when leaving reading", () => {
		let state = reduceReadingCursor(createReadingCursorState(), { type: "enter-reading" });
		state = reduceReadingCursor(state, { type: "apply-hide" });
		state = reduceReadingCursor(state, { type: "leave-reading" });

		expect(state.active).toBe(false);
		expect(state.hidden).toBe(false);
		expect(state.shouldCancelHide).toBe(true);
	});
});
