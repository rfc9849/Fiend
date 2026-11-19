<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';
  let { icon, title, message = '', compact = false, actions }: { icon: string; title: string; message?: string; compact?: boolean; actions?: Snippet } = $props();
</script>

<div class="empty-state" class:compact>
  <span class="icon-tile lg"><Icon name={icon} size={20} /></span>
  <h3>{title}</h3>
  {#if message}<p>{message}</p>{/if}
  {#if actions}<div class="actions">{@render actions()}</div>{/if}
</div>

<style>
  .empty-state { display: flex; flex-direction: column; align-items: center; justify-content: center; text-align: center; gap: 8px; padding: 38px 24px; background: var(--panel); border: 1px solid var(--line); border-radius: 8px; }
  .empty-state.compact { padding: 26px 20px; }
  h3 { margin-top: 4px; font-size: 14px; font-weight: 600; color: var(--text); }
  p { max-width: 380px; font-size: 12.5px; line-height: 1.5; }
  .actions { margin-top: 8px; display: flex; gap: 8px; flex-wrap: wrap; justify-content: center; }
</style>
