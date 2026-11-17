<!-- A settings form drawn from a schema (07-ui-plugins, Level 1). Every change is
     checked and reported field by field; valid values are passed on at once. -->
<script lang="ts">
  import type { SettingsSchema } from '@kumikit/ui-sdk';
  import { checkValue, fields, getPath, setPath, type Field } from './schema';

  let {
    schema,
    values,
    onchange,
  }: { schema: SettingsSchema; values: Record<string, unknown>; onchange: (v: Record<string, unknown>) => void } = $props();

  const parsed = $derived(fields(schema));
  let errors = $state<Record<string, string>>({});
  const uid = Math.random().toString(36).slice(2, 8);

  function value(f: Field): unknown {
    return getPath(values, f.key) ?? f.default;
  }

  function set(f: Field, raw: unknown) {
    let v = raw;
    if ((f.kind === 'number' || f.kind === 'integer') && typeof raw === 'string') v = raw === '' ? undefined : Number(raw);
    if (f.options && typeof f.options[0] === 'number' && typeof raw === 'string') v = Number(raw);
    const problem = checkValue(f, v);
    errors = { ...errors, [f.key]: problem ?? '' };
    if (!problem) onchange(setPath(values, f.key, v));
  }

  const id = (f: Field) => `kk-f-${uid}-${f.key.replaceAll('.', '-')}`;
</script>

<div class="form">
  {#each parsed.errors as e (e)}
    <p class="kk-muted">Not shown: {e}</p>
  {/each}
  {#each parsed.fields as f, i (f.key)}
    {#if f.group && f.group !== parsed.fields[i - 1]?.group}
      <h3>{f.group}</h3>
    {/if}
    <div class="field">
      {#if f.kind === 'boolean'}
        <label class="check">
          <input id={id(f)} type="checkbox" checked={value(f) === true} onchange={(e) => set(f, e.currentTarget.checked)} aria-describedby={f.description ? `${id(f)}-d` : undefined} />
          {f.title}
        </label>
      {:else if f.kind === 'radio'}
        <fieldset>
          <legend>{f.title}</legend>
          {#each f.options ?? [] as o (o)}
            <label class="check"><input type="radio" name={id(f)} value={o} checked={value(f) === o} onchange={() => set(f, o)} /> {o}</label>
          {/each}
        </fieldset>
      {:else}
        <label for={id(f)}>{f.title}{#if f.required}<span aria-hidden="true"> *</span>{/if}</label>
        {#if f.kind === 'select'}
          <select id={id(f)} class="kk-input" value={String(value(f) ?? '')} onchange={(e) => set(f, e.currentTarget.value)}>
            {#if !f.required}<option value="">—</option>{/if}
            {#each f.options ?? [] as o (o)}<option value={String(o)}>{o}</option>{/each}
          </select>
        {:else if f.kind === 'textarea'}
          <textarea id={id(f)} class="kk-input" rows="4" value={String(value(f) ?? '')} onchange={(e) => set(f, e.currentTarget.value)}></textarea>
        {:else if f.kind === 'secret'}
          <input id={id(f)} class="kk-input" type="password" disabled placeholder="Set on the core" />
        {:else}
          <input
            id={id(f)}
            class="kk-input"
            type={f.kind === 'number' || f.kind === 'integer' ? 'number' : 'text'}
            step={f.kind === 'integer' ? 1 : 'any'}
            min={f.minimum}
            max={f.maximum}
            value={value(f) === undefined ? '' : String(value(f))}
            aria-invalid={errors[f.key] ? 'true' : undefined}
            aria-describedby={f.description ? `${id(f)}-d` : undefined}
            onchange={(e) => set(f, e.currentTarget.value)}
          />
        {/if}
      {/if}
      {#if f.description}<p id={`${id(f)}-d`} class="kk-muted desc">{f.description}</p>{/if}
      {#if errors[f.key]}<p class="error" role="alert">{errors[f.key]}</p>{/if}
    </div>
  {/each}
</div>

<style>
  .form {
    display: grid;
    gap: 14px;
    max-width: 560px;
  }
  .field {
    display: grid;
    gap: 4px;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  fieldset {
    margin: 0;
    padding: 6px 10px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-small, 0);
  }
  h3 {
    margin: 8px 0 0;
    font-size: var(--kk-font-size-base, medium);
  }
  .desc,
  .error {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
  }
  .error {
    color: var(--kk-state-danger, CanvasText);
  }
</style>
