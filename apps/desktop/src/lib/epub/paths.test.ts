import { describe, expect, it } from "vitest";
import {
	buildChapterHrefMap,
	isExternalHttpUrl,
	isNativeAbsolutePath,
	isProtocolRelative,
	joinBookPath,
	joinNativePath,
	normalizeBookPath,
	resolveHrefToChapterId,
	safeDecodeUri,
	splitHref,
} from "./paths";
import type { Publication } from "./types";

describe("normalizeBookPath", () => {
	it("collapses separators and dot segments", () => {
		expect(normalizeBookPath("a//b/./c.xhtml")).toBe("a/b/c.xhtml");
		expect(normalizeBookPath("./a/b")).toBe("a/b");
	});

	it("resolves .. against ancestors and clamps at root", () => {
		expect(normalizeBookPath("a/b/../c")).toBe("a/c");
		expect(normalizeBookPath("../x")).toBe("x");
		expect(normalizeBookPath("a/../../b")).toBe("b");
	});

	it("normalizes backslashes and strips leading slashes", () => {
		expect(normalizeBookPath("a\\b\\c")).toBe("a/b/c");
		expect(normalizeBookPath("/abs/path")).toBe("abs/path");
	});

	it("decodes percent-escapes", () => {
		expect(normalizeBookPath("Text/%E7%AC%AC%E4%B8%80.xhtml")).toBe(
			"Text/第一.xhtml",
		);
	});

	it("returns empty for empty input", () => {
		expect(normalizeBookPath("")).toBe("");
		expect(normalizeBookPath("  ")).toBe("");
	});
});

describe("joinBookPath", () => {
	it("joins dir + relative then normalizes", () => {
		expect(joinBookPath("OEBPS/Text", "../Images/pic.png")).toBe(
			"OEBPS/Images/pic.png",
		);
		expect(joinBookPath("", "a/b")).toBe("a/b");
	});
});

describe("splitHref", () => {
	it("splits path/query/fragment", () => {
		expect(splitHref("text/ch1.xhtml?x=1#s2")).toEqual({
			path: "text/ch1.xhtml",
			query: "?x=1",
			fragment: "s2",
		});
	});

	it("handles pure fragments and bare paths", () => {
		expect(splitHref("#note-1")).toEqual({ path: "", query: "", fragment: "note-1" });
		expect(splitHref("a.xhtml")).toEqual({ path: "a.xhtml", query: "", fragment: "" });
	});

	it("decodes the fragment", () => {
		expect(splitHref("a.xhtml#%E8%8A%82").fragment).toBe("节");
	});
});

describe("url classification", () => {
	it("flags http(s) and protocol-relative", () => {
		expect(isExternalHttpUrl("https://x.com/a")).toBe(true);
		expect(isExternalHttpUrl("http://x.com")).toBe(true);
		expect(isExternalHttpUrl("ftp://x.com")).toBe(false);
		expect(isProtocolRelative("//x.com/a.png")).toBe(true);
	});

	it("detects native absolute paths", () => {
		expect(isNativeAbsolutePath("C:\\book\\a.png")).toBe(true);
		expect(isNativeAbsolutePath("/abs/a.png")).toBe(true);
		expect(isNativeAbsolutePath("rel/a.png")).toBe(false);
	});
});

describe("joinNativePath", () => {
	it("uses the base separator style", () => {
		expect(joinNativePath("C:\\book\\dir", "img/a.png")).toBe("C:\\book\\dir\\img\\a.png");
		expect(joinNativePath("/home/book/dir", "img/a.png")).toBe("/home/book/dir/img/a.png");
	});
});

function publicationFixture(): Publication {
	return {
		schemaVersion: 1,
		format: "epub",
		epubVersion: "3",
		title: "T",
		nav: [],
		spine: ["ch1", "ch2", "ch3"],
		resources: {
			ch1: "OEBPS/text/ch1.xhtml",
			ch2: "OEBPS/text/ch2.xhtml",
			ch3: "OEBPS/text/ch3.xhtml",
			"css-1": "OEBPS/style/main.css",
			"img-1": "OEBPS/images/cover.png",
		},
		unsupported: [],
	};
}

describe("buildChapterHrefMap", () => {
	it("maps spine document paths to chapterIds only", () => {
		const map = buildChapterHrefMap(publicationFixture());
		expect(map.get("OEBPS/text/ch1.xhtml")).toBe("ch1");
		expect(map.get("OEBPS/text/ch3.xhtml")).toBe("ch3");
		// non-spine resources never navigate
		expect(map.get("OEBPS/style/main.css")).toBeUndefined();
		expect(map.get("OEBPS/images/cover.png")).toBeUndefined();
	});

	it("supports reversed path→id serialization too", () => {
		const pub = publicationFixture();
		pub.resources = { "OEBPS/text/ch1.xhtml": "ch1" };
		const map = buildChapterHrefMap(pub);
		expect(map.get("OEBPS/text/ch1.xhtml")).toBe("ch1");
	});

	it("falls back to a bare chapterId", () => {
		const map = buildChapterHrefMap(publicationFixture());
		expect(map.get("ch2")).toBe("ch2");
	});
});

describe("resolveHrefToChapterId", () => {
	const map = buildChapterHrefMap(publicationFixture());

	it("resolves sibling hrefs relative to resourceDir", () => {
		expect(resolveHrefToChapterId(map, "OEBPS/text", "ch2.xhtml#s1")).toEqual({
			chapterId: "ch2",
			fragment: "s1",
		});
	});

	it("resolves ../ relative hrefs", () => {
		expect(resolveHrefToChapterId(map, "OEBPS/images", "../text/ch3.xhtml")).toEqual({
			chapterId: "ch3",
			fragment: "",
		});
	});

	it("resolves book-root-relative hrefs", () => {
		expect(resolveHrefToChapterId(map, "", "OEBPS/text/ch1.xhtml")).toEqual({
			chapterId: "ch1",
			fragment: "",
		});
	});

	it("returns null for assets and unknown targets", () => {
		expect(resolveHrefToChapterId(map, "OEBPS/text", "../images/cover.png")).toBeNull();
		expect(resolveHrefToChapterId(map, "OEBPS/text", "nowhere.xhtml")).toBeNull();
	});

	it("returns null for pure fragments", () => {
		expect(resolveHrefToChapterId(map, "OEBPS/text", "#local")).toBeNull();
	});
});

describe("safeDecodeUri", () => {
	it("returns malformed input unchanged", () => {
		expect(safeDecodeUri("%E4%xx")).toBe("%E4%xx");
		expect(safeDecodeUri("plain")).toBe("plain");
	});
});
