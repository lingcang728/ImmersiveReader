import { describe, expect, it } from 'vitest';
import {
	TOAST_LIMIT,
	appendToast,
	removeToast,
	toastDuration,
	type ToastItem,
} from './ui';

function toast(id: number, kind: ToastItem['kind'] = 'info'): ToastItem {
	return { id, text: `toast-${id}`, kind };
}

describe('toast queue', () => {
	it('appends in order', () => {
		expect(appendToast([], toast(1)).map((item) => item.id)).toEqual([1]);
		expect(appendToast([toast(1)], toast(2)).map((item) => item.id)).toEqual([1, 2]);
	});

	it('keeps only the newest toasts', () => {
		const items = [toast(1), toast(2), toast(3)];
		expect(TOAST_LIMIT).toBe(3);
		expect(appendToast(items, toast(4)).map((item) => item.id)).toEqual([2, 3, 4]);
		expect(appendToast([toast(1)], toast(2), 1).map((item) => item.id)).toEqual([2]);
	});

	it('removes a toast by id and ignores unknown ids', () => {
		const items = [toast(1), toast(2)];
		expect(removeToast(items, 1).map((item) => item.id)).toEqual([2]);
		expect(removeToast(items, 99)).toHaveLength(2);
	});

	it('keeps error toasts on screen longer than informational ones', () => {
		expect(toastDuration('error')).toBeGreaterThan(toastDuration('info'));
		expect(toastDuration('success')).toBeGreaterThan(0);
	});
});
