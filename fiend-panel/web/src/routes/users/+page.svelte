<script lang="ts">
  import { onMount } from 'svelte';
  import TopBar from '#lib/components/TopBar.svelte';
  import Button from '#lib/components/Button.svelte';
  import Tabs from '#lib/components/Tabs.svelte';
  import Dialog from '#lib/components/Dialog.svelte';
  import UserForm, { toDraft, toInput, type UserDraft } from '#lib/components/UserForm.svelte';
  import UserDetail from '#lib/components/UserDetail.svelte';
  import { api, fieldErrors } from '#lib/api/client.ts';
  import { onEvent } from '#lib/stores/events.ts';
  import { toast } from '#lib/stores/toasts.svelte.ts';
  import { ago, bytes, day, percent, stateLabel } from '#lib/format.ts';
  import type { NodeView, UserView } from '#lib/api/generated/index.ts';

  let users = $state<UserView[]>([]);
  let nodes = $state<NodeView[]>([]);
  let filter = $state('all');
  let selected = $state<number | null>(null);
  let adding = $state(false);
  let draft = $state<UserDraft>(toDraft(null));
  let fields = $state<Record<string, string>>({});
  let formError = $state('');
  let busy = $state(false);
  let error = $state('');

  const near = (user: UserView) => !!user.traffic_limit && user.used >= user.traffic_limit * 0.8;
  const filters: Record<string, (user: UserView) => boolean> = {
    all: () => true,
    active: (user) => user.state === 'active' || user.state === 'on_hold',
    disabled: (user) => user.state === 'disabled',
    expired: (user) => user.state === 'expired',
    near: near
  };
  const shown = $derived(users.filter(filters[filter]));
  const nodeName = (id: number) => nodes.find((node) => node.id === id)?.name ?? `#${id}`;

  async function load() {
    try {
      [users, nodes] = await Promise.all([api.get<UserView[]>('/users'), api.get<NodeView[]>('/nodes')]);
      error = '';
    } catch (failure) {
      error = fieldErrors(failure).form ?? '';
    }
  }

  onMount(() => {
    load();
    return onEvent((kind) => {
      if (kind === 'users' || kind === 'stats') load();
    });
  });

  async function create(event: SubmitEvent) {
    event.preventDefault();
    const result = toInput(draft);
    fields = result.fields;
    formError = '';
    if (!result.body) return;
    busy = true;
    try {
      const user = await api.post<UserView>('/users', result.body);
      adding = false;
      toast(`User ${user.name} added`);
      await load();
      selected = user.id;
    } catch (failure) {
      const outcome = fieldErrors(failure);
      formError = outcome.form ?? '';
      fields = outcome.fields;
    } finally {
      busy = false;
    }
  }
</script>

<svelte:head><title>Users · fiend</title></svelte:head>

<TopBar title="Users" subtitle="Each user has an own secret and link.">
  {#snippet actions()}<Button kind="primary" icon="plus" onclick={() => { draft = toDraft(null); fields = {}; formError = ''; adding = true; }}>Add user</Button>{/snippet}
</TopBar>
{#if error}<p class="note error" role="alert">{error}</p>{/if}
<div class="filters">
  <Tabs label="Filter users" bind:value={filter} tabs={[
    { value: 'all', label: 'All', count: users.length },
    { value: 'active', label: 'Active', count: users.filter(filters.active).length },
    { value: 'disabled', label: 'Disabled', count: users.filter(filters.disabled).length },
    { value: 'expired', label: 'Expired', count: users.filter(filters.expired).length },
    { value: 'near', label: 'Near limit', count: users.filter(near).length }
  ]} />
</div>
<div class="split" class:open={selected !== null}>
  <div class="list">
    {#if shown.length}
      <div class="table-wrap">
        <table>
          <thead><tr><th>Name</th><th class="num">Used / limit</th><th>Expiry</th><th>Last seen</th><th>Nodes</th></tr></thead>
          <tbody>
            {#each shown as user (user.id)}
              <tr class:selected={selected === user.id}>
                <td>
                  <button type="button" class="pick" aria-pressed={selected === user.id} onclick={() => (selected = user.id)}>
                    <span class="dot {user.state}" aria-hidden="true"></span><span>{user.name}</span><span class="visually-hidden">, {stateLabel(user.state)}</span>
                    {#if user.online}<span class="badge green">{user.online} online</span>{/if}
                  </button>
                </td>
                <td class="num">{bytes(user.used)} / {user.traffic_limit ? bytes(user.traffic_limit) : '∞'}{#if user.traffic_limit}<span class="pct" class:warn={near(user)}> {percent(user.used, user.traffic_limit)} %</span>{/if}</td>
                <td>{user.hold_days && !user.expire_at ? `${user.hold_days} d after first connect` : day(user.expire_at)}</td>
                <td>{ago(user.last_seen)}</td>
                <td>{user.nodes.length ? user.nodes.map(nodeName).join(', ') : 'All'}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {:else}
      <p class="empty">{users.length ? 'No users match this filter.' : 'No users yet.'}</p>
    {/if}
  </div>
  {#if selected !== null}
    <aside class="detail" aria-label="User details">
      {#key selected}<UserDetail id={selected} {nodes} onclose={() => (selected = null)} onchange={load} />{/key}
    </aside>
  {/if}
</div>

<Dialog bind:open={adding} title="Add user">
  <form id="add-user" novalidate onsubmit={create}>
    {#if formError}<p class="note error" role="alert">{formError}</p>{/if}
    <UserForm bind:draft {nodes} {fields} />
  </form>
  {#snippet actions()}
    <Button kind="ghost" onclick={() => (adding = false)}>Cancel</Button>
    <Button kind="primary" type="submit" form="add-user" {busy}>Add user</Button>
  {/snippet}
</Dialog>

<style>
  .filters { margin-bottom: 14px; }
  .split { display: grid; grid-template-columns: minmax(0, 1fr); gap: 16px; align-items: start; }
  .split.open { grid-template-columns: minmax(0, 1fr) 380px; }
  .list { min-width: 0; }
  .pick { display: inline-flex; align-items: center; gap: 8px; color: var(--text); text-align: left; }
  .pick:hover span:nth-child(2) { text-decoration: underline; }
  .pct { color: var(--muted); }
  .pct.warn { color: var(--warn); }
  .empty { padding: 18px; border: 1px dashed var(--line); border-radius: 9px; text-align: center; }
  .detail { position: sticky; top: 16px; min-width: 0; }
  @media (max-width: 1100px) {
    .split.open { grid-template-columns: minmax(0, 1fr); }
    .detail { position: static; }
  }
</style>
