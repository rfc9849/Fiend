<script lang="ts">
  import { onMount } from 'svelte';
  import TopBar from './TopBar.svelte';
  import Button from './Button.svelte';
  import IconButton from './IconButton.svelte';
  import Icon from './Icon.svelte';
  import Dialog from './Dialog.svelte';
  import Confirm from './Confirm.svelte';
  import EmptyState from './EmptyState.svelte';
  import Field from './Field.svelte';
  import Select from './Select.svelte';
  import Switch from './Switch.svelte';
  import WarpCard from './WarpCard.svelte';
  import { api, fieldErrors } from '#lib/api/client.ts';
  import { toast } from '#lib/stores/toasts.svelte.ts';
  import type { NodeView, OutboundView, Routing, RuleView } from '#lib/api/generated/index.ts';

  let routing = $state<Routing | null>(null);
  let nodes = $state<NodeView[]>([]);
  let loading = $state(true);
  let error = $state('');
  let outboundOpen = $state(false);
  let ruleOpen = $state(false);
  let removing = $state<{ kind: 'outbound' | 'rule'; id: number; name: string } | null>(null);
  let removeOpen = $state(false);
  let editing = $state<number | null>(null);
  let scope = $state<number | null>(null);
  let warpRevision = $state(0);
  let form = $state({ name: '', kind: 'socks5', server: '', username: '', password: '', udp: false, link: '', fallback: '' });
  let rule = $state({ pattern: '', outbound: 'direct', udp: '' });
  let fields = $state<Record<string, string>>({});
  let formError = $state('');
  let busy = $state(false);

  const kinds = [
    { value: 'socks5', label: 'SOCKS5', detail: 'TCP, UDP optional' },
    { value: 'http', label: 'HTTP CONNECT', detail: 'TCP only' },
    { value: 'fiend', label: 'Another fiend node', detail: 'TCP and UDP' },
    { value: 'warp', label: 'Cloudflare WARP', detail: 'TCP only' },
    { value: 'block', label: 'Block' },
    { value: 'direct', label: 'Direct' }
  ];
  const builtIn = [
    { value: 'direct', label: 'Direct' },
    { value: '@block', label: 'Block' },
    { value: '@warp', label: 'Cloudflare WARP' }
  ];
  const available = $derived((routing?.outbounds ?? []).filter((item) => item.node_id === null || item.node_id === scope));
  const outboundOptions = $derived([...builtIn, ...available.filter((item, i, list) =>
    list.findLastIndex((other) => other.name === item.name) === i
  ).map((item) => ({ value: item.name, label: item.name, detail: item.kind }))]);
  const udpOptions = $derived([
    { value: '', label: 'Block UDP' },
    { value: 'direct', label: 'Direct' },
    ...available.filter((item, i, list) => item.udp && list.findLastIndex((other) => other.name === item.name) === i)
      .map((item) => ({ value: item.name, label: item.name }))
  ]);
  const actionLabel = (value: string) => builtIn.find((item) => item.value === value)?.label ?? value;
  const actionTone = (value: string) => (value === '@block' ? 'danger' : value === '@warp' ? 'accent' : '');
  const actionIcon = (value: string) => (value === '@block' ? 'block' : value === '@warp' ? 'shield' : value === 'direct' ? 'route' : 'swap');
  const localNodes = $derived(nodes.filter((node) => node.local));

  async function load() {
    try {
      const [global, list] = await Promise.all([api.get<Routing>('/routing'), api.get<NodeView[]>('/nodes')]);
      const local = list.find((node) => node.local);
      const scoped = local ? await api.get<Routing>(`/routing?node=${local.id}`) : null;
      nodes = list;
      routing = {
        rules: [...global.rules, ...(scoped?.rules ?? [])],
        outbounds: [...global.outbounds, ...(scoped?.outbounds ?? [])]
      };
      error = '';
      warpRevision++;
    } catch (failure) {
      error = fieldErrors(failure).form ?? '';
    } finally {
      loading = false;
    }
  }

  onMount(load);

  function editOutbound(item: OutboundView | null) {
    editing = item?.id ?? null;
    scope = item?.node_id ?? null;
    fields = {};
    formError = '';
    form = item
      ? { name: item.name, kind: item.kind, server: item.server ?? '', username: item.username ?? '', password: '', udp: item.udp, link: item.link_node_id ? String(item.link_node_id) : '', fallback: item.udp_fallback ?? '' }
      : { name: '', kind: 'socks5', server: '', username: '', password: '', udp: false, link: '', fallback: '' };
    outboundOpen = true;
  }

  function editRule(item: RuleView | null) {
    editing = item?.id ?? null;
    scope = item?.node_id ?? null;
    fields = {};
    formError = '';
    rule = item ? { pattern: item.pattern, outbound: item.outbound, udp: item.udp_fallback ?? '' } : { pattern: '', outbound: 'direct', udp: '' };
    ruleOpen = true;
  }

  async function saveOutbound(event: SubmitEvent) {
    event.preventDefault();
    fields = {};
    formError = '';
    if (!form.name.trim()) fields.name = 'Enter a name.';
    if ((form.kind === 'socks5' || form.kind === 'http') && !form.server.trim()) fields.server = 'Enter host:port.';
    if (form.kind === 'fiend' && !form.link) fields.link_node_id = 'Choose a node.';
    if (Object.keys(fields).length) return;
    busy = true;
    const body = {
      node_id: scope,
      name: form.name.trim(),
      kind: form.kind,
      server: form.server.trim() || null,
      username: form.username.trim() || null,
      password: form.password || null,
      udp: form.udp,
      link_node_id: form.link ? Number(form.link) : null,
      udp_fallback: form.fallback || null
    };
    try {
      if (editing === null) await api.post('/outbounds', body);
      else await api.put(`/outbounds/${editing}`, body);
      outboundOpen = false;
      toast('Outbound saved');
      await load();
    } catch (failure) {
      const result = fieldErrors(failure);
      formError = result.form ?? '';
      fields = result.fields;
    } finally {
      busy = false;
    }
  }

  async function saveRule(event: SubmitEvent) {
    event.preventDefault();
    fields = {};
    formError = '';
    if (!rule.pattern.trim()) {
      fields.pattern = 'Enter at least one pattern.';
      return;
    }
    busy = true;
    const body = { node_id: scope, pattern: rule.pattern, outbound: rule.outbound, udp_fallback: rule.outbound === '@warp' ? rule.udp || null : null };
    try {
      if (editing === null) await api.post('/rules', body);
      else await api.put(`/rules/${editing}`, body);
      ruleOpen = false;
      toast('Rule saved');
      await load();
    } catch (failure) {
      const result = fieldErrors(failure);
      formError = result.form ?? '';
      fields = result.fields;
    } finally {
      busy = false;
    }
  }

  async function move(index: number, delta: number) {
    const item = routing?.rules[index];
    const neighbor = routing?.rules[index + delta];
    if (!item || !neighbor || item.node_id !== neighbor.node_id) return;
    const ids = routing!.rules.filter((rule) => rule.node_id === item.node_id).map((rule) => rule.id);
    const from = ids.indexOf(item.id);
    [ids[from], ids[from + delta]] = [ids[from + delta], ids[from]];
    try {
      await api.post('/rules/order', { ids });
      await load();
    } catch (failure) {
      toast(fieldErrors(failure).form ?? 'The order did not change', true);
    }
  }

  async function remove() {
    if (!removing) return;
    await api.del(removing.kind === 'outbound' ? `/outbounds/${removing.id}` : `/rules/${removing.id}`);
    toast(removing.kind === 'outbound' ? 'Outbound removed' : 'Rule removed');
    await load();
  }

  function patterns(pattern: string): { shown: string[]; more: number } {
    const lines = pattern.split('\n').map((line) => line.trim()).filter(Boolean);
    return { shown: lines.slice(0, 3), more: Math.max(0, lines.length - 3) };
  }
</script>

<TopBar title="Routing" subtitle="The first matching rule sets the route. Unmatched traffic goes direct.">
  {#snippet actions()}
    <Button icon="plus" onclick={() => editOutbound(null)}>Add outbound</Button>
    <Button kind="primary" icon="plus" onclick={() => editRule(null)}>Add rule</Button>
  {/snippet}
</TopBar>

{#if error}<p class="note error banner" role="alert">{error}</p>{/if}

{#if loading}
  <p class="muted">Loading…</p>
{:else}
  <div class="layout" class:has-side={localNodes.length > 0}>
    <div class="main">
      <section aria-labelledby="routing-rules">
        <div class="group-head">
          <h2 id="routing-rules" class="group-title">
            Rules
            <span class="badge">{routing?.rules.length ?? 0}</span>
          </h2>
          {#if routing?.rules.length}
            <Button size="compact" icon="plus" onclick={() => editRule(null)}>Add rule</Button>
          {/if}
        </div>
        {#if routing?.rules.length}
          <div class="table-wrap">
            <table>
              <thead><tr><th class="index">#</th><th>Patterns</th><th>Action</th><th><span class="visually-hidden">Actions</span></th></tr></thead>
              <tbody>
                {#each routing.rules as item, index (item.id)}
                  {@const parsed = patterns(item.pattern)}
                  <tr>
                    <td class="index">{index + 1}</td>
                    <td class="pattern-cell">
                      <div class="chips">
                        {#each parsed.shown as entry}<code class="chip">{entry}</code>{/each}
                        {#if parsed.more}<span class="small muted">+{parsed.more} more</span>{/if}
                      </div>
                    </td>
                    <td>
                      <div class="action-cell">
                        <span class="badge {actionTone(item.outbound)}">
                          <Icon name={actionIcon(item.outbound)} size={14} />
                          <span>{actionLabel(item.outbound)}</span>
                        </span>
                        {#if item.outbound === '@warp'}
                          <span class="small muted">UDP: {item.udp_fallback ? actionLabel(item.udp_fallback) : 'blocked'}</span>
                        {/if}
                      </div>
                    </td>
                    <td class="actions">
                      <IconButton icon="up" label="Move rule {index + 1} up" disabled={routing.rules[index - 1]?.node_id !== item.node_id} onclick={() => move(index, -1)} />
                      <IconButton icon="down" label="Move rule {index + 1} down" disabled={routing.rules[index + 1]?.node_id !== item.node_id} onclick={() => move(index, 1)} />
                      <IconButton icon="edit" label="Edit rule {index + 1}" onclick={() => editRule(item)} />
                      <IconButton icon="close" label="Remove rule {index + 1}" onclick={() => { removing = { kind: 'rule', id: item.id, name: `rule ${index + 1}` }; removeOpen = true; }} />
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        {:else}
          <EmptyState icon="route" title="No routing rules" message="Add a rule to send selected traffic through Cloudflare WARP, block domains or IPs, or forward through a custom outbound.">
            {#snippet actions()}
              <Button kind="primary" size="compact" icon="plus" onclick={() => editRule(null)}>Add rule</Button>
            {/snippet}
          </EmptyState>
        {/if}
      </section>

      <section aria-labelledby="routing-outbounds">
        <div class="group-head">
          <h2 id="routing-outbounds" class="group-title">
            Custom outbounds
            <span class="badge">{routing?.outbounds.length ?? 0}</span>
          </h2>
          {#if routing?.outbounds.length}
            <Button size="compact" icon="plus" onclick={() => editOutbound(null)}>Add outbound</Button>
          {/if}
        </div>
        {#if routing?.outbounds.length}
          <div class="table-wrap">
            <table>
              <thead><tr><th>Name</th><th>Type</th><th>Target</th><th>UDP</th><th><span class="visually-hidden">Actions</span></th></tr></thead>
              <tbody>
                {#each routing.outbounds as item (item.id)}
                  <tr>
                    <td class="mono name-col">{item.name}</td>
                    <td><span class="badge">{kinds.find((kind) => kind.value === item.kind)?.label ?? item.kind}</span></td>
                    <td class="mono">{item.kind === 'fiend' ? nodes.find((other) => other.id === item.link_node_id)?.name ?? '—' : item.server ?? '—'}{#if item.username} <span class="muted">· {item.username}</span>{/if}</td>
                    <td><span class="badge {item.udp ? 'green' : ''}">{item.udp ? 'Enabled' : item.udp_fallback ? `Via ${item.udp_fallback}` : 'Blocked'}</span></td>
                    <td class="actions">
                      <IconButton icon="edit" label="Edit {item.name}" onclick={() => editOutbound(item)} />
                      <IconButton icon="close" label="Remove {item.name}" onclick={() => { removing = { kind: 'outbound', id: item.id, name: item.name }; removeOpen = true; }} />
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        {:else}
          <EmptyState icon="swap" title="No custom outbounds" message="Define an external SOCKS5 or HTTP CONNECT proxy to use as a rule target." compact>
            {#snippet actions()}
              <Button size="compact" icon="plus" onclick={() => editOutbound(null)}>Add outbound</Button>
            {/snippet}
          </EmptyState>
        {/if}
      </section>
    </div>

    {#if localNodes.length}
      <aside class="side">
        {#each localNodes as node (node.id)}
          <WarpCard node={node.id} revision={warpRevision} />
        {/each}
      </aside>
    {/if}
  </div>
{/if}

<Dialog bind:open={outboundOpen} title={editing === null ? 'Add outbound' : 'Edit outbound'}>
  <form id="outbound-form" novalidate class="stack" onsubmit={saveOutbound}>
    {#if formError}<p class="note error" role="alert">{formError}</p>{/if}
    <Field label="Name" id="outbound-name" error={fields.name}><input id="outbound-name" bind:value={form.name} /></Field>
    <Field label="Type" id="outbound-kind" error={fields.kind}><Select id="outbound-kind" bind:value={form.kind} options={kinds.filter((kind) => !['warp', 'block', 'direct', 'fiend'].includes(kind.value) || (editing !== null && kind.value === form.kind))} label="Type" /></Field>
    {#if form.kind === 'socks5' || form.kind === 'http'}
      <Field label="Proxy server" id="outbound-server" error={fields.server}><input id="outbound-server" bind:value={form.server} placeholder="127.0.0.1:1080" /></Field>
      <div class="grid-2">
        <Field label="User name" id="outbound-user"><input id="outbound-user" autocomplete="off" bind:value={form.username} /></Field>
        <Field label="Password" id="outbound-password" error={fields.password} help={editing === null ? '' : 'Empty keeps the saved password.'}><input id="outbound-password" type="password" autocomplete="off" bind:value={form.password} /></Field>
      </div>
      {#if form.kind === 'socks5'}<Switch id="outbound-udp" bind:checked={form.udp} label="The proxy supports UDP ASSOCIATE" />{/if}
    {/if}
    {#if form.kind === 'fiend'}
      <Field label="Node" id="outbound-link" error={fields.link_node_id}><Select id="outbound-link" bind:value={form.link} label="Node" options={nodes.filter((item) => item.id !== scope).map((item) => ({ value: String(item.id), label: item.name, detail: item.address }))} /></Field>
    {/if}
    {#if form.kind === 'http' || form.kind === 'warp' || (form.kind === 'socks5' && !form.udp)}
      <Field label="UDP traffic" id="outbound-fallback" error={fields.udp_fallback}>
        <Select id="outbound-fallback" bind:value={form.fallback} label="UDP traffic" options={[{ value: '', label: 'Block UDP' }, { value: 'direct', label: 'direct' }, ...available.filter((item) => item.udp && item.name !== form.name).map((item) => ({ value: item.name, label: item.name, detail: item.kind }))]} />
      </Field>
    {/if}
  </form>
  {#snippet actions()}
    <Button kind="ghost" onclick={() => (outboundOpen = false)}>Cancel</Button>
    <Button kind="primary" type="submit" form="outbound-form" {busy}>Save</Button>
  {/snippet}
</Dialog>

<Dialog bind:open={ruleOpen} title={editing === null ? 'Add rule' : 'Edit rule'}>
  <form id="rule-form" novalidate class="stack" onsubmit={saveRule}>
    {#if formError}<p class="note error" role="alert">{formError}</p>{/if}
    <Field label="Patterns" id="rule-pattern" error={fields.pattern} help="One per line: domain:example.com, full:www.example.com, keyword:ads, ip:10.0.0.0/8"><textarea id="rule-pattern" spellcheck="false" bind:value={rule.pattern}></textarea></Field>
    <Field label="Action" id="rule-outbound" error={fields.outbound}><Select id="rule-outbound" bind:value={rule.outbound} options={outboundOptions} label="Action" /></Field>
    {#if rule.outbound === '@warp'}
      <Field label="UDP traffic" id="rule-udp" error={fields.udp_fallback} help="WARP carries TCP. Traffic routed to WARP stops when the tunnel is unavailable.">
        <Select id="rule-udp" bind:value={rule.udp} options={udpOptions} label="UDP traffic" />
      </Field>
    {/if}
  </form>
  {#snippet actions()}
    <Button kind="ghost" onclick={() => (ruleOpen = false)}>Cancel</Button>
    <Button kind="primary" type="submit" form="rule-form" {busy}>Save</Button>
  {/snippet}
</Dialog>

<Confirm bind:open={removeOpen} title="Remove {removing?.name ?? ''}" action="Remove" onconfirm={remove} />

<style>
  .banner { margin-bottom: 16px; }
  .layout { display: grid; grid-template-columns: minmax(0, 1fr); gap: 22px; align-items: start; }
  .layout.has-side { grid-template-columns: minmax(0, 1fr) 360px; }
  .main { display: flex; flex-direction: column; gap: 24px; min-width: 0; }
  .side { display: flex; flex-direction: column; gap: 16px; min-width: 0; }
  .actions { text-align: right; width: 1%; position: sticky; right: 0; background: var(--panel); }
  tbody tr:hover .actions { background: var(--hover); }
  .index { width: 1%; padding-right: 4px; color: var(--muted); font-variant-numeric: tabular-nums; }
  .name-col { font-weight: 500; }
  .pattern-cell { max-width: 440px; }
  .chips { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; padding: 6px 0; }
  .chip { padding: 2px 6px; border-radius: 4px; background: var(--soft); border: 1px solid var(--line); font-size: 11.5px; color: var(--text); }
  .action-cell { display: inline-flex; flex-direction: column; align-items: flex-start; gap: 2px; padding: 4px 0; }
  @media (max-width: 1080px) { .layout.has-side { grid-template-columns: minmax(0, 1fr); } }
</style>
