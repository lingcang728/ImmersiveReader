import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

test('force recrawl never deletes successful markdown before fetching', () => {
  for (const relative of ['src/server.ts', 'src/cli.ts']) {
    const source = fs.readFileSync(path.join(root, relative), 'utf8');
    const forceSection = source.match(/(?:force-restart-existing|option\('-f, --force')[\s\S]*?(?:retry-failed|command\('pause)/)?.[0] ?? '';
    assert.equal(forceSection.includes('unlinkSync'), false, `${relative} deletes Markdown during force recrawl`);
  }
});
