<script lang="ts">
  import type { Snippet } from 'svelte';
  let { title, subtitle = '', actions }: { title: string; subtitle?: string; actions?: Snippet } = $props();
</script>

<header class="top">
  <div class="titles"><h1>{title}</h1>{#if subtitle}<p>{subtitle}</p>{/if}</div>
  {#if actions}<div class="actions">{@render actions()}</div>{/if}
</header>

<style>
  .top { display: flex; align-items: flex-end; justify-content: space-between; gap: 16px; margin-bottom: 22px; flex-wrap: wrap; }
  .titles { min-width: 0; }
  .titles p { margin-top: 4px; }
  .actions { display: flex; gap: 8px; flex-wrap: wrap; }
</style>
