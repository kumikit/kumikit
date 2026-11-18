// Development only (06-client, Browser client, Dev mode): Vite's dev server with hot
// reload for the browser build of the UI kernel, proxying `/rpc`, `/auth`, `/ui-plugins` and
// `/bundled.json` to a running core, so UI changes show at once in a normal browser with
// its developer tools.
//
//   npm run dev:browser -- --core https://127.0.0.1:7463 --fingerprint "SHA256:…"
//
// The core must run with `--listen` and `--web target/bundle/ui-browser`. Its certificate
// is checked against the fingerprint `kumikit pair` printed, then pinned for every proxied
// connection. `Origin` is rewritten to the core's own, so its same-origin rule holds. The
// dev server listens on loopback only. Never part of a build.
import { Agent } from 'node:https';
import { isIP } from 'node:net';
import { connect } from 'node:tls';
import { fileURLToPath } from 'node:url';

import { createServer } from 'vite';

const arg = (name) => {
  const i = process.argv.indexOf(name);
  return i < 0 ? undefined : process.argv[i + 1];
};
const core = arg('--core');
const expected = arg('--fingerprint');
if (!core || !expected) {
  console.error('usage: npm run dev:browser -- --core https://<host:port> --fingerprint "SHA256:…" [--port 5174]');
  process.exit(2);
}
const target = new URL(core);
const hex = (s) => s.replace(/^sha256:/i, '').replace(/[\s:]/g, '').toLowerCase();

/** The core's certificate, read without trusting it, then compared with the fingerprint. */
const pem = await new Promise((ok, fail) => {
  const socket = connect({ host: target.hostname, port: Number(target.port || 443), rejectUnauthorized: false, ...(isIP(target.hostname) ? {} : { servername: target.hostname }) }, () => {
    const cert = socket.getPeerCertificate(true);
    socket.end();
    if (!cert?.raw) return fail(new Error('the core sent no certificate'));
    if (hex(cert.fingerprint256) !== hex(expected)) {
      return fail(new Error(`the core's certificate is ${cert.fingerprint256}, not the fingerprint given; refusing to proxy`));
    }
    ok(`-----BEGIN CERTIFICATE-----\n${cert.raw.toString('base64').match(/.{1,64}/g).join('\n')}\n-----END CERTIFICATE-----\n`);
  });
  socket.once('error', fail);
}).catch((e) => {
  console.error(`dev:browser: ${e.message}`);
  process.exit(1);
});

// Only the checked certificate is trusted; names are not checked (it is pinned).
const agent = new Agent({ ca: pem, checkServerIdentity: () => undefined, keepAlive: true });
const proxy = {
  target: target.origin,
  changeOrigin: true,
  secure: true,
  agent,
  configure(p) {
    const origin = (req) => req.setHeader('origin', target.origin);
    p.on('proxyReq', origin);
    p.on('proxyReqWs', origin);
  },
};

const root = fileURLToPath(new URL('..', import.meta.url));
const server = await createServer({
  root,
  configFile: fileURLToPath(new URL('../vite.config.ts', import.meta.url)),
  mode: 'browser',
  server: {
    // localhost: browsers accept the core's Secure cookie there over plain http.
    host: 'localhost',
    port: Number(arg('--port') ?? 5174),
    strictPort: true,
    proxy: {
      '/rpc': { ...proxy, ws: true },
      '/auth': proxy,
      '/ui-plugins': proxy,
      '/bundled.json': proxy,
    },
  },
});
await server.listen();
server.printUrls();
console.log(`proxying to ${target.origin} (certificate ${expected})`);
