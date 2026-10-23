import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createStudy, normalizeUsage } from './prompt-cache-study.mjs';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { sortedTools } from './prompt-cache-cases.mjs';

test('gateway placeholder counters do not overwrite authoritative cached tokens', () => {
  const usage = normalizeUsage({ prompt_tokens: 15685, completion_tokens: 14,
    prompt_tokens_details: { cached_tokens: 15488 }, prompt_cache_hit_tokens: 15488,
    cache_read_input_tokens: 0, cached_tokens: 0, cache_creation_input_tokens: 0 }, 'chat');
  assert.equal(usage.cached, 15488);
  assert.equal(usage.input, 15685);
  assert.equal(usage.uncached, 197);
});

test('missing cache fields stay unknown, including successful Messages calls', () => {
  const usage = normalizeUsage({ input_tokens: 13313, output_tokens: 35 }, 'messages');
  assert.equal(usage.input, 13313);
  assert.equal(usage.cached, null);
  assert.equal(usage.hitPercent, null);
});

test('Messages disjoint buckets and Chat inclusive totals normalize consistently', () => {
  const a = normalizeUsage({ input_tokens: 10, cache_read_input_tokens: 90, cache_creation_input_tokens: 100, output_tokens: 5 }, 'messages');
  const b = normalizeUsage({ prompt_tokens: 200, prompt_cache_hit_tokens: 90, cache_creation_input_tokens: 100, completion_tokens: 5 }, 'chat');
  assert.equal(a.input, b.input);
  assert.equal(a.hitPercent, 45);
  assert.equal(b.hitPercent, 45);
});

test('tool canonicalization changes only ordering and preserves the source fixture', () => {
  const body = { model: 'm', tools: [{ function: { name: 'z', parameters: { required: ['p'] } } }, { function: { name: 'a' } }] };
  const copy = structuredClone(body);
  assert.deepEqual(sortedTools(body).tools, [copy.tools[1], copy.tools[0]]);
  assert.deepEqual(body, copy);
});

test('gateway errors cannot echo credentials into returned rows or saved events', async () => {
  const originalFetch = globalThis.fetch;
  const directory = await mkdtemp(join(tmpdir(), 'rcode-cache-study-'));
  const credential = 'synthetic-probe-credential';
  try {
    globalThis.fetch = async () => Response.json({ error: { message: `Rejected ${credential}` } }, { status: 401 });
    const study = createStudy({ baseUrl: 'https://example.invalid', apiKey: credential, directory });
    const row = await study.call('error', { model: 'fixture' });
    assert.equal(row.status, 401);
    assert.ok(!JSON.stringify(row).includes(credential));
    assert.ok(!(await readFile(join(directory, '001-error.json'), 'utf8')).includes(credential));
    assert.ok(!(await readFile(join(directory, 'results.json'), 'utf8')).includes(credential));
  } finally {
    globalThis.fetch = originalFetch;
    await rm(directory, { recursive: true, force: true });
  }
});

test('SSE usage survives CRLF split across byte chunks', async () => {
  const originalFetch = globalThis.fetch;
  const directory = await mkdtemp(join(tmpdir(), 'rcode-cache-study-'));
  try {
    const payload = 'data: {"choices":[],"usage":{"prompt_tokens":100,"completion_tokens":2,"prompt_cache_hit_tokens":90}}\r\n\r\ndata: [DONE]\r\n\r\n';
    globalThis.fetch = async () => new Response(new ReadableStream({
      start(controller) {
        for (const byte of new TextEncoder().encode(payload)) controller.enqueue(Uint8Array.of(byte));
        controller.close();
      },
    }), { headers: { 'content-type': 'text/event-stream' } });
    const row = await createStudy({ baseUrl: 'https://example.invalid', apiKey: 'fixture', directory }).call('stream', { model: 'm' });
    assert.equal(row.error, null);
    assert.equal(row.usage.hitPercent, 90);
  } finally {
    globalThis.fetch = originalFetch;
    await rm(directory, { recursive: true, force: true });
  }
});

test('non-streaming gateway envelopes retain inclusive Chat usage on a Messages route', async () => {
  const originalFetch = globalThis.fetch;
  const directory = await mkdtemp(join(tmpdir(), 'rcode-cache-study-'));
  try {
    globalThis.fetch = async () => Response.json({ success: true, data: {
      model: 'resolved-upstream', choices: [{ message: { content: 'OK' }, finish_reason: 'stop' }],
      usage: { prompt_tokens: 100, completion_tokens: 2, prompt_tokens_details: { cached_tokens: 90 }, cache_creation_input_tokens: 0 },
    } });
    const row = await createStudy({ baseUrl: 'https://example.invalid', apiKey: 'fixture', directory }).call('wrapped', { model: 'm', stream: false }, 'messages');
    assert.equal(row.responseModel, 'resolved-upstream');
    assert.equal(row.text, 'OK');
    assert.equal(row.usage.input, 100);
    assert.equal(row.usage.hitPercent, 90);
  } finally {
    globalThis.fetch = originalFetch;
    await rm(directory, { recursive: true, force: true });
  }
});
