<script lang="ts">
  import Dialog from './Dialog.svelte';
  import Field from './Field.svelte';
  import Button from './Button.svelte';
  import Select from './Select.svelte';
  import Steps from './Steps.svelte';
  import { goto } from '$app/navigation';
  import { api, fieldErrors } from '#lib/api/client.ts';
  import { onEvent } from '#lib/stores/events.ts';
  import { toast } from '#lib/stores/toasts.svelte.ts';
  import type { DisguiseView, HostKeyView, JobStarted, JobView } from '#lib/api/generated/index.ts';

  let { open = $bindable(), ondone }: { open: boolean; ondone: () => void } = $props();
  let stage = $state<'host' | 'confirm' | 'credentials' | 'progress'>('host');
  let host = $state('');
  let sshPort = $state('22');
  let key = $state<HostKeyView | null>(null);
  let name = $state('');
  let user = $state('root');
  let auth = $state('key');
  let privateKey = $state('');
  let passphrase = $state('');
  let password = $state('');
  let sudo = $state('');
  let nodePort = $state('443');
  let disguise = $state('');
  let disguises = $state<DisguiseView[]>([]);
  let job = $state<JobView | null>(null);
  let busy = $state(false);
  let error = $state('');
  let fields = $state<Record<string, string>>({});
  let stop: (() => void) | null = null;

  $effect(() => {
    if (open) {
      stage = 'host';
      key = null;
      job = null;
      error = '';
      fields = {};
      api.get<DisguiseView[]>('/disguises').then((items) => {
        disguises = items;
        if (!disguise && items[0]) disguise = String(items[0].id);
      }).catch(() => {});
    } else {
      privateKey = '';
      passphrase = '';
      password = '';
      sudo = '';
      stop?.();
      stop = null;
    }
  });

  function fail(failure: unknown) {
    const result = fieldErrors(failure);
    error = result.form ?? '';
    fields = result.fields;
  }

  async function scan(event: SubmitEvent) {
    event.preventDefault();
    fields = {};
    error = '';
    const port = Number(sshPort);
    if (!host.trim()) fields.host = 'Enter the server address.';
    if (!Number.isInteger(port) || port < 1 || port > 65535) fields.ssh_port = 'Enter a port from 1 to 65535.';
    if (Object.keys(fields).length) return;
    busy = true;
    try {
      key = await api.post<HostKeyView>('/ssh/scan', { host: host.trim(), port });
      if (!name) name = host.trim().split('.')[0];
      stage = 'confirm';
    } catch (failure) {
      fail(failure);
    } finally {
      busy = false;
    }
  }

  async function install(event: SubmitEvent) {
    event.preventDefault();
    fields = {};
    error = '';
    const port = Number(nodePort);
    if (!name.trim()) fields.name = 'Enter a name.';
    if (!user.trim()) fields.user = 'Enter the SSH user.';
    if (auth === 'key' && !privateKey.trim()) fields.private_key = 'Paste the private key.';
    if (auth === 'password' && !password) fields.password = 'Enter the SSH password.';
    if (!Number.isInteger(port) || port < 1 || port > 65535) fields.node_port = 'Enter a port from 1 to 65535.';
    if (Object.keys(fields).length) return;
    busy = true;
    try {
      const started = await api.post<JobStarted>('/ssh/install', {
        scan_id: key?.scan_id,
        name: name.trim(),
        address: host.trim(),
        ssh_port: Number(sshPort),
        user: user.trim(),
        private_key: auth === 'key' ? privateKey : null,
        passphrase: auth === 'key' && passphrase ? passphrase : null,
        password: auth === 'password' ? password : null,
        sudo_password: user.trim() !== 'root' && sudo ? sudo : null,
        node_port: port,
        disguise_id: disguise ? Number(disguise) : null
      });
      privateKey = '';
      passphrase = '';
      password = '';
      sudo = '';
      stage = 'progress';
      const refresh = async () => {
        job = await api.get<JobView>(`/jobs/${started.job_id}`);
        if (job.done) {
          ondone();
          if (!job.error) toast(`Node ${name} is online`);
        }
      };
      stop = onEvent((kind, id) => {
        if (kind === 'job' && id === started.job_id) refresh();
      });
      await refresh();
    } catch (failure) {
      fail(failure);
      if (failure && (failure as { message?: string }).message?.includes('host key')) stage = 'host';
    } finally {
      busy = false;
    }
  }
</script>

<Dialog bind:open title="Add a node over SSH" wide>
  {#if error}<p class="note error" role="alert">{error}</p>{/if}
  {#if stage === 'host'}
    <form id="add-host" novalidate class="stack" onsubmit={scan}>
      <div class="grid-2">
        <Field label="Server address" id="host" error={fields.host}><input id="host" placeholder="203.0.113.20" bind:value={host} /></Field>
        <Field label="SSH port" id="ssh_port" error={fields.ssh_port}><input id="ssh_port" inputmode="numeric" bind:value={sshPort} /></Field>
      </div>
    </form>
  {:else if stage === 'confirm' && key}
    <div class="stack">
      <p>Check it on the server: <code>ssh-keygen -lf /etc/ssh/ssh_host_{key.algorithm.replace('ssh-', '').replace('ecdsa-sha2-nistp256', 'ecdsa')}_key.pub</code></p>
      <div class="fingerprint"><span class="muted small">{key.algorithm}</span><code>{key.fingerprint}</code></div>
    </div>
  {:else if stage === 'credentials'}
    <form id="add-credentials" novalidate class="stack" onsubmit={install}>
      <div class="grid-2">
        <Field label="Node name" id="name" error={fields.name}><input id="name" bind:value={name} /></Field>
        <Field label="Node port" id="node_port" error={fields.node_port}><input id="node_port" inputmode="numeric" bind:value={nodePort} /></Field>
        <Field label="SSH user" id="user" error={fields.user}><input id="user" autocomplete="off" bind:value={user} /></Field>
        <Field label="Sign in with" id="auth"><Select id="auth" bind:value={auth} label="Sign in with" options={[{ value: 'key', label: 'Private key' }, { value: 'password', label: 'Password' }]} /></Field>
      </div>
      {#if auth === 'key'}
        <Field label="Private key" id="private_key" error={fields.private_key} help="Not saved."><textarea id="private_key" autocomplete="off" spellcheck="false" bind:value={privateKey}></textarea></Field>
        <Field label="Key passphrase" id="passphrase"><input id="passphrase" type="password" autocomplete="off" bind:value={passphrase} /></Field>
      {:else}
        <Field label="SSH password" id="password" error={fields.password}><input id="password" type="password" autocomplete="off" bind:value={password} /></Field>
      {/if}
      {#if user.trim() !== 'root'}
        <Field label="sudo password" id="sudo" help="Empty: SSH password or passwordless sudo."><input id="sudo" type="password" autocomplete="off" bind:value={sudo} /></Field>
      {/if}
      <Field label="Disguise" id="disguise">
        <Select id="disguise" bind:value={disguise} label="Disguise" options={[...disguises.map((item) => ({ value: String(item.id), label: item.name, detail: item.origin_host })), { value: '', label: 'Choose later' }]} />
      </Field>
    </form>
  {:else if stage === 'progress'}
    {#if job}
      <Steps steps={job.steps} />
      {#if job.error}<p class="note error result" role="alert">{job.error}</p>{:else if job.done}<p class="note result">Installed.</p>{/if}
    {:else}
      <p class="muted">Starting…</p>
    {/if}
  {/if}
  {#snippet actions()}
    {#if stage === 'host'}
      <Button kind="ghost" onclick={() => (open = false)}>Cancel</Button>
      <Button kind="primary" type="submit" form="add-host" {busy}>Check host key</Button>
    {:else if stage === 'confirm'}
      <Button kind="ghost" onclick={() => (stage = 'host')}>Back</Button>
      <Button kind="primary" onclick={() => (stage = 'credentials')}>Key matches</Button>
    {:else if stage === 'credentials'}
      <Button kind="ghost" onclick={() => (stage = 'confirm')}>Back</Button>
      <Button kind="primary" type="submit" form="add-credentials" {busy}>Install node</Button>
    {:else}
      {#if job?.done && job.node_id && !job.error}<Button onclick={() => { open = false; goto(`/nodes/${job?.node_id}`); }}>Open node</Button>{/if}
      <Button kind="primary" disabled={!job?.done} onclick={() => (open = false)}>Close</Button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .fingerprint { display: flex; flex-direction: column; gap: 4px; padding: 12px; border-radius: 6px; background: var(--soft); border: 1px solid var(--line); }
  .fingerprint code { font-size: 13px; }
  .result { margin-top: 14px; }
</style>
