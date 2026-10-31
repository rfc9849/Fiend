<script lang="ts">
  let { compact = false }: { compact?: boolean } = $props();
</script>

<span class="brand" class:compact>
  <svg class="mark" viewBox="0 0 40 40" aria-hidden="true"><path d="M8 16 7 5l10 7h6L33 5l-1 11c3 4 3 10-1 14-6 6-16 6-22 0-4-4-4-10-1-14Z" fill="currentColor"/><path d="m12 20 6 2m10-2-6 2" stroke="var(--brand-eye, var(--sidebar))" stroke-width="3" stroke-linecap="round"/></svg>
  {#if !compact}<span class="word">fiend</span>{/if}
</span>

<style>
  .brand { display: inline-flex; align-items: center; gap: 9px; color: var(--text); }
  .mark { width: 26px; height: 26px; flex: none; }
  .word { font-weight: 600; font-size: 26px; line-height: 1; letter-spacing: -1.1px; }
</style>
