<script lang="ts">
  import DisguiseSwitch from '#lib/components/DisguiseSwitch.svelte';
  import NodeSync from '#lib/components/NodeSync.svelte';
  import { page } from '$app/state';
  import TopBar from '#lib/components/TopBar.svelte';
  import Button from '#lib/components/Button.svelte';
  import Field from '#lib/components/Field.svelte';
  import Confirm from '#lib/components/Confirm.svelte';
  import { api, fieldErrors } from '#lib/api/client.ts';
  import { onEvent } from '#lib/stores/events.ts';
  import { toast } from '#lib/stores/toasts.svelte.ts';
  import { ago, bytes, date, rate, stateLabel } from '#lib/format.ts';
  import type { NodeLogs, NodeView } from '#lib/api/generated/index.ts';

  const id = $derived(Number(page.params.id));
  let node = $state<NodeView | null>(null);
  let name = $state('');
  let address = $state('');
  let disguiseOpen = $state(false);
  let logs = $state<string[] | null>(null);
  let error = $state('');
  let fields = $state<Record<string, string>>({});
  let saving = $state(false);
  let restarting = $state(false);
  let loadingLogs = $state(false);

  async function load(reset = false) {
    try {
      node = await api.get<NodeView>(`/nodes/${id}`);
      if (reset) {
        name = node.name;
        address = node.address;
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
      node = await api.put<NodeView>(`/nodes/${id}`, { name: name.trim(), address: address.trim(), port: node.port, disguise_id: node.disguise_id });
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
  <TopBar title="Server" subtitle="{node.name} · {node.address}:{node.port}">
    {#snippet actions()}
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
          <dt>Settings</dt><dd><NodeSync {node} /></dd>
          <dt>CPU</dt><dd>{node.cpu === null ? '—' : `${node.cpu.toFixed(0)} %`}</dd>
          <dt>Memory</dt><dd>{node.memory_used === null ? '—' : `${bytes(node.memory_used)} of ${bytes(node.memory_total)}`}</dd>
          <dt>Traffic now</dt><dd>{rate(node.traffic_rate)}</dd>
          <dt>Users</dt><dd>{node.users}</dd>
        </dl>
      </section>

      <section class="card" aria-labelledby="settings-title">
        <h2 id="settings-title">Connection</h2>
        <form novalidate class="stack form-body" onsubmit={save}>
          <div class="grid-2">
            <Field label="Name" id="node-name" error={fields.name}><input id="node-name" bind:value={name} /></Field>
            <Field label="Public address" id="node-address" error={fields.address}><input id="node-address" bind:value={address} /></Field>
          </div>
          <Button kind="primary" type="submit" busy={saving}>Save address and name</Button>
        </form>
      </section>

      <section class="card" aria-labelledby="disguise-title">
        <div class="card-header">
          <h2 id="disguise-title">TLS disguise</h2>
          <Button onclick={() => (disguiseOpen = true)}>{node.disguise_id === null ? 'Set disguise' : 'Change disguise'}</Button>
        </div>
        {#if node.disguise_id === null}
          <p>Set a disguise before sharing user links.</p>
        {:else if node.applied_version < node.disguise_version}
          <p role="status">{node.disguise_name}: {node.status === 'error' ? 'The server rejected the settings.' : 'Waiting for the server to apply the change.'}</p>
        {:else}
          <p><strong>{node.disguise_name}</strong> is applied.</p>
          {#if node.disguise_changed_at}
            <p>Selected {date(node.disguise_changed_at)}. Existing profiles keep their old TLS template. <a href="/users">Open Users to share updated links</a>.</p>
          {/if}
        {/if}
      </section>

      <section class="card" aria-labelledby="logs-title">
        <div class="card-header" class:flat={!logs}><h2 id="logs-title">Logs</h2><Button icon="log" busy={loadingLogs} onclick={loadLogs}>{logs ? 'Reload' : 'Show logs'}</Button></div>
        {#if logs}
          {#if logs.length}<pre class="logs" aria-label="Node log lines">{logs.join('\n')}</pre>{:else}<p class="muted">Empty.</p>{/if}
        {/if}
      </section>
    </div>
  </div>
  {#if disguiseOpen}
    <DisguiseSwitch bind:open={disguiseOpen} {node} onsaved={(updated) => (node = updated)} />
  {/if}
  <section class="card">
    <h2>Diagnostics</h2>
    <p><a href="/probes">Connections forwarded to the origin</a></p>
  </section>
  <Confirm bind:open={restarting} title="Restart {node.name}" message="Open connections close." action="Restart" onconfirm={async () => { await api.post(`/nodes/${id}/restart`); toast('The node restarts'); }} />
{:else if error}
  <p class="note error" role="alert">{error}</p>
{:else}
  <p class="muted">Loading…</p>
{/if}

<style>
  .main { display: flex; flex-direction: column; gap: 16px; min-width: 0; }
  .main :global(.card + .card) { margin-top: 0; }
  .state { display: flex; gap: 12px; align-items: center; margin-bottom: 16px; }
  .dot.big { width: 12px; height: 12px; box-shadow: 0 0 0 4px var(--soft); }
  .flat { margin-bottom: 0; }
  .facts { display: grid; grid-template-columns: max-content minmax(0, 1fr); gap: 6px 18px; margin: 0; }
  dt { color: var(--muted); }
  dd { margin: 0; min-width: 0; overflow-wrap: anywhere; }
  .form-body { margin-top: 14px; }
  .logs { margin: 0; overflow-x: auto; padding: 12px; background: var(--soft); border-radius: 6px; font: 11px/1.6 var(--mono); white-space: pre-wrap; overflow-wrap: anywhere; }
</style>
