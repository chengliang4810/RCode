// 通过本机 RPC 驱动真实桌面：建会话、发真实 prompt、观察标题与流式回复。
// 只用 Node 原生 WebSocket —— 它不发送 Origin 头，因此能通过本地 IPC 的握手检查
// （.NET ClientWebSocket 会发送 Origin，被服务器按设计拒绝）。
// 不读取凭据：进程环境由调用方提供。
import { writeFileSync } from 'node:fs';

const port = Number(process.argv[2]);
const deviceHint = process.argv[3];
const project = process.argv[4];
const evidencePath = process.argv[5];

const out = { device: deviceHint, calls: [], notes: [] };
const ws = new WebSocket(`ws://127.0.0.1:${port}`);
let nextId = 100;
const pending = new Map();
const watchers = [];

const send = (method, params) => new Promise((resolve, reject) => {
  const id = nextId++;
  const timer = setTimeout(() => reject(Error(`${method} timed out`)), 60000);
  pending.set(id, (message) => { clearTimeout(timer); resolve(message); });
  ws.send(JSON.stringify({ id, method, params }));
});

// 单条流的后续帧不带新的 id；这类观察器收集所有匹配 id 的帧直到结束。
const watch = (method, params, predicate, timeoutMs) => new Promise((resolve, reject) => {
  const id = nextId++;
  const seen = [];
  const timer = setTimeout(() => resolve({ timedOut: true, seen }), timeoutMs);
  watchers.push({ id, seen, predicate, resolve: (value) => { clearTimeout(timer); resolve(value); } });
  ws.send(JSON.stringify({ id, method, params }));
});

function handle(message) {
  const routed = pending.get(message.id);
  if (routed) { pending.delete(message.id); routed(message); return; }
  for (const entry of watchers) {
    if (entry.id !== message.id) continue;
    entry.seen.push(message);
    if (entry.predicate && entry.predicate(message, entry.seen)) {
      watchers.splice(watchers.indexOf(entry), 1);
      entry.resolve({ matched: true, seen: entry.seen });
      return;
    }
  }
}

ws.addEventListener('message', (event) => handle(JSON.parse(event.data)));
ws.addEventListener('error', (error) => { out.notes.push(`ws error: ${error.message ?? error}`); });

ws.addEventListener('open', async () => {
  try {
    const ready = await send('EngineReady', {});
    out.calls.push({ method: 'EngineReady', ok: !!ready.ok });
    const device = await send('LocalDevice', {});
    const deviceId = device.ok?.deviceId ?? deviceHint;
    out.device = deviceId;
    const models = await send('ListModels', { harness: 'native' });
    out.calls.push({ method: 'ListModels', models: JSON.stringify(models.ok).slice(0, 400) });

    const spaceId = crypto.randomUUID();
    const chatId = crypto.randomUUID();
    const messageId = crypto.randomUUID();
    const prompt = '请用一句中文确认这是真实对话，并包含标记 NATIVE_DESKTOP_E2E。不要调用工具。';

    const spaces = await send('Mutate', { op: 'createSpace', spaceId, deviceId, path: project.replace(/\\/g, '/'), name: 'Native E2E', gitDetected: false });
    out.calls.push({ method: 'createSpace', ok: !!spaces.ok });
    const chat = await send('Mutate', { op: 'createChat', chatId, spaceId, config: { harness: 'native', model: null, reasoning: null, modelOptions: {}, sandbox: 'read-only' } });
    out.calls.push({ method: 'createChat', ok: !!chat.ok });

    // 先订阅标题变化（chat 列表）与 transcript，再发命令，避免漏帧。
    const titleWatch = watch('WatchChats', {}, (message) => JSON.stringify(message).includes('"title":"'), 90000);
    // 判定必须落在 assistant 角色上：用户的 prompt 本身含同一标记，只看文本会假阳性。
    const answerWatch = watch(
      'WatchDocMessages',
      { chatId },
      (message, seen) => seen.some((frame) => /"role":"assistant"/.test(JSON.stringify(frame))
        && /NATIVE_DESKTOP_E2E/.test(JSON.stringify(frame))),
      90000,
    );

    const queued = await send('QueueCommand', { chatId, command: { kind: 'run', messageId, request: { prompt, harness: 'native', model: null, reasoning: null, modelOptions: {}, cwd: project.replace(/\\/g, '/'), sandbox: 'read-only', autoApprove: false, resume: null } } });
    out.calls.push({ method: 'QueueCommand', ok: !!queued.ok, commandId: queued.ok?.commandId ?? null });

    const titled = await titleWatch;
    const answered = await answerWatch;
    out.titleObserved = !!titled.matched;
    out.answerObserved = !!answered.matched;
    out.titleFrames = titled.seen.slice(-3).map((m) => JSON.stringify(m).slice(0, 300));
    out.answerFrames = answered.seen.slice(-3).map((m) => JSON.stringify(m).slice(0, 400));
  } catch (error) {
    out.notes.push(`drive failed: ${error.message}`);
  } finally {
    writeFileSync(evidencePath, JSON.stringify(out, null, 2));
    ws.close();
    setTimeout(() => process.exit(0), 500);
  }
});
