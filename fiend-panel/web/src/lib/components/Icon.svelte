<script lang="ts">
  const files = import.meta.glob('@icons/*.svg', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;
  const icons: Record<string, string> = {};
  for (const [path, source] of Object.entries(files)) {
    const name = path.split('/').pop()!.replace('.svg', '');
    icons[name] = source.replaceAll('#000', 'currentColor').replace('<svg ', '<svg aria-hidden="true" focusable="false" ');
  }
  let { name, size = 18, label = '' }: { name: string; size?: number; label?: string } = $props();
</script>

<span class="icon" class:s14={size === 14} class:s16={size === 16} class:s20={size === 20} class:s24={size === 24} role={label ? 'img' : undefined} aria-label={label || undefined}>{@html icons[name] ?? ''}</span>

<style>
  .icon { display: inline-flex; width: 18px; height: 18px; flex: none; }
  .s14 { width: 14px; height: 14px; }
  .s16 { width: 16px; height: 16px; }
  .s20 { width: 20px; height: 20px; }
  .s24 { width: 24px; height: 24px; }
  .icon :global(svg) { width: 100%; height: 100%; }
</style>
