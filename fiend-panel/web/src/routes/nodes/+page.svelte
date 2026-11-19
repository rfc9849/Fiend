<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import TopBar from '#lib/components/TopBar.svelte';
  import { api, fieldErrors } from '#lib/api/client.ts';
  import type { NodeView } from '#lib/api/generated/index.ts';

  let error = $state('');
  let empty = $state(false);

  onMount(async () => {
    try {
      const nodes = await api.get<NodeView[]>('/nodes');
      const local = nodes.find((node) => node.local);
      if (local) await goto(`/nodes/${local.id}`, { replaceState: true });
      else empty = true;
    } catch (failure) {
      error = fieldErrors(failure).form ?? '';
    }
  });
</script>

<svelte:head><title>Server · fiend</title></svelte:head>

<TopBar title="Server" />
{#if error}
  <p class="note error" role="alert">{error}</p>
{:else if empty}
  <section class="card stack">
    <h2>No server connected</h2>
    <a href="/welcome?next=%2Fnodes">Connect the local server</a>
  </section>
{:else}
  <p class="muted">Loading…</p>
{/if}
