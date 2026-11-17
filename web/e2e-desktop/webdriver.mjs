// A minimal W3C WebDriver client: only what the exit test uses, with no dependencies.
// Waits poll the page (WebDriver has no events), each with a deadline.

const ELEMENT = 'element-6066-11e4-a52e-4f735466cecf';
const KEYS = { Enter: '', Control: '', Escape: '' };

export class Session {
  constructor(base, id) {
    this.base = base;
    this.id = id;
  }

  static async start(base, capabilities) {
    const r = await call(base, 'POST', '/session', { capabilities: { alwaysMatch: capabilities } });
    return new Session(base, r.sessionId);
  }

  cmd(method, path, body) {
    return call(this.base, method, `/session/${this.id}${path}`, body);
  }

  /** The first element matching a CSS selector whose text contains `text`, if any.
   * Matched in the page on `textContent`: WebKitWebDriver's element text follows layout
   * and can miss text that is on screen. */
  async find(css, text) {
    const el = await this.execute(
      'return [...document.querySelectorAll(arguments[0])].find((e) => arguments[1] === null || e.textContent.includes(arguments[1])) ?? null',
      [css, text ?? null],
    );
    return el ? el[ELEMENT] : undefined;
  }

  /** Waits for an element (with `text`, if given) and returns it. */
  async wait(css, text, timeout = 30_000) {
    const end = Date.now() + timeout;
    for (;;) {
      const id = await this.find(css, text).catch(() => undefined);
      if (id) return id;
      if (Date.now() > end) throw new Error(`no ${css}${text ? ` with "${text}"` : ''} after ${timeout} ms`);
      await new Promise((r) => setTimeout(r, 100));
    }
  }

  async until(check, what, timeout = 30_000) {
    const end = Date.now() + timeout;
    while (!(await check())) {
      if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
      await new Promise((r) => setTimeout(r, 100));
    }
  }

  click(id) {
    return this.cmd('POST', `/element/${id}/click`, {});
  }

  text(id) {
    return this.execute('return arguments[0].textContent', [{ [ELEMENT]: id }]);
  }

  type(id, text) {
    return this.cmd('POST', `/element/${id}/value`, { text });
  }

  /** Presses keys together (`['Control', 'k']`) or one named key. */
  keys(names) {
    const list = Array.isArray(names) ? names : [names];
    const value = list.map((k) => KEYS[k] ?? k);
    const actions = [
      ...value.map((v) => ({ type: 'keyDown', value: v })),
      ...value.reverse().map((v) => ({ type: 'keyUp', value: v })),
    ];
    return this.cmd('POST', '/actions', { actions: [{ type: 'key', id: 'keyboard', actions }] });
  }

  execute(script, args = []) {
    return this.cmd('POST', '/execute/sync', { script, args });
  }

  /** Saves a screenshot (PNG) for a failure report. */
  async screenshot(path) {
    const { writeFileSync } = await import('node:fs');
    writeFileSync(path, Buffer.from(await this.cmd('GET', '/screenshot'), 'base64'));
  }

  end() {
    return this.cmd('DELETE', '');
  }
}

async function call(base, method, path, body) {
  const r = await fetch(base + path, {
    method,
    headers: { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const json = await r.json();
  if (!r.ok || (json.value && json.value.error)) {
    throw new Error(`WebDriver ${method} ${path}: ${json.value?.error ?? r.status} ${json.value?.message ?? ''}`);
  }
  return json.value;
}
