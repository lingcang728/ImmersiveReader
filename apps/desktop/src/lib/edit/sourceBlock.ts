// P3-2: one full-text split serves both the range check and the splice —
// callers used to pay two splits per save (validate + replace).
function splitLinesInRange(source: string, startLine: number, endLine: number): string[] | null {
	if (!Number.isInteger(startLine) || !Number.isInteger(endLine)) return null;
	if (startLine < 1 || endLine < startLine) return null;

	const lines = source.split('\n');
	if (startLine > lines.length || endLine > lines.length) return null;

	return lines;
}

export function getMarkdownSourceBlock(source: string, startLine: number, endLine: number): string | null {
	const lines = splitLinesInRange(source, startLine, endLine);
	if (!lines) return null;

	return lines.slice(startLine - 1, endLine).join('\n');
}

export function replaceMarkdownSourceBlock(
	source: string,
	startLine: number,
	endLine: number,
	replacement: string,
): string | null {
	const lines = splitLinesInRange(source, startLine, endLine);
	if (!lines) return null;

	lines.splice(startLine - 1, endLine - startLine + 1, ...replacement.split('\n'));
	return lines.join('\n');
}

export function normalizeMarkdownEditText(text: string): string {
	return text.replace(/\r\n?/g, '\n');
}
