<script lang="ts">
  import { onMount } from 'svelte';
  import TopBar from '#lib/components/TopBar.svelte';
  import Button from '#lib/components/Button.svelte';
  import Dialog from '#lib/components/Dialog.svelte';
  import Confirm from '#lib/components/Confirm.svelte';
  import Select from '#lib/components/Select.svelte';
  import TemplatePicker from '#lib/components/TemplatePicker.svelte';
  import EmptyState from '#lib/components/EmptyState.svelte';
  import Icon from '#lib/components/Icon.svelte';
  import { api, fieldErrors } from '#lib/api/client.ts';
  import { toast } from '#lib/stores/toasts.svelte.ts';
  import { date } from '#lib/format.ts';
  import type { DisguiseView, NodeView, PresetView } from '#lib/api/generated/index.ts';

  let disguises = $state<DisguiseView[]>([]);
  let presets = $state<PresetView[]>([]);
  let nodes = $state<NodeView[]>([]);
  let loading = $state(true);
  let adding = $state(false);
  let preset = $state('wikipedia');
  let file = $state<string | null>(null);
  let fields = $state<Record<string, string>>({});
  let formError = $state('');
  let busy = $state(false);
  let probeNode = $state<Record<number, string>>({});
  let probing = $state<number | null>(null);
  let removing = $state<DisguiseView | null>(null);
  let removeOpen = $state(false);
  let error = $state('');
  const unused = $derived(presets.filter((preset) => !disguises.some((item) => item.name === preset.name)));

  function openAdd() {
    preset = unused[0]?.id ?? 'file';
    file = null;
    fields = {};
    formError = '';
    adding = true;
  }

  async function load() {
    try {
      [disguises, presets, nodes] = await Promise.all([api.get<DisguiseView[]>('/disguises'), api.get<PresetView[]>('/disguises/presets'), api.get<NodeView[]>('/nodes')]);
      for (const item of disguises) probeNode[item.id] ??= nodes[0] ? String(nodes[0].id) : '';
      error = '';
    } catch (failure) {
      error = fieldErrors(failure).form ?? '';
    } finally {
      loading = false;
    }
  }

  onMount(load);

  async function create(event: SubmitEvent) {
    event.preventDefault();
    fields = {};
    formError = '';
    if (preset === 'file' && !file) fields.template = 'Choose a template file.';
    if (Object.keys(fields).length) return;
    busy = true;
    try {
      await api.post('/disguises', { preset: preset === 'file' ? null : preset, template_base64: preset === 'file' ? file : null });
      adding = false;
      toast('Disguise added');
      await load();
    } catch (failure) {
      const result = fieldErrors(failure);
      formError = result.form ?? '';
      fields = result.fields;
    } finally {
      busy = false;
    }
  }

  async function probe(item: DisguiseView) {
    probing = item.id;
    try {
      await api.post(`/disguises/${item.id}/probe`, { node_id: Number(probeNode[item.id]) });
      toast('Origin probed');
    } catch (failure) {
      toast(fieldErrors(failure).form ?? 'The probe failed', true);
    } finally {
      probing = null;
      await load();
    }
  }
</script>

<svelte:head><title>Disguises · fiend</title></svelte:head>

<TopBar title="Disguises" subtitle="TLS templates and origin sites used for unauthenticated traffic.">
  {#snippet actions()}
    <Button kind="primary" icon="plus" onclick={openAdd}>Add disguise</Button>
  {/snippet}
</TopBar>

{#if error}<p class="note error banner" role="alert">{error}</p>{/if}

{#if loading}
  <p class="muted">Loading…</p>
{:else if disguises.length === 0}
  <EmptyState icon="globe" title="No disguises yet" message="Add a preset or import a TLS template, then assign it on the Server page.">
    {#snippet actions()}
      <Button kind="primary" icon="plus" onclick={openAdd}>Add disguise</Button>
    {/snippet}
  </EmptyState>
{:else}
  <div class="cards">
    {#each disguises as item (item.id)}
      <section class="group disguise-card" aria-labelledby="disguise-{item.id}">
        <div class="group-row card-top">
          <div class="setting-lead">
            <span class="icon-tile {item.nodes > 0 ? 'green' : ''}"><Icon name="globe" size={16} /></span>
            <div class="setting-copy">
              <div class="setting-title"><h2 id="disguise-{item.id}">{item.name}</h2></div>
              <p class="mono origin">{item.origin_host}:{item.origin_port}</p>
            </div>
          </div>
          <span class="badge {item.nodes > 0 ? 'green' : ''}">{item.nodes > 0 ? `In use · ${item.nodes} node${item.nodes === 1 ? '' : 's'}` : 'Unused'}</span>
        </div>

        {#if item.checks.length}
          <ul class="checks">
            {#each item.checks as check (check.name)}
              <li class:ok={check.ok}>
                <span class="check-icon"><Icon name={check.ok ? 'check' : 'info'} size={15} label={check.ok ? 'passed' : 'needs attention'} /></span>
                <div class="check-body">
                  <span class="check-name">{check.name}</span>
                  <span class="check-detail">{check.detail}</span>
                </div>
              </li>
            {/each}
          </ul>
        {/if}

        <div class="meta-rows">
          {#if item.profile}
            <div class="group-row compact-row">
              <span class="info-label">Record sizes</span>
              <span class="info-value mono">{item.profile.record_sizes.join(', ')}</span>
            </div>
          {/if}
          <div class="group-row compact-row">
            <span class="info-label">Last probe</span>
            <span class="info-value">{date(item.probed_at)}</span>
          </div>
        </div>

        <div class="group-footer">
          <div class="row">
            {#if nodes.length > 1}
              <div class="node"><Select id="probe-{item.id}" label="Probe from node" bind:value={probeNode[item.id]} options={nodes.map((node) => ({ value: String(node.id), label: node.name }))} /></div>
            {/if}
            {#if nodes.length}
              <Button size="compact" icon="refresh" busy={probing === item.id} onclick={() => probe(item)}>{nodes.length > 1 ? 'Probe origin' : `Probe from ${nodes[0].name}`}</Button>
            {/if}
          </div>
          {#if item.nodes === 0}
            <Button size="compact" kind="danger" onclick={() => { removing = item; removeOpen = true; }}>Remove</Button>
          {/if}
        </div>
      </section>
    {/each}
  </div>
{/if}

<Dialog bind:open={adding} title="Add disguise">
  <form id="add-disguise" novalidate class="stack" onsubmit={create}>
    {#if formError}<p class="note error" role="alert">{formError}</p>{/if}
    <TemplatePicker presets={unused} bind:preset bind:file error={fields.template} />
  </form>
  {#snippet actions()}
    <Button kind="ghost" onclick={() => (adding = false)}>Cancel</Button>
    <Button kind="primary" type="submit" form="add-disguise" {busy}>Add disguise</Button>
  {/snippet}
</Dialog>

<Confirm bind:open={removeOpen} title="Remove {removing?.name ?? ''}" action="Remove" onconfirm={async () => { if (removing) await api.del(`/disguises/${removing.id}`); toast('Disguise removed'); await load(); }} />

<style>
  .banner { margin-bottom: 16px; }
  .cards { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 460px), 1fr)); gap: 16px; align-items: start; }
  .disguise-card { display: flex; flex-direction: column; }
  .card-top { padding: 14px 16px; }
  .origin { font-size: 11.5px; color: var(--muted); margin-top: 2px; }
  .checks { list-style: none; margin: 0; padding: 12px 16px; border-top: 1px solid var(--line); display: flex; flex-direction: column; gap: 9px; background: var(--panel); }
  .checks li { display: flex; gap: 9px; align-items: flex-start; }
  .check-icon { margin-top: 1px; color: var(--warn); display: inline-flex; flex: none; }
  .checks li.ok .check-icon { color: var(--green); }
  .check-body { display: flex; flex-direction: column; gap: 1px; min-width: 0; }
  .check-name { font-size: 12px; font-weight: 600; color: var(--text); }
  .check-detail { font-size: 12px; color: var(--muted); line-height: 1.45; overflow-wrap: anywhere; }
  .meta-rows { border-top: 1px solid var(--line); }
  .compact-row { min-height: 38px; padding: 8px 16px; }
  .node { width: 150px; }
</style>
