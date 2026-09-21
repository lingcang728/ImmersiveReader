/**
 * Minimal Tab/Shift+Tab focus cycling for overlay "dialogs" that are not
 * native <dialog> elements (search bar, TOC palette). Native dialogs get a
 * trap for free; these don't, so focus must be wrapped manually.
 */

const FOCUSABLE_SELECTOR =
	'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

function visibleFocusables(container: HTMLElement): HTMLElement[] {
	return Array.from(
		container.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR),
	).filter((el) => el.getClientRects().length > 0);
}

/**
 * Call from a keydown handler on the overlay root. Returns true when the
 * event was consumed (focus wrapped to the other end of the dialog).
 */
export function cycleFocusWithin(
	container: HTMLElement,
	event: KeyboardEvent,
): boolean {
	if (event.key !== "Tab") return false;
	const items = visibleFocusables(container);
	if (items.length === 0) {
		event.preventDefault();
		return true;
	}
	const first = items[0];
	const last = items[items.length - 1];
	const active = document.activeElement as HTMLElement | null;
	if (event.shiftKey && (active === first || !container.contains(active))) {
		event.preventDefault();
		last.focus();
		return true;
	}
	if (!event.shiftKey && (active === last || !container.contains(active))) {
		event.preventDefault();
		first.focus();
		return true;
	}
	return false;
}
