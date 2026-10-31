<script lang="ts">
  import Button from './Button.svelte';
  import IconButton from './IconButton.svelte';
  import CopyField from './CopyField.svelte';
  import Qr from './Qr.svelte';
  import Confirm from './Confirm.svelte';
  import UserForm, { toDraft, toInput, type UserDraft } from './UserForm.svelte';
  import { api, fieldErrors } from '#lib/api/client.ts';
  import { toast } from '#lib/stores/toasts.svelte.ts';
  import { bytes, date, percent, stateLabel } from '#lib/format.ts';
  import type { NodeSessions, NodeView, UserDetail, UserLink, UserSecret, UserView } from '#lib/api/generated/index.ts';

  let { id, nodes, onclose, onchange }: { id: number; nodes: NodeView[]; onclose: () => void; onchange: () => void } = $props();
  let detail = $state<UserDetail | null>(null);
  let link = $state<UserLink | null>(null);
  let linkError = $state('');
  let sessions = $state<NodeSessions[] | null>(null);
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

  async function run() {
    if (confirm === 'secret') {
      await api.post<UserSecret>(`/users/${id}/secret`);
      secretKey += 1;
      toast('New secret made');
    } else if (confirm === 'reset') {
      await api.post(`/users/${id}/reset`);
      toast('Usage reset');
    } else if (confirm === 'kill') {
      await api.del(`/users/${id}/sessions`);
      toast('Sessions closed');
      sessions = null;
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
    kill: ['Close sessions', '', 'Close sessions'],
    delete: ['Remove user', '', 'Remove']
  } as const;
</script>

<div class="card pane">
  {#if detail}
    {@const user = detail.user}
    <div class="head">
      <div><h2>{user.name}</h2><p class="state"><span class="dot {user.state}" aria-hidden="true"></span>{stateLabel(user.state)}</p></div>
      <IconButton icon="close" label="Close details" onclick={onclose} />
    </div>
    {#if editing}
      <form id="edit-user" novalidate onsubmit={save} class="stack">
        {#if formError}<p class="note error" role="alert">{formError}</p>{/if}
        <UserForm bind:draft {nodes} {fields} />
        <div class="row"><Button kind="primary" type="submit" {busy}>Save</Button><Button kind="ghost" onclick={() => (editing = false)}>Cancel</Button></div>
      </form>
    {:else}
      <section class="stack" aria-label="Link">
        {#if link}
          <Qr svg={link.qr_svg} label="QR code with the link of {user.name}" />
          <CopyField value={link.link} label="link of {user.name}" />
          <p class="small muted">{link.servers.length} server{link.servers.length === 1 ? '' : 's'}: <span class="mono">{link.servers.join(', ')}</span></p>
          {#each link.warnings as warning}<p class="note warn">{warning}</p>{/each}
        {:else if linkError}
          <p class="note warn">{linkError}</p>
        {/if}
        {#key secretKey}<div><p class="label">Secret</p><CopyField value="" secret label="secret of {user.name}" onreveal={async () => (await api.get<UserSecret>(`/users/${id}/secret`)).secret} /></div>{/key}
      </section>
      <dl class="facts">
        <dt>Used</dt><dd>{bytes(user.used)}{#if user.traffic_limit} of {bytes(user.traffic_limit)} ({percent(user.used, user.traffic_limit)} %){/if}</dd>
        <dt>Reset</dt><dd>{user.reset_period === 'none' ? 'Never' : `${user.reset_period}, next ${date(user.reset_at)}`}</dd>
        <dt>Expiry</dt><dd>{user.hold_days && !user.expire_at ? `${user.hold_days} days after the first connection` : user.expire_at ? date(user.expire_at) : 'Never'}</dd>
        <dt>Nodes</dt><dd>{user.nodes.length ? user.nodes.map((node) => nodes.find((item) => item.id === node)?.name ?? node).join(', ') : 'All nodes'}</dd>
        <dt>Online</dt><dd>{user.online} session{user.online === 1 ? '' : 's'}</dd>
        {#if user.note}<dt>Note</dt><dd>{user.note}</dd>{/if}
      </dl>
      <section aria-label="Usage per node in the last 30 days">
        <p class="label">Last 30 days per node</p>
        <ul class="usage">
          {#each detail.usage as row (row.node_id)}<li><span>{row.node_name}</span><span class="num">↑ {bytes(row.up)} · ↓ {bytes(row.down)}</span></li>{/each}
        </ul>
      </section>
      <section aria-label="Open sessions">
        <div class="row between"><p class="label">Sessions</p><Button kind="ghost" onclick={async () => (sessions = await api.get<NodeSessions[]>(`/users/${id}/sessions`))}>Show</Button></div>
        {#if sessions}
          <ul class="usage">
            {#each sessions as row (row.node_id)}<li><span>{row.node_name}</span><span>{row.error ? 'not available' : `${row.started_at.length} open`}</span></li>{/each}
          </ul>
        {/if}
      </section>
      <div class="buttons">
        <Button icon="edit" onclick={() => { draft = toDraft(user); fields = {}; editing = true; }}>Edit</Button>
        <Button onclick={toggle}>{user.enabled ? 'Disable' : 'Enable'}</Button>
        <Button onclick={() => { confirm = 'reset'; confirmOpen = true; }}>Reset usage</Button>
        <Button onclick={() => { confirm = 'secret'; confirmOpen = true; }}>New secret</Button>
        <Button onclick={() => { confirm = 'kill'; confirmOpen = true; }}>Close sessions</Button>
        <Button kind="danger" onclick={() => { confirm = 'delete'; confirmOpen = true; }}>Remove</Button>
      </div>
    {/if}
  {:else}
    <p class="muted">Loading…</p>
  {/if}
</div>

{#if confirm}
  <Confirm bind:open={confirmOpen} title={texts[confirm][0]} message={texts[confirm][1]} action={texts[confirm][2]} danger={confirm === 'delete' || confirm === 'secret'} onconfirm={run} />
{/if}

<style>
  .pane { display: flex; flex-direction: column; gap: 18px; }
  .head { display: flex; justify-content: space-between; align-items: flex-start; gap: 10px; }
  .state { display: flex; align-items: center; gap: 6px; margin-top: 2px; }
  .label { font-weight: 500; font-size: 12px; color: var(--text); margin-bottom: 6px; }
  .facts { display: grid; grid-template-columns: max-content minmax(0, 1fr); gap: 4px 14px; margin: 0; }
  dt { color: var(--muted); }
  dd { margin: 0; overflow-wrap: anywhere; }
  .usage { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
  .usage li { display: flex; justify-content: space-between; gap: 10px; }
  .between { justify-content: space-between; }
  .buttons { display: flex; flex-wrap: wrap; gap: 8px; }
</style>
