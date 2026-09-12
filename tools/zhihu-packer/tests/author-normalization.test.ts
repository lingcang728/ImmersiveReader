import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';

import { writeMarkdownFile, type ExtractedContent } from '../src/extractor.ts';
import { publishTaskStage, resetTaskIncoming } from '../src/publish.ts';
import { sanitizeFilename } from '../src/utils.ts';

function makeExtracted(overrides: Partial<ExtractedContent> = {}): ExtractedContent {
  return {
    id: 'answer:1',
    type: 'answer',
    title: '测试标题',
    authorId: 'unknown',
    authorName: '抓取显示名',
    contentHtml: '<p>正文</p>',
    contentMarkdown: '正文',
    createdTime: 1700000000,
    updatedTime: 1700000000,
    voteupCount: 3,
    commentCount: 1,
    url: 'https://www.zhihu.com/question/1/answer/1',
    answerId: '1',
    questionId: '1',
    questionUrl: 'https://www.zhihu.com/question/1',
    ...overrides,
  };
}

function topLevelDirs(root: string): string[] {
  return fs.readdirSync(root, { withFileTypes: true })
    .filter(entry => entry.isDirectory())
    .map(entry => entry.name);
}

test('archive directory identity normalizes to the task peopleId', async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'zhihu-author-normalize-'));
  try {
    // 抓取到的 per-item 身份（unknown/anonymous/真实 ID）一律不得参与目录命名；
    // 目录统一用任务级身份 { id: peopleId, name: 任务规范名 }（P1-10）。
    const first = await writeMarkdownFile(
      makeExtracted({ id: 'answer:1', authorId: 'unknown', authorName: '抓取显示名' }),
      root,
      { id: 'people-1', name: '规范名' },
    );
    const second = await writeMarkdownFile(
      makeExtracted({ id: 'answer:2', type: 'answer', authorId: 'real-api-id', authorName: '另一个显示名' }),
      root,
      { id: 'people-1', name: '规范名' },
    );
    const third = await writeMarkdownFile(
      makeExtracted({ id: 'article:3', type: 'article', authorId: 'anonymous', authorName: '' }),
      root,
      { id: 'people-1', name: '规范名' },
    );

    const canonical = path.resolve(root, sanitizeFilename('规范名', 'people-1'));
    assert.equal(path.dirname(first), canonical);
    assert.equal(path.dirname(second), canonical);
    assert.equal(path.dirname(third), canonical);
    assert.deepEqual(topLevelDirs(root), [path.basename(canonical)], 'exactly one author directory');
    assert.ok(fs.readFileSync(first, 'utf8').includes('抓取显示名'), 'display name stays in content');
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test('without a task identity the extracted author still names the directory', async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'zhihu-author-fallback-'));
  try {
    const filePath = await writeMarkdownFile(
      makeExtracted({ authorId: 'anonymous', authorName: '匿名用户' }),
      root,
    );
    const expected = path.resolve(root, sanitizeFilename('匿名用户', 'anonymous'));
    assert.equal(path.dirname(filePath), expected);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test('publish merges legacy split author directories into the canonical one', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'zhihu-publish-merge-'));
  const taskId = 'task-merge';
  try {
    const incoming = resetTaskIncoming(root, taskId);
    const legacyDirName = sanitizeFilename('抓取显示名', 'unknown');
    const canonicalDirName = sanitizeFilename('规范名', 'people-1');
    fs.mkdirSync(path.join(incoming, legacyDirName), { recursive: true });
    fs.writeFileSync(path.join(incoming, legacyDirName, 'a.md'), '内容A');
    fs.mkdirSync(path.join(incoming, canonicalDirName), { recursive: true });
    fs.writeFileSync(path.join(incoming, canonicalDirName, 'b.md'), '内容B');

    // 条目 output_path 仍记着归并前的旧目录名——发布必须按文件名回退定位（P1-10）。
    const result = publishTaskStage(root, taskId, 'people-1', {
      authorName: '规范名',
      items: [
        {
          item_id: 'answer:1',
          output_path: `.incoming/${taskId}/${legacyDirName}/a.md`,
          author_id: 'people-1',
          author_name: '规范名',
          title: 'A篇',
          created_time: 1,
          voteup_count: 1,
        },
        {
          item_id: 'answer:2',
          output_path: `.incoming/${taskId}/${canonicalDirName}/b.md`,
          author_id: 'people-1',
          author_name: '规范名',
          title: 'B篇',
          created_time: 2,
          voteup_count: 2,
        },
      ] as any,
    });

    assert.equal(result.transaction.phase, 'committed');
    assert.equal(result.authorDirectory, canonicalDirName);
    assert.equal(fs.readFileSync(path.join(result.finalRoot, 'a.md'), 'utf8'), '内容A');
    assert.equal(fs.readFileSync(path.join(result.finalRoot, 'b.md'), 'utf8'), '内容B');
    assert.deepEqual(topLevelDirs(incoming), [], 'incoming stage is consumed by the commit');
    const manifest = JSON.parse(fs.readFileSync(path.join(result.finalRoot, 'manifest.json'), 'utf8'));
    assert.deepEqual(
      manifest.chapters.map((chapter: any) => chapter.path).sort(),
      ['a.md', 'b.md'],
    );
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});
