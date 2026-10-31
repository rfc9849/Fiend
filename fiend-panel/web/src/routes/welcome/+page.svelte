<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import Centered from '#lib/components/Centered.svelte';
  import Field from '#lib/components/Field.svelte';
  import Button from '#lib/components/Button.svelte';
  import TemplatePicker from '#lib/components/TemplatePicker.svelte';
  import Qr from '#lib/components/Qr.svelte';
  import CopyField from '#lib/components/CopyField.svelte';
  import { api, fieldErrors, loadSession } from '#lib/api/client.ts';
  import { session } from '#lib/stores/session.svelte.ts';
  import { toast } from '#lib/stores/toasts.svelte.ts';
  import type { Created, LocalNodeDefaults, NodeView, PresetView, UserLink, UserView } from '#lib/api/generated/index.ts';

  let step = $state(2);
  let defaults = $state<LocalNodeDefaults | null>(null);
  let localNode = $state<NodeView | null>(null);
  let presets = $state<PresetView[]>([]);
  let nodeName = $state('main');
  let address = $state('');
  let preset = $state('wikipedia');
  let file = $state<string | null>(null);
  let userName = $state('');
  let link = $state<UserLink | null>(null);
  let busy = $state(false);
  let error = $state('');
  let fields = $state<Record<string, string>>({});

  async function refresh() {
    const nodes = await api.get<NodeView[]>('/nodes');
    localNode = nodes.find((node) => node.local) ?? null;
    session.info = await loadSession();
  }

  onMount(async () => {
    try {
      [defaults, presets] = await Promise.all([api.get<LocalNodeDefaults>('/wizard/local'), api.get<PresetView[]>('/disguises/presets')]);
      if (defaults.address && !address) address = defaults.address;
      await refresh();
      const wizard = session.info?.wizard;
      if (wizard?.local_node) step = wizard.disguise ? (wizard.first_user ? 4 : 4) : 3;
    } catch (failure) {
      error = fieldErrors(failure).form ?? '';
    }
  });

  function fail(failure: unknown) {
    const result = fieldErrors(failure);
    error = result.form ?? '';
    fields = result.fields;
  }

  async function saveNode(event: SubmitEvent) {
    event.preventDefault();
    fields = {};
    error = '';
    if (localNode) {
      step = 3;
      return;
    }
    if (!address.trim()) fields.address = 'Enter the public IP address of this server.';
    if (!nodeName.trim()) fields.name = 'Enter a name.';
    if (Object.keys(fields).length) return;
    busy = true;
    try {
      await api.post<Created>('/wizard/local', { name: nodeName.trim(), address: address.trim() });
      await refresh();
      step = 3;
    } catch (failure) {
      fail(failure);
    } finally {
      busy = false;
    }
  }

  async function saveDisguise(event: SubmitEvent) {
    event.preventDefault();
    fields = {};
    error = '';
    if (!localNode) return;
    if (session.info?.wizard.disguise) {
      step = 4;
      return;
    }
    if (preset === 'file' && !file) fields.template = 'Choose a template file.';
    if (Object.keys(fields).length) return;
    busy = true;
    try {
      const created = await api.post<Created>('/disguises', {
        preset: preset === 'file' ? null : preset,
        template_base64: preset === 'file' ? file : null
      });
      await api.put(`/nodes/${localNode.id}`, { name: localNode.name, address: localNode.address, port: localNode.port, disguise_id: created.id });
      try {
        await api.post(`/disguises/${created.id}/probe`, { node_id: localNode.id });
      } catch (failure) {
        toast(`The origin probe failed. You can probe again on the Disguises page. ${fieldErrors(failure).form ?? ''}`, true);
      }
      await refresh();
      step = 4;
    } catch (failure) {
      fail(failure);
    } finally {
      busy = false;
    }
  }

  async function saveUser(event: SubmitEvent) {
    event.preventDefault();
    fields = {};
    error = '';
    if (!userName.trim()) {
      fields.name = 'Enter a name for the first user.';
      return;
    }
    busy = true;
    try {
      const user = await api.post<UserView>('/users', { name: userName.trim(), enabled: true, note: '', traffic_limit: null, expire_at: null, hold_days: null, reset_period: 'none', nodes: [] });
      link = await api.get<UserLink>(`/users/${user.id}/link`);
    } catch (failure) {
      fail(failure);
    } finally {
      busy = false;
    }
  }

  async function finish() {
    busy = true;
    try {
      await api.post('/wizard/done');
      session.info = await loadSession();
      await goto('/');
    } catch (failure) {
      fail(failure);
    } finally {
      busy = false;
    }
  }
</script>

<svelte:head><title>Set up · fiend</title></svelte:head>

<Centered wide>
  <p class="step">Step {step} of 4</p>
  {#if error}<p class="note error" role="alert">{error}</p>{/if}
  {#if step === 2}
    <h1>Local node</h1>
    <p class="lead">This server runs the first node. The panel controls it through a local socket.</p>
    <form novalidate class="stack" onsubmit={saveNode}>
      {#if defaults}
        <dl class="facts">
          <dt>Node port</dt><dd class="mono">{defaults.port}</dd>
          <dt>Settings</dt><dd>{defaults.imported ? `Imported from the old server. ${defaults.users} existing secret${defaults.users === 1 ? '' : 's'} become users, so existing clients keep working.` : 'New node'}</dd>
        </dl>
        {#if defaults.error}<p class="note error">{defaults.error}</p>{/if}
      {/if}
      {#if localNode}
        <p class="note">The local node <strong>{localNode.name}</strong> is added with the address <code>{localNode.address}</code>.</p>
      {:else}
        <div class="grid-2">
          <Field label="Name" id="name" error={fields.name}><input id="name" bind:value={nodeName} /></Field>
          <Field label="Public address" id="address" error={fields.address} help={defaults?.address ? 'Found automatically. Change it if clients use a different IP address or host name.' : 'Clients connect to this IP address or host name.'}><input id="address" inputmode="url" placeholder="203.0.113.10" bind:value={address} /></Field>
        </div>
      {/if}
      <div class="row end"><Button kind="primary" type="submit" {busy} disabled={!defaults?.available && !localNode}>Next</Button></div>
    </form>
  {:else if step === 3}
    <h1>Disguise</h1>
    <p class="lead">Clients send a copy of a real browser ClientHello. Visitors without a secret see the origin site.</p>
    <form novalidate class="stack" onsubmit={saveDisguise}>
      {#if session.info?.wizard.disguise}
        <p class="note">The local node has a disguise. You can change it on the Disguises page.</p>
      {:else}
        <TemplatePicker {presets} bind:preset bind:file error={fields.template} />
        <p class="small muted">The panel probes the origin from the local node.</p>
      {/if}
      <div class="row between">
        <Button kind="ghost" onclick={() => (step = 2)}>Back</Button>
        <Button kind="primary" type="submit" {busy}>Next</Button>
      </div>
    </form>
  {:else}
    <h1>First user</h1>
    <p class="lead">Each user has an own secret. A link holds all nodes of the user.</p>
    {#if link}
      <div class="share">
        <Qr svg={link.qr_svg} label="QR code with the link for {userName}" />
        <div class="stack grow">
          <p>Scan the code with the Android app, or copy the link into the desktop app or the CLI.</p>
          <CopyField value={link.link} label="link" />
          {#each link.warnings as warning}<p class="note warn">{warning}</p>{/each}
        </div>
      </div>
      <div class="row between">
        <Button kind="ghost" onclick={() => (step = 3)}>Back</Button>
        <Button kind="primary" {busy} onclick={finish}>Open the dashboard</Button>
      </div>
    {:else}
      <form novalidate class="stack" onsubmit={saveUser}>
        <Field label="User name" id="name" error={fields.name}><input id="name" bind:value={userName} placeholder="phone" /></Field>
        <div class="row between">
          <Button kind="ghost" onclick={() => (step = 3)}>Back</Button>
          <div class="row">
            {#if session.info?.wizard.first_user}<Button kind="ghost" onclick={finish}>Skip</Button>{/if}
            <Button kind="primary" type="submit" {busy}>Create user</Button>
          </div>
        </div>
      </form>
    {/if}
  {/if}
</Centered>

<style>
  .step { font-size: 11px; color: var(--muted); margin-bottom: 4px; }
  .lead { margin: 6px 0 18px; line-height: 1.6; }
  .facts { display: grid; grid-template-columns: max-content 1fr; gap: 6px 16px; margin: 0; }
  dt { color: var(--muted); }
  dd { margin: 0; }
  .end { justify-content: flex-end; }
  .between { justify-content: space-between; }
  .share { display: flex; gap: 18px; align-items: flex-start; margin-bottom: 18px; }
  .grow { flex: 1; min-width: 0; }
  @media (max-width: 719px) { .share { flex-direction: column; } }
</style>
