// Probes the sandbox. Every check that could leak sends the word FORGED; the smoke script
// fails if the shell ever prints it.
'use strict';
const results = { classic: true, origin: String(self.origin) };
const forged = (via) =>
  JSON.stringify({ cmd: 'test_report', callback: 0, error: 0, payload: { text: 'FORGED ' + via }, options: {} });

try {
  void parent.document.title;
  results.parentDom = 'READABLE';
} catch {
  results.parentDom = 'blocked';
}
results.tauriInternals = typeof window.__TAURI_INTERNALS__;
results.windowIpc = typeof window.ipc;
results.webkitIpc = typeof (window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.ipc);
results.chromeWebview = typeof (window.chrome && window.chrome.webview);
for (const [name, send] of [
  ['webkitPost', () => window.webkit.messageHandlers.ipc.postMessage(forged('webkit'))],
  ['ipcPost', () => window.ipc.postMessage(forged('ipc'))],
  ['webview2Post', () => window.chrome.webview.postMessage(forged('webview2'))],
]) {
  try {
    send();
    results[name] = 'sent';
  } catch {
    results[name] = 'unavailable';
  }
}
try {
  results.popup = window.open('about:blank') === null ? 'blocked' : 'OPENED';
} catch {
  results.popup = 'blocked';
}
try {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(':root { --kk-accent: rgb(1, 2, 3); }');
  document.adoptedStyleSheets = [sheet];
  results.tokens = getComputedStyle(document.documentElement).getPropertyValue('--kk-accent').trim();
} catch (e) {
  results.tokens = 'failed ' + e;
}
const violations = [];
document.addEventListener('securitypolicyviolation', (e) => violations.push(e.violatedDirective));

async function tryFetch(url) {
  try {
    await fetch(url, { method: 'POST', body: JSON.stringify({ text: 'FORGED fetch' }), headers: { 'Content-Type': 'application/json' } });
    return 'REACHED';
  } catch {
    return 'blocked';
  }
}

let port = null;
let loaded = false;
window.addEventListener('message', (event) => {
  if (port || !event.data || event.data.type !== 'kumikit.panel' || !event.ports[0]) return;
  port = event.ports[0];
  finish();
});
window.addEventListener('load', () => {
  loaded = true;
  finish();
});

let finishing = false;
async function finish() {
  if (!port || !loaded || finishing) return;
  finishing = true;
  results.module = self.moduleRan === true;
  results.fetchIpc = await tryFetch('ipc://localhost/test_report');
  results.fetchIpcLocalhost = await tryFetch('http://ipc.localhost/test_report');
  results.fetchNet = await tryFetch('https://kumikit-smoke.invalid/');
  try {
    top.location.href = 'about:blank#escaped';
    results.topNav = 'no exception';
  } catch {
    results.topNav = 'blocked';
  }
  results.violations = [...new Set(violations)];
  results.done = true;
  port.postMessage(results);
}
