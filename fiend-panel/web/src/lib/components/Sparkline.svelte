<script lang="ts">
  let { values, label }: { values: number[]; label: string } = $props();
  const path = $derived.by(() => {
    if (values.length < 2) return '';
    const max = Math.max(...values, 1);
    const step = 100 / (values.length - 1);
    return values.map((value, index) => `${index === 0 ? 'M' : 'L'}${(index * step).toFixed(2)} ${(28 - (value / max) * 26).toFixed(2)}`).join(' ');
  });
</script>

<svg viewBox="0 0 100 30" preserveAspectRatio="none" role="img" aria-label={label}>
  {#if path}<path d={path} fill="none" stroke="var(--accent)" stroke-width="1.5" vector-effect="non-scaling-stroke" />{/if}
</svg>

<style>
  svg { display: block; width: 100%; height: 30px; }
</style>
