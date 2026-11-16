// The UI kernel's compressed JavaScript against its budget (00-overview: ≤ 150 KB):
// gzip -9 of every JS file in the desktop build. Plugins are not part of the kernel.
// Usage: node scripts/size.mjs (in web/kernel).
import { execSync } from 'node:child_process';
import { readFileSync, readdirSync } from 'node:fs';
import { gzipSync } from 'node:zlib';

const BUDGET = 150 * 1024;
const dir = 'dist/desktop/assets';
// Always build, so the number is never from a stale bundle (a build takes well under a second).
execSync('npx vite build --logLevel warn', { stdio: 'inherit' });
let total = 0;
for (const f of readdirSync(dir).filter((f) => f.endsWith('.js'))) {
  const size = gzipSync(readFileSync(`${dir}/${f}`), { level: 9 }).length;
  total += size;
  console.log(`  ${f}: ${(size / 1024).toFixed(1)} KB`);
}
console.log(`UI kernel JavaScript, gzip -9: ${(total / 1024).toFixed(1)} KB (budget ${BUDGET / 1024} KB)`);
if (total > BUDGET) process.exit(1);
