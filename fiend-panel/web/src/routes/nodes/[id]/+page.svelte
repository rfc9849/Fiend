<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import TopBar from '#lib/components/TopBar.svelte';
  import Button from '#lib/components/Button.svelte';
  import Field from '#lib/components/Field.svelte';
  import Select from '#lib/components/Select.svelte';
  import Confirm from '#lib/components/Confirm.svelte';
  import WarpCard from '#lib/components/WarpCard.svelte';
  import RoutingEditor from '#lib/components/RoutingEditor.svelte';
  import UpdateNodes from '#lib/components/UpdateNodes.svelte';
  import { api, fieldErrors } from '#lib/api/client.ts';
  import { onEvent } from '#lib/stores/events.ts';
  import { toast } from '#lib/stores/toasts.svelte.ts';
  import { ago, bytes, date, rate, stateLabel } from '#lib/format.ts';
  import type { DisguiseView, NodeLogs, NodeView } from '#lib/api/generated/index.ts';

  const id = $derived(Number(page.params.id));
  let node = $state<NodeView | null>(null);
  let disguises = $state<DisguiseView[]>([]);
  let name = $state('');
  let address = $state('');
  let disguise = $state('');
  let logs = $state<string[] | null>(null);
  let error = $state('');
  let fields = $state<Record<string, string>>({});
  let saving = $state(false);
  let restarting = $state(false);
  let removing = $state(false);
  let updating = $state(false);
  let loadingLogs = $state(false);

  async function load(reset = false) {
    try {
      node = await api.get<NodeView>(`/nodes/${id}`);
      if (reset) {
        name = node.name;
        address = node.address;
        disguise = node.disguise_id ? String(node.disguise_id) : '';
        disguises = await api.get<DisguiseView[]>('/disguises');
      }
      error = '';
    } catch (failure) {
      error = fieldErrors(failure).form ?? '';
    }
  }

  $effect(() => {
    id;
    load(true);
    return onEvent((kind, changed) => {
      if ((kind === 'nodes' || kind === 'stats') && (!changed || changed === String(id))) load();
    });
  });

  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (!node) return;
    fields = {};
    saving = true;
    try {
      node = await api.put<NodeView>(`/nodes/${id}`, { name: name.trim(), address: address.trim(), port: node.port, disguise_id: disguise ? Number(disguise) : null });
      toast('Node saved');
    } catch (failure) {
      const result = fieldErrors(failure);
      error = result.form ?? '';
      fields = result.fields;
    } finally {
      saving = false;
    }
  }

  async function loadLogs() {
    loadingLogs = true;
    try {
      logs = (await api.get<NodeLogs>(`/nodes/${id}/logs`)).lines;
    } catch (failure) {
      toast(fieldErrors(failure).form ?? 'The logs are not available', true);
    } finally {
      loadingLogs = false;
    }
  }
</script>

<svelte:head><title>{node?.name ?? 'Node'} · fiend</title></svelte:head>

{#if node}
  <TopBar title={node.name} subtitle="{node.address}:{node.port}{node.local ? ' · local node' : ''}">
    {#snippet actions()}
      <Button icon="refresh" onclick={() => (updating = true)}>Update</Button>
      <Button icon="power" onclick={() => (restarting = true)}>Restart</Button>
    {/snippet}
  </TopBar>
  {#if error}<p class="note error" role="alert">{error}</p>{/if}
  <div class="layout">
    <div class="main">
      <section class="card" aria-labelledby="state-title">
        <div class="state">
          <span class="dot big {node.status}" aria-hidden="true"></span>
          <div>
            <h2 id="state-title">{stateLabel(node.status)}</h2>
            <p>{node.last_error ?? `Last seen ${ago(node.last_seen)}`}</p>
          </div>
        </div>
        <dl class="facts">
          <dt>Version</dt><dd>{#if node.version}<span class="mono">{node.version}</span> <span class="muted">{node.arch ?? ''}</span>{:else}—{/if}</dd>
          <dt>Started</dt><dd>{date(node.started_at)}</dd>
          <dt>Settings</dt><dd>{node.applied_version >= node.desired_version ? 'Up to date' : 'Waiting for sync'}</dd>
          <dt>CPU</dt><dd>{node.cpu === null ? '—' : `${node.cpu.toFixed(0)} %`}</dd>
          <dt>Memory</dt><dd>{node.memory_used === null ? '—' : `${bytes(node.memory_used)} of ${bytes(node.memory_total)}`}</dd>
          <dt>Traffic now</dt><dd>{rate(node.traffic_rate)}</dd>
          <dt>Users</dt><dd>{node.users}</dd>
        </dl>
      </section>

      <section class="card" aria-labelledby="settings-title">
        <h2 id="settings-title">Node settings</h2>
        <form novalidate class="stack form-body" onsubmit={save}>
          <div class="grid-2">
            <Field label="Name" id="node-name" error={fields.name}><input id="node-name" bind:value={name} /></Field>
            <Field label="Public address" id="node-address" error={fields.address}><input id="node-address" bind:value={address} /></Field>
          </div>
          <Field label="Disguise" id="node-disguise" error={fields.disguise_id}>
            <Select id="node-disguise" bind:value={disguise} label="Disguise" options={[{ value: '', label: 'None' }, ...disguises.map((item) => ({ value: String(item.id), label: item.name, detail: item.origin_host }))]} />
          </Field>
          <div class="row between"><Button kind="primary" type="submit" busy={saving}>Save</Button><Button kind="danger" onclick={() => (removing = true)}>Remove from panel</Button></div>
        </form>
      </section>

      <section class="card" aria-labelledby="logs-title">
        <div class="card-header" class:flat={!logs}><h2 id="logs-title">Logs</h2><Button icon="log" busy={loadingLogs} onclick={loadLogs}>{logs ? 'Reload' : 'Show logs'}</Button></div>
        {#if logs}
          {#if logs.length}<pre class="logs" aria-label="Node log lines">{logs.join('\n')}</pre>{:else}<p class="muted">Empty.</p>{/if}
        {/if}
      </section>
    </div>
    <div class="side">
      <WarpCard node={id} />
    </div>
  </div>
  <section class="routing" aria-labelledby="routing-title">
    <h2 id="routing-title">Routing on this node</h2>
    <RoutingEditor node={id} />
  </section>
  <Confirm bind:open={restarting} title="Restart {node.name}" message="Open connections close." action="Restart" onconfirm={async () => { await api.post(`/nodes/${id}/restart`); toast('The node restarts'); }} />
  <Confirm bind:open={removing} title="Remove {node.name} from the panel" message="The node keeps running with its last settings." action="Remove" onconfirm={async () => { await api.del(`/nodes/${id}`); toast('Node removed'); await goto('/nodes'); }} />
  <UpdateNodes bind:open={updating} nodes={[node]} selected={[id]} ondone={() => load()} />
{:else if error}
  <p class="note error" role="alert">{error}</p>
{:else}
  <p class="muted">Loading…</p>
{/if}

<style>
  .layout { display: grid; grid-template-columns: minmax(0, 1fr) 360px; gap: 16px; align-items: start; }
  .main, .side { display: flex; flex-direction: column; gap: 16px; min-width: 0; }
  .main :global(.card + .card) { margin-top: 0; }
  .state { display: flex; gap: 12px; align-items: center; margin-bottom: 16px; }
  .dot.big { width: 12px; height: 12px; box-shadow: 0 0 0 4px var(--soft); }
  .between { justify-content: space-between; }
  .flat { margin-bottom: 0; }
  .facts { display: grid; grid-template-columns: max-content minmax(0, 1fr); gap: 6px 18px; margin: 0; }
  dt { color: var(--muted); }
  dd { margin: 0; min-width: 0; overflow-wrap: anywhere; }
  .form-body { margin-top: 14px; }
  .logs { margin: 0; overflow-x: auto; padding: 12px; background: var(--soft); border-radius: 6px; font: 11px/1.6 var(--mono); white-space: pre-wrap; overflow-wrap: anywhere; }
  .routing { margin-top: 32px; }
  .routing > h2 { margin-bottom: 12px; font-size: 18px; }
  @media (max-width: 1100px) { .layout { grid-template-columns: minmax(0, 1fr); } }
</style>
