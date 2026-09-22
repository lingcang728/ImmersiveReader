import { writable } from 'svelte/store';

export interface DeviceInfo {
	isMobile: boolean;
	isAndroid: boolean;
	isiOS: boolean;
	isTouch: boolean;
	isOPPOFindX9: boolean;
	screenWidth: number;
	screenHeight: number;
	pixelRatio: number;
}

/**
 * Checks if the user agent or screen environment corresponds to a mobile device.
 */
export function detectDevice(
	userAgent = typeof navigator !== 'undefined' ? navigator.userAgent : '',
	maxTouchPoints = typeof navigator !== 'undefined' ? navigator.maxTouchPoints : 0,
	screenWidth = typeof window !== 'undefined' ? window.innerWidth : 1024,
	screenHeight = typeof window !== 'undefined' ? window.innerHeight : 768,
	pixelRatio = typeof window !== 'undefined' ? window.devicePixelRatio : 1
): DeviceInfo {
	const ua = userAgent.toLowerCase();
	const isAndroid = /android/.test(ua);
	const isiOS = /iphone|ipad|ipod/.test(ua) || (maxTouchPoints > 1 && /macintosh/.test(ua));
	const isTouch = maxTouchPoints > 0;
	const isMobile = isAndroid || isiOS || /mobile/.test(ua) || (isTouch && screenWidth <= 768);

	// OPPO Find X9 profile: 6.59" AMOLED, 2760x1256 (aspect ratio ~20:9 / 19.78:9, DPR ~3.0-3.5)
	const aspect = screenHeight > 0 && screenWidth > 0 ? Math.max(screenHeight, screenWidth) / Math.min(screenHeight, screenWidth) : 1;
	const isFindX9 = (isAndroid || /oppo|coloros/i.test(ua)) && aspect >= 2.0 && aspect <= 2.35;

	return {
		isMobile,
		isAndroid,
		isiOS,
		isTouch,
		isOPPOFindX9: isFindX9,
		screenWidth,
		screenHeight,
		pixelRatio
	};
}

let initialVolumePaging = true;
try {
	if (typeof localStorage !== 'undefined') {
		const stored = localStorage.getItem('mmbook-volume-key-paging');
		if (stored !== null) {
			initialVolumePaging = stored === 'true';
		}
	}
} catch {
	// Fall back to default
}

export const volumeKeyPaging = writable<boolean>(initialVolumePaging);

volumeKeyPaging.subscribe((enabled) => {
	try {
		if (typeof localStorage !== 'undefined') {
			localStorage.setItem('mmbook-volume-key-paging', String(enabled));
		}
	} catch {
		// Silently skip
	}
});

let initialTouchZones = true;
try {
	if (typeof localStorage !== 'undefined') {
		const stored = localStorage.getItem('mmbook-touch-zones');
		if (stored !== null) {
			initialTouchZones = stored === 'true';
		}
	}
} catch {
	// Fall back
}

export const touchZonesEnabled = writable<boolean>(initialTouchZones);

touchZonesEnabled.subscribe((enabled) => {
	try {
		if (typeof localStorage !== 'undefined') {
			localStorage.setItem('mmbook-touch-zones', String(enabled));
		}
	} catch {
		// Silently skip
	}
});
