<!-- Daily tokens over the past 20 weeks, a column per week (Sunday first). Arrow keys move
     between days; the focused or hovered day's tokens show below the grid. -->
<script lang="ts">
  import type { UsageSummary } from '@kumikit/protocol';

  import { exactTokens, heatmap, isoDate, longDate, tokens } from './format';

  let { summary }: { summary: UsageSummary } = $props();

  /** Columns; the stylesheet's grid has as many. */
  const weeks = 20;

  const today = new Date();
  const grid = $derived(heatmap(summary.daily, today, weeks));
  const total = $derived(summary.daily.reduce((n, d) => n + d.tokens, 0));
  const activeDays = $derived(summary.daily.filter((d) => d.tokens > 0).length);
  let focused = $state(isoDate(today));
  const focusedTokens = $derived(summary.daily.find((d) => d.date === focused)?.tokens ?? 0);
  const rows = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];

  function onKey(e: KeyboardEvent) {
    const all = grid.flatMap((w) => w.cells).filter((c) => !c.future);
    const i = all.findIndex((c) => c.date === focused);
    const step: Record<string, number> = { ArrowUp: -1, ArrowDown: 1, ArrowLeft: -7, ArrowRight: 7 };
    const d = step[e.key];
    if (d === undefined || i < 0) return;
    e.preventDefault();
    const next = all[Math.max(0, Math.min(all.length - 1, i + d))];
    if (!next) return;
    focused = next.date;
    (e.currentTarget as HTMLElement).querySelector<HTMLElement>(`[data-date="${next.date}"]`)?.focus();
  }
</script>

<section class="activity" aria-labelledby="kk-usage-activity">
  <header>
    <h3 id="kk-usage-activity">Activity</h3>
    <span class="sum">{tokens(total)} tokens · {activeDays} active {activeDays === 1 ? 'day' : 'days'} · past {weeks} weeks</span>
  </header>
  <div class="months" aria-hidden="true">
    <span class="gutter"></span>
    {#each grid as w, i (i)}<span class="month">{w.month ?? ''}</span>{/each}
  </div>
  <div class="body">
    <div class="days" aria-hidden="true">
      {#each rows as r, i (r)}<span>{i % 2 === 1 ? r : ''}</span>{/each}
    </div>
    <!-- svelte-ignore a11y_interactive_supports_focus -->
    <div class="grid" role="grid" aria-label="Tokens per day" onkeydown={onKey}>
      {#each rows as _, d (d)}
        <div role="row" class="gridrow">
          {#each grid as w, wi (wi)}
            {@const c = w.cells[d]!}
            {#if c.future}
              <span role="gridcell" class="cell future" aria-hidden="true"></span>
            {:else}
              <span
                role="gridcell"
                class="cell"
                data-level={c.level}
                data-date={c.date}
                tabindex={c.date === focused ? 0 : -1}
                aria-label={`${longDate(c.date)}: ${exactTokens(c.tokens)} tokens`}
                title={`${longDate(c.date)}: ${exactTokens(c.tokens)} tokens`}
                onfocus={() => (focused = c.date)}
                onmouseenter={() => (focused = c.date)}
              ></span>
            {/if}
          {/each}
        </div>
      {/each}
    </div>
  </div>
  <footer>
    <span class="day" aria-live="polite"><strong>{exactTokens(focusedTokens)} tokens</strong> {longDate(focused)}</span>
    <span class="legend" aria-hidden="true">
      Less
      {#each [0, 1, 2, 3, 4] as l (l)}<span class="cell" data-level={l}></span>{/each}
      More
    </span>
  </footer>
</section>

<style>
  .activity {
    margin: 22px 0 6px;
    padding: 14px;
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-card, Canvas);
  }
  header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 10px;
  }
  h3 {
    margin: 0;
    font-size: var(--kk-font-size-large, large);
    font-weight: var(--kk-font-weight-strong, bold);
  }
  .sum,
  .months,
  .days,
  footer {
    color: var(--kk-text-secondary, CanvasText);
    font-size: var(--kk-font-size-small, small);
  }
  .months,
  .gridrow {
    display: grid;
    gap: 3px;
  }
  .months {
    grid-template-columns: 34px repeat(20, 14px);
    margin-bottom: 4px;
    white-space: nowrap;
  }
  .month {
    overflow: visible;
  }
  .body {
    display: flex;
    gap: 3px;
  }
  .days {
    display: grid;
    grid-template-rows: repeat(7, 14px);
    gap: 3px;
    width: 31px;
  }
  .days span {
    line-height: 14px;
  }
  .grid {
    display: grid;
    gap: 3px;
  }
  .gridrow {
    grid-template-columns: repeat(20, 14px);
  }
  .cell {
    display: inline-block;
    width: 14px;
    height: 14px;
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-surface-hover, ButtonFace);
  }
  .cell.future {
    background: transparent;
  }
  .cell[data-level='1'] {
    background: color-mix(in srgb, var(--kk-accent, Highlight) 30%, var(--kk-surface-hover, ButtonFace));
  }
  .cell[data-level='2'] {
    background: color-mix(in srgb, var(--kk-accent, Highlight) 55%, var(--kk-surface-hover, ButtonFace));
  }
  .cell[data-level='3'] {
    background: color-mix(in srgb, var(--kk-accent, Highlight) 78%, var(--kk-surface-hover, ButtonFace));
  }
  .cell[data-level='4'] {
    background: var(--kk-accent, Highlight);
  }
  .grid .cell:focus-visible {
    outline: 2px solid var(--kk-focus, Highlight);
    outline-offset: 1px;
  }
  footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-top: 10px;
  }
  footer strong {
    color: var(--kk-text-primary, CanvasText);
  }
  .legend {
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }
  .legend .cell {
    width: 10px;
    height: 10px;
  }
</style>
