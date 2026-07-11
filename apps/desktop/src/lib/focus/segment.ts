export type SentenceRange = { start: number; end: number };

// Regex fallback: split after CJK/Latin sentence terminators (closing quotes
// and brackets stay attached to the sentence they end).
export function splitSentencesFallback(text: string): SentenceRange[] {
	const ranges: SentenceRange[] = [];
	const terminators = /[。！？!?…]+[」』”’"'）)\]】]*/g;
	let last = 0;
	let match: RegExpExecArray | null;
	while ((match = terminators.exec(text)) !== null) {
		const end = match.index + match[0].length;
		if (text.slice(last, end).trim() !== '') {
			ranges.push({ start: last, end });
		}
		last = end;
	}
	if (last < text.length && text.slice(last).trim() !== '') {
		ranges.push({ start: last, end: text.length });
	}
	return ranges;
}

export function splitSentences(text: string): SentenceRange[] {
	if (typeof Intl !== 'undefined' && 'Segmenter' in Intl) {
		try {
			const segmenter = new Intl.Segmenter(undefined, { granularity: 'sentence' });
			const ranges: SentenceRange[] = [];
			for (const segment of segmenter.segment(text)) {
				const start = segment.index;
				const end = segment.index + segment.segment.length;
				if (text.slice(start, end).trim() !== '') {
					ranges.push({ start, end });
				}
			}
			if (ranges.length > 0) return ranges;
		} catch {
			// fall through to the regex fallback
		}
	}
	return splitSentencesFallback(text);
}
