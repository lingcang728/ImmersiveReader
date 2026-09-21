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

// Intl.Segmenter construction is expensive enough to matter when
// splitSentences runs once per paragraph during focus entry, so the
// (stateless) segmenter is shared module-wide. `segment()` returns a fresh
// iterable per call, so reuse across texts is safe.
let sentenceSegmenter: Intl.Segmenter | null | undefined;

function getSentenceSegmenter(): Intl.Segmenter | null {
	if (sentenceSegmenter === undefined) {
		sentenceSegmenter = null;
		if (typeof Intl !== 'undefined' && 'Segmenter' in Intl) {
			try {
				// Pin the locale: Focus 分句是锁定行为，随系统语言漂移不可复现。
				sentenceSegmenter = new Intl.Segmenter('zh', { granularity: 'sentence' });
			} catch {
				sentenceSegmenter = null;
			}
		}
	}
	return sentenceSegmenter;
}

export function splitSentences(text: string): SentenceRange[] {
	const segmenter = getSentenceSegmenter();
	if (segmenter) {
		try {
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
