<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import Centered from '#lib/components/Centered.svelte';
  import Field from '#lib/components/Field.svelte';
  import Button from '#lib/components/Button.svelte';
  import Switch from '#lib/components/Switch.svelte';
  import Qr from '#lib/components/Qr.svelte';
  import CopyField from '#lib/components/CopyField.svelte';
  import { api, fieldErrors, loadSession } from '#lib/api/client.ts';
  import { session } from '#lib/stores/session.svelte.ts';
  import type { TotpSetup } from '#lib/api/generated/index.ts';

  const token = page.params.token ?? '';
  let valid = $state<boolean | null>(null);
  let name = $state('admin');
  let password = $state('');
  let confirm = $state('');
  let useTotp = $state(true);
  let totp = $state<TotpSetup | null>(null);
  let code = $state('');
  let busy = $state(false);
  let error = $state('');
  let fields = $state<Record<string, string>>({});

  onMount(async () => {
    try {
      await api.get(`/setup/${token}`);
      totp = await api.get<TotpSetup>(`/setup/${token}/totp`);
      valid = true;
    } catch {
      valid = false;
    }
  });

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    fields = {};
    error = '';
    if (!name.trim()) fields.name = 'Enter a name.';
    if ([...password].length < 12) fields.password = 'Use at least 12 characters.';
    else if (password !== confirm) fields.confirm = 'The passwords are not the same.';
    if (useTotp && !/^\d{6}$/.test(code.trim())) fields.totp_code = 'Enter the 6-digit code from your authenticator app.';
    if (Object.keys(fields).length) return;
    busy = true;
    try {
      await api.post('/setup', { token, name: name.trim(), password, totp_secret: useTotp ? totp?.secret ?? null : null, totp_code: useTotp ? code.trim() : null });
      session.info = await loadSession();
      await goto('/welcome');
    } catch (failure) {
      const result = fieldErrors(failure);
      error = result.form ?? '';
      fields = result.fields;
    } finally {
      busy = false;
    }
  }
</script>

<svelte:head><title>Set up · fiend</title></svelte:head>

<Centered wide>
  {#if valid === false}
    <h1>This setup link does not work</h1>
    <p class="lead">The link works one time and for one hour. On the server, run <code>sudo /opt/fiend/bin/fiend-panel renew-setup /etc/fiend/panel.toml</code> and restart the panel with <code>sudo systemctl restart fiend-panel</code> to get a new link.</p>
  {:else if valid}
    <p class="step">Step 1 of 4</p>
    <h1>Admin account</h1>
    <p class="lead">This account manages all nodes and users. The panel is open only through your SSH tunnel.</p>
    <form novalidate onsubmit={submit} class="stack">
      {#if error}<p class="note error" role="alert">{error}</p>{/if}
      <Field label="Name" id="name" error={fields.name}><input id="name" autocomplete="username" bind:value={name} /></Field>
      <div class="grid-2">
        <Field label="Password" id="password" error={fields.password} help="12 characters or more."><input id="password" type="password" autocomplete="new-password" bind:value={password} /></Field>
        <Field label="Repeat password" id="confirm" error={fields.confirm}><input id="confirm" type="password" autocomplete="new-password" bind:value={confirm} /></Field>
      </div>
      <Switch id="use-totp" bind:checked={useTotp} label="Use two-factor sign-in (recommended)" />
      {#if useTotp && totp}
        <div class="totp">
          <Qr svg={totp.qr_svg} label="QR code for the authenticator app" />
          <div class="stack grow">
            <p>Scan the code with an authenticator app, or enter the key by hand.</p>
            <CopyField value={totp.secret} label="authenticator key" />
            <Field label="Code from the app" id="totp" error={fields.totp_code}><input id="totp" inputmode="numeric" autocomplete="one-time-code" maxlength="6" bind:value={code} /></Field>
          </div>
        </div>
      {:else}
        <p class="note warn">Without two-factor sign-in you cannot add or remove other admins.</p>
      {/if}
      <div class="row end"><Button kind="primary" type="submit" {busy}>Create account and continue</Button></div>
    </form>
  {:else}
    <p class="muted">Checking the setup link…</p>
  {/if}
</Centered>

<style>
  .step { font-size: 11px; color: var(--muted); margin-bottom: 4px; }
  .lead { margin: 6px 0 18px; line-height: 1.6; }
  .totp { display: flex; gap: 18px; align-items: flex-start; }
  .grow { flex: 1; min-width: 0; }
  .end { justify-content: flex-end; }
  @media (max-width: 719px) { .totp { flex-direction: column; } }
</style>
