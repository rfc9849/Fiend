<script lang="ts">
  import TopBar from '#lib/components/TopBar.svelte';
  import RoutingEditor from '#lib/components/RoutingEditor.svelte';
</script>

<svelte:head><title>Routing · fiend</title></svelte:head>

<TopBar title="Routing" subtitle="Outbounds and rules for all nodes. Node pages add their own rules." />
<RoutingEditor node={null} />
