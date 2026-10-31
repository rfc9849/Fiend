<script lang="ts">
  import type { TrafficPoint } from '#lib/api/generated/index.ts';
  import { bytes } from '#lib/format.ts';
  let { points, step }: { points: TrafficPoint[]; step: number } = $props();
  const width = 600;
  const height = 180;
  const max = $derived(Math.max(1, ...points.map((point) => point.up + point.down)));
  function line(key: 'up' | 'down' | 'total') {
    if (points.length < 2) return '';
    const dx = width / (points.length - 1);
    return points
      .map((point, index) => {
        const value = key === 'total' ? point.up + point.down : point[key];
        return `${index ? 'L' : 'M'}${(index * dx).toFixed(1)} ${(height - (value / max) * (height - 8)).toFixed(1)}`;
      })
      .join(' ');
  }
  const total = $derived(points.reduce((sum, point) => sum + point.up + point.down, 0));
  const label = (ts: number) => new Date(ts * 1000).toLocaleString(undefined, step >= 86400 ? { month: 'short', day: 'numeric' } : { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
</script>

<figure>
  <svg viewBox="0 0 {width} {height}" preserveAspectRatio="none" role="img" aria-label="Traffic chart. Total {bytes(total)}. Highest point {bytes(max)} per interval.">
    <line x1="0" x2={width} y1={height - 0.5} y2={height - 0.5} stroke="var(--line)" />
    <line x1="0" x2={width} y1={height / 2} y2={height / 2} stroke="var(--line)" stroke-dasharray="3 4" />
    <path d={line('total')} fill="none" stroke="var(--accent)" stroke-width="1.5" vector-effect="non-scaling-stroke" />
    <path d={line('up')} fill="none" stroke="var(--green)" stroke-width="1.5" stroke-dasharray="4 3" vector-effect="non-scaling-stroke" />
  </svg>
  <figcaption>
    <span class="key"><span class="swatch total"></span>Total</span>
    <span class="key"><span class="swatch up"></span>Upload</span>
    <span class="muted">Peak {bytes(max)} per {step >= 86400 ? 'day' : step >= 3600 ? `${step / 3600} h` : 'interval'}</span>
    {#if points.length}<span class="muted range">{label(points[0].ts)} – {label(points[points.length - 1].ts)}</span>{/if}
  </figcaption>
</figure>

<style>
  figure { margin: 0; }
  svg { display: block; width: 100%; height: 180px; }
  figcaption { display: flex; gap: 16px; flex-wrap: wrap; font-size: 11px; margin-top: 8px; color: var(--muted); }
  .key { display: inline-flex; align-items: center; gap: 6px; }
  .swatch { width: 14px; height: 2px; background: var(--accent); }
  .swatch.up { background: var(--green); }
  .range { margin-left: auto; }
</style>
