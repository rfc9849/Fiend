<script lang="ts">
  import Button from './Button.svelte';
  import IconButton from './IconButton.svelte';
  import Dialog from './Dialog.svelte';
  import Confirm from './Confirm.svelte';
  import Field from './Field.svelte';
  import Select from './Select.svelte';
  import Switch from './Switch.svelte';
  import { api, fieldErrors } from '#lib/api/client.ts';
  import { toast } from '#lib/stores/toasts.svelte.ts';
  import type { NodeView, OutboundView, Routing, RuleView } from '#lib/api/generated/index.ts';

  let { node = null }: { node?: number | null } = $props();
  let routing = $state<Routing | null>(null);
  let global = $state<Routing | null>(null);
  let nodes = $state<NodeView[]>([]);
  let error = $state('');
  let outboundOpen = $state(false);
  let ruleOpen = $state(false);
  let removing = $state<{ kind: 'outbound' | 'rule'; id: number; name: string } | null>(null);
  let removeOpen = $state(false);
  let editing = $state<number | null>(null);
  let form = $state({ name: '', kind: 'socks5', server: '', username: '', password: '', udp: false, link: '', fallback: '' });
  let rule = $state({ pattern: '', outbound: 'direct' });
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
  const base = $derived(node === null ? '' : `?node=${node}`);
  const available = $derived([...(global?.outbounds ?? []), ...(node === null ? [] : routing?.outbounds ?? [])]);
  const outboundOptions = $derived([{ value: 'direct', label: 'direct', detail: 'built in' }, ...available.filter((item, index, list) => list.findIndex((other) => other.name === item.name) === index).map((item) => ({ value: item.name, label: item.name, detail: item.kind }))]);

  async function load() {
    try {
      routing = await api.get<Routing>(`/routing${base}`);
      global = node === null ? routing : await api.get<Routing>('/routing');
      nodes = await api.get<NodeView[]>('/nodes');
    } catch (failure) {
      error = fieldErrors(failure).form ?? '';
    }
  }

  $effect(() => {
    node;
    load();
  });

  function editOutbound(item: OutboundView | null) {
    editing = item?.id ?? null;
    fields = {};
    formError = '';
    form = item
      ? { name: item.name, kind: item.kind, server: item.server ?? '', username: item.username ?? '', password: '', udp: item.udp, link: item.link_node_id ? String(item.link_node_id) : '', fallback: item.udp_fallback ?? '' }
      : { name: '', kind: 'socks5', server: '', username: '', password: '', udp: false, link: '', fallback: '' };
    outboundOpen = true;
  }

  function editRule(item: RuleView | null) {
    editing = item?.id ?? null;
    fields = {};
    formError = '';
    rule = item ? { pattern: item.pattern, outbound: item.outbound } : { pattern: '', outbound: outboundOptions[1]?.value ?? 'direct' };
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
      node_id: node,
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
    const body = { node_id: node, pattern: rule.pattern, outbound: rule.outbound };
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
    if (!routing) return;
    const ids = routing.rules.map((item) => item.id);
    const target = index + delta;
    if (target < 0 || target >= ids.length) return;
    [ids[index], ids[target]] = [ids[target], ids[index]];
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

  const summary = (pattern: string) => {
    const lines = pattern.split('\n').map((line) => line.trim()).filter(Boolean);
    return lines.length > 2 ? `${lines.slice(0, 2).join(', ')} and ${lines.length - 2} more` : lines.join(', ');
  };
</script>

{#if error}<p class="note error" role="alert">{error}</p>{/if}
<p class="note">The first rule that matches wins. {node === null ? 'Node rules come after these global rules.' : 'Global rules come first, then these node rules.'} No match means direct. Destinations on the bypass list of a client never reach the node, so server rules do not apply to them.</p>

<section class="block" aria-labelledby="outbounds-{node ?? 'global'}">
  <div class="card-header"><h2 id="outbounds-{node ?? 'global'}">{node === null ? 'Global outbounds' : 'Outbounds of this node'}</h2><Button icon="plus" onclick={() => editOutbound(null)}>Add outbound</Button></div>
  {#if routing?.outbounds.length}
    <div class="table-wrap">
      <table>
        <thead><tr><th>Name</th><th>Type</th><th>Target</th><th>UDP</th><th><span class="visually-hidden">Actions</span></th></tr></thead>
        <tbody>
          {#each routing.outbounds as item (item.id)}
            <tr>
              <td class="mono">{item.name}</td>
              <td>{kinds.find((kind) => kind.value === item.kind)?.label ?? item.kind}</td>
              <td class="mono">{item.kind === 'fiend' ? nodes.find((other) => other.id === item.link_node_id)?.name ?? '—' : item.server ?? '—'}{#if item.username} · {item.username}{/if}</td>
              <td>{item.udp ? 'yes' : item.udp_fallback ? `to ${item.udp_fallback}` : 'blocked'}</td>
              <td class="actions"><IconButton icon="edit" label="Edit {item.name}" onclick={() => editOutbound(item)} /><IconButton icon="close" label="Remove {item.name}" onclick={() => { removing = { kind: 'outbound', id: item.id, name: item.name }; removeOpen = true; }} /></td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {:else}
    <p class="empty">No outbounds. All traffic goes direct.</p>
  {/if}
</section>

<section class="block" aria-labelledby="rules-{node ?? 'global'}">
  <div class="card-header"><h2 id="rules-{node ?? 'global'}">{node === null ? 'Global rules' : 'Rules of this node'}</h2><Button icon="plus" onclick={() => editRule(null)}>Add rule</Button></div>
  {#if routing?.rules.length}
    <div class="table-wrap">
      <table>
        <thead><tr><th class="num">#</th><th>Patterns</th><th>Outbound</th><th><span class="visually-hidden">Actions</span></th></tr></thead>
        <tbody>
          {#each routing.rules as item, index (item.id)}
            <tr>
              <td class="num">{index + 1}</td>
              <td class="mono pattern">{summary(item.pattern)}</td>
              <td class="mono">{item.outbound}</td>
              <td class="actions">
                <IconButton icon="up" label="Move rule {index + 1} up" disabled={index === 0} onclick={() => move(index, -1)} />
                <IconButton icon="down" label="Move rule {index + 1} down" disabled={index === routing.rules.length - 1} onclick={() => move(index, 1)} />
                <IconButton icon="edit" label="Edit rule {index + 1}" onclick={() => editRule(item)} />
                <IconButton icon="close" label="Remove rule {index + 1}" onclick={() => { removing = { kind: 'rule', id: item.id, name: `rule ${index + 1}` }; removeOpen = true; }} />
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {:else}
    <p class="empty">No rules.</p>
  {/if}
</section>

<Dialog bind:open={outboundOpen} title={editing === null ? 'Add outbound' : 'Edit outbound'}>
  <form id="outbound-form" novalidate class="stack" onsubmit={saveOutbound}>
    {#if formError}<p class="note error" role="alert">{formError}</p>{/if}
    <Field label="Name" id="outbound-name" error={fields.name} help="Rules use this name."><input id="outbound-name" bind:value={form.name} /></Field>
    <Field label="Type" id="outbound-kind" error={fields.kind}><Select id="outbound-kind" bind:value={form.kind} options={kinds} label="Type" /></Field>
    {#if form.kind === 'socks5' || form.kind === 'http'}
      <Field label="Proxy server" id="outbound-server" error={fields.server} help="host:port"><input id="outbound-server" bind:value={form.server} placeholder="127.0.0.1:1080" /></Field>
      <div class="grid-2">
        <Field label="User name" id="outbound-user" help="Optional."><input id="outbound-user" autocomplete="off" bind:value={form.username} /></Field>
        <Field label="Password" id="outbound-password" error={fields.password} help={editing === null ? 'Optional.' : 'Leave empty to keep the saved password.'}><input id="outbound-password" type="password" autocomplete="off" bind:value={form.password} /></Field>
      </div>
      {#if form.kind === 'socks5'}<Switch id="outbound-udp" bind:checked={form.udp} label="The proxy supports UDP ASSOCIATE" />{/if}
    {/if}
    {#if form.kind === 'fiend'}
      <Field label="Node" id="outbound-link" error={fields.link_node_id} help="The panel adds a service user on that node."><Select id="outbound-link" bind:value={form.link} label="Node" options={nodes.filter((item) => item.id !== node).map((item) => ({ value: String(item.id), label: item.name, detail: item.address }))} /></Field>
    {/if}
    {#if form.kind === 'http' || form.kind === 'warp' || (form.kind === 'socks5' && !form.udp)}
      <Field label="UDP traffic" id="outbound-fallback" error={fields.udp_fallback} help="This outbound has no UDP. Block UDP or send it to another outbound.">
        <Select id="outbound-fallback" bind:value={form.fallback} label="UDP traffic" options={[{ value: '', label: 'Block UDP' }, { value: 'direct', label: 'direct' }, ...available.filter((item) => item.udp && item.name !== form.name).map((item) => ({ value: item.name, label: item.name, detail: item.kind }))]} />
      </Field>
    {/if}
    {#if form.kind === 'warp'}<p class="small muted">Turn on WARP on the node page. Nodes without WARP skip this outbound.</p>{/if}
  </form>
  {#snippet actions()}
    <Button kind="ghost" onclick={() => (outboundOpen = false)}>Cancel</Button>
    <Button kind="primary" type="submit" form="outbound-form" {busy}>Save</Button>
  {/snippet}
</Dialog>

<Dialog bind:open={ruleOpen} title={editing === null ? 'Add rule' : 'Edit rule'}>
  <form id="rule-form" novalidate class="stack" onsubmit={saveRule}>
    {#if formError}<p class="note error" role="alert">{formError}</p>{/if}
    <Field label="Patterns" id="rule-pattern" error={fields.pattern} help="One per line: domain:example.com, full:www.example.com, keyword:tracker, ip:10.0.0.0/8."><textarea id="rule-pattern" spellcheck="false" bind:value={rule.pattern}></textarea></Field>
    <Field label="Outbound" id="rule-outbound" error={fields.outbound}><Select id="rule-outbound" bind:value={rule.outbound} options={outboundOptions} label="Outbound" /></Field>
  </form>
  {#snippet actions()}
    <Button kind="ghost" onclick={() => (ruleOpen = false)}>Cancel</Button>
    <Button kind="primary" type="submit" form="rule-form" {busy}>Save</Button>
  {/snippet}
</Dialog>

<Confirm bind:open={removeOpen} title="Remove {removing?.name ?? ''}" message="The nodes get the new routing at once." action="Remove" onconfirm={remove} />

<style>
  .block { margin-top: 22px; }
  .empty { padding: 14px; border: 1px dashed var(--line); border-radius: 9px; }
  .actions { text-align: right; width: 1%; }
  .pattern { max-width: 420px; overflow: hidden; text-overflow: ellipsis; }
</style>
