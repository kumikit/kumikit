import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import type { Permissions } from '@kumikit/protocol';
import { expect, it } from 'vitest';

import { describePermissions } from '../src/plugins/permissions';

const fixture = JSON.parse(
  readFileSync(fileURLToPath(new URL('../../../crates/kumikit-core/tests/fixtures/permission-wording.json', import.meta.url)), 'utf8'),
) as { cases: { permissions: Permissions; lines: string[] }[] };

it('words permissions exactly as the CLI does', () => {
  for (const c of fixture.cases) expect(describePermissions(c.permissions)).toEqual(c.lines);
});
