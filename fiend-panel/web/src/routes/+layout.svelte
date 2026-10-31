<script lang="ts">
  import '../app.css';
  import type { Snippet } from 'svelte';
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import Shell from '#lib/components/Shell.svelte';
  import Toasts from '#lib/components/Toasts.svelte';
  import { loadSession } from '#lib/api/client.ts';
  import { session } from '#lib/stores/session.svelte.ts';
  import { initTheme } from '#lib/stores/theme.svelte.ts';

  let { children }: { children: Snippet } = $props();
  let ready = $state(false);
  let failure = $state('');
  const open = $derived(page.url.pathname.startsWith('/login') || page.url.pathname.startsWith('/setup'));

  async function check(path: string) {
    if (path.startsWith('/login') || path.startsWith('/setup')) {
      ready = true;
      return;
    }
    try {
      const info = await loadSession();
      session.info = info;
      if (!info) {
        await goto(`/login?next=${encodeURIComponent(path)}`, { replaceState: true });
        ready = true;
        return;
      }
      if (!info.wizard.done && !path.startsWith('/welcome')) {
        await goto('/welcome', { replaceState: true });
      }
      ready = true;
    } catch (error) {
      failure = error instanceof Error ? error.message : 'The panel does not answer.';
    }
  }

  onMount(() => {
    initTheme();
    check(page.url.pathname);
  });
</script>

{#if failure}
  <main class="failure"><h1>The panel does not answer</h1><p>{failure}</p></main>
{:else if ready}
  {#if open || page.url.pathname.startsWith('/welcome') || !session.info}
    {@render children()}
  {:else}
    <Shell admin={session.info.admin.name}>{@render children()}</Shell>
  {/if}
{/if}
<Toasts />

<style>
  .failure { max-width: 520px; margin: 18vh auto; padding: 24px; }
  .failure p { margin-top: 8px; }
</style>
