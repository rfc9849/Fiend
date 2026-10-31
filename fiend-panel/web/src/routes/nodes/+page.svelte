<script lang="ts">
  import { onMount } from 'svelte';
  import TopBar from '#lib/components/TopBar.svelte';
  import Button from '#lib/components/Button.svelte';
  import NodeTable from '#lib/components/NodeTable.svelte';
  import AddNode from '#lib/components/AddNode.svelte';
  import UpdateNodes from '#lib/components/UpdateNodes.svelte';
  import { api, fieldErrors } from '#lib/api/client.ts';
  import { onEvent } from '#lib/stores/events.ts';
  import type { NodeView } from '#lib/api/generated/index.ts';

  let nodes = $state<NodeView[]>([]);
  let adding = $state(false);
  let updating = $state(false);
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

<TopBar title="Nodes">
  {#snippet actions()}
    <Button icon="refresh" disabled={nodes.length === 0} onclick={() => (updating = true)}>Update nodes</Button>
    <Button kind="primary" icon="plus" onclick={() => (adding = true)}>Add node</Button>
  {/snippet}
</TopBar>
{#if error}<p class="note error" role="alert">{error}</p>{/if}
{#if loaded}<NodeTable {nodes} />{/if}

<AddNode bind:open={adding} ondone={load} />
<UpdateNodes bind:open={updating} {nodes} ondone={load} />
