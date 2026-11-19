<script lang="ts">
  import type { Snippet } from 'svelte';
  let { label, id, error = '', help = '', children }: { label: string; id: string; error?: string; help?: string; children: Snippet } = $props();
</script>

<div class="field" class:invalid={!!error}>
  <label for={id}>{label}</label>
  {@render children()}
  {#if error}<p class="error" id="{id}-error" role="alert">{error}</p>{:else if help}<p class="help" id="{id}-help">{help}</p>{/if}
</div>

<style>
  .field { display: flex; flex-direction: column; gap: 6px; min-width: 0; }
  label { font-weight: 500; font-size: 12px; color: var(--text); }
  .help { font-size: 11px; line-height: 1.45; }
  .error { font-size: 11px; color: var(--danger); }
  .field :global(input:not([type=checkbox]):not([type=file])), .field :global(textarea) { width: 100%; min-height: 35px; padding: 6px 10px; border: 1px solid var(--line); border-radius: 6px; background: var(--panel); color: var(--text); transition: border-color .12s; }
  .field :global(input:not([type=checkbox]):not([type=file]):hover), .field :global(textarea:hover) { border-color: var(--dial-edge); }
  .field :global(textarea) { min-height: 92px; resize: vertical; font-family: var(--mono); font-size: 12px; line-height: 1.5; }
  .field :global(input:focus-visible), .field :global(textarea:focus-visible) { outline: 2px solid var(--accent); outline-offset: 0; border-color: transparent; }
  .invalid :global(input), .invalid :global(textarea), .invalid :global(.trigger) { border-color: var(--danger) !important; }
</style>
