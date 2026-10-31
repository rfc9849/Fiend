<script lang="ts">
  let { values, label }: { values: number[]; label: string } = $props();
  const path = $derived.by(() => {
    const first = values.findIndex((value) => value !== 0);
    const shown = first < 0 ? [] : values.slice(first);
    if (shown.length < 2 || shown.every((value) => value === shown[0])) return '';
    const max = Math.max(...shown, 1);
    const step = 100 / (shown.length - 1);
    return shown.map((value, index) => `${index === 0 ? 'M' : 'L'}${(index * step).toFixed(2)} ${(28 - (value / max) * 26).toFixed(2)}`).join(' ');
  });
</script>

<svg viewBox="0 0 100 30" preserveAspectRatio="none" role="img" aria-label={label}>
  {#if path}<path d={path} fill="none" stroke="var(--accent)" stroke-width="1.5" vector-effect="non-scaling-stroke" />{/if}
</svg>

<style>
  svg { display: block; width: 100%; height: 30px; }
</style>
