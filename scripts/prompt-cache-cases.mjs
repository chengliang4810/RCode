// Controlled fixtures share model-facing content; only the named variable changes.
import { createHash } from 'node:crypto';

export function makeRecords(count = 350) {
  return Array.from({ length: count }, (_, i) =>
    `Record ${i}: module=component_${i}; owner=cache_study; checksum=${createHash('sha256').update('row:' + i).digest('hex').slice(0, 24)}; status=verified.`,
  ).join('\n');
}

export function runtimeCase(wire, { model, seed, placement, code, history }) {
  const body = structuredClone(wire);
  body.model = model;
  body.max_tokens = 128;
  body.stream_options = { include_usage: true };
  const system = `Cache experiment ${seed}.\n${wire.messages[0].content}\n`
    + 'For this isolated protocol fixture, do not call tools. Reply only with the current runtime code. '
    + 'The newest runtime-context snapshot supersedes earlier snapshots.';
  const context = `Current runtime context: runtime code is ${code}.`;
  body.messages = [{ role: 'system', content: system + (placement === 'system' ? '\n' + context : '') }, ...structuredClone(history)];
  if (placement === 'tail') body.messages.push({ role: 'user', content: context });
  body.messages.push({ role: 'user', content: 'What is the current runtime code? Reply with the code only.' });
  return body;
}

export function retainedMessages(body, response) {
  const result = structuredClone(body.messages.slice(1));
  const assistant = { role: 'assistant', content: response.text };
  const reasoning = response.responseEvents?.map(event => event.choices?.[0]?.delta)
    .map(delta => delta?.reasoning_content ?? delta?.reasoning ?? '').join('');
  if (reasoning) assistant.reasoning_content = reasoning;
  result.push(assistant);
  return result;
}

export function sortedTools(body) {
  const result = structuredClone(body);
  result.tools.sort((a, b) => {
    const left = a.function?.name ?? a.name;
    const right = b.function?.name ?? b.name;
    return left < right ? -1 : left > right ? 1 : 0;
  });
  return result;
}
