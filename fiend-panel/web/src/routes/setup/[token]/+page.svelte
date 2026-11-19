<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { signedInPath } from '#lib/navigation.ts';
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
      await goto(session.info ? signedInPath(session.info, page.url, '/') : '/login', { replaceState: true });
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

<Centered wide step={valid ? 1 : 0} steps={valid ? 4 : 0}>
  {#if valid === false}
    <h1>Setup link expired</h1>
    <p class="lead">Generate a fresh setup link on the server:</p>
    <pre class="cmd">docker compose exec panel fiend-panel renew-setup /etc/fiend/panel.toml && docker compose restart panel</pre>
  {:else if valid}
    <div class="title-block">
      <h1>Admin account</h1>
      <p>Create the first administrator account for this panel.</p>
    </div>
    <form novalidate onsubmit={submit} class="stack">
      {#if error}<p class="note error" role="alert">{error}</p>{/if}
      <Field label="Name" id="name" error={fields.name}><input id="name" autocomplete="username" bind:value={name} /></Field>
      <div class="grid-2">
        <Field label="Password" id="password" error={fields.password} help="At least 12 characters"><input id="password" type="password" autocomplete="new-password" bind:value={password} /></Field>
        <Field label="Repeat password" id="confirm" error={fields.confirm}><input id="confirm" type="password" autocomplete="new-password" bind:value={confirm} /></Field>
      </div>
      <div class="group">
        <div class="group-row">
          <div class="setting-copy">
            <div class="setting-title">Two-factor sign-in</div>
            <p class="setting-desc">Recommended. Required to add or remove other admins.</p>
          </div>
          <Switch id="use-totp" bind:checked={useTotp} label="" />
        </div>
        {#if useTotp && totp}
          <div class="group-body totp-box">
            <Qr svg={totp.qr_svg} label="QR code for the authenticator app" />
            <div class="stack grow">
              <p class="small muted">Scan the QR code or enter this key in your authenticator app.</p>
              <CopyField value={totp.secret} label="authenticator key" />
              <Field label="Code from the app" id="totp" error={fields.totp_code}><input id="totp" inputmode="numeric" autocomplete="one-time-code" maxlength="6" placeholder="000000" bind:value={code} /></Field>
            </div>
          </div>
        {/if}
      </div>
      {#if !useTotp}
        <p class="note warn">Two-factor sign-in is needed later to add other admins.</p>
      {/if}
      <div class="row end"><Button kind="primary" type="submit" {busy}>Continue</Button></div>
    </form>
  {:else}
    <p class="muted">Checking…</p>
  {/if}
</Centered>

<style>
  .title-block { margin-bottom: 18px; }
  .title-block p { margin-top: 4px; font-size: 12.5px; }
  .lead { margin: 8px 0 12px; }
  .cmd { margin: 0; padding: 12px 14px; border-radius: 6px; background: var(--soft); border: 1px solid var(--line); font: 12px/1.55 var(--mono); white-space: pre-wrap; overflow-wrap: anywhere; color: var(--text); }
  .totp-box { display: flex; gap: 18px; align-items: flex-start; border-top: 1px solid var(--line); background: var(--soft); }
  .totp-box :global(.qr) { width: 170px; }
  .grow { flex: 1; min-width: 0; }
  .end { justify-content: flex-end; margin-top: 4px; }
  @media (max-width: 719px) { .totp-box { flex-direction: column; } }
</style>
