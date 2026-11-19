// The stand-in UI kernel: frames the panel, hands it a port, relays its report.
'use strict';
// In the shell, reports go to its stdout (test_report); in a browser (served by a core
// with --web), to the console, where the Playwright spec reads them.
const shell = typeof window.__TAURI_INTERNALS__ === 'object';
const report = shell
  ? (text) => window.__TAURI_INTERNALS__.invoke('test_report', { text })
  : (text) => console.log('report ' + text);
const base = !shell
  ? '/ui-plugins/test.panel-smoke/'
  : location.protocol === 'http:'
    ? 'http://kumikit-plugin.localhost/test.panel-smoke/'
    : 'kumikit-plugin://test.panel-smoke/';
report('parent ' + JSON.stringify({ origin: location.origin, ua: navigator.userAgent }));
const frame = document.createElement('iframe');
frame.setAttribute('sandbox', 'allow-scripts');
frame.setAttribute('referrerpolicy', 'no-referrer');
let loads = 0;
frame.addEventListener('load', () => {
  loads += 1;
  if (loads > 1) {
    report('navigated');
    return;
  }
  const channel = new MessageChannel();
  channel.port1.onmessage = (event) => {
    report('panel ' + JSON.stringify(event.data));
    if (event.data && event.data.done) setTimeout(() => report('done'), 1500);
  };
  frame.contentWindow.postMessage({ type: 'kumikit.panel', version: 1 }, '*', [channel.port2]);
});
frame.src = base + 'index.html';
document.getElementById('host').append(frame);
setTimeout(() => report('timeout'), 20000);
