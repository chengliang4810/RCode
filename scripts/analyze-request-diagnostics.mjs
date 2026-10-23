import { readFileSync, existsSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

export function compareRequests(previous, current) {
  if (!previous) return { classification: 'baseline', commonMessages: 0 };
  const old = previous.fingerprint;
  const next = current.fingerprint;
  const systemChanged = old.system.sha256 !== next.system.sha256;
  const toolsChanged = old.tools.sha256 !== next.tools.sha256;
  const parametersChanged = old.parameters.sha256 !== next.parameters.sha256;
  const routeChanged = previous.protocol !== current.protocol || previous.origin !== current.origin
    || previous.endpointSha256 !== current.endpointSha256;
  let commonMessages = 0;
  while (commonMessages < Math.min(old.messages.length, next.messages.length)
      && old.messages[commonMessages].sha256 === next.messages[commonMessages].sha256
      && old.messages[commonMessages].role === next.messages[commonMessages].role) commonMessages++;
  let classification;
  if (systemChanged || toolsChanged || parametersChanged || routeChanged) classification = 'prefix_changed';
  else if (old.wire.sha256 === next.wire.sha256) classification = 'identical';
  else if (old.messagesTruncated || next.messagesTruncated) classification = 'unknown_truncated';
  else if (commonMessages < Math.min(old.messageCount, next.messageCount)) classification = 'history_changed';
  else if (next.messageCount < old.messageCount) classification = 'history_reduced';
  else if (next.messageCount > old.messageCount) classification = 'append_only';
  else classification = 'other_wire_change';
  return { classification, systemChanged, toolsChanged, parametersChanged, routeChanged, commonMessages,
    previousMessages: old.messageCount, messages: next.messageCount };
}

export function analyze(records, sessionId) {
  const attempts = new Map();
  const links = new Map();
  for (const record of records) {
    if (record.version !== 1 || (sessionId && record.sessionId !== sessionId)) continue;
    if (record.kind === 'model_call_link') { links.set(record.callId, record.modelCallId); continue; }
    if (!record.attemptId) continue;
    const attempt = attempts.get(record.attemptId) ?? { metadata: [] };
    if (record.kind === 'request') attempt.request = record;
    if (record.kind === 'headers') attempt.headers = record;
    if (record.kind === 'metadata') attempt.metadata.push(record);
    if (record.kind === 'finished') attempt.finished = record;
    attempts.set(record.attemptId, attempt);
  }
  const previous = new Map();
  return [...attempts.values()].filter(x => x.request).map(({ request, headers, metadata, finished }) => {
    const group = JSON.stringify([request.sessionId, request.purpose]);
    const comparison = compareRequests(previous.get(group), request);
    previous.set(group, request);
    const usage = finished?.usage;
    const input = usage?.input_tokens ?? null;
    const cached = usage?.cache_read_input_tokens ?? null;
    return { sessionId: request.sessionId, assistantMessageId: request.assistantMessageId, callId: request.callId,
      modelCallId: links.get(request.callId) ?? null, attemptId: request.attemptId, attempt: request.attempt,
      timestampMs: request.timestampMs, purpose: request.purpose, ...comparison,
      outcome: finished?.outcome ?? 'incomplete', errorKind: finished?.errorKind ?? null,
      status: headers?.status ?? null, durationMs: finished?.durationMs ?? null,
      inputTokens: input, cacheReadTokens: cached,
      uncachedTokens: input !== null && cached !== null ? input - cached : null,
      cacheHitPercent: input > 0 && cached !== null && cached <= input ? 100 * cached / input : null,
      returnedModels: [...new Set(metadata.flatMap(x => x.model ? [x.model] : []))],
      requestIds: [...new Set([headers?.requestId, finished?.requestId].filter(Boolean))],
      responseIds: [...new Set(metadata.flatMap(x => x.responseId ? [x.responseId] : []))],
      rawUsage: metadata.filter(x => Object.keys(x.rawUsage ?? {}).length).map(x => ({ source: x.source, usage: x.rawUsage })),
    };
  });
}

export function loadRecords(path) {
  const files = statSync(path).isDirectory()
    ? ['requests.2.jsonl', 'requests.1.jsonl', 'requests.jsonl'].map(name => join(path, name)).filter(existsSync)
    : [path];
  const records = [];
  for (const file of files) {
    const text = readFileSync(file, 'utf8');
    const lines = text.split('\n');
    for (let i = 0; i < lines.length; i++) {
      if (!lines[i].trim()) continue;
      // A running writer may still be appending the final line; never treat it as a complete record.
      if (i === lines.length - 1 && !text.endsWith('\n')) continue;
      try { records.push(JSON.parse(lines[i])); }
      catch { throw new Error(`Invalid completed diagnostic record in ${file}, line ${i + 1}`); }
    }
  }
  return records;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const path = process.argv[2];
  if (!path) { console.error('Usage: node scripts/analyze-request-diagnostics.mjs <log-directory-or-file> [native-session-id]'); process.exitCode = 1; }
  else {
    try { console.log(JSON.stringify(analyze(loadRecords(path), process.argv[3]), null, 2)); }
    catch (error) { console.error(error.message); process.exitCode = 1; }
  }
}
