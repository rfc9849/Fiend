<script lang="ts">
  import type { Snippet } from 'svelte';
  let { title, subtitle = '', meta, actions }: { title: string; subtitle?: string; meta?: Snippet; actions?: Snippet } = $props();
</script>

<header class="top">
  <div class="titles">
    <div class="headline">
      <h1>{title}</h1>
      {#if meta}{@render meta()}{/if}
    </div>
    {#if subtitle}<p>{subtitle}</p>{/if}
  </div>
  {#if actions}<div class="actions">{@render actions()}</div>{/if}
</header>

<style>
  .top { display: flex; align-items: center; justify-content: space-between; gap: 16px; margin-bottom: 20px; flex-wrap: wrap; }
  .titles { min-width: 0; }
  .headline { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .titles p { margin-top: 4px; font-size: 12.5px; }
  .actions { display: flex; gap: 8px; flex-wrap: wrap; align-items: center; }
</style>
