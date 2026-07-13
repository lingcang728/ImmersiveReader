import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { DatabaseSync } from 'node:sqlite';
import { closeDb } from '../src/db.ts';
import { migrateLegacyArchive } from '../src/migrate-legacy-archive.ts';

function sha256(filePath: string): string {
  return createHash('sha256').update(fs.readFileSync(filePath)).digest('hex');
}

test('legacy manifests become an idempotent archive catalog with provenance', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'zhihu-legacy-migration-'));
  const database = path.join(root, 'zhihu.db');
  const outputRoot = path.join(root, 'Library', '知乎');
  const authorRoot = path.join(outputRoot, '测试作者');
  fs.mkdirSync(authorRoot, { recursive: true });
  const markdownPath = path.join(authorRoot, '001.md');
  fs.writeFileSync(markdownPath, '# 原文\n\n不得修改。\n', 'utf8');
  const originalMarkdownHash = sha256(markdownPath);
  const manifest = {
    schemaVersion: 1,
    bookId: 'zhihu:legacy-author',
    title: '测试作者 · 知乎归档',
    source: 'zhihu',
    sourceId: 'legacy-author',
    generatedAt: '2026-07-10T00:00:00.000Z',
    updatedAt: '2026-07-10T00:00:00.000Z',
    chapters: [{
      id: 'inferred:001',
      path: '001.md',
      title: '原文',
      voteCount: 0,
      wordCount: 6,
      metadataStatus: 'inferred',
    }],
  };
  fs.writeFileSync(path.join(authorRoot, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`, 'utf8');

  const first = migrateLegacyArchive({ databasePath: database, outputRoot });
  closeDb();
  const second = migrateLegacyArchive({ databasePath: database, outputRoot });
  closeDb();

  assert.equal(first.authors, 1);
  assert.equal(first.items, 1);
  assert.equal(first.provenanceCreated, 1);
  assert.equal(second.authors, 1);
  assert.equal(second.items, 1);
  assert.equal(second.provenanceCreated, 0);
  assert.equal(sha256(markdownPath), originalMarkdownHash);

  const verification = new DatabaseSync(database, { readOnly: true });
  assert.equal((verification.prepare('PRAGMA user_version').get() as { user_version: number }).user_version, 2);
  assert.equal((verification.prepare('SELECT COUNT(*) AS count FROM archive_authors').get() as { count: number }).count, 1);
  assert.equal((verification.prepare('SELECT COUNT(*) AS count FROM archive_items').get() as { count: number }).count, 1);
  assert.equal((verification.prepare('SELECT COUNT(*) AS count FROM archive_revisions').get() as { count: number }).count, 1);
  const revision = verification.prepare(
    'SELECT task_id, output_path, manifest_sha256, provenance_sha256 FROM archive_revisions'
  ).get() as Record<string, string>;
  assert.equal(revision.task_id, 'legacy-migration');
  assert.equal(revision.output_path.replaceAll('\\', '/'), '测试作者/001.md');
  assert.match(revision.manifest_sha256, /^[a-f0-9]{64}$/);
  assert.match(revision.provenance_sha256, /^[a-f0-9]{64}$/);
  verification.close();

  const provenance = JSON.parse(fs.readFileSync(path.join(authorRoot, 'provenance.json'), 'utf8')) as Record<string, unknown>;
  assert.equal(provenance.bookId, manifest.bookId);
  assert.equal(provenance.sourceId, manifest.sourceId);
  assert.equal(provenance.manifestSha256, sha256(path.join(authorRoot, 'manifest.json')));
  assert.equal(provenance.revision, 1);
  fs.rmSync(root, { recursive: true, force: true });
});
