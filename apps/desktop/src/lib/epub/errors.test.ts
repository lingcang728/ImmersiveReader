import { describe, expect, it } from "vitest";
import { describeEpubError, describeUnsupportedFlag } from "./errors";

describe("describeEpubError", () => {
	it("maps EPUB_* codes to Chinese copy", () => {
		expect(describeEpubError(new Error("EPUB_DRM: book is encrypted"))).toBe(
			"本书受 DRM 保护，无法阅读",
		);
		expect(describeEpubError("EPUB_PARSE failed at line 9")).toBe(
			"EPUB 文件损坏或格式异常",
		);
		expect(describeEpubError("EPUB_CHAPTER_NOT_FOUND")).toBe("章节不存在或已被移除");
	});

	it("falls through EPUB_ catch-all then shared describeError", () => {
		expect(describeEpubError("EPUB_WHATEVER_NEW")).toBe("EPUB 加载失败");
		// non-EPUB codes go to the shared table
		expect(describeEpubError("TASK_NOT_FOUND")).toBe("任务不存在或已结束");
		// already-localized copy passes through
		expect(describeEpubError("章节加载失败")).toBe("章节加载失败");
	});
});

describe("describeUnsupportedFlag", () => {
	it("translates known flags", () => {
		expect(describeUnsupportedFlag("fixed-layout")).toBe("固定版式");
		expect(describeUnsupportedFlag("media-overlays")).toBe("媒体叠加");
		expect(describeUnsupportedFlag("scripted")).toBe("脚本内容");
		expect(describeUnsupportedFlag("drm")).toBe("加密/DRM");
	});

	it("passes unknown flags through", () => {
		expect(describeUnsupportedFlag("something-else")).toBe("something-else");
	});
});
