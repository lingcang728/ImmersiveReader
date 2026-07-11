import { execSync } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";

import { loadManifest } from "./manifest-io.js";
import { renderMarkdownArticle, renderReaderHtml, type ReaderArticle } from "./reader-builder.js";
import { resolveArchiveOutputDir } from "./runtime-paths.js";

function ensureTemplate(projectRoot: string): string {
  const templatePath = path.join(projectRoot, "dist", "reader-template.html");
  if (!fs.existsSync(templatePath)) {
    execSync("npm run compile-reader", { cwd: projectRoot, stdio: "inherit" });
  }
  if (!fs.existsSync(templatePath)) {
    throw new Error(`Reader 模板不存在：${templatePath}`);
  }
  return fs.readFileSync(templatePath, "utf8");
}

function discoverManifests(outputRoot: string): readonly string[] {
  if (!fs.existsSync(outputRoot)) {
    throw new Error(`输出目录不存在：${outputRoot}`);
  }
  return fs
    .readdirSync(outputRoot, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => path.join(outputRoot, entry.name, "manifest.json"))
    .filter((manifestPath) => fs.existsSync(manifestPath))
    .sort((left, right) => left.localeCompare(right, "zh-CN"));
}

function chapterFilePath(bookRoot: string, relativePath: string): string {
  const candidate = path.resolve(bookRoot, ...relativePath.split("/"));
  const relative = path.relative(bookRoot, candidate);
  if (relative.startsWith("..") || path.isAbsolute(relative)) {
    throw new Error(`章节路径越过书目录：${relativePath}`);
  }
  return candidate;
}

async function buildBook(template: string, manifestPath: string): Promise<number> {
  const manifest = loadManifest(manifestPath);
  const bookRoot = path.dirname(manifestPath);
  const articles: ReaderArticle[] = [];
  let missing = 0;
  for (const chapter of manifest.chapters) {
    const filePath = chapterFilePath(bookRoot, chapter.path);
    if (!fs.existsSync(filePath) || !fs.statSync(filePath).isFile()) {
      console.warn(`缺少章节文件：${manifest.title} / ${chapter.path}`);
      missing += 1;
      continue;
    }
    const markdown = fs.readFileSync(filePath, "utf8");
    articles.push(await renderMarkdownArticle(chapter, markdown, manifest.title));
  }
  if (articles.length === 0) {
    console.warn(`书目没有可构建章节：${manifest.title}`);
    return missing;
  }
  const html = renderReaderHtml(template, articles, `${manifest.title} - 沉浸阅读`);
  const outputPath = path.join(bookRoot, "reader.html");
  fs.writeFileSync(outputPath, html, "utf8");
  console.log(`Reader: ${outputPath} (${articles.length} 篇)`);
  return missing;
}

async function main(): Promise<void> {
  const projectRoot = path.resolve();
  const outputRoot = resolveArchiveOutputDir({ cwd: projectRoot, environment: process.env });
  const template = ensureTemplate(projectRoot);
  const manifests = discoverManifests(outputRoot);
  if (manifests.length === 0) {
    throw new Error(`没有找到 manifest.json：${outputRoot}`);
  }
  let missing = 0;
  for (const manifestPath of manifests) {
    missing += await buildBook(template, manifestPath);
  }
  if (missing > 0) {
    console.warn(`构建完成，但有 ${missing} 个章节文件缺失。`);
    process.exitCode = 2;
  }
}

main().catch((error: unknown) => {
  const message = error instanceof Error ? error.message : String(error);
  console.error(`Reader 构建失败：${message}`);
  process.exitCode = 1;
});
