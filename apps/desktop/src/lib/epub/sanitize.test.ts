import { describe, expect, it } from "vitest";
import {
	escapeHtml,
	markdownFallbackHtml,
	sanitizeSearchSnippet,
	snippetQueryText,
	textExcerpt,
	unescapeHtml,
} from "./sanitize";

describe("escapeHtml", () => {
	it("escapes markup-significant characters", () => {
		expect(escapeHtml(`<a href="x">&'`)).toBe("&lt;a href=&quot;x&quot;&gt;&amp;&#39;");
	});
});

describe("sanitizeSearchSnippet", () => {
	it("keeps <b>/</b> highlights and escapes everything else", () => {
		expect(sanitizeSearchSnippet("前文 <b>关键词</b> 后文")).toBe(
			"前文 <b>关键词</b> 后文",
		);
		expect(sanitizeSearchSnippet("<img src=x onerror=alert(1)> <b>hit</b>")).toBe(
			"&lt;img src=x onerror=alert(1)&gt; <b>hit</b>",
		);
	});

	it("does not unescape other tags", () => {
		expect(sanitizeSearchSnippet("<i>x</i><b>y</b><script>z</script>")).toBe(
			"&lt;i&gt;x&lt;/i&gt;<b>y</b>&lt;script&gt;z&lt;/script&gt;",
		);
	});
});

describe("snippetQueryText", () => {
	it("joins highlighted runs and unescapes entities", () => {
		expect(snippetQueryText("…a<b>关键</b>b<b>词</b>c…")).toBe("关键词");
		expect(snippetQueryText("a<b>x &amp; y</b>b")).toBe("x & y");
		expect(snippetQueryText("no marks")).toBe("");
	});
});

describe("unescapeHtml", () => {
	it("handles common entities and leaves others alone", () => {
		expect(unescapeHtml("&lt;a&gt;&amp;&#39;q&#39;")).toBe("<a>&'q'");
		expect(unescapeHtml("&unknown;")).toBe("&unknown;");
	});
});

describe("markdownFallbackHtml", () => {
	it("escapes content and wraps paragraphs", () => {
		const html = markdownFallbackHtml("段落一 <x>\n\n段落二");
		expect(html).toBe("<p>段落一 &lt;x&gt;</p>\n<p>段落二</p>");
	});

	it("turns single newlines into <br>", () => {
		expect(markdownFallbackHtml("a\nb")).toBe("<p>a<br>b</p>");
	});
});

describe("textExcerpt", () => {
	it("collapses whitespace and truncates", () => {
		expect(textExcerpt("  a   b\n c ", 40)).toBe("a b c");
		expect(textExcerpt("x".repeat(100), 10)).toBe("xxxxxxxxxx…");
	});
});
