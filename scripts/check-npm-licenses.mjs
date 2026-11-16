// Checks every npm package in package-lock.json against the licence list in AGENTS.md
// (Security and privacy). Run after npm dependencies change; CI runs it.
// Usage: node scripts/check-npm-licenses.mjs   (from the repository root)
import { readFileSync } from 'node:fs';

const ALLOWED = new Set([
  'MIT', 'MIT-0', 'Apache-2.0', 'BSD-2-Clause', 'BSD-3-Clause', 'ISC', 'Zlib', 'MPL-2.0',
  // Accepted in the M0 plan for Rust (deny.toml); the same reasoning applies here.
  'Unicode-3.0',
]);
// Licences outside the AGENTS.md list must be accepted here, each with a reason.
const ACCEPTED = {};

const lock = JSON.parse(readFileSync('package-lock.json', 'utf8'));
// An SPDX expression is allowed if one side of every OR is allowed and every part of an
// AND is allowed.
function allowed(expr) {
  const e = expr.replace(/[()]/g, ' ').trim();
  if (e.includes(' OR ')) return e.split(' OR ').some((p) => allowed(p));
  if (e.includes(' AND ')) return e.split(' AND ').every((p) => allowed(p));
  return ALLOWED.has(e);
}
const problems = [];
for (const [path, pkg] of Object.entries(lock.packages ?? {})) {
  if (path === '' || pkg.link || !path.includes('node_modules/')) continue;
  const name = path.slice(path.lastIndexOf('node_modules/') + 'node_modules/'.length);
  const license = pkg.license;
  if (ACCEPTED[name]) continue;
  if (typeof license !== 'string' || !allowed(license)) {
    problems.push(`${name}@${pkg.version}: ${JSON.stringify(license ?? 'unknown')}`);
  }
}
if (problems.length > 0) {
  console.error(`npm packages with licences outside the allowed list:\n  ${problems.join('\n  ')}`);
  process.exit(1);
}
console.log(`npm licences ok (${Object.keys(lock.packages).length - 1} packages)`);
