#!/usr/bin/env python
"""Generate small synthetic Markdown/EPUB fixtures for real-device QA (task-7).

Writes into docs/audit/2026-10-18/fixtures/:
  sample-notes.md          single-file Markdown (~6 KB, headings/code/table/quote/math)
  sample-library/          a 2-chapter Markdown "book" folder for folder import
  sample-book-epub.epub    minimal valid EPUB 3 (~10 KB, 2 chapters, nested nav, image)

Everything here is original synthetic text — never real user content. Deterministic
output (fixed timestamps inside the EPUB zip) so QA can re-run it.

Usage:
    G:\\python\\python.exe scripts\\qa\\mobile\\make_device_fixtures.py
"""
from __future__ import annotations

import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
FIXTURES = ROOT / "docs" / "audit" / "2026-10-18" / "fixtures"

NOTES_MD = """# 移动端 QA 合成文档（Markdown）

> 本文件由 `scripts/qa/mobile/make_device_fixtures.py` 生成，仅用于 Android 真机
> 阅读验证（打开/滚动/目录/进度/深色主题）。不含任何真实用户内容。

## 第一节：长文排版

沉浸阅读的移动端 QA 合成正文。**粗体**、*斜体*、`行内代码`、[外链（仅 http/https）](https://example.com/ir-qa)
以及脚注引用[^qa]。

[^qa]: 合成脚注内容，用来验证脚注渲染与跳转。

段落一。长文阅读器需要在手机上保持稳定的行高与页宽；这段文字刻意重复，用来拉长
可滚动内容，观察滚动是否跟手、进度条是否随滚动更新、返回书架后进度是否保存。

段落二。山月记式的短句与长句交替出现。夜色里的树林没有声音，只有风从远处推过来，
一层一层，像有人在低声念一段很旧的文章。这里不需要真实内容，只需要足够的行数。

段落三。QA 检查点：`IR-QA-DEVICE-MD`。看到这个标记说明真机打开的是合成 fixture。

## 第二节：代码块与表格

```rust
fn reading_progress(chapter: usize, total: usize) -> f64 {
    if total == 0 { 0.0 } else { chapter as f64 / total as f64 }
}
```

```javascript
export function nextPage(scroller) {
  return scroller.scrollTop + scroller.clientHeight;
}
```

| 检查项 | 期望 |
|---|---|
| 正文字号 | 可在阅读设置中调整 |
| 深色主题 | 背景为深色、对比度可读 |
| 进度保存 | 退出重进后回到原位置 |
| 目录 | 可跳转章节/标题 |

## 第三节：更长的正文

""" + "\n\n".join(
    f"第 {index} 段。合成文本第 {index} 节：阅读器在滚动时不应出现明显掉帧；"
    "横滑翻页与纵向滚动不能互相抢夺手势；长按选择文本时不应误触翻页。"
    for index in range(1, 25)
) + """

## 第四节：结尾

QA 结束标记：`IR-QA-DEVICE-MD-END`。如果真机滚动到这里没有卡死、没有崩溃，
并且返回书架后书架仍可交互，本次 fixture 的阅读冒烟即完成。
"""

CHAPTER_1 = """# 第一章 合成书稿

书稿第一章（合成）。本文件夹用于验证 Android 端「文件夹导入」与多章节 Markdown
阅读：目录、上一章/下一章、进度保存。

`IR-QA-DEVICE-BOOK-C1`

""" + "\n\n".join(
    f"第一章第 {index} 段。合成正文用来制造足够长的章节，验证分块挂载与滚动性能。"
    for index in range(1, 16)
)

CHAPTER_2 = """# 第二章 合成书稿

书稿第二章（合成）。从第一章滚到底应能无缝进入本章，返回时应回到原阅读位置。

`IR-QA-DEVICE-BOOK-C2`

""" + "\n\n".join(
    f"第二章第 {index} 段。合成正文继续，用来检查章节切换时的加载过渡与滚动复位。"
    for index in range(1, 16)
)

MANIFEST_JSON = """{
  "schemaVersion": 1,
  "bookId": "manual:qa-sample-library",
  "title": "QA 合成书稿",
  "source": "manual",
  "generatedAt": "2026-10-18T00:00:00Z",
  "updatedAt": "2026-10-18T00:00:00Z",
  "chapters": [
    { "id": "q1", "path": "001.md", "title": "第一章 合成书稿", "date": null, "voteCount": 0, "wordCount": 520 },
    { "id": "q2", "path": "002.md", "title": "第二章 合成书稿", "date": null, "voteCount": 0, "wordCount": 520 }
  ]
}
"""

MIMETYPE = "application/epub+zip"

CONTAINER_XML = """<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>
"""

CONTENT_OPF = """<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="qa-book-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="qa-book-id">urn:uuid:qa-sample-book-2026-10-18</dc:identifier>
    <dc:title>QA 合成 EPUB</dc:title>
    <dc:creator>ImmersiveReader QA</dc:creator>
    <dc:language>zh-CN</dc:language>
    <meta property="dcterms:modified">2026-10-18T00:00:00Z</meta>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="cover" href="cover.xhtml" media-type="application/xhtml+xml"/>
    <item id="c1" href="chapter-1.xhtml" media-type="application/xhtml+xml"/>
    <item id="c2" href="chapter-2.xhtml" media-type="application/xhtml+xml"/>
    <item id="pixel" href="../images/qa-pixel.png" media-type="image/png"/>
    <item id="css" href="style.css" media-type="text/css"/>
  </manifest>
  <spine>
    <itemref idref="cover"/>
    <itemref idref="c1"/>
    <itemref idref="c2"/>
  </spine>
</package>
"""

NAV_XHTML = """<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" lang="zh-CN">
<head><title>目录</title><link rel="stylesheet" type="text/css" href="style.css"/></head>
<body>
  <nav epub:type="toc" id="toc">
    <h1>目录</h1>
    <ol>
      <li><a href="chapter-1.xhtml">第一章 合成章节</a>
        <ol><li><a href="chapter-2.xhtml">第二章 续章</a></li></ol>
      </li>
    </ol>
  </nav>
</body>
</html>
"""

STYLE_CSS = """body { font-family: serif; line-height: 1.75; margin: 1.2em; color: #222; }
h1 { font-size: 1.35em; margin: 1.2em 0 0.8em; }
p { margin: 0 0 0.9em; }
"""

COVER_XHTML = """<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" lang="zh-CN">
<head><title>封面</title><link rel="stylesheet" type="text/css" href="style.css"/></head>
<body><h1>QA 合成 EPUB</h1><p>封面页（合成，无版权内容）。</p></body>
</html>
"""


def chapter_xhtml(title: str, marker: str, chapter_body: str) -> str:
    return f"""<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" lang="zh-CN">
<head><title>{title}</title><link rel="stylesheet" type="text/css" href="style.css"/></head>
<body>
  <section>
    <h1>{title}</h1>
    <p>{marker}</p>
    {chapter_body}
    <p><a href="chapter-2.xhtml">跳到第二章（内链）</a></p>
    <p><img src="../images/qa-pixel.png" alt="QA 像素图"/></p>
  </section>
</body>
</html>
"""


def build_epub(target: Path) -> None:
    """Write a minimal, valid EPUB 3 (mimetype stored first, per OCF)."""
    body_1 = "\n    ".join(
        f"<p>第一章第 {index} 段合成正文，用于验证 EPUB 纵滚与目录跳转。</p>"
        for index in range(1, 12)
    )
    body_2 = "\n    ".join(
        f"<p>第二章第 {index} 段合成正文，用于验证跨章加载与进度定位。</p>"
        for index in range(1, 12)
    )
    # 1x1 transparent PNG (67 bytes) — original synthetic asset.
    pixel_png = bytes.fromhex(
        "89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4"
        "890000000d49444154789c6260000000020001e221bc330000000049454e44ae426082"
    )
    entries = [
        ("mimetype", MIMETYPE, zipfile.ZIP_STORED),
        ("META-INF/container.xml", CONTAINER_XML, zipfile.ZIP_DEFLATED),
        ("OEBPS/content.opf", CONTENT_OPF, zipfile.ZIP_DEFLATED),
        ("OEBPS/nav.xhtml", NAV_XHTML, zipfile.ZIP_DEFLATED),
        ("OEBPS/style.css", STYLE_CSS, zipfile.ZIP_DEFLATED),
        ("OEBPS/cover.xhtml", COVER_XHTML, zipfile.ZIP_DEFLATED),
        ("OEBPS/chapter-1.xhtml",
         chapter_xhtml("第一章 合成章节", "IR-QA-DEVICE-EPUB-C1", body_1), zipfile.ZIP_DEFLATED),
        ("OEBPS/chapter-2.xhtml",
         chapter_xhtml("第二章 续章", "IR-QA-DEVICE-EPUB-C2", body_2), zipfile.ZIP_DEFLATED),
        ("images/qa-pixel.png", pixel_png, zipfile.ZIP_DEFLATED),
    ]
    target.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(target, "w") as archive:
        for name, payload, method in entries:
            info = zipfile.ZipInfo(name, date_time=(2026, 10, 18, 0, 0, 0))
            info.compress_type = method
            info.external_attr = 0o644 << 16
            data = payload.encode("utf-8") if isinstance(payload, str) else payload
            archive.writestr(info, data)


def main() -> int:
    FIXTURES.mkdir(parents=True, exist_ok=True)
    notes = FIXTURES / "sample-notes.md"
    notes.write_text(NOTES_MD, encoding="utf-8")

    library = FIXTURES / "sample-library"
    library.mkdir(parents=True, exist_ok=True)
    (library / "manifest.json").write_text(MANIFEST_JSON, encoding="utf-8")
    (library / "001.md").write_text(CHAPTER_1, encoding="utf-8")
    (library / "002.md").write_text(CHAPTER_2, encoding="utf-8")
    # The importer treats the folder name as the book title suggestion.
    (library / "README.txt").write_text(
        "QA synthetic Markdown book folder — no real user content.\n", encoding="utf-8"
    )

    epub = FIXTURES / "sample-book-epub.epub"
    build_epub(epub)

    for path in sorted(FIXTURES.rglob("*")):
        if path.is_file():
            print(f"{path.relative_to(ROOT)}  {path.stat().st_size} B")
    return 0


if __name__ == "__main__":
    sys.exit(main())
