<script lang="ts">
  import { onMount } from 'svelte';
  import TopBar from '#lib/components/TopBar.svelte';
  import Button from '#lib/components/Button.svelte';
  import Field from '#lib/components/Field.svelte';
  import Tabs from '#lib/components/Tabs.svelte';
  import Dialog from '#lib/components/Dialog.svelte';
  import Qr from '#lib/components/Qr.svelte';
  import CopyField from '#lib/components/CopyField.svelte';
  import Brand from '#lib/components/Brand.svelte';
  import Icon from '#lib/components/Icon.svelte';
  import EmptyState from '#lib/components/EmptyState.svelte';
  import { ApiError, api, base64Text, fieldErrors, loadSession } from '#lib/api/client.ts';
  import { session } from '#lib/stores/session.svelte.ts';
  import { setTheme, theme, type Theme } from '#lib/stores/theme.svelte.ts';
  import { toast } from '#lib/stores/toasts.svelte.ts';
  import { date, initials } from '#lib/format.ts';
  import type { AdminView, AuditView, TotpSetup } from '#lib/api/generated/index.ts';

  let admins = $state<AdminView[]>([]);
  let audit = $state<AuditView[]>([]);
  let auditQuery = $state('');
  let themeValue = $state<string>(theme.value);
  let error = $state('');

  let passwordOpen = $state(false);
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
  let restorePassword = $state('');
  let restoreCode = $state('');
  let restoreFields = $state<Record<string, string>>({});
  let restoreError = $state('');
  let restoreBusy = $state(false);
  let restarting = $state(false);
  let restartError = $state('');

  const me = $derived(session.info?.admin);
  const version = $derived(session.info?.version ?? '');
  const shownAudit = $derived.by(() => {
    const q = auditQuery.trim().toLowerCase();
    if (!q) return audit;
    return audit.filter((row) =>
      (row.admin ?? 'panel').toLowerCase().includes(q) ||
      row.action.toLowerCase().includes(q) ||
      row.target.toLowerCase().includes(q) ||
      row.detail.toLowerCase().includes(q)
    );
  });

  async function load() {
    try {
      [admins, audit] = await Promise.all([api.get<AdminView[]>('/admins'), api.get<AuditView[]>('/audit?limit=100')]);
      session.info = await loadSession();
      error = '';
    } catch (failure) {
      error = fieldErrors(failure).form ?? '';
    }
  }

  onMount(load);

  function openPassword() {
    password = { current: '', next: '', confirm: '', code: '' };
    passwordFields = {};
    passwordOpen = true;
  }

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
      passwordOpen = false;
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

  function openRestore(file: File | null) {
    restoreFile = file;
    restorePassword = '';
    restoreCode = '';
    restoreFields = {};
    restoreError = '';
    if (file) restoreOpen = true;
  }

  async function panelInstance(): Promise<string> {
    const response = await fetch('/health', { cache: 'no-store', signal: AbortSignal.timeout(3000) }).catch(() => null);
    const instance = response?.headers.get('x-panel-instance');
    if (!response?.ok || !instance) throw new ApiError(0, 'Could not check whether the panel is running.');
    return instance;
  }

  async function waitForRestart(previous: string) {
    const deadline = Date.now() + 45_000;
    while (Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 1000));
      try {
        if (await panelInstance() !== previous) {
          location.replace('/login');
          return;
        }
      } catch {
        continue;
      }
    }
    restarting = false;
    restartError = 'The panel has not come back online. Check the panel service before trying another restore.';
  }

  async function restore(event: SubmitEvent) {
    event.preventDefault();
    if (restoreBusy || restarting) return;
    restoreFields = {};
    restoreError = '';
    if (!restorePassword) restoreFields.current = 'Enter the current password.';
    if (me?.totp && !restoreCode.trim()) restoreFields.totp_code = 'Enter the code from your app.';
    if (!restoreFile || Object.keys(restoreFields).length) return;
    restartError = '';
    restoreBusy = true;
    try {
      const headers: Record<string, string> = { 'x-current-password': base64Text(restorePassword) };
      if (me?.totp) headers['x-current-code'] = restoreCode.trim();
      const bytes = await restoreFile.arrayBuffer();
      const instance = await panelInstance();
      await api.upload('/restore', bytes, headers);
      restoreOpen = false;
      restoreFile = null;
      restorePassword = '';
      restoreCode = '';
      restarting = true;
      await waitForRestart(instance);
    } catch (failure) {
      const result = fieldErrors(failure);
      restoreError = result.form ?? '';
      restoreFields = result.fields;
    } finally {
      restorePassword = '';
      restoreBusy = false;
    }
  }
</script>

<svelte:head><title>Settings · fiend</title></svelte:head>

<TopBar title="Settings" subtitle="Account security, administrators, backups, and appearance." />
{#if error}<p class="note error banner" role="alert">{error}</p>{/if}

<div class="columns">
  <div class="col">
    <section aria-labelledby="account-title">
      <div class="group-head"><h2 id="account-title" class="group-title">Your account</h2></div>
      <div class="group">
        {#if me}
          <div class="group-row">
            <div class="setting-lead">
              <span class="avatar accent" aria-hidden="true">{initials(me.name)}</span>
              <div class="setting-copy">
                <div class="setting-title">{me.name} <span class="badge">Administrator</span></div>
                <p class="setting-desc">Signed in to this panel session.</p>
              </div>
            </div>
          </div>
        {/if}

        <div class="group-row">
          <div class="setting-lead">
            <span class="icon-tile {me?.totp ? 'green' : 'warn'}"><Icon name="key" size={16} /></span>
            <div class="setting-copy">
              <div class="setting-title">
                <span>Two-factor sign-in</span>
                {#if me?.totp}<span class="badge green">On</span>{:else}<span class="badge warn">Off</span>{/if}
              </div>
              <p class="setting-desc">Require a six-digit authenticator code when signing in or managing admins.</p>
            </div>
          </div>
          <div class="setting-control">
            {#if me?.totp}
              <Button size="compact" kind="ghost" onclick={() => { disableCode = ''; disableOpen = true; }}>Turn off</Button>
            {:else}
              <Button size="compact" kind="primary" onclick={startTotp}>Turn on</Button>
            {/if}
          </div>
        </div>

        <div class="group-row">
          <div class="setting-lead">
            <span class="icon-tile"><Icon name="shield" size={16} /></span>
            <div class="setting-copy">
              <div class="setting-title">Password</div>
              <p class="setting-desc">Update the password used to sign in as {me?.name ?? 'this admin'}.</p>
            </div>
          </div>
          <div class="setting-control">
            <Button size="compact" onclick={openPassword}>Change password</Button>
          </div>
        </div>
      </div>
    </section>

    <section aria-labelledby="admins-title">
      <div class="group-head">
        <h2 id="admins-title" class="group-title">
          Admins
          <span class="badge">{admins.length}</span>
        </h2>
        <Button size="compact" icon="plus" disabled={!me?.totp} onclick={() => { admin = { name: '', password: '', code: '' }; adminFields = {}; adminError = ''; adminOpen = true; }}>Add admin</Button>
      </div>
      {#if !me?.totp}<p class="note warn admins-note">Turn on two-factor sign-in to add or remove admins.</p>{/if}
      <div class="table-wrap">
        <table>
          <thead><tr><th>Name</th><th>Two-factor</th><th>Added</th><th><span class="visually-hidden">Actions</span></th></tr></thead>
          <tbody>
            {#each admins as item (item.id)}
              <tr>
                <td>
                  <span class="name-cell">
                    <span class="avatar" class:accent={item.id === me?.id} aria-hidden="true">{initials(item.name)}</span>
                    <span class="admin-name">{item.name}</span>
                    {#if item.id === me?.id}<span class="badge">you</span>{/if}
                  </span>
                </td>
                <td><span class="badge {item.totp ? 'green' : ''}">{item.totp ? 'On' : 'Off'}</span></td>
                <td class="muted">{date(item.created_at)}</td>
                <td class="actions">
                  {#if item.id !== me?.id}
                    <Button size="compact" kind="ghost" disabled={!me?.totp} onclick={() => { removingAdmin = item; removeCode = ''; removeAdminOpen = true; }}>Remove</Button>
                  {/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    </section>

    <section aria-labelledby="backup-title">
      <div class="group-head"><h2 id="backup-title" class="group-title">Backup &amp; restore</h2></div>
      {#if restarting}<p class="note admins-note" role="status">Backup accepted. Waiting for the panel to restart.</p>{/if}
      {#if restartError}<p class="note error admins-note" role="alert">{restartError}</p>{/if}
      <div class="group">
        <div class="group-row">
          <div class="setting-lead">
            <span class="icon-tile"><Icon name="data" size={16} /></span>
            <div class="setting-copy">
              <div class="setting-title">Download backup</div>
              <p class="setting-desc">Export the SQLite panel database. Contains all user and node secrets.</p>
            </div>
          </div>
          <div class="setting-control">
            {#if me?.totp}
              <input class="inline-code" id="backup-code" aria-label="Two-factor code for backup" placeholder="2FA code" inputmode="numeric" autocomplete="one-time-code" maxlength="6" bind:value={backupCode} />
            {/if}
            <Button size="compact" icon="down" busy={backupBusy} onclick={backup}>Download</Button>
          </div>
        </div>

        <div class="group-row">
          <div class="setting-lead">
            <span class="icon-tile"><Icon name="file" size={16} /></span>
            <div class="setting-copy">
              <div class="setting-title">Restore from file</div>
              <p class="setting-desc">Replace all panel data from a saved <span class="mono">.db</span> backup and restart.</p>
            </div>
          </div>
          <div class="setting-control">
            <label class="upload" class:disabled={restoreBusy || restarting}>
              <Icon name="up" size={14} />
              <span>Restore file</span>
              <input class="visually-hidden" type="file" disabled={restoreBusy || restarting} accept=".db,application/vnd.sqlite3,application/octet-stream" onchange={(event) => { const input = event.currentTarget as HTMLInputElement; const file = input.files?.[0] ?? null; input.value = ''; openRestore(file); }} />
            </label>
          </div>
        </div>
      </div>
    </section>
  </div>

  <div class="col">
    <section aria-labelledby="look-title">
      <div class="group-head"><h2 id="look-title" class="group-title">Appearance</h2></div>
      <div class="group">
        <div class="group-row theme-row">
          <div class="setting-copy">
            <div class="setting-title">Theme</div>
            <p class="setting-desc">Follow your operating system or pick a fixed appearance.</p>
          </div>
          <Tabs label="Theme" bind:value={themeValue} onchange={(value) => setTheme(value as Theme)} tabs={[{ value: 'system', label: 'System' }, { value: 'light', label: 'Light' }, { value: 'dark', label: 'Dark' }]} />
        </div>
      </div>
    </section>

    <section aria-labelledby="about-title">
      <div class="group-head"><h2 id="about-title" class="group-title">About</h2></div>
      <div class="group">
        <div class="group-row about-hero">
          <Brand />
          {#if version}<span class="badge mono">v{version}</span>{/if}
        </div>
        <a class="group-row interactive" href="https://github.com/rfc9849/Fiend" target="_blank" rel="noopener noreferrer">
          <div class="setting-copy">
            <div class="setting-title">GitHub</div>
            <p class="setting-desc mono">rfc9849/Fiend</p>
          </div>
          <span class="open-pill">Open <Icon name="arrow" size={14} /></span>
        </a>
        <a class="group-row interactive" href="https://github.com/rfc9849/Fiend/blob/master/LICENSE" target="_blank" rel="noopener noreferrer">
          <div class="setting-copy">
            <div class="setting-title">License</div>
            <p class="setting-desc">PolyForm Noncommercial 1.0.0</p>
          </div>
          <span class="open-pill">Open <Icon name="arrow" size={14} /></span>
        </a>
      </div>
    </section>
  </div>
</div>

<section class="audit-section" aria-labelledby="audit-title">
  <div class="group-head">
    <h2 id="audit-title" class="group-title">
      Audit log
      <span class="badge">{shownAudit.length}</span>
    </h2>
    <div class="search-input">
      <input type="search" aria-label="Filter audit log" placeholder="Filter by admin, action, target…" bind:value={auditQuery} />
      {#if auditQuery}
        <button type="button" class="clear" aria-label="Clear filter" onclick={() => (auditQuery = '')}><Icon name="close" size={14} /></button>
      {/if}
    </div>
  </div>
  {#if shownAudit.length}
    <div class="table-wrap audit">
      <table>
        <thead><tr><th>Time</th><th>By</th><th>Action</th><th>Target</th><th>Detail</th></tr></thead>
        <tbody>
          {#each shownAudit as row (row.id)}
            <tr>
              <td class="muted">{date(row.ts)}</td>
              <td>{row.admin ?? 'panel'}</td>
              <td><span class="badge">{row.action.charAt(0).toUpperCase() + row.action.slice(1)}</span></td>
              <td class="mono">{row.target}</td>
              <td class="detail muted" title={row.detail}>{row.detail || '—'}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {:else}
    <EmptyState icon="log" title={audit.length ? 'No matching events' : 'No audit events'} message={audit.length ? 'Try another search query.' : 'Administrative changes appear here.'} compact />
  {/if}
</section>

<Dialog bind:open={passwordOpen} title="Change password">
  <form id="password-form" novalidate class="stack" onsubmit={changePassword}>
    <Field label="Current password" id="current" error={passwordFields.current}><input id="current" type="password" autocomplete="current-password" bind:value={password.current} /></Field>
    <div class="grid-2">
      <Field label="New password" id="new-password" error={passwordFields.password} help="At least 12 characters"><input id="new-password" type="password" autocomplete="new-password" bind:value={password.next} /></Field>
      <Field label="Repeat new password" id="confirm-password" error={passwordFields.confirm}><input id="confirm-password" type="password" autocomplete="new-password" bind:value={password.confirm} /></Field>
    </div>
    {#if me?.totp}<Field label="Two-factor code" id="password-code" error={passwordFields.totp_code}><input id="password-code" inputmode="numeric" autocomplete="one-time-code" bind:value={password.code} /></Field>{/if}
  </form>
  {#snippet actions()}
    <Button kind="ghost" onclick={() => (passwordOpen = false)}>Cancel</Button>
    <Button kind="primary" type="submit" form="password-form" busy={passwordBusy}>Change password</Button>
  {/snippet}
</Dialog>

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

<Dialog bind:open={restoreOpen} title="Restore from {restoreFile?.name ?? ''}">
  <form id="restore-form" novalidate class="stack" onsubmit={restore}>
    <p>This replaces all panel data and signs out all administrators. The panel keeps a copy of the previous database.</p>
    {#if restoreError}<p class="note error" role="alert">{restoreError}</p>{/if}
    <Field label="Current password" id="restore-password" error={restoreFields.current}><input id="restore-password" type="password" autocomplete="current-password" bind:value={restorePassword} /></Field>
    {#if me?.totp}<Field label="Two-factor code" id="restore-code" error={restoreFields.totp_code}><input id="restore-code" inputmode="numeric" autocomplete="one-time-code" bind:value={restoreCode} /></Field>{/if}
  </form>
  {#snippet actions()}<Button kind="ghost" onclick={() => (restoreOpen = false)}>Cancel</Button><Button kind="danger" type="submit" form="restore-form" busy={restoreBusy}>Restore</Button>{/snippet}
</Dialog>

<style>
  .banner { margin-bottom: 16px; }
  .columns { display: grid; grid-template-columns: minmax(0, 1.5fr) minmax(0, 1fr); gap: 22px; align-items: start; }
  .col { display: flex; flex-direction: column; gap: 22px; min-width: 0; }
  .admins-note { margin-bottom: 10px; }
  .admin-name { font-weight: 500; }
  .actions { text-align: right; width: 1%; }
  .inline-code { width: 92px; height: 28px; padding: 0 8px; border: 1px solid var(--line); border-radius: 5px; background: var(--panel); color: var(--text); font-family: var(--mono); font-size: 12px; }
  .inline-code:focus-visible { outline: 2px solid var(--accent); outline-offset: 0; border-color: transparent; }
  .upload { display: inline-flex; align-items: center; gap: 6px; min-height: 28px; padding: 4px 10px; border: 1px solid var(--line); border-radius: 5px; background: var(--panel); cursor: pointer; font-size: 11.5px; font-weight: 500; color: var(--text); transition: background .12s, border-color .12s; }
  .upload:hover:not(.disabled) { background: var(--hover); border-color: var(--dial-edge); }
  .upload.disabled { opacity: .45; cursor: not-allowed; }
  .upload:focus-within { outline: 2px solid var(--accent); outline-offset: 2px; }
  .theme-row { flex-direction: column; align-items: flex-start; gap: 12px; padding: 14px 16px; }
  .about-hero { padding: 16px; --brand-eye: var(--panel); }
  .open-pill { display: inline-flex; align-items: center; gap: 4px; font-size: 12px; color: var(--muted); }
  .audit-section { margin-top: 26px; }
  .audit { max-height: 440px; overflow: auto; }
  .detail { max-width: 380px; overflow: hidden; text-overflow: ellipsis; }
  .totp { display: flex; gap: 16px; align-items: flex-start; }
  .grow { flex: 1; min-width: 0; }
  @media (max-width: 1023px) { .columns { grid-template-columns: minmax(0, 1fr); } }
  @media (max-width: 719px) { .totp { flex-direction: column; } }
</style>
