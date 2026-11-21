// Runs one turn through a core's local socket (used by scripts/check-templates.sh): opens
// a workspace, starts a thread with a provider, sends a message, and checks the reply.
// Usage: node scripts/template-turn.mjs <socket> <workspace dir> <provider> <message> <expected reply>
import net from 'node:net';
import { createInterface } from 'node:readline';

const [socket, workspace, provider, message, expected] = process.argv.slice(2);
const c = net.connect(socket);
const lines = createInterface({ input: c });
let next = 0;
const waiting = new Map();
const events = [];
let onEvent = () => {};
lines.on('line', (l) => {
  const m = JSON.parse(l);
  if (m.method === 'events.event') {
    events.push(m.params);
    onEvent();
  } else if (waiting.has(m.id)) {
    const w = waiting.get(m.id);
    waiting.delete(m.id);
    m.error ? w.fail(new Error(`${m.error.message}`)) : w.ok(m.result);
  }
});
const call = (method, params) =>
  new Promise((ok, fail) => {
    const id = ++next;
    waiting.set(id, { ok, fail });
    c.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
  });
const deadline = setTimeout(() => {
  console.error('the turn did not end within 60 s');
  process.exit(1);
}, 60_000);

await new Promise((r) => c.once('connect', r));
await call('core.hello', { protocol: '0.6', client: { name: 'template-check', version: '0' } });
const { workspace: ws } = await call('workspace.open', { path: workspace });
const { thread } = await call('thread.create', { workspace: ws.id, provider });
await call('events.subscribe', { topic: `thread:${thread.id}`, after: 0 });
await call('thread.send', { thread: thread.id, input: { text: message } });
await new Promise((r) => {
  onEvent = () => events.some((e) => e.kind === 'turn.ended') && r();
  onEvent();
});
clearTimeout(deadline);
const ended = events.find((e) => e.kind === 'turn.ended').data;
const done = events.find((e) => e.kind === 'text.done')?.data.text;
c.end();
if (ended.outcome !== 'done' || done !== expected) {
  console.error(`expected a done turn replying ${JSON.stringify(expected)}, got ${JSON.stringify(ended)} / ${JSON.stringify(done)}`);
  process.exit(1);
}
console.log(`turn ok: ${done}`);
