import { createHash } from 'node:crypto';
import { readFileSync, readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { describe, expect, it } from 'vitest';

// @ts-expect-error: a plain .mjs module without types.
import { render, versions } from '../scripts/render.mjs';
import { SLOTS, TOKENS, isComposing, tokenVar } from '../src/index';

const root = fileURLToPath(new URL('..', import.meta.url));

/** SHA-256 over every file of a directory, in name order, with `\n` line endings: the
 * same rule as `wit/PUBLISHED`. */
function dirHash(dir: string): string {
  const h = createHash('sha256');
  for (const name of readdirSync(dir).sort()) {
    const text = readFileSync(`${dir}/${name}`, 'utf8').replaceAll('\r\n', '\n');
    h.update(name).update('\0').update(text).update('\0');
  }
  return h.digest('hex');
}

describe('published API versions', () => {
  const lines = readFileSync(`${root}api/PUBLISHED`, 'utf8')
    .split('\n')
    .filter((l) => l.trim() !== '' && !l.startsWith('#'));

  it('never change', () => {
    const actual = Object.fromEntries(
      (versions() as string[]).map((v) => [v, dirHash(`${root}api/${v}`)]),
    );
    for (const line of lines) {
      const [version, hash] = line.split(' ');
      expect(actual[version!], `api/${version} changed; publish a new version instead`).toBe(
        hash,
      );
    }
  });

  it('every version directory is listed', () => {
    const listed = lines.map((l) => l.split(' ')[0]);
    expect(listed).toEqual(versions());
  });

  it('generated types are current', () => {
    const current = readFileSync(`${root}src/generated.ts`, 'utf8');
    expect(current, 'run `npm run generate -w @kumikit/ui-sdk`').toBe(render());
  });

  it('a change is noticed', () => {
    expect(dirHash(`${root}api/0.1.0`)).not.toBe(
      createHash('sha256').update('').digest('hex'),
    );
  });
});

describe('the lists', () => {
  it('have unique names', () => {
    expect(new Set(SLOTS).size).toBe(SLOTS.length);
    expect(new Set(TOKENS.map((t) => t.name)).size).toBe(TOKENS.length);
  });

  it('include the spec examples', () => {
    expect(SLOTS).toContain('thread.view');
    const names = TOKENS.map((t) => t.name);
    for (const n of ['surface.panel', 'text.primary', 'accent', 'radius.medium', 'focus']) {
      expect(names).toContain(n);
    }
  });
});

describe('helpers', () => {
  it('map tokens to custom properties', () => {
    expect(tokenVar('surface.panel')).toBe('var(--kk-surface-panel)');
    expect(tokenVar('font.size.base')).toBe('var(--kk-font-size-base)');
  });

  it('recognize IME composition', () => {
    expect(isComposing({ isComposing: true, keyCode: 13 } as KeyboardEvent)).toBe(true);
    expect(isComposing({ isComposing: false, keyCode: 229 } as KeyboardEvent)).toBe(true);
    expect(isComposing({ isComposing: false, keyCode: 13 } as KeyboardEvent)).toBe(false);
  });
});
