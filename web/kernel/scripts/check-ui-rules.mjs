// UI rules from AGENTS.md that a script can check in web/kernel/src:
// - no literal colors, radii, font sizes or shadows (use --kk-* tokens; fallbacks inside
//   var() may only be system colors or keywords);
// - Enter/Escape handlers use the IME check.
// Usage: node scripts/check-ui-rules.mjs (in web/kernel).
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';

function files(dir) {
  return readdirSync(dir).flatMap((f) => {
    const p = join(dir, f);
    return statSync(p).isDirectory() ? files(p) : [p];
  });
}

const problems = [];
const COLOR = /#[0-9a-f]{3,8}\b|\b(?:rgba?|hsla?|oklch|lab|lch)\(/i;
const NAMED = /:\s*(?:white|black|red|green|blue|gray|grey|yellow|orange|purple|pink)\b/i;
for (const file of files('src')) {
  const text = readFileSync(file, 'utf8');
  const css = file.endsWith('.css') ? text : (/<style[^>]*>([\s\S]*?)<\/style>/.exec(text)?.[1] ?? '');
  // Every declaration, wherever it sits on a line.
  for (const decl of css.replace(/\/\*[\s\S]*?\*\//g, '').split(/[;{}]/)) {
    const m = /^\s*([a-z-]+)\s*:\s*([\s\S]+)$/.exec(decl);
    if (!m) continue;
    const [, prop, value] = m;
    const where = `${file} (${prop}: ${value.trim()})`;
    if (COLOR.test(value) || NAMED.test(`:${value}`)) problems.push(`${where}: literal color`);
    if (/^(font-size|border-radius|box-shadow)$/.test(prop)) {
      // Token names may contain digits; fallbacks may not hold sizes other than 0.
      const rest = value.replace(/--kk-[a-z0-9-]+/g, '').replace(/\b0(?:px)?\b/g, '');
      if (/\d/.test(rest)) problems.push(`${where}: literal ${prop}`);
    }
  }
  if (/\.(svelte|ts)$/.test(file) && /key === '(Enter|Escape)'/.test(text) && !/isComposing/.test(text)) {
    problems.push(`${file}: Enter/Escape handler without the IME check (isComposing)`);
  }
}
if (problems.length) {
  console.error(`UI rule problems:\n  ${problems.join('\n  ')}`);
  process.exit(1);
}
console.log('UI rules ok');
