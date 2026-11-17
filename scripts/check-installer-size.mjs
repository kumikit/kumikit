// Installer sizes against the budget (00-overview: ≤ 20 MB per platform; .deb, .rpm,
// .dmg and NSIS .exe; an AppImage bundles WebKitGTK and is not counted).
// Usage: node scripts/check-installer-size.mjs <tauri bundle directory>
import { readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';

const BUDGET = 20 * 1000 * 1000;
const dir = process.argv[2] ?? 'target/release/bundle';
const found = [];
const walk = (d) => {
  for (const f of readdirSync(d)) {
    const p = join(d, f);
    if (statSync(p).isDirectory()) walk(p);
    else if (/\.(deb|rpm|dmg)$|-setup\.exe$/.test(f)) found.push([p, statSync(p).size]);
  }
};
walk(dir);
if (found.length === 0) {
  console.error(`no installers under ${dir}`);
  process.exit(1);
}
let over = false;
for (const [p, size] of found) {
  const mb = (size / 1e6).toFixed(1);
  console.log(`${p}: ${mb} MB (budget ${BUDGET / 1e6} MB)`);
  if (size > BUDGET) over = true;
}
if (over) process.exit(1);
