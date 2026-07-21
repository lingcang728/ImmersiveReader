import assert from 'node:assert/strict';
import test from 'node:test';

import { isUnavailableZhihuPage } from '../src/extractor.js';

test('recognizes the Zhihu unavailable-content page', () => {
  assert.equal(
    isUnavailableZhihuPage('你似乎来到了没有知识存在的荒原 - 知乎', '返回问题页'),
    true
  );
});

test('does not classify a normal answer page as unavailable', () => {
  assert.equal(
    isUnavailableZhihuPage('如何看待这个问题？ - 知乎', '这是回答正文'),
    false
  );
});
