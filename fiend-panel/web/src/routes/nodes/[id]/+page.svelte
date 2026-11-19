<script lang="ts">
  import DisguiseSwitch from '#lib/components/DisguiseSwitch.svelte';
  import NodeSync from '#lib/components/NodeSync.svelte';
  import { page } from '$app/state';
  import TopBar from '#lib/components/TopBar.svelte';
  import Button from '#lib/components/Button.svelte';
  import Field from '#lib/components/Field.svelte';
  import Confirm from '#lib/components/Confirm.svelte';
  import Icon from '#lib/components/Icon.svelte';
  import { api, fieldErrors } from '#lib/api/client.ts';
  import { onEvent } from '#lib/stores/events.ts';
  import { toast } from '#lib/stores/toasts.svelte.ts';
  import { ago, bytes, date, percent, rate, splitUnit, stateLabel } from '#lib/format.ts';
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
  let copiedLogs = $state(false);

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
      name = node.name;
      address = node.address;
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

  async function copyLogs() {
    if (!logs?.length) return;
    try {
      await navigator.clipboard.writeText(logs.join('\n'));
      copiedLogs = true;
      setTimeout(() => (copiedLogs = false), 2000);
    } catch {
      copiedLogs = false;
    }
  }

  const dirty = $derived(!!node && (name.trim() !== node.name || address.trim() !== node.address));
  const cpuPct = $derived(node?.cpu === null || node?.cpu === undefined ? null : Math.min(100, Math.max(0, Math.round(node.cpu))));
  const memPct = $derived(node?.memory_used !== null && node?.memory_used !== undefined && node?.memory_total ? percent(node.memory_used, node.memory_total) : null);
  const traffic = $derived(splitUnit(rate(node?.traffic_rate ?? 0)));
  const meterTone = (pct: number | null) => (pct === null ? '' : pct >= 90 ? 'danger' : pct >= 75 ? 'warn' : '');
</script>

<svelte:head><title>{node?.name ?? 'Server'} · fiend</title></svelte:head>

{#if node}
  <TopBar title={node.name} subtitle="{node.address}:{node.port} · Last seen {ago(node.last_seen)}">
    {#snippet meta()}
      {#if node}
        {#if node.local}<span class="badge">local</span>{/if}
        <NodeSync {node} />
      {/if}
    {/snippet}
    {#snippet actions()}
      <Button icon="power" onclick={() => (restarting = true)}>Restart</Button>
    {/snippet}
  </TopBar>

  {#if error}<p class="note error banner" role="alert">{error}</p>{/if}
  {#if node.last_error}<p class="note warn banner" role="status">{node.last_error}</p>{/if}

  <div class="kpi-grid">
    <section class="kpi" aria-label="Status">
      <div class="kpi-head"><span>Status</span><Icon name="server" size={15} /></div>
      <div class="kpi-val">
        <span class="dot {node.status}" aria-hidden="true"></span>
        <span class="kpi-num">{stateLabel(node.status)}</span>
      </div>
      <div class="kpi-foot">{node.users} user{node.users === 1 ? '' : 's'} assigned</div>
    </section>

    <section class="kpi" aria-label="CPU">
      <div class="kpi-head"><span>CPU</span><Icon name="speed" size={15} /></div>
      <div class="kpi-val">
        <span class="kpi-num">{cpuPct === null ? '—' : cpuPct}</span>
        {#if cpuPct !== null}<span class="kpi-unit">%</span>{/if}
      </div>
      <div class="kpi-foot">
        {#if cpuPct !== null}
          <progress class="meter {meterTone(cpuPct)}" value={cpuPct} max="100" aria-label="CPU usage {cpuPct}%"></progress>
        {:else}
          <span>Waiting for sample</span>
        {/if}
      </div>
    </section>

    <section class="kpi" aria-label="Memory">
      <div class="kpi-head"><span>Memory</span><Icon name="data" size={15} /></div>
      <div class="kpi-val">
        <span class="kpi-num">{node.memory_used === null ? '—' : bytes(node.memory_used)}</span>
        {#if node.memory_total}<span class="kpi-unit">of {bytes(node.memory_total)}</span>{/if}
      </div>
      <div class="kpi-foot">
        {#if memPct !== null}
          <progress class="meter {meterTone(memPct)}" value={memPct} max="100" aria-label="Memory usage {memPct}%"></progress>
        {:else}
          <span>Waiting for sample</span>
        {/if}
      </div>
    </section>

    <section class="kpi" aria-label="Traffic now">
      <div class="kpi-head"><span>Traffic now</span><Icon name="arc" size={15} /></div>
      <div class="kpi-val">
        <span class="kpi-num">{traffic[0]}</span>
        {#if traffic[1]}<span class="kpi-unit">{traffic[1]}</span>{/if}
      </div>
      <div class="kpi-foot">Started {date(node.started_at)}</div>
    </section>
  </div>

  <div class="workspace">
    <div class="col">
      <section aria-labelledby="runtime-title">
        <div class="group-head"><h2 id="runtime-title" class="group-title">Overview</h2></div>
        <div class="group">
          <div class="group-row">
            <span class="info-label">Version</span>
            <span class="info-value">
              {#if node.version}<span class="mono">{node.version}</span>{#if node.arch}<span class="badge">{node.arch}</span>{/if}{:else}—{/if}
            </span>
          </div>
          <div class="group-row">
            <span class="info-label">Address</span>
            <span class="info-value mono">{node.address}:{node.port}</span>
          </div>
          <div class="group-row">
            <span class="info-label">Started</span>
            <span class="info-value">{date(node.started_at)}</span>
          </div>
          <div class="group-row">
            <span class="info-label">Settings</span>
            <span class="info-value"><NodeSync {node} /></span>
          </div>
          <div class="group-row">
            <span class="info-label">Last seen</span>
            <span class="info-value">{ago(node.last_seen)}</span>
          </div>
        </div>
      </section>

      <section aria-labelledby="disguise-title">
        <div class="group-head">
          <h2 id="disguise-title" class="group-title">TLS disguise</h2>
          <a class="small" href="/disguises">Manage templates</a>
        </div>
        <div class="group">
          <div class="group-row">
            <div class="setting-lead">
              <span class="icon-tile {node.disguise_id === null ? 'warn' : node.applied_version < node.disguise_version ? 'warn' : 'green'}">
                <Icon name="globe" size={16} />
              </span>
              <div class="setting-copy">
                {#if node.disguise_id === null}
                  <div class="setting-title">No disguise selected <span class="badge warn">Required</span></div>
                  <p class="setting-desc">Set a disguise before sharing user links.</p>
                {:else if node.applied_version < node.disguise_version}
                  <div class="setting-title">
                    <span>{node.disguise_name}</span>
                    <span class="badge {node.status === 'error' ? 'danger' : 'warn'}">{node.status === 'error' ? 'Rejected' : 'Applying'}</span>
                  </div>
                  <p class="setting-desc" role="status">{node.status === 'error' ? 'The server rejected the settings.' : 'Waiting for the server to apply the change.'}</p>
                {:else}
                  <div class="setting-title">
                    <span>{node.disguise_name}</span>
                    <span class="badge green">Applied</span>
                  </div>
                  <p class="setting-desc">
                    {#if node.disguise_changed_at}Selected {date(node.disguise_changed_at)}.{:else}Active TLS template for incoming connections.{/if}
                  </p>
                {/if}
              </div>
            </div>
            <div class="setting-control">
              <Button size="compact" onclick={() => (disguiseOpen = true)}>{node.disguise_id === null ? 'Set disguise' : 'Change'}</Button>
            </div>
          </div>
          {#if node.disguise_id !== null && node.applied_version >= node.disguise_version && node.disguise_changed_at}
            <div class="group-row note-row">
              <p class="small muted">Existing profiles keep their old TLS template. <a href="/users">Open Users</a> to share updated links.</p>
            </div>
          {/if}
        </div>
      </section>
    </div>

    <div class="col">
      <section aria-labelledby="settings-title">
        <div class="group-head"><h2 id="settings-title" class="group-title">Connection</h2></div>
        <form novalidate class="group" onsubmit={save}>
          <div class="group-body stack">
            <Field label="Name" id="node-name" error={fields.name}><input id="node-name" bind:value={name} /></Field>
            <Field label="Public address" id="node-address" error={fields.address} help="Hostname or public IP placed in shared user links."><input id="node-address" bind:value={address} /></Field>
          </div>
          <div class="group-footer">
            <span class="small muted">Port <span class="mono">{node.port}</span></span>
            <Button kind="primary" size="compact" type="submit" busy={saving} disabled={!dirty && !saving}>Save changes</Button>
          </div>
        </form>
      </section>

      <section aria-labelledby="diag-title">
        <div class="group-head"><h2 id="diag-title" class="group-title">Diagnostics</h2></div>
        <div class="group">
          <a class="group-row interactive" href="/probes">
            <div class="setting-lead">
              <span class="icon-tile"><Icon name="shield" size={16} /></span>
              <div class="setting-copy">
                <div class="setting-title">Origin probes</div>
                <p class="setting-desc">Unauthenticated connections forwarded to the origin.</p>
              </div>
            </div>
            <span class="muted"><Icon name="chevron" size={14} /></span>
          </a>
          <a class="group-row interactive" href="/disguises">
            <div class="setting-lead">
              <span class="icon-tile"><Icon name="globe" size={16} /></span>
              <div class="setting-copy">
                <div class="setting-title">TLS disguises</div>
                <p class="setting-desc">Inspect saved templates and probe origin health.</p>
              </div>
            </div>
            <span class="muted"><Icon name="chevron" size={14} /></span>
          </a>
        </div>
      </section>
    </div>
  </div>

  <section class="logs-section" aria-labelledby="logs-title">
    <div class="group-head">
      <h2 id="logs-title" class="group-title">
        Logs
        {#if logs}<span class="badge">{logs.length} line{logs.length === 1 ? '' : 's'}</span>{/if}
      </h2>
      <div class="row">
        {#if logs?.length}
          <Button size="compact" kind="ghost" icon={copiedLogs ? 'check' : 'copy'} onclick={copyLogs}>{copiedLogs ? 'Copied' : 'Copy'}</Button>
        {/if}
        <Button size="compact" icon={logs ? 'refresh' : 'log'} busy={loadingLogs} onclick={loadLogs}>{logs ? 'Reload' : 'Show logs'}</Button>
      </div>
    </div>
    <div class="group">
      {#if logs === null}
        <div class="group-row">
          <div class="setting-lead">
            <span class="icon-tile sm"><Icon name="log" size={15} /></span>
            <p class="setting-desc">Fetch recent daemon output from this server on demand.</p>
          </div>
        </div>
      {:else if logs.length}
        <pre class="logs" aria-label="Node log lines">{logs.join('\n')}</pre>
      {:else}
        <div class="group-row"><p class="muted small">No log lines available.</p></div>
      {/if}
    </div>
  </section>

  {#if disguiseOpen}
    <DisguiseSwitch bind:open={disguiseOpen} {node} onsaved={(updated) => (node = updated)} />
  {/if}
  <Confirm bind:open={restarting} title="Restart {node.name}" message="Open connections close." action="Restart" onconfirm={async () => { await api.post(`/nodes/${id}/restart`); toast('The node restarts'); }} />
{:else if error}
  <p class="note error" role="alert">{error}</p>
{:else}
  <p class="muted">Loading…</p>
{/if}

<style>
  .banner { margin-bottom: 16px; }
  .workspace { display: grid; grid-template-columns: minmax(0, 1.3fr) minmax(0, 1fr); gap: 20px; margin-top: 22px; align-items: start; }
  .col { display: flex; flex-direction: column; gap: 20px; min-width: 0; }
  .note-row { background: var(--soft); min-height: 0; padding: 9px 16px; }
  .logs-section { margin-top: 22px; }
  .logs { margin: 0; max-height: 380px; overflow: auto; padding: 14px 16px; background: var(--soft); font: 11.5px/1.65 var(--mono); color: var(--text); white-space: pre-wrap; overflow-wrap: anywhere; }
  @media (max-width: 960px) { .workspace { grid-template-columns: minmax(0, 1fr); } }
</style>
