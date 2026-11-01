<script lang="ts">
  import { onMount } from 'svelte';
  import TopBar from '#lib/components/TopBar.svelte';
  import Button from '#lib/components/Button.svelte';
  import Field from '#lib/components/Field.svelte';
  import Select from '#lib/components/Select.svelte';
  import Dialog from '#lib/components/Dialog.svelte';
  import Confirm from '#lib/components/Confirm.svelte';
  import Qr from '#lib/components/Qr.svelte';
  import CopyField from '#lib/components/CopyField.svelte';
  import { api, fieldErrors, loadSession } from '#lib/api/client.ts';
  import { session } from '#lib/stores/session.svelte.ts';
  import { setTheme, theme, type Theme } from '#lib/stores/theme.svelte.ts';
  import { toast } from '#lib/stores/toasts.svelte.ts';
  import { date } from '#lib/format.ts';
  import type { AdminView, AuditView, TotpSetup } from '#lib/api/generated/index.ts';

  let admins = $state<AdminView[]>([]);
  let audit = $state<AuditView[]>([]);
  let themeValue = $state<string>(theme.value);
  let error = $state('');

  let password = $state({ current: '', next: '', confirm: '', code: '' });
  let passwordFields = $state<Record<string, string>>({});
  let passwordBusy = $state(false);

  let totp = $state<TotpSetup | null>(null);
  let totpCode = $state('');
  let totpOpen = $state(false);
  let totpFields = $state<Record<string, string>>({});
  let disableOpen = $state(false);
  let disableCode = $state('');

  let adminOpen = $state(false);
  let admin = $state({ name: '', password: '', code: '' });
  let adminFields = $state<Record<string, string>>({});
  let adminError = $state('');
  let removingAdmin = $state<AdminView | null>(null);
  let removeAdminOpen = $state(false);
  let removeCode = $state('');

  let backupCode = $state('');
  let backupBusy = $state(false);
  let restoreOpen = $state(false);
  let restoreFile = $state<File | null>(null);

  const me = $derived(session.info?.admin);

  async function load() {
    try {
      [admins, audit] = await Promise.all([api.get<AdminView[]>('/admins'), api.get<AuditView[]>('/audit?limit=100')]);
      session.info = await loadSession();
    } catch (failure) {
      error = fieldErrors(failure).form ?? '';
    }
  }

  onMount(load);

  async function changePassword(event: SubmitEvent) {
    event.preventDefault();
    passwordFields = {};
    if (!password.current) passwordFields.current = 'Enter the current password.';
    if ([...password.next].length < 12) passwordFields.password = 'Use at least 12 characters.';
    else if (password.next !== password.confirm) passwordFields.confirm = 'The passwords are not the same.';
    if (me?.totp && !password.code) passwordFields.totp_code = 'Enter the code from your app.';
    if (Object.keys(passwordFields).length) return;
    passwordBusy = true;
    try {
      await api.post('/account/password', { current: password.current, password: password.next, totp_code: password.code || null });
      password = { current: '', next: '', confirm: '', code: '' };
      toast('Password changed');
      await load();
    } catch (failure) {
      const result = fieldErrors(failure);
      passwordFields = result.form ? { current: result.form } : result.fields;
    } finally {
      passwordBusy = false;
    }
  }

  async function startTotp() {
    totp = await api.get<TotpSetup>('/account/totp');
    totpCode = '';
    totpFields = {};
    totpOpen = true;
  }

  async function enableTotp(event: SubmitEvent) {
    event.preventDefault();
    try {
      await api.post('/account/totp', { secret: totp?.secret, code: totpCode.trim() });
      totpOpen = false;
      toast('Two-factor sign-in is on');
      await load();
    } catch (failure) {
      const result = fieldErrors(failure);
      totpFields = result.form ? { totp_code: result.form } : result.fields;
    }
  }

  async function addAdmin(event: SubmitEvent) {
    event.preventDefault();
    adminFields = {};
    adminError = '';
    if (!admin.name.trim()) adminFields.name = 'Enter a name.';
    if ([...admin.password].length < 12) adminFields.password = 'Use at least 12 characters.';
    if (!admin.code.trim()) adminFields.current_code = 'Enter the code from your app.';
    if (Object.keys(adminFields).length) return;
    try {
      await api.post('/admins', { name: admin.name.trim(), password: admin.password, current_code: admin.code.trim() });
      adminOpen = false;
      toast('Admin added');
      await load();
    } catch (failure) {
      const result = fieldErrors(failure);
      adminError = result.form ?? '';
      adminFields = result.fields;
    }
  }

  async function backup() {
    backupBusy = true;
    try {
      const blob = await api.post<Blob>('/backup', { code: backupCode });
      const url = URL.createObjectURL(blob);
      const link = document.createElement('a');
      link.href = url;
      link.download = `fiend-panel-${new Date().toISOString().slice(0, 10)}.db`;
      link.click();
      URL.revokeObjectURL(url);
      backupCode = '';
      toast('Backup downloaded');
    } catch (failure) {
      toast(fieldErrors(failure).form ?? 'The backup failed', true);
    } finally {
      backupBusy = false;
    }
  }
</script>

<svelte:head><title>Settings · fiend</title></svelte:head>

<TopBar title="Settings" />
{#if error}<p class="note error" role="alert">{error}</p>{/if}

<div class="columns">
  <section class="card" aria-labelledby="account-title">
    <h2 id="account-title">Your account</h2>
    <div class="row status">
      <span>Two-factor sign-in:</span>
      {#if me?.totp}<span class="badge green">On</span><Button kind="ghost" onclick={() => { disableCode = ''; disableOpen = true; }}>Turn off</Button>
      {:else}<span class="badge warn">Off</span><Button onclick={startTotp}>Turn on</Button>{/if}
    </div>
    <form novalidate class="stack" onsubmit={changePassword}>
      <Field label="Current password" id="current" error={passwordFields.current}><input id="current" type="password" autocomplete="current-password" bind:value={password.current} /></Field>
      <div class="grid-2">
        <Field label="New password" id="new-password" error={passwordFields.password}><input id="new-password" type="password" autocomplete="new-password" bind:value={password.next} /></Field>
        <Field label="Repeat new password" id="confirm-password" error={passwordFields.confirm}><input id="confirm-password" type="password" autocomplete="new-password" bind:value={password.confirm} /></Field>
      </div>
      {#if me?.totp}<Field label="Two-factor code" id="password-code" error={passwordFields.totp_code}><input id="password-code" inputmode="numeric" autocomplete="one-time-code" bind:value={password.code} /></Field>{/if}
      <div><Button type="submit" busy={passwordBusy}>Change password</Button></div>
    </form>
  </section>

  <section class="card" aria-labelledby="look-title">
    <h2 id="look-title">Appearance</h2>
    <div class="theme"><Field label="Theme" id="theme"><Select id="theme" bind:value={themeValue} label="Theme" onchange={(value) => setTheme(value as Theme)} options={[{ value: 'system', label: 'Same as the system' }, { value: 'light', label: 'Light' }, { value: 'dark', label: 'Dark' }]} /></Field></div>
  </section>
</div>

<section class="card" aria-labelledby="admins-title">
  <div class="card-header"><h2 id="admins-title">Admins</h2><Button icon="plus" disabled={!me?.totp} onclick={() => { admin = { name: '', password: '', code: '' }; adminFields = {}; adminError = ''; adminOpen = true; }}>Add admin</Button></div>
  {#if !me?.totp}<p class="note warn">Needs two-factor sign-in.</p>{/if}
  <div class="table-wrap">
    <table>
      <thead><tr><th>Name</th><th>Two-factor</th><th>Added</th><th><span class="visually-hidden">Actions</span></th></tr></thead>
      <tbody>
        {#each admins as item (item.id)}
          <tr><td>{item.name}{#if item.id === me?.id} <span class="badge">you</span>{/if}</td><td>{item.totp ? 'On' : 'Off'}</td><td>{date(item.created_at)}</td>
            <td class="actions">{#if item.id !== me?.id}<Button kind="ghost" disabled={!me?.totp} onclick={() => { removingAdmin = item; removeCode = ''; removeAdminOpen = true; }}>Remove</Button>{/if}</td></tr>
        {/each}
      </tbody>
    </table>
  </div>
</section>

<section class="card" aria-labelledby="backup-title">
  <h2 id="backup-title">Backup</h2>
  <p class="lead">Contains all secrets.</p>
  <div class="row">
    {#if me?.totp}<div class="code"><Field label="Two-factor code" id="backup-code"><input id="backup-code" inputmode="numeric" autocomplete="one-time-code" bind:value={backupCode} /></Field></div>{/if}
    <Button icon="down" busy={backupBusy} onclick={backup}>Download backup</Button>
    <label class="upload">
      <input class="visually-hidden" type="file" accept=".db,application/vnd.sqlite3,application/octet-stream" onchange={(event) => { restoreFile = (event.currentTarget as HTMLInputElement).files?.[0] ?? null; (event.currentTarget as HTMLInputElement).value = ''; if (restoreFile) restoreOpen = true; }} />
      Restore from file
    </label>
  </div>
</section>

<section class="card" aria-labelledby="audit-title">
  <h2 id="audit-title" class="spaced">Audit log</h2>
  <div class="table-wrap audit">
    <table>
      <thead><tr><th>Time</th><th>By</th><th>Action</th><th>Target</th><th>Detail</th></tr></thead>
      <tbody>
        {#each audit as row (row.id)}<tr><td>{date(row.ts)}</td><td>{row.admin ?? 'panel'}</td><td>{row.action.charAt(0).toUpperCase() + row.action.slice(1)}</td><td>{row.target}</td><td class="detail" title={row.detail}>{row.detail}</td></tr>{/each}
      </tbody>
    </table>
  </div>
</section>

<Dialog bind:open={totpOpen} title="Turn on two-factor sign-in">
  {#if totp}
    <form id="totp-form" novalidate class="stack" onsubmit={enableTotp}>
      <div class="totp"><Qr svg={totp.qr_svg} label="QR code for the authenticator app" /><div class="stack grow"><p>Scan the code or enter the key.</p><CopyField value={totp.secret} label="authenticator key" /></div></div>
      <Field label="Code from the app" id="totp-code" error={totpFields.totp_code}><input id="totp-code" inputmode="numeric" autocomplete="one-time-code" maxlength="6" bind:value={totpCode} /></Field>
    </form>
  {/if}
  {#snippet actions()}<Button kind="ghost" onclick={() => (totpOpen = false)}>Cancel</Button><Button kind="primary" type="submit" form="totp-form">Turn on</Button>{/snippet}
</Dialog>

<Dialog bind:open={disableOpen} title="Turn off two-factor sign-in">
  <div class="stack"><Field label="Two-factor code" id="disable-code"><input id="disable-code" inputmode="numeric" bind:value={disableCode} /></Field></div>
  {#snippet actions()}<Button kind="ghost" onclick={() => (disableOpen = false)}>Cancel</Button><Button kind="danger" onclick={async () => { try { await api.del('/account/totp', { code: disableCode }); disableOpen = false; toast('Two-factor sign-in is off'); await load(); } catch (failure) { toast(fieldErrors(failure).form ?? Object.values(fieldErrors(failure).fields)[0] ?? 'Failed', true); } }}>Turn off</Button>{/snippet}
</Dialog>

<Dialog bind:open={adminOpen} title="Add admin">
  <form id="admin-form" novalidate class="stack" onsubmit={addAdmin}>
    {#if adminError}<p class="note error" role="alert">{adminError}</p>{/if}
    <Field label="Name" id="admin-name" error={adminFields.name}><input id="admin-name" autocomplete="off" bind:value={admin.name} /></Field>
    <Field label="Password" id="admin-password" error={adminFields.password}><input id="admin-password" type="password" autocomplete="new-password" bind:value={admin.password} /></Field>
    <Field label="Your two-factor code" id="admin-code" error={adminFields.current_code}><input id="admin-code" inputmode="numeric" autocomplete="one-time-code" bind:value={admin.code} /></Field>
  </form>
  {#snippet actions()}<Button kind="ghost" onclick={() => (adminOpen = false)}>Cancel</Button><Button kind="primary" type="submit" form="admin-form">Add admin</Button>{/snippet}
</Dialog>

<Dialog bind:open={removeAdminOpen} title="Remove {removingAdmin?.name ?? ''}">
  <div class="stack"><Field label="Your two-factor code" id="remove-code"><input id="remove-code" inputmode="numeric" bind:value={removeCode} /></Field></div>
  {#snippet actions()}<Button kind="ghost" onclick={() => (removeAdminOpen = false)}>Cancel</Button><Button kind="danger" onclick={async () => { try { await api.del(`/admins/${removingAdmin?.id}?code=${encodeURIComponent(removeCode)}`); removeAdminOpen = false; toast('Admin removed'); await load(); } catch (failure) { toast(fieldErrors(failure).form ?? Object.values(fieldErrors(failure).fields)[0] ?? 'Failed', true); } }}>Remove</Button>{/snippet}
</Dialog>

<Confirm bind:open={restoreOpen} title="Restore from {restoreFile?.name ?? ''}" message="All panel data is replaced. The panel restarts." action="Restore" onconfirm={async () => { if (restoreFile) await api.upload('/restore', await restoreFile.arrayBuffer()); toast('Restarting'); setTimeout(() => location.assign('/login'), 4000); }} />

<style>
  .columns { display: grid; grid-template-columns: minmax(0, 2fr) minmax(0, 1fr); gap: 16px; margin-bottom: 16px; align-items: start; }
  .columns :global(.card + .card) { margin-top: 0; }
  .status { margin: 12px 0 16px; }
  .theme { max-width: 220px; margin: 12px 0; }
  .lead { margin: 6px 0 12px; }
  .spaced { margin-bottom: 14px; }
  .upload { display: inline-flex; align-items: center; min-height: 32px; padding: 6px 12px; border: 1px solid var(--line); border-radius: 6px; background: var(--panel); cursor: pointer; font-size: 12px; font-weight: 500; }
  .upload:hover { background: var(--hover); }
  .upload:focus-within { outline: 2px solid var(--accent); outline-offset: 2px; }
  .code { width: 160px; }
  .actions { text-align: right; width: 1%; }
  .audit { max-height: 420px; overflow: auto; }
  .detail { max-width: 360px; overflow: hidden; text-overflow: ellipsis; }
  .totp { display: flex; gap: 16px; align-items: flex-start; }
  .grow { flex: 1; min-width: 0; }
  @media (max-width: 1023px) { .columns { grid-template-columns: minmax(0, 1fr); } }
  @media (max-width: 719px) { .totp { flex-direction: column; } }
</style>
