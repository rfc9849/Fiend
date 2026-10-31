<script lang="ts">
  import { toasts } from '#lib/stores/toasts.svelte.ts';
</script>

<div class="toasts" role="status" aria-live="polite">
  {#each toasts as item (item.id)}
    <div class="toast" class:error={item.error}>{item.text}</div>
  {/each}
</div>

<style>
  .toasts { position: fixed; right: 16px; bottom: 16px; display: flex; flex-direction: column; gap: 8px; z-index: 60; max-width: min(380px, calc(100vw - 32px)); }
  .toast { padding: 10px 14px; border-radius: 8px; background: var(--text); color: var(--panel); box-shadow: var(--shadow); line-height: 1.5; }
  .toast.error { background: var(--danger); color: var(--on-accent); }
</style>
