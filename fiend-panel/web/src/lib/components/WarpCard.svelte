<script lang="ts">
  import Button from './Button.svelte';
  import Field from './Field.svelte';
  import Select from './Select.svelte';
  import Switch from './Switch.svelte';
  import Confirm from './Confirm.svelte';
  import { api, fieldErrors } from '#lib/api/client.ts';
  import { toast } from '#lib/stores/toasts.svelte.ts';
  import { ago, date } from '#lib/format.ts';
  import type { WarpRotation, WarpView } from '#lib/api/generated/index.ts';

  let { node, title = 'Cloudflare WARP' }: { node: number; title?: string } = $props();
  let warp = $state<WarpView | null>(null);
  let rotation = $state<WarpRotation | null>(null);
  let busy = $state('');
  let error = $state('');
  let deleting = $state(false);
  let every = $state('off');

  async function load() {
    try {
      warp = await api.get<WarpView>(`/nodes/${node}/warp`);
      every = warp.rotate_every ? String(warp.rotate_every) : 'off';
    } catch (failure) {
      error = fieldErrors(failure).form ?? '';
    }
  }

  $effect(() => {
    node;
    load();
  });

  async function act(name: string, run: () => Promise<void>) {
    busy = name;
    error = '';
    try {
      await run();
    } catch (failure) {
      error = fieldErrors(failure).form ?? '';
    } finally {
      busy = '';
    }
  }

  const settings = (enabled: boolean, rotate: string) =>
    act('settings', async () => {
      warp = await api.put<WarpView>(`/nodes/${node}/warp`, { enabled, rotate_every: rotate === 'off' ? null : Number(rotate) });
      toast(enabled ? 'WARP is on for this node' : 'WARP is off for this node');
      await load();
    });

  const register = () => act('register', async () => {
    warp = await api.post<WarpView>(`/nodes/${node}/warp/register`);
    toast('WARP account registered');
  });

  const rotate = () => act('rotate', async () => {
    rotation = await api.post<WarpRotation>(`/nodes/${node}/warp/rotate`);
    warp = rotation.warp;
    toast('New WARP IP');
  });
</script>

<section class="card" aria-labelledby="warp-title-{node}">
  <div class="card-header">
    <h2 id="warp-title-{node}">{title}</h2>
    {#if warp?.registered}<Button kind="primary" icon="refresh" busy={busy === 'rotate'} disabled={!warp.running || !!busy} onclick={rotate}>New IP</Button>{/if}
  </div>
  {#if error}<p class="note error" role="alert">{error}</p>{/if}
  {#if warp}
    <div class="stack">
      <Switch id="warp-enabled-{node}" checked={warp.enabled} label="Use WARP on this node" disabled={!!busy} onchange={(value) => settings(value, every)} />
      {#if warp.enabled && !warp.registered}
        <div class="row"><Button busy={busy === 'register'} onclick={register}>Register again</Button></div>
      {/if}
      {#if warp.registered}
        <dl class="facts">
          <dt>Exit IPv4</dt><dd class="mono">{warp.exit_v4 ?? '—'}</dd>
          <dt>Exit IPv6</dt><dd class="mono">{warp.exit_v6 ?? '—'}</dd>
          <dt>Location</dt><dd>{warp.colo ?? '—'}{#if !warp.warp_on} · <span class="warn-text">not on WARP</span>{/if}</dd>
          <dt>Account made</dt><dd>{ago(warp.created_at)}</dd>
          <dt>Last new IP</dt><dd>{date(warp.rotated_at)}</dd>
        </dl>
        {#if rotation}
          <p class="note">
            IPv4: <span class="mono">{rotation.before_v4 ?? '—'}</span> → <span class="mono">{rotation.after_v4 ?? '—'}</span><br />
            IPv6: <span class="mono">{rotation.before_v6 ?? '—'}</span> → <span class="mono">{rotation.after_v6 ?? '—'}</span>
          </p>
        {/if}
        <div class="rotate"><Field label="Automatic new IP" id="warp-rotate-{node}"><Select id="warp-rotate-{node}" bind:value={every} label="Automatic new IP" onchange={(value) => settings(warp?.enabled ?? false, value)} options={[{ value: 'off', label: 'Off' }, { value: '21600', label: 'Every 6 h' }, { value: '43200', label: 'Every 12 h' }, { value: '86400', label: 'Every 24 h' }, { value: '604800', label: 'Every 7 d' }]} /></Field></div>
      {/if}
      {#if warp.error}<p class="note warn">{warp.error}</p>{/if}
      {#if warp.registered}<div><Button kind="ghost" onclick={() => (deleting = true)}>Delete account</Button></div>{/if}
    </div>
  {:else}
    <p class="muted">Loading…</p>
  {/if}
</section>

<Confirm bind:open={deleting} title="Delete the WARP account"  action="Delete account" onconfirm={async () => { warp = await api.del<WarpView>(`/nodes/${node}/warp`); toast('WARP account deleted'); }} />

<style>
  .facts { display: grid; grid-template-columns: max-content minmax(0, 1fr); gap: 4px 16px; margin: 0; }
  dt { color: var(--muted); }
  dd { margin: 0; min-width: 0; overflow-wrap: anywhere; }
  .rotate { width: 200px; }
  .warn-text { color: var(--warn); }
</style>
