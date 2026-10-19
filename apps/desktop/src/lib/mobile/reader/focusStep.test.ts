import { expect, it } from 'vitest';
import { sentencePageTarget } from './focusStep';
it('reads an oversized sentence in overlapping pages before advancing', () => {
  expect(sentencePageTarget(100, 80, 1500, 150, 750, 1)).toBe(592);
  expect(sentencePageTarget(100, 80, 1500, 900, 1500, 1)).toBeNull();
  expect(sentencePageTarget(900, 80, 1500, 900, 1500, -1)).toBeCloseTo(408);
  expect(sentencePageTarget(0, 100, 200, 50, 750, 1)).toBeNull();
});
