from __future__ import annotations

import argparse
import re
from pathlib import Path

from pypdf import PdfReader


def sanitize_filename(value: str, fallback: str) -> str:
    value = re.sub(r'[<>:"/\\|?*\x00-\x1f]', "", value).strip()
    value = re.sub(r"\s+", " ", value)
    value = value.rstrip(". ")
    return (value[:120] or fallback).strip()


def collapse_exact_repeat(value: str) -> str:
    value = re.sub(r"\s+", " ", value).strip()
    value = re.sub(r"^\d{1,3}\s+", "", value)
    value = re.sub(r"\s+\d{1,3}$", "", value)
    if len(value) % 2 == 0:
        half = len(value) // 2
        if value[:half] == value[half:]:
            return value[:half].strip()
    changed = True
    while changed:
        changed = False
        max_size = min(40, len(value) // 2)
        for size in range(max_size, 1, -1):
            pattern = re.compile(r"(.{%d})\s*\1" % size)
            new_value = pattern.sub(r"\1", value)
            if new_value != value:
                value = new_value
                changed = True
                break
    value = re.sub(r"^(.)(\1)(?=\d|[A-Za-z]|[\u4e00-\u9fff])", r"\1", value)
    return value.strip()


def normalize_text(text: str) -> str:
    text = text.replace("\r\n", "\n").replace("\r", "\n")
    text = text.replace("\u00a0", " ")
    text = re.sub(r"[ \t]+", " ", text)
    text = re.sub(r"\n{3,}", "\n\n", text)
    return text.strip()


def format_body(text: str) -> str:
    text = normalize_text(text)
    lines = [line.strip() for line in text.splitlines()]
    paragraphs: list[str] = []
    for line in lines:
        if re.fullmatch(r"\d{1,3}", line):
            continue
        if not line:
            continue
        paragraphs.append(line)

    return "\n\n".join(p for p in paragraphs if p).strip()


def parse_articles(text: str) -> list[dict[str, object]]:
    marker = re.compile(r"第+\s*(\d+)\s*篇+:\s*(https?://\S+)")
    matches = list(marker.finditer(text))
    articles: list[dict[str, object]] = []

    for idx, match in enumerate(matches):
        seq = int(match.group(1))
        url = match.group(2).strip()
        start = match.end()
        end = matches[idx + 1].start() if idx + 1 < len(matches) else len(text)
        segment = text[start:end].strip()

        meta_match = re.search(
            r"(?P<title>.*?)\n\s*(?:赞同数)+\s*:?.*?\(\s*(?P<votes>\d+)\s*(?:赞同)+?\s*\)\s*(?:创建时间)+\s*:?\s*\(\s*(?P<date>\d{4}-\d{2}-\d{2})\s*\)",
            segment,
            flags=re.S,
        )
        if not meta_match:
            continue

        title = collapse_exact_repeat(meta_match.group("title").strip())
        votes = int(meta_match.group("votes"))
        date = meta_match.group("date")
        body = segment[meta_match.end() :].strip()
        body = re.sub(r"^\s*[:：]\s*", "", body)
        body = format_body(body)

        url_id = re.sub(r"\D+", "", url.rsplit("/", 1)[-1]) or str(seq)
        articles.append(
            {
                "seq": seq,
                "url": url,
                "id": url_id,
                "title": title,
                "votes": votes,
                "date": date,
                "body": body,
            }
        )

    return articles


def write_articles(articles: list[dict[str, object]], out_dir: Path, author: str) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    for old in out_dir.glob("*.md"):
        old.unlink()

    sorted_articles = sorted(articles, key=lambda item: (-int(item["votes"]), str(item["date"]), int(item["seq"])))

    index_lines = [
        f"# {author} 的内容归档",
        "",
        "> 本归档由 PDF 文本拆分生成。  ",
        f"> 共归档回答/文章: **{len(sorted_articles)}** 篇。  ",
        "",
        "## 回答列表",
        "",
    ]

    for article in sorted_articles:
        title = str(article["title"])
        date = str(article["date"])
        votes = int(article["votes"])
        article_id = str(article["id"])
        filename = f"{date}-{sanitize_filename(title, article_id)}_{article_id}.md"
        body = str(article["body"]).strip()
        content = (
            f"<h1 style=\"text-align: center; margin-bottom: 20px;\">{title}</h1>\n\n"
            "<div style=\"display: flex; justify-content: space-between; align-items: center; "
            "border-bottom: 1px solid #e0e0e0; padding-bottom: 8px; margin-bottom: 20px;\">"
            f"<span style=\"font-weight: bold; color: #333;\">作者：{author}</span>"
            f"<span style=\"color: #666;\">日期：{date}</span>"
            f"<span style=\"color: #666;\">赞同数：{votes}</span></div>\n\n"
            f"{body}\n"
        )
        (out_dir / filename).write_text(content, encoding="utf-8")
        index_title = title.replace("[", "").replace("]", "") or title
        index_lines.append(f"- [[{filename}|{index_title}]] (发布于: {date} | 赞同数: {votes})")

    (out_dir / "index.md").write_text("\n".join(index_lines) + "\n", encoding="utf-8")


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Split a Zhihu archive PDF into Markdown files using extracted PDF text."
    )
    parser.add_argument("--input", required=True, help="Path to the source PDF file.")
    parser.add_argument("--output", required=True, help="Directory where Markdown files will be written.")
    parser.add_argument("--author", required=True, help="Author name used in generated files and index title.")
    args = parser.parse_args()

    pdf_path = Path(args.input).expanduser().resolve()
    out_dir = Path(args.output).expanduser().resolve()

    reader = PdfReader(str(pdf_path))
    page_texts = []
    for page in reader.pages:
        page_texts.append(page.extract_text() or "")
    full_text = "\n".join(page_texts)
    articles = parse_articles(full_text)
    write_articles(articles, out_dir, args.author)

    print(f"pages={len(reader.pages)}")
    print(f"articles={len(articles)}")
    print(f"out={out_dir}")
    if articles:
        top = sorted(articles, key=lambda item: -int(item["votes"]))[:5]
        for item in top:
            print(f"{item['votes']}\t{item['date']}\t{item['title']}")


if __name__ == "__main__":
    main()
