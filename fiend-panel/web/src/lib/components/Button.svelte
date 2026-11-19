<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';
  let {
    kind = 'default',
    size = 'default',
    icon = '',
    type = 'button',
    disabled = false,
    busy = false,
    onclick,
    children,
    label = '',
    form
  }: {
    kind?: 'default' | 'primary' | 'ghost' | 'danger';
    size?: 'default' | 'compact';
    icon?: string;
    type?: 'button' | 'submit';
    disabled?: boolean;
    busy?: boolean;
    onclick?: (event: MouseEvent) => void;
    children?: Snippet;
    label?: string;
    form?: string;
  } = $props();
</script>

<button class="button {kind} {size}" {type} {form} disabled={disabled || busy} aria-busy={busy || undefined} aria-label={label || undefined} {onclick}>
  {#if busy}<span class="spinner" aria-hidden="true"></span>{:else if icon}<Icon name={icon} size={size === 'compact' ? 14 : 15} />{/if}
  {#if children}<span>{@render children()}</span>{/if}
</button>

<style>
  .button { display: inline-flex; align-items: center; justify-content: center; gap: 6px; min-height: 33px; padding: 6px 12px; background: var(--panel); border: 1px solid var(--line); border-radius: 6px; font-size: 12px; font-weight: 500; white-space: nowrap; color: var(--text); transition: background .12s, border-color .12s, color .12s; }
  .button.compact { min-height: 28px; padding: 4px 10px; font-size: 11.5px; border-radius: 5px; }
  .button:hover:not(:disabled) { background: var(--hover); border-color: var(--dial-edge); }
  .primary { background: var(--accent); color: var(--on-accent); border-color: transparent; }
  .primary:hover:not(:disabled) { background: var(--accent-hover); border-color: transparent; }
  .ghost { border-color: transparent; background: none; color: var(--muted); }
  .ghost:hover:not(:disabled) { background: var(--hover); border-color: transparent; color: var(--text); }
  .danger { color: var(--danger); }
  .danger:hover:not(:disabled) { background: var(--danger-soft); border-color: var(--danger); }
  .spinner { width: 13px; height: 13px; border: 2px solid currentColor; border-right-color: transparent; border-radius: 50%; animation: spin .8s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
</style>
