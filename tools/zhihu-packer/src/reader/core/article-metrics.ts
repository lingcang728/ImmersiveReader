export interface ArticleMetric {
  index: number;
  top: number;
  bottom: number;
}

function distanceToMetric(metric: ArticleMetric, position: number): number {
  if (position < metric.top) return metric.top - position;
  if (position > metric.bottom) return position - metric.bottom;
  return 0;
}

export function findArticleIndexAtPosition(
  metrics: readonly ArticleMetric[],
  position: number,
  fallbackIndex: number,
): number {
  if (metrics.length === 0) return fallbackIndex;

  let low = 0;
  let high = metrics.length - 1;
  while (low <= high) {
    const middle = Math.floor((low + high) / 2);
    const metric = metrics[middle];
    if (position < metric.top) {
      high = middle - 1;
    } else if (position > metric.bottom) {
      low = middle + 1;
    } else {
      return metric.index;
    }
  }

  const before = high >= 0 ? metrics[high] : null;
  const after = low < metrics.length ? metrics[low] : null;
  if (!before) return after?.index ?? fallbackIndex;
  if (!after) return before.index;
  return distanceToMetric(before, position) <= distanceToMetric(after, position)
    ? before.index
    : after.index;
}
