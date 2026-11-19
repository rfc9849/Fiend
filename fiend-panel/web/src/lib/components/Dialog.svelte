<script lang="ts">
  import type { Snippet } from 'svelte';
  import IconButton from './IconButton.svelte';
  let { open = $bindable(), title, wide = false, children, actions, onclose }: { open: boolean; title: string; wide?: boolean; children: Snippet; actions?: Snippet; onclose?: () => void } = $props();
  let dialog: HTMLDialogElement;
  const id = `dialog-${Math.random().toString(36).slice(2)}`;
  $effect(() => {
    if (!dialog) return;
    if (open && !dialog.open) dialog.showModal();
    if (!open && dialog.open) dialog.close();
  });
</script>

<dialog bind:this={dialog} class:wide aria-labelledby={id} onclose={() => { open = false; onclose?.(); }} onclick={(event) => { if (event.target === dialog) dialog.close(); }}>
  {#if open}
    <div class="frame">
      <header><h2 {id}>{title}</h2><IconButton icon="close" label="Close dialog" onclick={() => dialog.close()} /></header>
      <div class="body">{@render children()}</div>
      {#if actions}<footer>{@render actions()}</footer>{/if}
    </div>
  {/if}
</dialog>

<style>
  dialog { border: 1px solid var(--line); padding: 0; border-radius: 9px; background: var(--panel); color: var(--text); box-shadow: var(--shadow); width: min(490px, calc(100vw - 24px)); max-height: calc(100dvh - 24px); }
  dialog.wide { width: min(720px, calc(100vw - 24px)); }
  dialog::backdrop { background: var(--scrim); }
  .frame { display: flex; flex-direction: column; max-height: calc(100dvh - 24px); }
  header { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 13px 14px 13px 18px; border-bottom: 1px solid var(--line); }
  .body { padding: 16px 18px; overflow: auto; }
  footer { display: flex; justify-content: flex-end; gap: 8px; padding: 11px 18px; background: var(--soft); border-top: 1px solid var(--line); flex-wrap: wrap; }
</style>
