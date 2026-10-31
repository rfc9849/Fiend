<script lang="ts">
  import { onMount } from 'svelte';
  import TopBar from '#lib/components/TopBar.svelte';
  import Button from '#lib/components/Button.svelte';
  import Dialog from '#lib/components/Dialog.svelte';
  import Confirm from '#lib/components/Confirm.svelte';
  import Field from '#lib/components/Field.svelte';
  import Select from '#lib/components/Select.svelte';
  import TemplatePicker from '#lib/components/TemplatePicker.svelte';
  import Icon from '#lib/components/Icon.svelte';
  import { api, fieldErrors } from '#lib/api/client.ts';
  import { toast } from '#lib/stores/toasts.svelte.ts';
  import { date } from '#lib/format.ts';
  import type { DisguiseView, NodeView, PresetView } from '#lib/api/generated/index.ts';

  let disguises = $state<DisguiseView[]>([]);
  let presets = $state<PresetView[]>([]);
  let nodes = $state<NodeView[]>([]);
  let adding = $state(false);
  let name = $state('');
  let host = $state('');
  let port = $state('443');
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

  async function load() {
    try {
      [disguises, presets, nodes] = await Promise.all([api.get<DisguiseView[]>('/disguises'), api.get<PresetView[]>('/disguises/presets'), api.get<NodeView[]>('/nodes')]);
      for (const item of disguises) probeNode[item.id] ??= nodes[0] ? String(nodes[0].id) : '';
    } catch (failure) {
      error = fieldErrors(failure).form ?? '';
    }
  }

  onMount(load);

  async function create(event: SubmitEvent) {
    event.preventDefault();
    fields = {};
    formError = '';
    const number = Number(port);
    if (!name.trim()) fields.name = 'Enter a name.';
    if (!host.trim()) fields.origin_host = 'Enter the origin host.';
    if (!Number.isInteger(number) || number < 1 || number > 65535) fields.origin_port = 'Enter a port from 1 to 65535.';
    if (preset === 'file' && !file) fields.template = 'Choose a template file.';
    if (Object.keys(fields).length) return;
    busy = true;
    try {
      await api.post('/disguises', { name: name.trim(), origin_host: host.trim(), origin_port: number, preset: preset === 'file' ? null : preset, template_base64: preset === 'file' ? file : null });
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

<TopBar title="Disguises" subtitle="Origins and TLS templates. A node copies the TLS behavior of its origin.">
  {#snippet actions()}<Button kind="primary" icon="plus" onclick={() => { name = ''; host = ''; port = '443'; preset = presets[0]?.id ?? 'file'; file = null; fields = {}; formError = ''; adding = true; }}>Add disguise</Button>{/snippet}
</TopBar>
{#if error}<p class="note error" role="alert">{error}</p>{/if}
{#if disguises.length === 0}
  <p class="empty">No disguises yet. Add one, then select it on the node page.</p>
{/if}
<div class="cards">
  {#each disguises as item (item.id)}
    <section class="card" aria-labelledby="disguise-{item.id}">
      <div class="card-header">
        <div><h2 id="disguise-{item.id}">{item.name}</h2><p class="mono">{item.origin_host}:{item.origin_port}</p></div>
        <span class="badge">{item.nodes} node{item.nodes === 1 ? '' : 's'}</span>
      </div>
      <ul class="checks">
        {#each item.checks as check (check.name)}
          <li class:ok={check.ok}><Icon name={check.ok ? 'check' : 'info'} size={16} label={check.ok ? 'passed' : 'needs attention'} /><span><strong>{check.name}.</strong> {check.detail}</span></li>
        {/each}
      </ul>
      {#if item.profile}<p class="small muted">Record sizes: <span class="mono">{item.profile.record_sizes.join(', ')}</span></p>{/if}
      <p class="small muted">Last probe: {date(item.probed_at)}</p>
      <div class="row probe">
        {#if nodes.length}
          <div class="node"><Select id="probe-{item.id}" label="Probe from node" bind:value={probeNode[item.id]} options={nodes.map((node) => ({ value: String(node.id), label: node.name }))} /></div>
          <Button icon="refresh" busy={probing === item.id} onclick={() => probe(item)}>Probe origin</Button>
        {/if}
        <Button kind="danger" disabled={item.nodes > 0} onclick={() => { removing = item; removeOpen = true; }}>Remove</Button>
      </div>
    </section>
  {/each}
</div>

<Dialog bind:open={adding} title="Add disguise">
  <form id="add-disguise" novalidate class="stack" onsubmit={create}>
    {#if formError}<p class="note error" role="alert">{formError}</p>{/if}
    <Field label="Name" id="disguise-name" error={fields.name}><input id="disguise-name" bind:value={name} /></Field>
    <div class="grid-2">
      <Field label="Origin host" id="disguise-host" error={fields.origin_host} help="A real HTTPS site."><input id="disguise-host" bind:value={host} placeholder="www.example.com" /></Field>
      <Field label="Origin port" id="disguise-port" error={fields.origin_port}><input id="disguise-port" inputmode="numeric" bind:value={port} /></Field>
    </div>
    <TemplatePicker {presets} bind:preset bind:file error={fields.template} />
  </form>
  {#snippet actions()}
    <Button kind="ghost" onclick={() => (adding = false)}>Cancel</Button>
    <Button kind="primary" type="submit" form="add-disguise" {busy}>Add disguise</Button>
  {/snippet}
</Dialog>

<Confirm bind:open={removeOpen} title="Remove {removing?.name ?? ''}" message="The template and the origin profile are removed from the panel." action="Remove" onconfirm={async () => { if (removing) await api.del(`/disguises/${removing.id}`); toast('Disguise removed'); await load(); }} />

<style>
  .cards { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 380px), 1fr)); gap: 16px; }
  .cards :global(.card + .card) { margin-top: 0; }
  .checks { list-style: none; margin: 0 0 10px; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .checks li { display: flex; gap: 8px; align-items: flex-start; color: var(--warn); }
  .checks li.ok { color: var(--green); }
  .checks li span { color: var(--text); overflow-wrap: anywhere; }
  .probe { margin-top: 12px; }
  .node { width: 160px; }
  .empty { padding: 18px; border: 1px dashed var(--line); border-radius: 9px; text-align: center; }
</style>
