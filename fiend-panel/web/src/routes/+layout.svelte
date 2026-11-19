<script lang="ts">
  import '../app.css';
  import type { Snippet } from 'svelte';
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import Shell from '#lib/components/Shell.svelte';
  import Toasts from '#lib/components/Toasts.svelte';
  import { isOpenPath, loginPath, panelPath, signedInPath, type PanelUrl } from '#lib/navigation.ts';
  import { loadSession } from '#lib/api/client.ts';
  import { session } from '#lib/stores/session.svelte.ts';
  import { initTheme } from '#lib/stores/theme.svelte.ts';

  let { children }: { children: Snippet } = $props();
  let ready = $state(false);
  let failure = $state('');
  const open = $derived(isOpenPath(page.url.pathname));

  async function check(url: PanelUrl) {
    if (isOpenPath(url.pathname)) {
      ready = true;
      return;
    }
    try {
      const info = await loadSession(false);
      session.info = info;
      const current = panelPath(url);
      const destination = info ? signedInPath(info, url, current) : loginPath(url);
      if (destination !== current) await goto(destination, { replaceState: true });
      ready = true;
    } catch (error) {
      failure = error instanceof Error ? error.message : 'The panel does not answer.';
    }
  }

  onMount(() => {
    initTheme();
    check(page.url);
  });
</script>

{#if failure}
  <main class="failure"><h1>The panel does not answer</h1><p>{failure}</p></main>
{:else if ready}
  {#if open || page.url.pathname === '/welcome' || !session.info}
    {@render children()}
  {:else}
    <Shell admin={session.info.admin.name} version={session.info.version}>{@render children()}</Shell>
  {/if}
{/if}
<Toasts />

<style>
  .failure { max-width: 520px; margin: 18vh auto; padding: 24px; }
  .failure p { margin-top: 8px; }
</style>
