import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { TOKENS } from '@kumikit/ui-sdk';
import { describe, expect, it } from 'vitest';

import { CONTRAST_PAIRS, contrast } from '../../../web/kernel/src/theme/contrast';
import { checkTokens } from '../../../web/kernel/src/theme/validate';

const read = (f: string) => JSON.parse(readFileSync(fileURLToPath(new URL(`../${f}`, import.meta.url)), 'utf8')) as Record<string, string>;

for (const variant of ['light', 'dark']) {
  describe(`the default ${variant} theme`, () => {
    const raw = read(`${variant}.json`);

    it('defines every token with a valid value, since it is the fallback', () => {
      const { tokens, errors } = checkTokens(raw);
      expect(errors).toEqual([]);
      expect(Object.keys(tokens).sort()).toEqual(TOKENS.map((t) => t.name).sort());
    });

    it('meets WCAG AA for text on the surfaces it is used on', () => {
      const failing = CONTRAST_PAIRS.filter(([fg, bg, min]) => contrast(raw[fg]!, raw[bg]!) < min).map(
        ([fg, bg, min]) => `${fg} on ${bg}: ${contrast(raw[fg]!, raw[bg]!).toFixed(2)} < ${min}`,
      );
      expect(failing).toEqual([]);
    });
  });
}
