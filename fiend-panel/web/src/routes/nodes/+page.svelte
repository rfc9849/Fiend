<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import TopBar from '#lib/components/TopBar.svelte';
  import EmptyState from '#lib/components/EmptyState.svelte';
  import Button from '#lib/components/Button.svelte';
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
  <EmptyState icon="server" title="No server connected" message="Connect the local server to manage its address, TLS disguise, routing, and logs.">
    {#snippet actions()}
      <Button kind="primary" icon="plus" onclick={() => goto('/welcome?next=%2Fnodes')}>Connect local server</Button>
    {/snippet}
  </EmptyState>
{:else}
  <p class="muted">Loading…</p>
{/if}
