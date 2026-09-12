import * as fs from "node:fs";
import * as path from "node:path";

import { getAllSuccessAuthors, getAuthorSuccessItems, initDb } from "./db.js";
import { authorDirectoryName, generateAuthorManifest } from "./manifest-io.js";
import { resolveArchiveOutputDir } from "./runtime-paths.js";

function main(): void {
  const projectRoot = path.resolve();
  const outputRoot = resolveArchiveOutputDir({ cwd: projectRoot, environment: process.env });
  const dryRun = process.argv.includes("--dry-run");
  initDb(process.env.IMMERSIVE_ZHIHU_DB ?? "zhihu-packer.db");
  const authors = getAllSuccessAuthors();
  // P3-28：「磁盘目录是否已被数据库作者覆盖」必须按真实目录名
  // sanitizeFilename(author_name, author_id) 判定——旧代码拿裸 author_name 跟
  // 带 _id 后缀的目录名比，永不命中，导致每个真实作者目录都被当成 legacy 目录，
  // 再生成出一个 `*_legacy-<hash>` 幻影空书（generateAuthorManifest 的 authorPath
  // 是 name_id 拼名，根本写不进那个 legacy 目录，只会新建一个空目录+空 manifest）。
  const representedDirectories = new Set<string>();
  let missing = 0;
  for (const author of authors) {
    representedDirectories.add(
      authorDirectoryName(author.author_name, author.author_id).toLocaleLowerCase("zh-CN"),
    );
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
    // P3-28：当前管线产生的每个归档目录都有 archive_authors 记录（发布时
    // recordPublishedTaskItems 写入），不存在数据库覆盖不到的目录来源；
    // 旧的 legacy 分支只会把 .incoming/.revisions/.transactions 与真实作者目录
    // 误判为 legacy 并造出 `*_legacy-<hash>` 幻影空书，已删除。真有历史遗留目录
    // （旧独立工具产物、手工放入的归档）时只提示走 migrate-legacy 先入库。
    const unrepresented = fs.readdirSync(outputRoot, { withFileTypes: true })
      .filter((entry) => entry.isDirectory() && !entry.name.startsWith("."))
      .map((entry) => entry.name)
      .filter((name) => !representedDirectories.has(name.toLocaleLowerCase("zh-CN")));
    if (unrepresented.length > 0) {
      console.warn(
        `以下目录没有数据库作者记录，未生成 manifest（如需纳入归档，请先运行 migrate-legacy 入库）: ${unrepresented.join(", ")}`,
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
