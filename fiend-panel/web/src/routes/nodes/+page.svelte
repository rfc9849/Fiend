<script lang="ts">
  import { onMount } from 'svelte';
  import TopBar from '#lib/components/TopBar.svelte';
  import NodeTable from '#lib/components/NodeTable.svelte';
  import { api, fieldErrors } from '#lib/api/client.ts';
  import { onEvent } from '#lib/stores/events.ts';
  import type { NodeView } from '#lib/api/generated/index.ts';

  let nodes = $state<NodeView[]>([]);
  let error = $state('');
  let loaded = $state(false);

  async function load() {
    try {
      nodes = await api.get<NodeView[]>('/nodes');
      error = '';
    } catch (failure) {
      error = fieldErrors(failure).form ?? '';
    } finally {
      loaded = true;
    }
  }

  onMount(() => {
    load();
    return onEvent((kind) => {
      if (kind === 'nodes' || kind === 'stats') load();
    });
  });
</script>

<svelte:head><title>Nodes · fiend</title></svelte:head>

<TopBar title="Nodes" />
{#if error}<p class="note error" role="alert">{error}</p>{/if}
{#if loaded}<NodeTable {nodes} />{/if}
