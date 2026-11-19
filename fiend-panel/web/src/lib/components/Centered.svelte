<script lang="ts">
  import type { Snippet } from 'svelte';
  import Brand from './Brand.svelte';
  let { children, wide = false, step = 0, steps = 0 }: { children: Snippet; wide?: boolean; step?: number; steps?: number } = $props();
</script>

<main class="centered" id="content">
  <div class="frame" class:wide>
    <div class="head">
      <Brand />
      {#if steps > 0}
        <div class="progress" aria-label="Step {step} of {steps}">
          <span class="step-label">Step {step} of {steps}</span>
          <div class="bars" aria-hidden="true">
            {#each Array.from({ length: steps }, (_, i) => i + 1) as index (index)}
              <span class="bar" class:done={index < step} class:active={index === step}></span>
            {/each}
          </div>
        </div>
      {/if}
    </div>
    <div class="panel">{@render children()}</div>
  </div>
</main>

<style>
  .centered { min-height: 100dvh; display: flex; align-items: flex-start; justify-content: center; padding: 9vh 16px 48px; background: var(--surface); --brand-eye: var(--surface); }
  .frame { width: min(430px, 100%); }
  .frame.wide { width: min(680px, 100%); }
  .head { display: flex; align-items: center; justify-content: space-between; gap: 16px; margin-bottom: 20px; flex-wrap: wrap; }
  .progress { display: flex; flex-direction: column; align-items: flex-end; gap: 6px; }
  .step-label { font-size: 11px; font-weight: 500; color: var(--muted); font-variant-numeric: tabular-nums; }
  .bars { display: flex; gap: 4px; }
  .bar { width: 24px; height: 4px; border-radius: 999px; background: var(--line); }
  .bar.done { background: var(--green); }
  .bar.active { background: var(--accent); }
  .panel { background: var(--panel); border: 1px solid var(--line); border-radius: 9px; padding: 24px 26px; box-shadow: var(--shadow-sm); }
  @media (max-width: 719px) {
    .centered { padding-top: 28px; }
    .panel { padding: 18px 16px; }
  }
</style>
