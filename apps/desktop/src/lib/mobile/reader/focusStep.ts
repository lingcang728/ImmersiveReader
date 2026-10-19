/** Page within an oversized sentence before moving the sentence cursor. */
export function sentencePageTarget(current: number, top: number, bottom: number, visibleTop: number, visibleBottom: number, direction: -1 | 1): number | null {
  const height = Math.max(1, visibleBottom - visibleTop);
  if (bottom - top <= height) return null;
  if (direction > 0 && bottom > visibleBottom + 2) return current + Math.min(height * 0.82, bottom - visibleBottom);
  if (direction < 0 && top < visibleTop - 2) return current - Math.min(height * 0.82, visibleTop - top);
  return null;
}
