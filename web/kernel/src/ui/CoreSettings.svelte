<!-- A core plugin's settings page (07-ui-plugins, Level 1): drawn from `settings.schema`,
     saved with `settings.set`; `x-secret` fields write with `secret.set` and only show
     whether a value is set. -->
<script lang="ts">
  import type { SettingsSchema } from '@kumikit/ui-sdk';
  import type { CoreConnection } from '../connections/connection.svelte';
  import Form from '../declarative/Form.svelte';
  import type { SecretAccess } from '../declarative/schema';

  let { connection, plugin, notify }: { connection: CoreConnection; plugin: string; notify: (m: string) => void } = $props();

  let schema = $state.raw<SettingsSchema | undefined>();
  let values = $state.raw<Record<string, unknown>>({});
  let setNames = $state.raw<Set<string>>(new Set());
  let failed = $state<string | undefined>();

  const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

  async function refreshSecrets() {
    const r = await connection.call('secret.list', {});
    setNames = new Set(r.secrets.filter((s) => s.set).map((s) => s.name));
  }

  $effect(() => {
    const p = plugin;
    failed = undefined;
    schema = undefined;
    void (async () => {
      try {
        const [s, v] = await Promise.all([
          connection.call('settings.schema', { plugin: p }),
          connection.call('settings.get', { plugin: p }),
        ]);
        await refreshSecrets();
        schema = s.schema as unknown as SettingsSchema;
        values = v.values;
      } catch (e) {
        failed = message(e);
      }
    })();
  });

  const secrets: SecretAccess = {
    isSet: (name) => setNames.has(name),
    async set(name, value) {
      await connection.call('secret.set', { name, value });
      await refreshSecrets();
    },
    async clear(name) {
      await connection.call('secret.delete', { name });
      await refreshSecrets();
    },
  };

  async function save(v: Record<string, unknown>) {
    values = v;
    try {
      await connection.call('settings.set', { plugin, values: v });
    } catch (e) {
      notify(`Cannot save the settings: ${message(e)}`);
    }
  }
</script>

{#if failed}
  <p class="kk-muted" role="alert">Cannot read the settings: {failed}</p>
{:else if schema}
  <Form {schema} {values} onchange={save} {secrets} />
{:else}
  <p class="kk-muted">Loading…</p>
{/if}
