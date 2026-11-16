/**
 * Token values are checked by type (08-themes): anything that is not a valid value of
 * its type is rejected, which also rules out CSS injection (no `;`, `{`, `}`, `url(`,
 * comments or escapes can pass).
 */
import { TOKENS, type TokenName, type TokenType } from '@kumikit/ui-sdk';

const NUM = String.raw`-?(?:\d+\.?\d*|\.\d+)`;
const LEN = String.raw`(?:0|${NUM}(?:px|rem|em))`;
const PCT = String.raw`${NUM}%`;
const COLOR_FN = new RegExp(
  String.raw`^(?:rgba?|hsla?)\(\s*${NUM}(?:deg)?%?\s*(?:,\s*|\s+)${NUM}%?\s*(?:,\s*|\s+)${NUM}%?\s*(?:(?:,|/)\s*(?:${NUM}|${PCT})\s*)?\)$`,
);
const HEX = /^#(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i;
const SHADOW_ONE = new RegExp(String.raw`^(?:inset\s+)?${LEN}\s+${LEN}(?:\s+${LEN}){0,2}\s+(.+)$`);
const FONT_NAME = /^(?:"[^"\;{}]+"|'[^'\;{}]+'|[a-z][a-z0-9 -]*)$/i;

export function isColor(v: string): boolean {
  const s = v.trim();
  return s === 'transparent' || HEX.test(s) || COLOR_FN.test(s.toLowerCase());
}

/** Splits on commas outside parentheses. */
function splitTop(v: string): string[] {
  const out: string[] = [];
  let depth = 0;
  let cur = '';
  for (const ch of v) {
    if (ch === '(') depth++;
    if (ch === ')') depth--;
    if (ch === ',' && depth === 0) {
      out.push(cur);
      cur = '';
    } else cur += ch;
  }
  out.push(cur);
  return out.map((s) => s.trim());
}

export function isValid(type: TokenType, value: string): boolean {
  if (typeof value !== 'string' || value.length > 200 || /[;{}\\<>]|\/\*/.test(value)) return false;
  const v = value.trim();
  switch (type) {
    case 'color':
      return isColor(v);
    case 'length':
      return new RegExp(`^${LEN}$`).test(v);
    case 'number':
      return new RegExp(`^${NUM}$`).test(v);
    case 'duration':
      return new RegExp(`^${NUM}(?:ms|s)$`).test(v);
    case 'easing':
      return (
        /^(?:linear|ease|ease-in|ease-out|ease-in-out)$/.test(v) ||
        new RegExp(String.raw`^cubic-bezier\(\s*${NUM}\s*,\s*${NUM}\s*,\s*${NUM}\s*,\s*${NUM}\s*\)$`).test(v)
      );
    case 'font-list':
      return splitTop(v).every((f) => FONT_NAME.test(f));
    case 'shadow':
      return (
        v === 'none' ||
        splitTop(v).every((s) => {
          const m = SHADOW_ONE.exec(s);
          return m !== null && isColor(m[1]!);
        })
      );
  }
}

const TYPES = new Map<string, TokenType>(TOKENS.map((t) => [t.name, t.type]));

export interface Checked {
  tokens: Partial<Record<TokenName, string>>;
  /** One message per rejected entry. */
  errors: string[];
}

/** Keeps the known tokens with valid values; reports the rest. `$schema` is ignored. */
export function checkTokens(input: unknown): Checked {
  const tokens: Partial<Record<TokenName, string>> = {};
  const errors: string[] = [];
  if (typeof input !== 'object' || input === null || Array.isArray(input)) {
    return { tokens, errors: ['tokens must be a JSON object'] };
  }
  for (const [name, value] of Object.entries(input)) {
    if (name === '$schema') continue;
    const type = TYPES.get(name);
    if (!type) errors.push(`unknown token ${name}`);
    else if (typeof value !== 'string' || !isValid(type, value)) errors.push(`${name}: not a valid ${type}`);
    else tokens[name as TokenName] = value.trim();
  }
  return { tokens, errors };
}
