import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

test('legacy control center is absent from the packaged sidecar', () => {
  const server = fs.readFileSync(path.join(root, 'src/server.ts'), 'utf8');
  const packageJson = JSON.parse(fs.readFileSync(path.join(root, 'package.json'), 'utf8'));

  for (const marker of [
    'express.static',
    '/api/config',
    '/api/events',
    'force-restart-existing',
    '/api/open-dir',
    '/api/download',
  ]) {
    assert.equal(server.includes(marker), false, `sidecar still exposes legacy console marker: ${marker}`);
  }
  for (const script of ['web', 'cli', 'doctor', 'login']) {
    assert.equal(packageJson.scripts[script], undefined, `legacy package script remains: ${script}`);
  }
  assert.equal(fs.existsSync(path.join(root, 'public/index.html')), false);
  assert.equal(fs.existsSync(path.join(root, '一键启动')), false);
});

test('staged scheduler never deletes the published archive before fetching', () => {
  const scheduler = fs.readFileSync(path.join(root, 'src/scheduler.ts'), 'utf8');
  assert.equal(scheduler.includes('unlinkSync'), false);
  assert.equal(scheduler.includes('resetTaskIncoming'), true);
});

test('partial results publish before the task becomes terminal', () => {
  const scheduler = fs.readFileSync(path.join(root, 'src/scheduler.ts'), 'utf8');
  assert.equal(scheduler.includes('isPublishableTaskStatus(status, finalTask.success_count)'), true);
  const publish = scheduler.indexOf('publishTaskStage(outputBaseDir');
  const terminal = scheduler.indexOf('saveTask({ id: taskId, status });', publish);
  assert.ok(publish >= 0, 'scheduler must publish successful files');
  assert.ok(terminal > publish, 'terminal status must follow a durable publish');
});
