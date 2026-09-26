// EPUB-surface error copy. Backend commands surface stable `EPUB_*` codes;
// map those first, then defer to the shared describeError table. Pure —
// unit-testable without a DOM.

import { describeError } from "../errors";

const EPUB_ERROR_COPY: [pattern: RegExp, message: string][] = [
	[/EPUB_DRM/, "本书受 DRM 保护，无法阅读"],
	[/EPUB_TOO_LARGE|EPUB_SIZE_LIMIT/, "EPUB 文件过大，无法打开"],
	[/EPUB_ENTRY_LIMIT|EPUB_TOO_MANY/, "EPUB 内容条目过多"],
	[/EPUB_UNSUPPORTED/, "该 EPUB 特性暂不支持"],
	[/EPUB_CHAPTER_NOT_FOUND/, "章节不存在或已被移除"],
	[/EPUB_NOT_FOUND/, "书籍文件不存在"],
	[/EPUB_PARSE|EPUB_CORRUPT|EPUB_INVALID|EPUB_MALFORMED/, "EPUB 文件损坏或格式异常"],
	[/EPUB_ENCRYPTED/, "本书包含加密内容，无法阅读"],
	[/EPUB_IO|EPUB_READ/, "读取 EPUB 文件失败"],
	[/EPUB_/, "EPUB 加载失败"],
];

/** User-facing description of an EPUB command failure. */
export function describeEpubError(error: unknown): string {
	const raw = (error instanceof Error ? error.message : String(error)).trim();
	if (/^EPUB_/.test(raw) || /EPUB_/.test(raw)) {
		for (const [pattern, message] of EPUB_ERROR_COPY) {
			if (pattern.test(raw)) return message;
		}
	}
	return describeError(error);
}

/** Log the raw error, return the localized description. */
export function reportEpubError(context: string, error: unknown): string {
	console.error(`[${context}]`, error);
	return describeEpubError(error);
}

/** Map a `publication.unsupported` flag to readable Chinese copy. */
export function describeUnsupportedFlag(flag: string): string {
	const normalized = flag.toLowerCase().replace(/[-_]/g, "");
	if (/fixedlayout|fxl|prepaginated/.test(normalized)) return "固定版式";
	if (/mediaoverlay|mediaoverlays|mo$|readaloud/.test(normalized)) return "媒体叠加";
	if (/script/.test(normalized)) return "脚本内容";
	if (/drm|encrypt/.test(normalized)) return "加密/DRM";
	if (/audio|video|media/.test(normalized)) return "音视频";
	if (/svg|mathml/.test(normalized)) return "复杂图形";
	return flag;
}
