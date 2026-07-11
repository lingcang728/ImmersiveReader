export type FocusScrollTargetInput = {
	viewportHeight: number;
	currentScrollTop: number;
	blockTop: number;
	blockBottom: number;
	anchorOffset: number;
	longBlockRatio: number;
	longBlockEdgeBuffer: number;
};

export function getFocusScrollTarget({
	viewportHeight,
	currentScrollTop,
	blockTop,
	blockBottom,
	anchorOffset,
	longBlockRatio,
	longBlockEdgeBuffer,
}: FocusScrollTargetInput): number | null {
	const blockHeight = blockBottom - blockTop;
	const anchorScrollTop = currentScrollTop + anchorOffset;

	if (blockHeight > viewportHeight * longBlockRatio) {
		if (anchorScrollTop >= blockTop && anchorScrollTop <= blockBottom) {
			return null;
		}

		return blockTop > anchorScrollTop
			? blockTop - longBlockEdgeBuffer
			: blockBottom - (viewportHeight - longBlockEdgeBuffer);
	}

	return blockTop + blockHeight / 2 - anchorOffset;
}
