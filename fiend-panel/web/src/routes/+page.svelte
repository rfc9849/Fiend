<script lang="ts">
  import { onMount } from 'svelte';
  import TopBar from '#lib/components/TopBar.svelte';
  import StatCard from '#lib/components/StatCard.svelte';
  import Tabs from '#lib/components/Tabs.svelte';
  import TrafficChart from '#lib/components/TrafficChart.svelte';
  import NodeTable from '#lib/components/NodeTable.svelte';
  import { api, fieldErrors } from '#lib/api/client.ts';
  import { onEvent } from '#lib/stores/events.ts';
  import { rate, splitUnit } from '#lib/format.ts';
  import type { Dashboard, TrafficSeries } from '#lib/api/generated/index.ts';

  let data = $state<Dashboard | null>(null);
  let series = $state<TrafficSeries | null>(null);
  let range = $state('24h');
  let error = $state('');
  let timer: ReturnType<typeof setTimeout> | undefined;

  async function load() {
    try {
      data = await api.get<Dashboard>('/dashboard');
      error = '';
    } catch (failure) {
      error = fieldErrors(failure).form ?? '';
    }
  }

  async function loadSeries() {
    try {
      series = await api.get<TrafficSeries>(`/traffic?range=${range}`);
    } catch (failure) {
      error = fieldErrors(failure).form ?? '';
    }
  }

  onMount(() => {
    load();
    loadSeries();
    const stop = onEvent((kind) => {
      if (kind === 'stats' || kind === 'nodes') {
        clearTimeout(timer);
        timer = setTimeout(load, 300);
      }
    });
    const slow = setInterval(loadSeries, 60_000);
    return () => {
      stop();
      clearInterval(slow);
      clearTimeout(timer);
    };
  });

  const traffic = $derived(splitUnit(rate(data?.traffic_rate ?? 0)));
</script>

<svelte:head><title>Dashboard · fiend</title></svelte:head>

<TopBar title="Dashboard" />
{#if error}<p class="note error" role="alert">{error}</p>{/if}
{#if data}
  <div class="cards">
    <StatCard label="Traffic now" icon="speed" value={traffic[0]} unit={traffic[1]} samples={data.traffic_samples} />
    <StatCard label="Users online" icon="key" value={String(data.users_online)} unit="of {data.users_total}" samples={data.online_samples} />
    <StatCard label="Nodes online" icon="server" value={String(data.nodes_online)} unit="of {data.nodes_total}" samples={data.node_samples} />
    <a class="stat-link" href="/probes" aria-label="Probes sent to origin: {data.origin_recent} in 1 h. Show probes"><StatCard label="Probes sent to origin" icon="shield" value={String(data.origin_recent)} unit="in 1 h" samples={data.origin_samples} /></a>
  </div>
  <section class="section" aria-labelledby="nodes-title">
    <div class="card-header"><h2 id="nodes-title">Nodes</h2><a href="/nodes" class="small">All nodes</a></div>
    <NodeTable nodes={data.nodes} />
  </section>
  <section class="card section" aria-labelledby="traffic-title">
    <div class="card-header">
      <h2 id="traffic-title">Traffic</h2>
      <Tabs label="Traffic range" bind:value={range} onchange={loadSeries} tabs={[{ value: '24h', label: '24 h' }, { value: '7d', label: '7 d' }, { value: '30d', label: '30 d' }]} />
    </div>
    {#if series}<TrafficChart points={series.points} step={series.step} />{/if}
  </section>
{:else if !error}
  <p class="muted">Loading…</p>
{/if}

<style>
  .cards { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 14px; }
  .section { margin-top: 24px; }
  .stat-link { display: block; color: inherit; text-decoration: none; border-radius: 9px; min-width: 0; }
  .stat-link:hover :global(.stat) { border-color: var(--muted); }
  @media (max-width: 1100px) { .cards { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
  @media (max-width: 480px) { .cards { grid-template-columns: minmax(0, 1fr); } }
</style>
