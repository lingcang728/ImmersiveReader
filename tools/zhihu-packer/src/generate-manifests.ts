import { createHash } from "node:crypto";
import * as fs from "node:fs";
import * as path from "node:path";

import { getAllSuccessAuthors, getAuthorSuccessItems, initDb } from "./db.js";
import { generateAuthorManifest } from "./manifest-io.js";
import { resolveArchiveOutputDir } from "./runtime-paths.js";

function main(): void {
  const projectRoot = path.resolve();
  const outputRoot = resolveArchiveOutputDir({ cwd: projectRoot, environment: process.env });
  const dryRun = process.argv.includes("--dry-run");
  initDb(process.env.IMMERSIVE_ZHIHU_DB ?? "zhihu-packer.db");
  const authors = getAllSuccessAuthors();
  const representedDirectories = new Set<string>();
  let missing = 0;
  for (const author of authors) {
    representedDirectories.add(author.author_name.toLocaleLowerCase("zh-CN"));
    const report = generateAuthorManifest({
      projectRoot,
      outputRoot,
      author,
      items: getAuthorSuccessItems(author.author_id),
      write: !dryRun,
    });
    missing += report.missingItems;
    console.log(
      `${dryRun ? "dry-run" : "manifest"}: ${report.manifestPath} ` +
      `(数据库 ${report.databaseItems}，推断 ${report.inferredItems}，缺失 ${report.missingItems})`,
    );
  }
  if (fs.existsSync(outputRoot)) {
    for (const entry of fs.readdirSync(outputRoot, { withFileTypes: true })) {
      if (!entry.isDirectory() || representedDirectories.has(entry.name.toLocaleLowerCase("zh-CN"))) {
        continue;
      }
      const legacyId = createHash("sha256").update(entry.name).digest("hex").slice(0, 16);
      const report = generateAuthorManifest({
        projectRoot,
        outputRoot,
        author: { author_id: `legacy-${legacyId}`, author_name: entry.name },
        items: [],
        write: !dryRun,
      });
      console.log(
        `${dryRun ? "dry-run" : "manifest"}: ${report.manifestPath} ` +
        `(数据库 0，推断 ${report.inferredItems}，缺失 0)`,
      );
    }
  }
  if (authors.length === 0 && !fs.existsSync(outputRoot)) {
    console.warn("数据库与输出目录中都没有可生成书目的内容。");
  }
  if (missing > 0) {
    console.warn(`共有 ${missing} 条成功记录缺少 Markdown 文件。`);
    process.exitCode = 2;
  }
}

main();
