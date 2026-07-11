export function cleanReaderMarkdown(markdownText: string): string {
  let markdown = markdownText.trim();
  if (markdown.startsWith('---')) {
    const yamlMatch = markdown.match(/^---[\s\S]*?\n---(?:\r?\n|$)/);
    if (yamlMatch) {
      markdown = markdown.slice(yamlMatch[0].length).trim();
    }
  }

  return markdown
    .replace(/^<h1[\s\S]*?<\/h1>\s*/i, '')
    .trim()
    .replace(/^<div[\s\S]*?<\/div>\s*/i, '')
    .trim();
}
