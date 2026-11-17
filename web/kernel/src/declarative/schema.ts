/**
 * Settings schemas (07-ui-plugins, Level 1): JSON Schema with UI hints, turned into a
 * flat list of fields for the form, and values checked against it.
 */
import type { SettingsProperty, SettingsSchema } from '@kumikit/ui-sdk';

export type FieldKind = 'text' | 'textarea' | 'number' | 'integer' | 'boolean' | 'select' | 'radio' | 'secret';

export interface Field {
  /** Dotted path for nested objects: `proxy.host`. */
  key: string;
  title: string;
  description?: string;
  kind: FieldKind;
  options?: (string | number)[];
  default?: unknown;
  required: boolean;
  minimum?: number;
  maximum?: number;
  /** The title of the group for nested fields. */
  group?: string;
}

function kindOf(p: SettingsProperty): FieldKind {
  if (p['x-secret']) return 'secret';
  if (p.enum) return p['x-widget'] === 'radio' ? 'radio' : 'select';
  switch (p.type) {
    case 'boolean':
      return 'boolean';
    case 'number':
      return 'number';
    case 'integer':
      return 'integer';
    default:
      return p['x-widget'] === 'textarea' ? 'textarea' : 'text';
  }
}

const ordered = (props: Record<string, SettingsProperty>) =>
  Object.entries(props).sort(([a, x], [b, y]) => (x['x-order'] ?? 100) - (y['x-order'] ?? 100) || a.localeCompare(b));

/** The fields of a schema, in `x-order`. Unsupported shapes are reported, not shown. */
export function fields(schema: SettingsSchema): { fields: Field[]; errors: string[] } {
  const out: Field[] = [];
  const errors: string[] = [];
  const walk = (props: Record<string, SettingsProperty>, required: string[], prefix: string, group?: string) => {
    for (const [name, p] of ordered(props)) {
      const key = prefix ? `${prefix}.${name}` : name;
      if (p.type === 'object') {
        if (prefix) errors.push(`${key}: only one level of nesting is supported`);
        else walk(p.properties ?? {}, [], key, p.title ?? name);
        continue;
      }
      if (!['string', 'number', 'integer', 'boolean'].includes(p.type)) {
        errors.push(`${key}: type ${String(p.type)} is not supported`);
        continue;
      }
      const f: Field = { key, title: p.title ?? name, kind: kindOf(p), required: required.includes(name) };
      if (p.description) f.description = p.description;
      if (p.enum) f.options = p.enum;
      if (p.default !== undefined) f.default = p.default;
      if (p.minimum !== undefined) f.minimum = p.minimum;
      if (p.maximum !== undefined) f.maximum = p.maximum;
      if (group) f.group = group;
      out.push(f);
    }
  };
  if (schema?.type !== 'object' || typeof schema.properties !== 'object') {
    return { fields: [], errors: ['a settings schema must be an object with properties'] };
  }
  walk(schema.properties, schema.required ?? [], '');
  return { fields: out, errors };
}

/** Problems with one value, or undefined if it is fine. */
export function checkValue(f: Field, v: unknown): string | undefined {
  if (v === undefined || v === '') return f.required ? `${f.title} is required` : undefined;
  switch (f.kind) {
    case 'boolean':
      return typeof v === 'boolean' ? undefined : `${f.title} must be on or off`;
    case 'number':
    case 'integer': {
      if (typeof v !== 'number' || Number.isNaN(v)) return `${f.title} must be a number`;
      if (f.kind === 'integer' && !Number.isInteger(v)) return `${f.title} must be a whole number`;
      if (f.minimum !== undefined && v < f.minimum) return `${f.title} must be at least ${f.minimum}`;
      if (f.maximum !== undefined && v > f.maximum) return `${f.title} must be at most ${f.maximum}`;
      return undefined;
    }
    case 'select':
    case 'radio':
      return f.options?.includes(v as string | number) ? undefined : `${f.title} must be one of the choices`;
    default:
      return typeof v === 'string' ? undefined : `${f.title} must be text`;
  }
}

export function getPath(values: Record<string, unknown>, key: string): unknown {
  return key.split('.').reduce<unknown>((o, k) => (o && typeof o === 'object' ? (o as Record<string, unknown>)[k] : undefined), values);
}

export function setPath(values: Record<string, unknown>, key: string, v: unknown): Record<string, unknown> {
  const [head, ...rest] = key.split('.');
  if (rest.length === 0) return { ...values, [head!]: v };
  const inner = (values[head!] as Record<string, unknown> | undefined) ?? {};
  return { ...values, [head!]: setPath(inner, rest.join('.'), v) };
}
