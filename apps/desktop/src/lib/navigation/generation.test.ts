import { describe, expect, it } from 'vitest';
import { isCurrentNavigation } from './generation';

describe('navigation generation guard', () => {
	it('accepts the same generation and path', () => {
		expect(
			isCurrentNavigation(
				{ generation: 4, path: 'A.md' },
				{ generation: 4, path: 'A.md' },
			),
		).toBe(true);
	});

	it('rejects a response from an older generation', () => {
		expect(
			isCurrentNavigation(
				{ generation: 4, path: 'A.md' },
				{ generation: 5, path: 'A.md' },
			),
		).toBe(false);
	});

	it('rejects a response for a different path in the same generation', () => {
		expect(
			isCurrentNavigation(
				{ generation: 4, path: 'A.md' },
				{ generation: 4, path: 'B.md' },
			),
		).toBe(false);
	});
});
