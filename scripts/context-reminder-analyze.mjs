// Offline analysis; no credentials, networking, or changes to original response records.
import { readFile, readdir, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { applyContextEvent, digest, metrics, parseAnswer } from './context-reminder-prototype.mjs';

const ordered = object => Object.fromEntries(Object.entries(object).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0));
const equal = (a, b) => a !== null && JSON.stringify(ordered(a)) === JSON.stringify(ordered(b));

export function extractBusinessAnswer(text) {
  const strict = parseAnswer(text);
  if (strict !== null) return strict;
  // Keep the strict-format result separate. Explanations do not make correct fields incorrect.
  const fenced = [...text.matchAll(/```(?:json)?\s*([\s\S]*?)```/gi)];
  if (fenced.length !== 1) return null;
  try { return JSON.parse(fenced[0][1]); } catch { return null; }
}

function stateInRequest(body, mode) {
  if (mode === 'head') {
    const marker = '\n\nCurrent host context:\n';
    const system = body.messages[0].content;
    const position = system.lastIndexOf(marker);
    if (position < 0) throw new Error('Missing head context');
    return applyContextEvent(null, JSON.parse(system.slice(position + marker.length)));
  }
  let state = null;
  for (const message of body.messages) {
    if (message.role !== 'user' || typeof message.content !== 'string' || !message.content.startsWith('<system-reminder>\n')) continue;
    const text = message.content.slice('<system-reminder>\n'.length, -'\n</system-reminder>'.length)
      .replaceAll('&lt;', '<').replaceAll('&gt;', '>').replaceAll('&amp;', '&');
    state = applyContextEvent(state, JSON.parse(text));
  }
  return state;
}

function expectationFromState(state, domain) {
  if (domain === 'mcp-instructions') {
    return { serviceTag: state.entries.fixture?.match(/exactly ([\w-]+)\./)?.[1] ?? null };
  }
  const value = (key, label) => state.status !== 'ready' ? null
    : state.entries[key]?.match(new RegExp(`${label}: (.+)\\.$`))?.[1] ?? null;
  return { status: state.status, deploymentRegion: value('project_deployment.md', 'Deployment region'),
    legacyEndpoint: value('reference_legacy.md', 'Legacy endpoint'), runbookPath: value('reference_runbook.md', 'Runbook path') };
}

export async function analyze(directory) {
  const evaluations = JSON.parse(await readFile(join(directory, 'evaluations.json'), 'utf8'));
  const records = JSON.parse(await readFile(join(directory, 'results.json'), 'utf8'));
  const names = await readdir(directory);
  const byIndex = new Map(records.map(row => [row.index, row]));
  const checked = [];
  for (const evaluation of evaluations) {
    const row = byIndex.get(evaluation.requestIndex);
    const artifactName = names.find(name => name.startsWith(String(row.index).padStart(3, '0') + '-') && name.endsWith('.json'));
    const artifact = JSON.parse(await readFile(join(directory, artifactName), 'utf8'));
    const state = stateInRequest(artifact.requestBody, evaluation.mode);
    const expectedFromWire = expectationFromState(state, evaluation.domain);
    let inputMatches = true;
    if (evaluation.scenario === 'read-latest-pointer') inputMatches = expectedFromWire.runbookPath === evaluation.detail?.filePath;
    else if (evaluation.scenario !== 'read-result') inputMatches = equal(expectedFromWire, evaluation.expected);
    const businessAnswer = extractBusinessAnswer(row.text);
    const businessPassed = evaluation.scenario === 'read-latest-pointer' ? evaluation.passed
      : row.status === 200 && !row.error && row.toolCalls.length === 0 && equal(businessAnswer, evaluation.expected);
    checked.push({ ...evaluation, inputMatches, businessAnswer, businessPassed, formatPassed: evaluation.passed,
      finish: row.finish, reasoningCharacters: row.reasoningCharacters });
  }
  const summarize = values => ({ cases: values.length, businessPassed: values.filter(x => x.businessPassed).length,
    formatPassed: values.filter(x => x.formatPassed).length, inputsVerified: values.filter(x => x.inputMatches).length,
    ...metrics(values.map(x => byIndex.get(x.requestIndex))) });
  const groups = {};
  for (const e of checked) {
    for (const phase of ['all', e.phase]) {
      for (const model of ['all', e.responseModel]) {
        (groups[`${e.domain}/${e.mode}/${phase}/${model}`] ??= []).push(e);
      }
    }
  }
  const memoryChanges = checked.filter(x => x.domain === 'memory-index' && x.phase === 'changed');
  const matchedScenarios = memoryChanges.filter(x => x.mode === 'head' && memoryChanges.some(y =>
    y.mode === 'delta' && x.scenario === y.scenario && x.responseModel === y.responseModel)).map(x => x.scenario);
  const result = { definitions: {
    business: 'Exact expected fields, allowing a single fenced JSON object plus explanatory prose. Tool case requires exact Read path.',
    format: 'Original strict judge: only requested JSON (a surrounding code fence is accepted), without extra prose.',
    cache: 'Sum cached input tokens / sum reported input tokens; missing counters excluded, never zero-filled.',
    matched: 'Same update scenario and same returned model identifier in head and delta arms; previous routing can still differ.',
    scope: 'Synthetic versioned-context prototype over captured RCode system/tools; not a production client trial. No real file or MCP tools executed.',
  }, overall: summarize(checked), groups: Object.fromEntries(Object.entries(groups).map(([key, values]) => [key, summarize(values)])),
    matchedMemoryChanges: { scenarios: matchedScenarios, head: summarize(memoryChanges.filter(x => x.mode === 'head' && matchedScenarios.includes(x.scenario))),
      delta: summarize(memoryChanges.filter(x => x.mode === 'delta' && matchedScenarios.includes(x.scenario))) },
    modelCounts: Object.fromEntries([...new Set(records.map(x => x.responseModel))].map(model => [model, records.filter(x => x.responseModel === model).length])),
    failures: checked.filter(x => !x.businessPassed || !x.inputMatches),
    formatOnlyFailures: checked.filter(x => x.businessPassed && !x.formatPassed).map(x => x.requestIndex),
    sourceHashes: Object.fromEntries(await Promise.all(['context-reminder-prototype.mjs', 'context-reminder-live.mjs', 'prompt-cache-study.mjs']
      .map(async name => [name, digest(await readFile(new URL(name, import.meta.url), 'utf8'))]))) };
  await writeFile(join(directory, 'analysis.json'), JSON.stringify(result, null, 2));
  await writeFile(join(directory, 'evaluations-reviewed.json'), JSON.stringify(checked, null, 2));
  const columns = ['requestIndex', 'domain', 'mode', 'scenario', 'phase', 'responseModel', 'businessPassed', 'formatPassed', 'inputMatches', 'input', 'cached', 'uncached', 'hitPercent', 'durationMs'];
  const csv = [columns.join(','), ...checked.map(row => columns.map(key => JSON.stringify(row[key] ?? row.usage?.[key] ?? '')).join(','))].join('\n');
  await writeFile(join(directory, 'results.csv'), csv + '\n');
  return result;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  if (!process.argv[2]) throw new Error('Pass the experiment artifact directory');
  const result = await analyze(resolve(process.argv[2]));
  console.log(JSON.stringify({ overall: result.overall, matchedMemoryChanges: result.matchedMemoryChanges,
    modelCounts: result.modelCounts, failures: result.failures.map(x => ({ request: x.requestIndex, scenario: x.scenario, mode: x.mode })) }, null, 2));
}
