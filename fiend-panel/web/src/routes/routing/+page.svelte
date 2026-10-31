<script lang="ts">
  import { onMount } from 'svelte';
  import TopBar from '#lib/components/TopBar.svelte';
  import RoutingEditor from '#lib/components/RoutingEditor.svelte';
  import WarpCard from '#lib/components/WarpCard.svelte';
  import { api } from '#lib/api/client.ts';
  import type { NodeView } from '#lib/api/generated/index.ts';

  let nodes = $state<NodeView[]>([]);
  onMount(async () => {
    nodes = await api.get<NodeView[]>('/nodes');
  });
</script>

<svelte:head><title>Routing · fiend</title></svelte:head>

<TopBar title="Routing" />
<RoutingEditor node={null} />
<div class="warp">
  {#each nodes as node (node.id)}
    <WarpCard node={node.id} title="WARP on {node.name}" />
  {/each}
</div>

<style>
  .warp { display: flex; flex-direction: column; gap: 16px; margin-top: 24px; }
  .warp :global(.card + .card) { margin-top: 0; }
</style>
