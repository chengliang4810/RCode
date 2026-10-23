import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { analyze, compareRequests, loadRecords } from './analyze-request-diagnostics.mjs';

function request(id, messages, extra = {}) {
  return { version: 1, kind: 'request', attemptId: id, callId: id, sessionId: 'session', purpose: 'conversation', attempt: 1,
    protocol: 'chat', origin: 'https://example.invalid', fingerprint: {
      wire: { sha256: id }, system: { sha256: 'system' }, tools: { sha256: 'tools' }, parameters: { sha256: 'parameters' },
      messageCount: messages.length, messagesTruncated: false, messages: messages.map(sha256 => ({ role: 'user', sha256 })), ...extra }, };
}

test('stable prefix with low cache remains append-only and is not blamed on a client change', () => {
  const a = request('a', ['1']); const b = request('b', ['1', '2']);
  const rows = analyze([a, b, { version: 1, kind: 'finished', attemptId: 'b', outcome: 'completed', usage: { input_tokens: 1000, cache_read_input_tokens: 0 } }]);
  assert.equal(rows[1].classification, 'append_only'); assert.equal(rows[1].cacheHitPercent, 0);
  assert.equal(rows[0].cacheHitPercent, null); assert.equal(rows[0].outcome, 'incomplete');
});
test('tools, system, parameters, old messages and reductions are distinguished', () => {
  const a = request('a', ['1', '2']);
  for (const key of ['system', 'tools', 'parameters']) {
    const result = compareRequests(a, request('b', ['1', '2'], { [key]: { sha256: 'changed' } }));
    assert.equal(result.classification, 'prefix_changed'); assert.equal(result[`${key}Changed`], true);
  }
  assert.equal(compareRequests(a, request('b', ['1', 'x'])).classification, 'history_changed');
  assert.equal(compareRequests(a, request('b', ['1'])).classification, 'history_reduced');
  assert.equal(compareRequests(a, a).classification, 'identical');
  assert.equal(compareRequests(a, request('b', ['1', '2'], { messagesTruncated: true })).classification, 'unknown_truncated');
});
test('retries, usage snapshots and transcript call links survive interleaving and late links', () => {
  const a = request('a', ['1']); const b = { ...request('b', ['1'], { wire: { sha256: 'a' } }), callId: 'a', attempt: 2 };
  const rows = analyze([a, { ...request('other', ['x']), sessionId: 'other' }, b,
    { version: 1, kind: 'metadata', attemptId: 'b', sessionId: 'session', model: 'backend-model', rawUsage: { prompt_tokens: 1000, cache_read_input_tokens: 0, prompt_tokens_details: { cached_tokens: 800 } } },
    { version: 1, kind: 'finished', attemptId: 'b', sessionId: 'session', outcome: 'completed', usage: { input_tokens: 1000, cache_read_input_tokens: 800 } },
    { version: 1, kind: 'model_call_link', callId: 'a', modelCallId: 'doc-usage-id', sessionId: 'session' }], 'session');
  assert.equal(rows.length, 2); assert.equal(rows[1].attempt, 2); assert.equal(rows[1].classification, 'identical');
  assert.equal(rows[1].modelCallId, 'doc-usage-id'); assert.equal(rows[1].cacheHitPercent, 80);
  assert.equal(rows[1].rawUsage[0].usage.cache_read_input_tokens, 0); assert.deepEqual(rows[1].returnedModels, ['backend-model']);
});
test('rotation order and a partially written last line do not fabricate completed records', () => {
  const root = mkdtempSync(join(tmpdir(), 'rcode-diagnostics-'));
  try {
    writeFileSync(join(root, 'requests.1.jsonl'), `${JSON.stringify(request('a', ['1']))}\n`);
    writeFileSync(join(root, 'requests.jsonl'), `${JSON.stringify(request('b', ['1', '2']))}\n{"partial":`);
    assert.deepEqual(loadRecords(root).map(x => x.attemptId), ['a', 'b']);
    writeFileSync(join(root, 'requests.jsonl'), '{broken}\n');
    assert.throws(() => loadRecords(root), /Invalid completed diagnostic record/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});
