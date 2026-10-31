<script lang="ts">
  let { svg, label }: { svg: string; label: string } = $props();
</script>

<div class="qr" role="img" aria-label={label}>{@html svg}</div>

<style>
  .qr { width: 220px; max-width: 100%; aspect-ratio: 1; background: #fff; border-radius: 8px; border: 1px solid var(--line); padding: 4px; }
  .qr :global(svg) { width: 100%; height: 100%; display: block; }
</style>
