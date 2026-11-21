<!-- A view a core plugin's command returned (07-ui-plugins, Level 1), drawn with the
     UI kernel's declarative renderer. Its commands (list items, a form's submit) run the
     same plugin's commands, in the same workspace. -->
<script lang="ts">
  import type { Kernel } from '../app.svelte';
  import type { CoreView } from '../commands/core.svelte';
  import View from '../declarative/View.svelte';

  let { kernel, view }: { kernel: Kernel; view: CoreView } = $props();

  function run(command: string, args?: unknown) {
    const c = kernel.connectionFor(view.connection);
    if (!c) {
      kernel.notify('That core is no longer connected', 'danger');
      return;
    }
    void kernel.coreCommands.run(c, view.plugin, command, args, view.workspace, view.title);
  }
</script>

<section class="core-view" aria-label={view.title}>
  <h1>{view.title}</h1>
  <View view={view.view} {run} />
</section>

<style>
  .core-view {
    padding: 16px 24px;
    overflow: auto;
    height: 100%;
  }
  h1 {
    margin: 0 0 12px;
    font-size: var(--kk-font-size-large, large);
  }
</style>
