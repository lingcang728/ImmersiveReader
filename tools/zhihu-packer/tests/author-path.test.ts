import test from 'node:test';
import assert from 'node:assert/strict';

import { authorDirectoryName } from '../src/manifest-io.ts';

test('keeps author ids in canonical directories for colliding display names', () => {
  const sameNameA = authorDirectoryName('同名作者', 'author-a');
  const sameNameB = authorDirectoryName('同名作者', 'author-b');
  assert.notEqual(sameNameA, sameNameB);

  const caseA = authorDirectoryName('A', 'author-a');
  const caseB = authorDirectoryName('a', 'author-b');
  assert.notEqual(caseA.toLocaleLowerCase('en-US'), caseB.toLocaleLowerCase('en-US'));

  const cleanedA = authorDirectoryName('作者/清洗后', 'author-a');
  const cleanedB = authorDirectoryName('作者清洗后', 'author-b');
  assert.notEqual(cleanedA, cleanedB);

  const longPrefix = 'x'.repeat(120);
  const longA = authorDirectoryName(longPrefix, 'author-a');
  const longB = authorDirectoryName(longPrefix, 'author-b');
  assert.notEqual(longA, longB);
});
