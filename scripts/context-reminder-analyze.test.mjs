import { test } from 'node:test';
import assert from 'node:assert/strict';
import { extractBusinessAnswer } from './context-reminder-analyze.mjs';

test('business-field analysis separates extra explanation from incorrect values', () => {
  assert.deepEqual(extractBusinessAnswer('Explanation\n```json\n{"region":"new"}\n```\nMore text'), { region: 'new' });
  assert.deepEqual(extractBusinessAnswer('{"region":"old"}'), { region: 'old' });
  assert.equal(extractBusinessAnswer('```json\n{"region":"old"}\n```\n```json\n{"region":"new"}\n```'), null);
  assert.equal(extractBusinessAnswer('No answer generated'), null);
});
