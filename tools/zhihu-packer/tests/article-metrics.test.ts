import assert from 'node:assert/strict';
import test from 'node:test';

import { findArticleIndexAtPosition } from '../src/reader/core/article-metrics.ts';

const metrics = [
  { index: 0, top: 0, bottom: 499 },
  { index: 1, top: 600, bottom: 1099 },
  { index: 2, top: 1200, bottom: 1799 },
];

test('finds the active article with logarithmic ordered-metric lookup', () => {
  assert.equal(findArticleIndexAtPosition(metrics, 250, 0), 0);
  assert.equal(findArticleIndexAtPosition(metrics, 800, 0), 1);
  assert.equal(findArticleIndexAtPosition(metrics, 1700, 0), 2);
});

test('chooses the nearest article while the viewport marker is in a gap', () => {
  assert.equal(findArticleIndexAtPosition(metrics, 520, 1), 0);
  assert.equal(findArticleIndexAtPosition(metrics, 580, 0), 1);
});

test('uses the fallback only when no metrics are available', () => {
  assert.equal(findArticleIndexAtPosition([], 500, 7), 7);
});
