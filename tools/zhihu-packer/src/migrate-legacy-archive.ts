import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { DatabaseSync } from 'node:sqlite';

import { parseManifest, type BookManifest } from '../../../packages/contracts/dist/index.js';
import { closeDb, initDb } from './db.js';

export type LegacyArchiveMigrationInput = {
  readonly databasePath: string;
  readonly outputRoot: string;
};

export type LegacyArchiveMigrationReport = {
  readonly databasePath: string;
  readonly outputRoot: string;
  readonly authors: number;
  readonly items: number;
  readonly provenanceCreated: number;
};

function sha256(filePath: string): string {
  return createHash('sha256').update(fs.readFileSync(filePath)).digest('hex');
}

function writeJsonAtomic(filePath: string, value: unknown): void {
  const temporary = `${filePath}.tmp-${process.pid}`;
  fs.writeFileSync(temporary, `${JSON.stringify(value, null, 2)}\n`, 'utf8');
  fs.renameSync(temporary, filePath);
}

function authorName(manifest: BookManifest, directoryName: string): string {
  const suffix = ' · 知乎归档';
  return manifest.title.endsWith(suffix) ? manifest.title.slice(0, -suffix.length) : directoryName;
}

function safeChapterPath(authorRoot: string, relativePath: string): string {
  const root = fs.realpathSync(authorRoot);
  const candidate = path.resolve(authorRoot, ...relativePath.split('/'));
  if (!fs.existsSync(candidate) || !fs.statSync(candidate).isFile()) {
    throw new Error(`Legacy chapter is missing: ${relativePath}`);
  }
  const resolved = fs.realpathSync(candidate);
  if (resolved === root || !resolved.startsWith(`${root}${path.sep}`)) {
    throw new Error(`Legacy chapter escapes its author directory: ${relativePath}`);
  }
  return resolved;
}

function ensureProvenance(
  authorRoot: string,
  manifest: BookManifest,
  manifestSha256: string,
): { created: boolean; sha256: string } {
  const sourceId = manifest.sourceId;
  if (!sourceId) throw new Error(`Legacy Zhihu manifest has no sourceId: ${manifest.bookId}`);
  const provenancePath = path.join(authorRoot, 'provenance.json');
  if (fs.existsSync(provenancePath)) {
    const existing = JSON.parse(fs.readFileSync(provenancePath, 'utf8')) as Record<string, unknown>;
    if (existing.bookId !== manifest.bookId
      || existing.sourceId !== sourceId
      || existing.manifestSha256 !== manifestSha256
      || !Number.isInteger(existing.revision)
      || Number(existing.revision) < 1) {
      throw new Error(`Existing provenance conflicts with manifest: ${provenancePath}`);
    }
    return { created: false, sha256: sha256(provenancePath) };
  }
  writeJsonAtomic(provenancePath, {
    schemaVersion: 1,
    bookId: manifest.bookId,
    sourceId,
    sourceKind: 'zhihu',
    createdByTaskId: 'legacy-migration',
    lastSuccessfulTaskId: 'legacy-migration',
    revision: 1,
    manifestSha256,
    engineVersion: 'zhihu-packer@1.0.0',
    updatedAt: manifest.updatedAt,
  });
  return { created: true, sha256: sha256(provenancePath) };
}

export function migrateLegacyArchive(input: LegacyArchiveMigrationInput): LegacyArchiveMigrationReport {
  const databasePath = path.resolve(input.databasePath);
  const outputRoot = path.resolve(input.outputRoot);
  if (!fs.existsSync(outputRoot) || !fs.statSync(outputRoot).isDirectory()) {
    throw new Error(`Legacy Zhihu output root is missing: ${outputRoot}`);
  }
  fs.mkdirSync(path.dirname(databasePath), { recursive: true });
  initDb(databasePath);
  closeDb();
  const database = new DatabaseSync(databasePath);
  database.exec('PRAGMA foreign_keys = ON');
  const upsertAuthor = database.prepare(`
    INSERT INTO archive_authors (author_id, author_name, created_at, updated_at)
    VALUES (?, ?, ?, ?)
    ON CONFLICT(author_id) DO UPDATE SET
      author_name = excluded.author_name,
      updated_at = MAX(archive_authors.updated_at, excluded.updated_at)
  `);
  const existingItem = database.prepare('SELECT author_id FROM archive_items WHERE item_id = ?');
  const insertItem = database.prepare(`
    INSERT OR IGNORE INTO archive_items
      (item_id, author_id, source_url, current_revision, created_at, updated_at)
    VALUES (?, ?, '', 0, ?, ?)
  `);
  const existingRevision = database.prepare(`
    SELECT revision FROM archive_revisions WHERE item_id = ? AND output_path = ?
  `);
  const nextRevision = database.prepare(`
    SELECT COALESCE(MAX(revision), 0) + 1 AS revision FROM archive_revisions WHERE item_id = ?
  `);
  const insertRevision = database.prepare(`
    INSERT INTO archive_revisions
      (item_id, revision, task_id, output_path, manifest_sha256, provenance_sha256, created_at)
    VALUES (?, ?, 'legacy-migration', ?, ?, ?, ?)
  `);
  const updateRevisionHashes = database.prepare(`
    UPDATE archive_revisions
    SET manifest_sha256 = ?, provenance_sha256 = ?
    WHERE item_id = ? AND output_path = ?
  `);
  const updateCurrentRevision = database.prepare(`
    UPDATE archive_items
    SET current_revision = (SELECT MAX(revision) FROM archive_revisions WHERE item_id = ?),
        updated_at = MAX(updated_at, ?)
    WHERE item_id = ?
  `);

  let authors = 0;
  let items = 0;
  let provenanceCreated = 0;
  try {
    database.exec('BEGIN IMMEDIATE');
    for (const entry of fs.readdirSync(outputRoot, { withFileTypes: true })) {
      if (!entry.isDirectory() || entry.isSymbolicLink() || entry.name.startsWith('.')) continue;
      const authorRoot = path.join(outputRoot, entry.name);
      const manifestPath = path.join(authorRoot, 'manifest.json');
      if (!fs.existsSync(manifestPath)) continue;
      const manifest = parseManifest(JSON.parse(fs.readFileSync(manifestPath, 'utf8')));
      if (manifest.source !== 'zhihu' || !manifest.sourceId) {
        throw new Error(`Legacy archive manifest is not a Zhihu book: ${manifestPath}`);
      }
      const manifestHash = sha256(manifestPath);
      const provenance = ensureProvenance(authorRoot, manifest, manifestHash);
      if (provenance.created) provenanceCreated += 1;
      const timestamp = Date.parse(manifest.updatedAt);
      const name = authorName(manifest, entry.name);
      upsertAuthor.run(manifest.sourceId, name, timestamp, timestamp);
      authors += 1;
      for (const chapter of manifest.chapters) {
        safeChapterPath(authorRoot, chapter.path);
        const conflict = existingItem.get(chapter.id) as { author_id: string } | undefined;
        if (conflict && conflict.author_id !== manifest.sourceId) {
          throw new Error(`Legacy item id belongs to another author: ${chapter.id}`);
        }
        insertItem.run(chapter.id, manifest.sourceId, timestamp, timestamp);
        const outputPath = path.join(entry.name, ...chapter.path.split('/')).replaceAll('\\', '/');
        const existing = existingRevision.get(chapter.id, outputPath) as { revision: number } | undefined;
        if (existing) {
          updateRevisionHashes.run(manifestHash, provenance.sha256, chapter.id, outputPath);
        } else {
          const row = nextRevision.get(chapter.id) as { revision: number };
          insertRevision.run(
            chapter.id,
            row.revision,
            outputPath,
            manifestHash,
            provenance.sha256,
            timestamp,
          );
        }
        updateCurrentRevision.run(chapter.id, timestamp, chapter.id);
        items += 1;
      }
    }
    database.exec('COMMIT');
  } catch (error) {
    try { database.exec('ROLLBACK'); } catch {}
    database.close();
    throw error;
  }
  database.close();
  return { databasePath, outputRoot, authors, items, provenanceCreated };
}

function argument(name: string): string {
  const index = process.argv.indexOf(name);
  const value = index >= 0 ? process.argv[index + 1] : undefined;
  if (!value) throw new Error(`Missing required argument: ${name}`);
  return value;
}

if (process.argv[1] && pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url) {
  const report = migrateLegacyArchive({
    databasePath: argument('--database'),
    outputRoot: argument('--output'),
  });
  console.log(JSON.stringify(report));
}
