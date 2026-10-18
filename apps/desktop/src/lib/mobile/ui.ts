// Shared shell UI state helpers (toast queue) for the mobile surface.
//
// Pure module: no DOM/Svelte imports so the queue rules are unit-testable.

export type ToastKind = 'info' | 'success' | 'error';

export interface ToastItem {
	id: number;
	text: string;
	kind: ToastKind;
}

/** At most this many toasts are visible; older ones drop off the top. */
export const TOAST_LIMIT = 3;

const TOAST_DURATION_MS: Record<ToastKind, number> = {
	info: 2600,
	success: 2200,
	error: 4000,
};

export function toastDuration(kind: ToastKind): number {
	return TOAST_DURATION_MS[kind] ?? TOAST_DURATION_MS.info;
}

/** Append a toast, keeping the newest `limit` entries. */
export function appendToast(
	items: readonly ToastItem[],
	item: ToastItem,
	limit: number = TOAST_LIMIT,
): ToastItem[] {
	const safeLimit = Number.isFinite(limit) ? Math.max(1, Math.trunc(limit)) : TOAST_LIMIT;
	const next = [...items, item];
	return next.length > safeLimit ? next.slice(next.length - safeLimit) : next;
}

/** Remove one toast by id (dismiss timer or manual close). */
export function removeToast(items: readonly ToastItem[], id: number): ToastItem[] {
	return items.filter((item) => item.id !== id);
}
