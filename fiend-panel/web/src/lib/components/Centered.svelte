<script lang="ts">
  import type { Snippet } from 'svelte';
  import Brand from './Brand.svelte';
  let { children, wide = false }: { children: Snippet; wide?: boolean } = $props();
</script>

<main class="centered" id="content">
  <div class="frame" class:wide>
    <div class="brand"><Brand /></div>
    <div class="card">{@render children()}</div>
  </div>
</main>

<style>
  .centered { min-height: 100dvh; display: flex; align-items: flex-start; justify-content: center; padding: 10vh 16px 40px; background: var(--surface); --brand-eye: var(--surface); }
  .frame { width: min(440px, 100%); }
  .frame.wide { width: min(720px, 100%); }
  .brand { margin-bottom: 22px; }
  @media (max-width: 719px) { .centered { padding-top: 32px; } }
</style>
