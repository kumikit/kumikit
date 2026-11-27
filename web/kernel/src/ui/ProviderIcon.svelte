<!-- A provider's icon from its manifest (a data: URL, shown with <img> so an SVG cannot run
     scripts), or its initial on a tinted badge when it has none. -->
<script lang="ts">
  import { providerInitial } from './model-picker';

  let { name, icon, size = 'small' }: { name: string; icon?: string | undefined; size?: 'small' | 'medium' } = $props();
</script>

{#if icon}
  <img class="kk-provider-icon {size}" src={icon} alt="" aria-hidden="true" draggable="false" />
{:else}
  <span class="kk-provider-icon badge {size}" aria-hidden="true">{providerInitial(name)}</span>
{/if}

<style>
  .kk-provider-icon {
    flex: none;
    width: 16px;
    height: 16px;
    object-fit: contain;
  }
  .kk-provider-icon.medium {
    width: 18px;
    height: 18px;
  }
  .badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-accent-tint, ButtonFace);
    color: var(--kk-accent, ButtonText);
    font-size: var(--kk-font-size-small, small);
    font-weight: var(--kk-font-weight-strong, bold);
    line-height: 1;
  }
</style>
