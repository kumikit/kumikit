import { describe, expect, it } from 'vitest';

import { meetsRequirements, parseRequirement, satisfies } from '../src/plugins/requires';

const plugin = (id: string, version: string) => ({ id, version, contributes: { providers: [], commands: [], settings: false } });

describe('requires.core', () => {
  it('reads semver ranges', () => {
    const yes: [string, string][] = [
      ['1.4.0', '^1'],
      ['1.4.0', '^1.2'],
      ['1.2.0', '^1.2.0'],
      ['0.3.9', '^0.3'],
      ['0.0.3', '^0.0.3'],
      ['1.2.9', '~1.2.3'],
      ['1.9.0', '~1'],
      ['2.0.0', '>=1.5'],
      ['1.2.3', '1.2.3'],
      ['1.2.7', '1.2'],
      ['3.0.0', '*'],
      ['3.0.0', ''],
      ['1.2.3-beta', '^1'],
    ];
    for (const [v, r] of yes) expect(satisfies(v, r), `${v} in ${r}`).toBe(true);
    const no: [string, string][] = [
      ['2.0.0', '^1'],
      ['1.1.0', '^1.2'],
      ['0.4.0', '^0.3'],
      ['0.0.4', '^0.0.3'],
      ['1.3.0', '~1.2.3'],
      ['1.4.0', '>=1.5'],
      ['1.2.4', '1.2.3'],
      ['x', '^1'],
      ['1.0.0', '^one'],
      ['1.0.0', '<2'],
    ];
    for (const [v, r] of no) expect(satisfies(v, r), `${v} not in ${r}`).toBe(false);
  });

  it('parses entries', () => {
    expect(parseRequirement('kumikit.git@^1')).toEqual({ id: 'kumikit.git', range: '^1' });
    expect(parseRequirement('kumikit.git')).toEqual({ id: 'kumikit.git', range: '*' });
  });

  it('is met only when every entry is enabled on the core', () => {
    const core = [plugin('kumikit.git', '1.3.0'), plugin('a.b', '0.1.0')];
    expect(meetsRequirements(undefined, [])).toBe(true);
    expect(meetsRequirements([], [])).toBe(true);
    expect(meetsRequirements(['kumikit.git@^1'], core)).toBe(true);
    expect(meetsRequirements(['kumikit.git@^1', 'a.b'], core)).toBe(true);
    expect(meetsRequirements(['kumikit.git@^2'], core)).toBe(false);
    expect(meetsRequirements(['kumikit.git@^1', 'c.d'], core)).toBe(false);
  });
});
