// Generates src/generated.ts (the Slot and TokenName types) from the newest published
// lists in api/. A test fails when the file is stale.
// Usage: npm run generate -w @kumikit/ui-sdk
import { writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { render } from './render.mjs';

const out = fileURLToPath(new URL('../src/generated.ts', import.meta.url));
writeFileSync(out, render());
console.log(`wrote ${out}`);
