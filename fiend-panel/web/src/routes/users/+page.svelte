<script lang="ts">
  import { onMount } from 'svelte';
  import TopBar from '#lib/components/TopBar.svelte';
  import Button from '#lib/components/Button.svelte';
  import Tabs from '#lib/components/Tabs.svelte';
  import Dialog from '#lib/components/Dialog.svelte';
  import EmptyState from '#lib/components/EmptyState.svelte';
  import Icon from '#lib/components/Icon.svelte';
  import UserForm, { toDraft, toInput, type UserDraft } from '#lib/components/UserForm.svelte';
  import UserDetail from '#lib/components/UserDetail.svelte';
  import { api, fieldErrors } from '#lib/api/client.ts';
  import { onEvent } from '#lib/stores/events.ts';
  import { toast } from '#lib/stores/toasts.svelte.ts';
  import { ago, bytes, day, initials, percent, stateLabel } from '#lib/format.ts';
  import type { NodeView, UserView } from '#lib/api/generated/index.ts';

  let users = $state<UserView[]>([]);
  let nodes = $state<NodeView[]>([]);
  let loading = $state(true);
  let filter = $state('all');
  let query = $state('');
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
  const onlineCount = $derived(users.filter((user) => user.online > 0).length);
  const shown = $derived.by(() => {
    const q = query.trim().toLowerCase();
    return users.filter((user) => {
      if (!filters[filter](user)) return false;
      if (!q) return true;
      return user.name.toLowerCase().includes(q) || user.note.toLowerCase().includes(q);
    });
  });
  const nodeName = (id: number) => nodes.find((node) => node.id === id)?.name ?? `#${id}`;
  const stateBadge = (state: string) => (state === 'disabled' ? '' : state === 'expired' || state === 'limited' ? 'danger' : state === 'on_hold' ? 'warn' : '');

  function openAdd() {
    draft = toDraft(null);
    fields = {};
    formError = '';
    adding = true;
  }

  async function load() {
    try {
      [users, nodes] = await Promise.all([api.get<UserView[]>('/users'), api.get<NodeView[]>('/nodes')]);
      error = '';
    } catch (failure) {
      error = fieldErrors(failure).form ?? '';
    } finally {
      loading = false;
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

<TopBar title="Users" subtitle={loading ? '' : `${users.length} user${users.length === 1 ? '' : 's'} · ${onlineCount} online`}>
  {#snippet actions()}
    <Button kind="primary" icon="plus" onclick={openAdd}>Add user</Button>
  {/snippet}
</TopBar>

{#if error}<p class="note error banner" role="alert">{error}</p>{/if}

<div class="toolbar">
  <Tabs label="Filter users" bind:value={filter} tabs={[
    { value: 'all', label: 'All', count: users.length },
    { value: 'active', label: 'Active', count: users.filter(filters.active).length },
    { value: 'disabled', label: 'Disabled', count: users.filter(filters.disabled).length },
    { value: 'expired', label: 'Expired', count: users.filter(filters.expired).length },
    { value: 'near', label: 'Near limit', count: users.filter(near).length }
  ]} />
  <div class="search-input">
    <input type="search" aria-label="Search users" placeholder="Filter by name or note…" bind:value={query} />
    {#if query}
      <button type="button" class="clear" aria-label="Clear search" onclick={() => (query = '')}><Icon name="close" size={14} /></button>
    {/if}
  </div>
</div>

<div class="split" class:open={selected !== null}>
  <div class="list">
    {#if loading}
      <p class="muted">Loading…</p>
    {:else if shown.length}
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>User</th>
              <th class="num">Traffic</th>
              <th>Expiry</th>
              <th>Last seen</th>
              {#if nodes.length > 1 && selected === null}<th>Nodes</th>{/if}
            </tr>
          </thead>
          <tbody>
            {#each shown as user (user.id)}
              {@const pct = user.traffic_limit ? percent(user.used, user.traffic_limit) : 0}
              <tr class:selected={selected === user.id}>
                <td>
                  <button type="button" class="pick" aria-pressed={selected === user.id} onclick={() => (selected = selected === user.id ? null : user.id)}>
                    <span class="avatar" class:accent={selected === user.id} aria-hidden="true">{initials(user.name)}</span>
                    <span class="user-meta">
                      <span class="user-line">
                        <span class="dot {user.state}" aria-hidden="true"></span>
                        <span class="user-name">{user.name}</span>
                        <span class="visually-hidden">, {stateLabel(user.state)}</span>
                        {#if user.online}
                          <span class="badge green" title="{user.online} open session{user.online === 1 ? '' : 's'}">online</span>
                        {:else if user.state !== 'active'}
                          <span class="badge {stateBadge(user.state)}">{stateLabel(user.state)}</span>
                        {/if}
                      </span>
                      {#if user.note}<span class="user-note">{user.note}</span>{/if}
                    </span>
                  </button>
                </td>
                <td class="num traffic-cell">
                  <div class="traffic-stack">
                    <span>{bytes(user.used)} <span class="muted">/ {user.traffic_limit ? bytes(user.traffic_limit) : '∞'}</span></span>
                    {#if user.traffic_limit}
                      <progress class="meter mini" class:warn={pct >= 80 && pct < 100} class:danger={pct >= 100} value={pct} max="100" aria-label="{pct}% of limit"></progress>
                    {/if}
                  </div>
                </td>
                <td>{user.hold_days && !user.expire_at ? `${user.hold_days} d after connect` : user.expire_at ? day(user.expire_at) : 'Never'}</td>
                <td class="muted">{ago(user.last_seen)}</td>
                {#if nodes.length > 1 && selected === null}
                  <td class="muted">{user.nodes.length ? user.nodes.map(nodeName).join(', ') : 'All'}</td>
                {/if}
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {:else if users.length}
      <EmptyState icon="users" title="No matching users" message="No users match the active filter or search query." compact>
        {#snippet actions()}
          <Button size="compact" onclick={() => { filter = 'all'; query = ''; }}>Reset filters</Button>
        {/snippet}
      </EmptyState>
    {:else}
      <EmptyState icon="users" title="No users yet" message="Add a user to generate a Fiend profile link and QR code.">
        {#snippet actions()}
          <Button kind="primary" icon="plus" onclick={openAdd}>Add user</Button>
        {/snippet}
      </EmptyState>
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
    {#if formError}<p class="note error banner" role="alert">{formError}</p>{/if}
    <UserForm bind:draft {nodes} {fields} />
  </form>
  {#snippet actions()}
    <Button kind="ghost" onclick={() => (adding = false)}>Cancel</Button>
    <Button kind="primary" type="submit" form="add-user" {busy}>Add user</Button>
  {/snippet}
</Dialog>

<style>
  .banner { margin-bottom: 14px; }
  .toolbar { display: flex; align-items: center; justify-content: space-between; gap: 12px; flex-wrap: wrap; margin-bottom: 14px; }
  .split { display: grid; grid-template-columns: minmax(0, 1fr); gap: 18px; align-items: start; }
  .split.open { grid-template-columns: minmax(0, 1fr) 410px; }
  .list { min-width: 0; }
  tbody tr { position: relative; cursor: pointer; }
  tbody td { height: 52px; }
  .pick { display: inline-flex; align-items: center; gap: 10px; color: var(--text); text-align: left; max-width: 100%; }
  .pick::after { content: ''; position: absolute; inset: 0; }
  .user-meta { display: flex; flex-direction: column; min-width: 0; gap: 1px; }
  .user-line { display: inline-flex; align-items: center; gap: 7px; }
  .user-name { font-weight: 500; }
  .pick:hover .user-name { text-decoration: underline; }
  .user-note { font-size: 11px; color: var(--muted); max-width: 220px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .traffic-cell { min-width: 148px; }
  .traffic-stack { display: inline-flex; flex-direction: column; align-items: flex-end; gap: 4px; width: 100%; }
  .meter.mini { width: 104px; height: 3px; }
  .detail { position: sticky; top: 18px; min-width: 0; }
  @media (max-width: 1100px) {
    .split.open { grid-template-columns: minmax(0, 1fr); }
    .detail { position: static; }
  }
</style>
