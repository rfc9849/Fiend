<script lang="ts">
  import Button from './Button.svelte';
  import IconButton from './IconButton.svelte';
  import CopyField from './CopyField.svelte';
  import Qr from './Qr.svelte';
  import Confirm from './Confirm.svelte';
  import UserForm, { toDraft, toInput, type UserDraft } from './UserForm.svelte';
  import { api, fieldErrors } from '#lib/api/client.ts';
  import { toast } from '#lib/stores/toasts.svelte.ts';
  import { bytes, date, initials, percent, stateLabel } from '#lib/format.ts';
  import type { NodeSessionClose, NodeSessions, NodeView, UserDetail, UserLink, UserSecret, UserView } from '#lib/api/generated/index.ts';

  let { id, nodes, onclose, onchange }: { id: number; nodes: NodeView[]; onclose: () => void; onchange: () => void } = $props();
  let detail = $state<UserDetail | null>(null);
  let link = $state<UserLink | null>(null);
  let linkError = $state('');
  let sessions = $state<NodeSessions[] | null>(null);
  let checkingSessions = $state(false);
  let closed = $state<NodeSessionClose[] | null>(null);
  let editing = $state(false);
  let draft = $state<UserDraft>(toDraft(null));
  let fields = $state<Record<string, string>>({});
  let formError = $state('');
  let busy = $state(false);
  let confirm = $state<'secret' | 'reset' | 'delete' | 'kill' | null>(null);
  let confirmOpen = $state(false);
  let secretKey = $state(0);

  async function load() {
    detail = await api.get<UserDetail>(`/users/${id}`);
    try {
      link = await api.get<UserLink>(`/users/${id}/link`);
      linkError = '';
    } catch (failure) {
      link = null;
      linkError = fieldErrors(failure).form ?? '';
    }
  }

  $effect(() => {
    load().catch((failure) => (linkError = fieldErrors(failure).form ?? ''));
  });

  async function save(event: SubmitEvent) {
    event.preventDefault();
    const result = toInput(draft);
    fields = result.fields;
    formError = '';
    if (!result.body) return;
    busy = true;
    try {
      await api.put<UserView>(`/users/${id}`, result.body);
      editing = false;
      toast('User saved');
      await load();
      onchange();
    } catch (failure) {
      const outcome = fieldErrors(failure);
      formError = outcome.form ?? '';
      fields = outcome.fields;
    } finally {
      busy = false;
    }
  }

  async function toggle() {
    if (!detail) return;
    const user = detail.user;
    const result = toInput({ ...toDraft(user), enabled: !user.enabled });
    if (!result.body) return;
    await api.put(`/users/${id}`, result.body);
    toast(user.enabled ? 'User disabled' : 'User enabled');
    await load();
    onchange();
  }

  async function loadSessions() {
    checkingSessions = true;
    try {
      sessions = await api.get<NodeSessions[]>(`/users/${id}/sessions`);
    } finally {
      checkingSessions = false;
    }
  }

  async function run() {
    if (confirm === 'secret') {
      await api.post<UserSecret>(`/users/${id}/secret`);
      secretKey += 1;
      toast('New secret made');
    } else if (confirm === 'reset') {
      await api.post(`/users/${id}/reset`);
      toast('Usage reset');
    } else if (confirm === 'kill') {
      closed = await api.del<NodeSessionClose[]>(`/users/${id}/sessions`);
      sessions = null;
      onchange();
      return;
    } else if (confirm === 'delete') {
      await api.del(`/users/${id}`);
      toast('User removed');
      onchange();
      onclose();
      return;
    }
    await load();
    onchange();
  }

  const texts = {
    secret: ['New secret', 'The old link stops working.', 'Make new secret'],
    reset: ['Reset usage', '', 'Reset'],
    kill: ['Close sessions', 'This disconnects the user. They can reconnect if their access is enabled.', 'Close sessions'],
    delete: ['Remove user', '', 'Remove']
  } as const;
</script>

<div class="pane">
  {#if detail}
    {@const user = detail.user}
    {@const pct = user.traffic_limit ? percent(user.used, user.traffic_limit) : 0}
    <section class="group identity">
      <div class="group-row identity-top">
        <div class="setting-lead">
          <span class="avatar lg accent" aria-hidden="true">{initials(user.name)}</span>
          <div class="setting-copy">
            <h2 class="user-title">{user.name}</h2>
            <p class="state">
              <span class="dot {user.state}" aria-hidden="true"></span>
              <span>{stateLabel(user.state)}</span>
              {#if user.online}<span class="badge green">{user.online} online</span>{/if}
            </p>
          </div>
        </div>
        <IconButton icon="close" label="Close details" onclick={onclose} />
      </div>

      {#if !editing}
        <div class="group-footer quick-bar">
          <Button size="compact" icon="edit" onclick={() => { draft = toDraft(user); fields = {}; editing = true; }}>Edit</Button>
          <Button size="compact" onclick={toggle}>{user.enabled ? 'Disable' : 'Enable'}</Button>
          <Button size="compact" icon="power" onclick={() => { confirm = 'kill'; confirmOpen = true; }}>Disconnect</Button>
        </div>
      {/if}
    </section>

    {#if editing}
      <form id="edit-user" novalidate onsubmit={save} class="group">
        <div class="group-body stack">
          {#if formError}<p class="note error" role="alert">{formError}</p>{/if}
          <UserForm bind:draft {nodes} {fields} />
        </div>
        <div class="group-footer end">
          <Button size="compact" kind="ghost" onclick={() => (editing = false)}>Cancel</Button>
          <Button size="compact" kind="primary" type="submit" {busy}>Save changes</Button>
        </div>
      </form>
    {:else}
      <section aria-label="Access">
        <div class="group-head"><h3 class="group-title">Access link</h3></div>
        <div class="group">
          {#if link}
            <div class="qr-stage">
              <Qr svg={link.qr_svg} label="QR code with the link of {user.name}" />
            </div>
            <div class="group-body stack compact-stack">
              <CopyField value={link.link} label="link of {user.name}" />
              <p class="small muted">{link.servers.length} server{link.servers.length === 1 ? '' : 's'}: <span class="mono">{link.servers.join(', ')}</span></p>
              {#each link.warnings as warning}<p class="note warn">{warning}</p>{/each}
            </div>
          {:else if linkError}
            <div class="group-body"><p class="note warn">{linkError}</p></div>
          {/if}
          <div class="group-row secret-row">
            <div class="secret-wrap">
              <span class="info-label">Secret</span>
              {#key secretKey}
                <CopyField value="" secret label="secret of {user.name}" onreveal={async () => (await api.get<UserSecret>(`/users/${id}/secret`)).secret} />
              {/key}
            </div>
          </div>
        </div>
      </section>

      <section aria-label="Account details">
        <div class="group-head"><h3 class="group-title">Details</h3></div>
        <div class="group">
          <div class="group-row">
            <span class="info-label">Used</span>
            <span class="info-value">
              {bytes(user.used)}{#if user.traffic_limit} <span class="muted">of {bytes(user.traffic_limit)} ({pct} %)</span>{/if}
            </span>
          </div>
          {#if user.traffic_limit}
            <div class="meter-row">
              <progress class="meter" class:warn={pct >= 80 && pct < 100} class:danger={pct >= 100} value={pct} max="100" aria-label="Usage {pct}%"></progress>
            </div>
          {/if}
          <div class="group-row">
            <span class="info-label">Reset</span>
            <span class="info-value">{user.reset_period === 'none' ? 'Never' : `${user.reset_period}, next ${date(user.reset_at)}`}</span>
          </div>
          <div class="group-row">
            <span class="info-label">Expiry</span>
            <span class="info-value">{user.hold_days && !user.expire_at ? `${user.hold_days} d after connect` : user.expire_at ? date(user.expire_at) : 'Never'}</span>
          </div>
          <div class="group-row">
            <span class="info-label">Nodes</span>
            <span class="info-value">{user.nodes.length ? user.nodes.map((node) => nodes.find((item) => item.id === node)?.name ?? node).join(', ') : 'All nodes'}</span>
          </div>
          <div class="group-row">
            <span class="info-label">Sessions</span>
            <span class="info-value">
              <span>{user.online} open</span>
              <Button size="compact" kind="ghost" busy={checkingSessions} onclick={loadSessions}>Inspect</Button>
            </span>
          </div>
          {#if user.note}
            <div class="group-row">
              <span class="info-label">Note</span>
              <span class="info-value">{user.note}</span>
            </div>
          {/if}
        </div>
      </section>

      {#if sessions || closed}
        <section aria-label="Session status">
          <div class="group-head"><h3 class="group-title">{closed ? 'Disconnect result' : 'Open sessions'}</h3></div>
          <div class="group">
            {#if closed}
              {#if closed.length}
                {#each closed as row (row.node_id)}
                  <div class="group-row">
                    <span class="info-label">{row.node_name}</span>
                    <span class="info-value" class:close-failed={row.closed === null}>
                      {row.closed === null ? `Not confirmed: ${row.error}` : `${row.closed} signaled`}
                    </span>
                  </div>
                {/each}
              {:else}
                <div class="group-row"><p class="small muted">No nodes to close sessions on.</p></div>
              {/if}
            {/if}
            {#if sessions}
              {#each sessions as row (row.node_id)}
                <div class="group-row">
                  <span class="info-label">{row.node_name}</span>
                  <span class="info-value">{row.error ? 'Not available' : `${row.started_at.length} open`}</span>
                </div>
              {/each}
            {/if}
          </div>
        </section>
      {/if}

      {#if detail.usage.length}
        <section aria-label="Usage per node in the last 30 days">
          <div class="group-head"><h3 class="group-title">Last 30 days</h3></div>
          <div class="group">
            {#each detail.usage as row (row.node_id)}
              <div class="group-row">
                <span class="info-label">{row.node_name}</span>
                <span class="info-value mono">↑ {bytes(row.up)} · ↓ {bytes(row.down)}</span>
              </div>
            {/each}
          </div>
        </section>
      {/if}

      <section aria-label="Maintenance">
        <div class="group-head"><h3 class="group-title">Actions</h3></div>
        <div class="group">
          <div class="group-row">
            <div class="setting-copy">
              <div class="setting-title">Reset usage</div>
              <p class="setting-desc">Clear recorded traffic for this user.</p>
            </div>
            <Button size="compact" onclick={() => { confirm = 'reset'; confirmOpen = true; }}>Reset</Button>
          </div>
          <div class="group-row">
            <div class="setting-copy">
              <div class="setting-title">Rotate secret</div>
              <p class="setting-desc">Generate a new key; the old link stops working.</p>
            </div>
            <Button size="compact" onclick={() => { confirm = 'secret'; confirmOpen = true; }}>New secret</Button>
          </div>
          <div class="group-row">
            <div class="setting-copy">
              <div class="setting-title">Remove user</div>
              <p class="setting-desc">Revoke access and delete this user.</p>
            </div>
            <Button size="compact" kind="danger" onclick={() => { confirm = 'delete'; confirmOpen = true; }}>Remove</Button>
          </div>
        </div>
      </section>
    {/if}
  {:else}
    <div class="card"><p class="muted">Loading…</p></div>
  {/if}
</div>

{#if confirm}
  <Confirm bind:open={confirmOpen} title={texts[confirm][0]} message={texts[confirm][1]} action={texts[confirm][2]} danger={confirm === 'delete' || confirm === 'secret'} onconfirm={run} />
{/if}

<style>
  .pane { display: flex; flex-direction: column; gap: 16px; }
  .identity-top { padding: 14px 16px; }
  .user-title { font-size: 17px; font-weight: 600; letter-spacing: -.3px; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .state { display: flex; align-items: center; gap: 6px; margin-top: 3px; font-size: 12px; }
  .quick-bar { justify-content: flex-start; gap: 6px; }
  .qr-stage { display: flex; justify-content: center; padding: 16px; background: var(--soft); border-bottom: 1px solid var(--line); }
  .qr-stage :global(.qr) { width: 176px; padding: 6px; }
  .compact-stack { gap: 10px; }
  .secret-row { padding: 12px 16px; }
  .secret-wrap { display: flex; flex-direction: column; gap: 6px; width: 100%; min-width: 0; }
  .meter-row { padding: 0 16px 12px; }
  .close-failed { color: var(--danger); }
</style>
