<script lang="ts">
  import Icon from './Icon.svelte';
  import Sparkline from './Sparkline.svelte';
  let { label, icon, value, unit = '', samples = [], detail = '' }: { label: string; icon: string; value: string; unit?: string; samples?: number[]; detail?: string } = $props();
</script>

<section class="stat" aria-label={label}>
  <header><span>{label}</span><Icon name={icon} size={16} /></header>
  <p class="value"><span class="number">{value}</span>{#if unit}<span class="unit">{unit}</span>{/if}</p>
  {#if detail}<p class="detail">{detail}</p>{/if}
  <Sparkline values={samples} label="{label}, last 60 samples" />
</section>

<style>
  .stat { background: var(--panel); border: 1px solid var(--line); border-radius: 9px; padding: 14px 16px 10px; min-width: 0; }
  header { display: flex; justify-content: space-between; align-items: center; color: var(--muted); font-size: 12px; }
  .value { color: var(--text); margin-top: 8px; display: flex; align-items: baseline; gap: 5px; }
  .number { font-size: 24px; font-weight: 600; letter-spacing: -.5px; font-variant-numeric: tabular-nums; }
  .unit { color: var(--muted); font-size: 12px; }
  .detail { font-size: 11px; margin-bottom: 4px; }
</style>
