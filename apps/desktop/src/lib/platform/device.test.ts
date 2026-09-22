import { describe, expect, it } from 'vitest';
import { detectDevice } from './device';

describe('device and mobile platform detection', () => {
	it('detects desktop environment by default', () => {
		const desktop = detectDevice(
			'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/120.0.0.0 Safari/537.36',
			0,
			1920,
			1080,
			1
		);
		expect(desktop.isMobile).toBe(false);
		expect(desktop.isAndroid).toBe(false);
		expect(desktop.isiOS).toBe(false);
		expect(desktop.isTouch).toBe(false);
		expect(desktop.isOPPOFindX9).toBe(false);
	});

	it('detects Android phone environment and OPPO Find X9 specs', () => {
		// OPPO Find X9 running ColorOS 16 / Android 16
		const findX9 = detectDevice(
			'Mozilla/5.0 (Linux; U; Android 16; zh-cn; PKX110 Build/UKQ1.230924.001; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/128.0.0.0 Mobile Safari/537.36 ColorOS/16.0',
			5,
			412,
			906,
			3.0
		);
		expect(findX9.isMobile).toBe(true);
		expect(findX9.isAndroid).toBe(true);
		expect(findX9.isiOS).toBe(false);
		expect(findX9.isTouch).toBe(true);
		expect(findX9.isOPPOFindX9).toBe(true);
	});

	it('detects iOS device environment', () => {
		const iphone = detectDevice(
			'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1',
			5,
			393,
			852,
			3.0
		);
		expect(iphone.isMobile).toBe(true);
		expect(iphone.isAndroid).toBe(false);
		expect(iphone.isiOS).toBe(true);
		expect(iphone.isTouch).toBe(true);
	});

	it('detects small touch screen as mobile even with generic user agent', () => {
		const touchSmall = detectDevice('', 2, 400, 800, 2.0);
		expect(touchSmall.isMobile).toBe(true);
		expect(touchSmall.isTouch).toBe(true);
	});
});
