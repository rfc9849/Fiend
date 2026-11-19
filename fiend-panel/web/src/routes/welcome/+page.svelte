<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { returnPath } from '#lib/navigation.ts';
  import Centered from '#lib/components/Centered.svelte';
  import Field from '#lib/components/Field.svelte';
  import Button from '#lib/components/Button.svelte';
  import DisguiseSwitch from '#lib/components/DisguiseSwitch.svelte';
  import Qr from '#lib/components/Qr.svelte';
  import CopyField from '#lib/components/CopyField.svelte';
  import { api, fieldErrors, loadSession } from '#lib/api/client.ts';
  import { session } from '#lib/stores/session.svelte.ts';
  import type { Created, LocalNodeDefaults, NodeView, UserLink, UserView } from '#lib/api/generated/index.ts';

  let step = $state(2);
  let defaults = $state<LocalNodeDefaults | null>(null);
  let localNode = $state<NodeView | null>(null);
  let nodeName = $state('main');
  let address = $state('');
  let disguiseOpen = $state(false);
  let userName = $state('');
  let link = $state<UserLink | null>(null);
  let createdUser = $state<number | null>(null);
  let busy = $state(false);
  let checking = $state(false);
  let error = $state('');
  let fields = $state<Record<string, string>>({});

  async function refresh() {
    const nodes = await api.get<NodeView[]>('/nodes');
    localNode = nodes.find((node) => node.local) ?? null;
    session.info = await loadSession();
  }

  async function checkNode() {
    checking = true;
    error = '';
    try {
      defaults = await api.get<LocalNodeDefaults>('/wizard/local');
      if (defaults.address && !address) address = defaults.address;
    } catch (failure) {
      fail(failure);
    } finally {
      checking = false;
    }
  }

  onMount(async () => {
    await checkNode();
    try {
      await refresh();
      const wizard = session.info?.wizard;
      if (wizard?.local_node) step = wizard.disguise ? 4 : 3;
    } catch (failure) {
      fail(failure);
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
    if (!address.trim()) fields.address = 'Enter the public IP address or host name of this server.';
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

  async function loadLink() {
    if (createdUser === null) return;
    busy = true;
    error = '';
    try {
      link = await api.get<UserLink>(`/users/${createdUser}/link`);
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
      createdUser = user.id;
      await loadLink();
    } catch (failure) {
      fail(failure);
    } finally {
      busy = false;
    }
  }

  async function finish() {
    if (busy) return;
    fields = {};
    error = '';
    busy = true;
    try {
      await api.post('/wizard/done');
      session.info = await loadSession();
      const next = returnPath(page.url);
      await goto(new URL(next, page.url.origin).pathname === '/welcome' ? '/' : next, { replaceState: true });
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
    <h1 class="title">Local node</h1>
    <form novalidate class="stack" onsubmit={saveNode}>
      {#if defaults}
        <dl class="facts">
          <dt>Port</dt><dd class="mono">{defaults.port}</dd>
        </dl>
        {#if defaults.error}<p class="note error">{defaults.error}</p>{/if}
      {/if}
      {#if localNode}
        <p class="note"><strong>{localNode.name}</strong> · <code>{localNode.address}</code></p>
      {:else}
        <div class="grid-2">
          <Field label="Name" id="name" error={fields.name}><input id="name" bind:value={nodeName} /></Field>
          <Field label="Public address" id="address" error={fields.address}><input id="address" inputmode="url" placeholder="203.0.113.10" bind:value={address} /></Field>
        </div>
      {/if}
      <div class="row between">
        <Button kind="ghost" disabled={busy} onclick={finish}>Finish setup later</Button>
        <div class="row">
          {#if !localNode && !defaults?.available}
            <Button busy={checking} disabled={busy} onclick={checkNode}>Check node again</Button>
          {/if}
          <Button kind="primary" type="submit" {busy} disabled={checking || (!defaults?.available && !localNode)}>Next</Button>
        </div>
      </div>
    </form>
  {:else if step === 3}
    <h1 class="title">TLS disguise</h1>
    <div class="stack">
      {#if localNode?.disguise_id}
        <p class="note">Selected: {localNode.disguise_name}.</p>
      {:else}
        <p>Choose and check the TLS disguise before sharing a link.</p>
      {/if}
      <div class="row between">
        <Button kind="ghost" disabled={busy} onclick={() => (step = 2)}>Back</Button>
        <div class="row">
          <Button kind="ghost" disabled={busy} onclick={finish}>Finish setup later</Button>
          {#if localNode?.disguise_id}
            <Button kind="primary" onclick={() => (step = 4)}>Next</Button>
          {:else if localNode}
            <Button kind="primary" onclick={() => (disguiseOpen = true)}>Set disguise</Button>
          {/if}
        </div>
      </div>
    </div>
    {#if disguiseOpen && localNode}
      <DisguiseSwitch bind:open={disguiseOpen} node={localNode} onsaved={(updated) => { localNode = updated; step = 4; }} />
    {/if}
  {:else}
    <h1 class="title">First user</h1>
    {#if link}
      <div class="share">
        <Qr svg={link.qr_svg} label="QR code with the link for {userName}" />
        <div class="stack grow">
          <CopyField value={link.link} label="link" />
          {#each link.warnings as warning}<p class="note warn">{warning}</p>{/each}
        </div>
      </div>
      <div class="row between">
        <Button kind="ghost" onclick={() => (step = 3)}>Back</Button>
        <Button kind="primary" {busy} onclick={finish}>Finish setup</Button>
      </div>
    {:else if createdUser !== null}
      <div class="stack">
        <p>User {userName} was created.</p>
        <div class="row between">
          <Button kind="ghost" disabled={busy} onclick={finish}>Open panel</Button>
          <Button kind="primary" {busy} onclick={loadLink}>Retry getting link</Button>
        </div>
      </div>
    {:else}
      <form novalidate class="stack" onsubmit={saveUser}>
        <Field label="User name" id="name" error={fields.name}><input id="name" bind:value={userName} placeholder="phone" /></Field>
        <div class="row between">
          <Button kind="ghost" onclick={() => (step = 3)}>Back</Button>
          <div class="row">
            <Button kind="ghost" disabled={busy} onclick={finish}>{session.info?.wizard.first_user ? 'Finish setup' : 'Finish setup later'}</Button>
            <Button kind="primary" type="submit" {busy}>Create user</Button>
          </div>
        </div>
      </form>
    {/if}
  {/if}
</Centered>

<style>
  .step { font-size: 11px; color: var(--muted); margin-bottom: 4px; }
  .title { margin-bottom: 18px; }
  .facts { display: grid; grid-template-columns: max-content 1fr; gap: 6px 16px; margin: 0; }
  dt { color: var(--muted); }
  dd { margin: 0; }
  .between { justify-content: space-between; }
  .share { display: flex; gap: 18px; align-items: flex-start; margin-bottom: 18px; }
  .grow { flex: 1; min-width: 0; }
  @media (max-width: 719px) { .share { flex-direction: column; } }
</style>
