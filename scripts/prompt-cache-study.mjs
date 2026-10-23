// Explicitly invoked live-cache experiment helper. Credentials never enter artifacts.
import { mkdir, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { join } from 'node:path';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { pathToFileURL } from 'node:url';

export const hash = value => createHash('sha256').update(JSON.stringify(value)).digest('hex');

export function normalizeUsage(usage, protocol) {
  if (!usage) return null;
  const read = usage.prompt_tokens_details?.cached_tokens
    ?? usage.input_tokens_details?.cached_tokens
    ?? usage.prompt_cache_hit_tokens ?? usage.cache_read_input_tokens;
  const write = usage.cache_creation_input_tokens ?? null;
  const input = usage.prompt_tokens ?? (usage.input_tokens === undefined ? null
    : usage.input_tokens + (protocol === 'messages' ? (read ?? 0) + (write ?? 0) : 0));
  return {
    input, cached: read ?? null, cacheWrite: write,
    output: usage.completion_tokens ?? usage.output_tokens ?? null,
    uncached: input === null || read === undefined ? null : input - read,
    hitPercent: input > 0 && read !== undefined ? 100 * read / input : null,
    cost: usage.cost ?? null,
  };
}

export function createStudy({ baseUrl, apiKey, directory, timeoutMs = 55000, records = [] }) {
  if (typeof apiKey !== 'string' || apiKey.length === 0) throw new Error('An in-memory API key is required');
  const safeJson = value => JSON.stringify(value, null, 2).split(apiKey).join('[redacted]');
  let sequence = records.length;
  const paths = { chat: '/v1/chat/completions', messages: '/v1/messages', responses: '/v1/responses' };
  async function call(label, body, protocol = 'chat', sessionId, options = {}) {
    const index = ++sequence;
    const started = Date.now();
    const headers = { 'Content-Type': 'application/json', Authorization: `Bearer ${apiKey}` };
    if (sessionId) headers['x-session-id'] = sessionId;
    if (protocol === 'messages') {
      headers['x-api-key'] = apiKey;
      headers['anthropic-version'] = '2023-06-01';
      if (options.anthropicBeta) headers['anthropic-beta'] = options.anthropicBeta;
    }
    let status = null, firstByteMs = null, usage = null, text = '', reasoning = '';
    let finish = null, error = null, responseModel = null, systemFingerprint = null, assistantMessage = null;
    const responseHeaders = {};
    const toolCalls = [], events = [];
    const accept = event => {
      events.push(event);
      // This gateway wraps non-streaming responses, including Messages, in data.
      if (event.success === true && event.data && typeof event.data === 'object') {
        accept(event.data);
        return;
      }
      if (event.success === false) {
        error = typeof event.message === 'string' ? event.message : 'Gateway reported failure';
      }
      responseModel ??= event.model ?? event.message?.model ?? event.response?.model ?? null;
      systemFingerprint ??= event.system_fingerprint ?? null;
      if (event.usage) usage = { ...usage, ...event.usage };
      if (event.message?.usage) usage = { ...usage, ...event.message.usage };
      if (event.response?.usage) usage = event.response.usage;
      const choice = event.choices?.[0];
      if (choice) {
        // Keep non-streaming assistant fields (including reasoning/tool IDs) for exact replay.
        if (choice.message) assistantMessage = { ...choice.message, role: 'assistant' };
        const content = choice.delta ?? choice.message ?? {};
        text += content.content ?? '';
        reasoning += content.reasoning_content ?? content.reasoning ?? '';
        if (content.tool_calls) toolCalls.push(...content.tool_calls);
        finish = choice.finish_reason ?? finish;
      }
      if (event.type === 'content_block_delta') {
        text += event.delta?.text ?? '';
        reasoning += event.delta?.thinking ?? '';
      }
      if (event.type === 'message') {
        for (const block of event.content ?? []) {
          if (block.type === 'text') text += block.text;
          if (block.type === 'thinking') reasoning += block.thinking;
          if (block.type === 'tool_use') toolCalls.push(block);
        }
        finish = event.stop_reason ?? finish;
      }
      if (event.type === 'message_delta') finish = event.delta?.stop_reason ?? finish;
      if (event.type === 'response.output_text.delta') text += event.delta ?? '';
      if (event.type === 'response.completed') finish = event.response?.status;
      if (event.error) error = typeof event.error === 'string' ? event.error : JSON.stringify(event.error);
    };
    try {
      const response = await fetch(baseUrl.replace(/\/$/, '') + paths[protocol], {
        method: 'POST', headers, body: JSON.stringify(body), signal: AbortSignal.timeout(timeoutMs),
      });
      status = response.status;
      for (const name of ['request-id', 'x-request-id', 'x-deepseek-request-id', 'x-oneapi-request-id', 'x-cache', 'x-cache-status']) {
        const value = response.headers.get(name);
        if (value) responseHeaders[name] = value;
      }
      if (!(response.headers.get('content-type') ?? '').includes('text/event-stream')) {
        const raw = await response.text();
        firstByteMs = Date.now() - started;
        try { accept(JSON.parse(raw)); } catch { error = `Non-JSON response (${raw.length} characters)`; }
        if (!response.ok && !error) error = `HTTP ${status}`;
      } else {
        let pending = '';
        const decoder = new TextDecoder();
        const consume = frame => {
          const data = frame.split('\n').filter(line => line.startsWith('data:'))
            .map(line => line.slice(5).trimStart()).join('\n');
          if (data && data !== '[DONE]') accept(JSON.parse(data));
        };
        for await (const bytes of response.body) {
          firstByteMs ??= Date.now() - started;
          pending = (pending + decoder.decode(bytes, { stream: true })).replace(/\r\n/g, '\n');
          let end;
          while ((end = pending.indexOf('\n\n')) !== -1) {
            consume(pending.slice(0, end));
            pending = pending.slice(end + 2);
          }
        }
        pending += decoder.decode();
        if (pending.trim()) consume(pending);
      }
    } catch (caught) { error = `${caught.name}: ${caught.message}`; }
    // The key is never sent as model content, and error text is scrubbed defensively.
    if (error) error = error.split(apiKey).join('[redacted]');
    const row = JSON.parse(safeJson({
      index, label, protocol, requestedModel: body.model, responseModel, sessionId,
      systemFingerprint, responseHeaders,
      ...(options.anthropicBeta ? { anthropicBeta: options.anthropicBeta } : {}),
      startedAt: new Date(started).toISOString(), durationMs: Date.now() - started,
      firstByteMs, status, finish, error, requestHash: hash(body),
      usage: normalizeUsage(usage, protocol), rawUsage: usage,
      text, reasoningCharacters: reasoning.length, toolCalls, assistantMessage,
    }));
    await mkdir(directory, { recursive: true });
    const name = String(index).padStart(3, '0') + '-' + label.replace(/[^a-z0-9_-]/gi, '_');
    await writeFile(join(directory, name + '.json'), safeJson({ ...row, requestBody: body, responseEvents: events }));
    records.push(row);
    await writeFile(join(directory, 'results.json'), safeJson(records));
    return row;
  }
  return { call, records };
}

/** Runs the unmodified DSH text/tool serialization functions without booting its plugin graph.
 * Framework error construction is replaced locally; images are explicitly unsupported by this probe.
 * This is a serialization comparison, not a full DSH application run.
 */
export async function loadDshTextSerializer(root, generatedDirectory) {
  const folder = join(root, 'packages/llm/llm-deepseek/src');
  const sources = await Promise.all(['replay.ts', 'serialize.ts'].map(name => readFile(join(folder, name), 'utf8')));
  const compile = source => stripTypeScriptTypes(source)
    .replaceAll("'@deepseek-ai/dsh-llm'", "'./framework.mjs'").replaceAll("'./replay.ts'", "'./replay.mjs'");
  await mkdir(generatedDirectory, { recursive: true });
  await writeFile(join(generatedDirectory, 'framework.mjs'),
    'export class LlmError extends Error { constructor(message, code) { super(message); this.code = code; } }\n'
    + 'export function requestImageHandleText() { throw new Error("The cache probe supports text/tool fixtures only"); }\n');
  await writeFile(join(generatedDirectory, 'replay.mjs'), compile(sources[0]));
  await writeFile(join(generatedDirectory, 'serialize.mjs'), compile(sources[1]));
  const { serialize } = await import(pathToFileURL(join(generatedDirectory, 'serialize.mjs')).href);
  return { serialize, sourceHashes: sources.map(hash) };
}
