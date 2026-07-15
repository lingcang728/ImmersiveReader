function stripTags(html: string): string {
  return html.replace(/<[^>]+>/g, ' ');
}

/** Collapse whitespace and case for title equality checks. */
export function normalizeHeadingText(value: string): string {
  return stripTags(value)
    .replace(/\s+/g, ' ')
    .trim()
    .toLocaleLowerCase();
}

/**
 * Clean archive markdown/HTML for continuous reading.
 * Leading archive `<h1>` is removed only when it matches the chapter title
 * (normalized). Unrelated first headings are kept.
 */
export function cleanReaderMarkdown(markdownText: string, chapterTitle?: string): string {
  let markdown = markdownText.trim();
  if (markdown.startsWith('---')) {
    const yamlMatch = markdown.match(/^---[\s\S]*?\n---(?:\r?\n|$)/);
    if (yamlMatch) {
      markdown = markdown.slice(yamlMatch[0].length).trim();
    }
  }

  const h1Match = markdown.match(/^<h1[\s\S]*?<\/h1>\s*/i);
  if (h1Match) {
    const headingText = normalizeHeadingText(h1Match[0]);
    const titleNorm = chapterTitle ? normalizeHeadingText(chapterTitle) : '';
    if (!titleNorm || headingText === titleNorm) {
      markdown = markdown.slice(h1Match[0].length).trim();
    }
  }

  return markdown.replace(/^<div[\s\S]*?<\/div>\s*/i, '').trim();
}
