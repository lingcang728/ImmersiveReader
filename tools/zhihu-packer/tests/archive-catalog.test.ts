import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { DatabaseSync } from 'node:sqlite';
import {
  closeDb,
  deleteTask,
  getAllSuccessAuthors,
  getAuthorSuccessItems,
  initDb,
  saveItem,
  saveTask,
  saveTaskItem,
} from '../src/db.ts';

test('deleting task history keeps permanent archive catalog and revision', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'zhihu-archive-catalog-'));
  const database = path.join(root, 'zhihu.db');
  initDb(database);
  const now = Date.now();
  saveTask({ id: 'task-1', author_id: 'author-1', author_name: '作者', created_at: now });
  saveItem({
    id: 'answer:1',
    item_type: 'answer',
    author_id: 'author-1',
    author_name: '作者',
    title: '标题',
    answer_id: '1',
    question_id: 'question-1',
    article_id: null,
    url: 'https://www.zhihu.com/question/1/answer/1',
    question_url: 'https://www.zhihu.com/question/1',
    created_time: now,
    updated_time: now,
    voteup_count: 1,
    comment_count: 0,
  });
  saveTaskItem({
    task_id: 'task-1',
    item_id: 'answer:1',
    status: 'success',
    output_path: '作者/answer-1.md',
    failure_code: null,
    error_message: null,
    created_at: now,
    updated_at: now,
  });
  saveTaskItem({
    task_id: 'task-1',
    item_id: 'answer:1',
    status: 'success',
    output_path: '作者/answer-1.md',
    failure_code: null,
    error_message: null,
    created_at: now,
    updated_at: now + 1,
  });

  deleteTask('task-1');

  const authors = getAllSuccessAuthors();
  assert.equal(authors.length, 1);
  assert.equal(authors[0].author_id, 'author-1');
  assert.equal(authors[0].author_name, '作者');
  const archived = getAuthorSuccessItems('author-1');
  assert.equal(archived.length, 1);
  assert.equal(archived[0].output_path, '作者/answer-1.md');
  closeDb();
  const verification = new DatabaseSync(database, { readOnly: true });
  const revisions = verification.prepare(
    "SELECT COUNT(*) AS count FROM archive_revisions WHERE item_id = 'answer:1'"
  ).get() as { count: number };
  assert.equal(revisions.count, 1);
  verification.close();
  fs.rmSync(root, { recursive: true, force: true });
});
