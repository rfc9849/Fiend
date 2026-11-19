<script lang="ts">
  import { onMount } from 'svelte';
  import Button from './Button.svelte';
  import Select from './Select.svelte';
  import Confirm from './Confirm.svelte';
  import Icon from './Icon.svelte';
  import { api, fieldErrors } from '#lib/api/client.ts';
  import { toast } from '#lib/stores/toasts.svelte.ts';
  import { onEvent } from '#lib/stores/events.ts';
  import { ago, date } from '#lib/format.ts';
  import type { WarpRotation, WarpView } from '#lib/api/generated/index.ts';

  let { node, revision }: { node: number; revision: number } = $props();
  let warp = $state<WarpView | null>(null);
  let rotation = $state<WarpRotation | null>(null);
  let busy = $state('');
  let error = $state('');
  let deleting = $state(false);
  let every = $state('off');
  let request = 0;

  const ready = $derived(!!warp && warp.rule_count > 0 && warp.running && warp.warp_on && !warp.error);
  const badgeTone = $derived(!warp ? '' : warp.error ? 'warn' : ready ? 'green' : warp.rule_count > 0 ? 'warn' : '');
  const badgeLabel = $derived(!warp ? 'Loading' : warp.error ? 'Needs attention' : ready ? 'Ready' : warp.rule_count > 0 ? 'Pending' : 'Standby');

  async function load() {
    const current = ++request;
    try {
      const status = await api.get<WarpView>(`/nodes/${node}/warp`);
      if (current !== request) return;
      warp = status;
      every = status.rotate_every ? String(status.rotate_every) : 'off';
    } catch (failure) {
      if (current === request) error = fieldErrors(failure).form ?? '';
    }
  }

  onMount(() => onEvent((kind, id) => {
    if (kind === 'nodes' && id === String(node) && !busy) load();
  }));

  $effect(() => {
    node;
    revision;
    load();
  });

  async function act(name: string, run: () => Promise<void>) {
    busy = name;
    request++;
    error = '';
    try {
      await run();
    } catch (failure) {
      error = fieldErrors(failure).form ?? '';
    } finally {
      busy = '';
    }
  }

  const settings = (rotate: string) =>
    act('settings', async () => {
      warp = await api.put<WarpView>(`/nodes/${node}/warp`, { rotate_every: rotate === 'off' ? null : Number(rotate) });
      toast('Rotation schedule saved');
      await load();
    });

  const register = () => act('register', async () => {
    warp = await api.post<WarpView>(`/nodes/${node}/warp/register`);
    toast('WARP account registered');
  });

  const rotate = () => act('rotate', async () => {
    rotation = await api.post<WarpRotation>(`/nodes/${node}/warp/rotate`);
    warp = rotation.warp;
    const changed = (rotation.after_v4 && rotation.after_v4 !== rotation.before_v4) || (rotation.after_v6 && rotation.after_v6 !== rotation.before_v6);
    toast(changed ? 'WARP exit IP changed' : 'Rotation finished without a confirmed IP change');
  });
</script>

<section aria-labelledby="warp-title-{node}">
  <div class="group-head">
    <h2 id="warp-title-{node}" class="group-title">
      Cloudflare WARP
      {#if warp}<span class="badge {badgeTone}">{badgeLabel}</span>{/if}
    </h2>
    {#if warp?.registered}
      <Button size="compact" kind="primary" icon="refresh" busy={busy === 'rotate'} disabled={!warp.running || !!busy} onclick={rotate}>New IP</Button>
    {/if}
  </div>

  <div class="group">
    {#if error}
      <div class="group-body"><p class="note error" role="alert">{error}</p></div>
    {/if}

    {#if warp}
      <div class="group-row">
        <div class="setting-lead">
          <span class="icon-tile {ready ? 'green' : warp.error ? 'warn' : 'accent'}"><Icon name="shield" size={16} /></span>
          <div class="setting-copy">
            {#if warp.rule_count}
              <div class="setting-title">{warp.rule_count} rule{warp.rule_count === 1 ? '' : 's'} routed via WARP</div>
              <p class="setting-desc">{ready ? 'Tunnel active and verified.' : 'Tunnel is not confirmed yet.'}</p>
            {:else}
              <div class="setting-title">Built-in WARP outbound</div>
              <p class="setting-desc">Select Cloudflare WARP on any rule to activate the tunnel automatically.</p>
            {/if}
          </div>
        </div>
      </div>

      {#if warp.rule_count && !warp.registered}
        <div class="group-row">
          <span class="info-label">Registration</span>
          <Button size="compact" busy={busy === 'register'} disabled={!!busy} onclick={register}>Retry registration</Button>
        </div>
      {/if}

      {#if warp.registered}
        <div class="group-row">
          <span class="info-label">Exit IPv4</span>
          <span class="info-value mono">{warp.exit_v4 ?? '—'}</span>
        </div>
        <div class="group-row">
          <span class="info-label">Exit IPv6</span>
          <span class="info-value mono">{warp.exit_v6 ?? '—'}</span>
        </div>
        <div class="group-row">
          <span class="info-label">Location</span>
          <span class="info-value">
            {warp.colo ?? '—'}
            {#if !warp.warp_on}<span class="badge warn">not on WARP</span>{/if}
          </span>
        </div>
        <div class="group-row">
          <span class="info-label">Account made</span>
          <span class="info-value">{ago(warp.created_at)}</span>
        </div>
        <div class="group-row">
          <span class="info-label">Last new IP</span>
          <span class="info-value">{date(warp.rotated_at)}</span>
        </div>

        {#if rotation}
          <div class="group-body diff-box">
            <p class="note">
              IPv4: <span class="mono">{rotation.before_v4 ?? '—'}</span> → <span class="mono">{rotation.after_v4 ?? '—'}</span><br />
              IPv6: <span class="mono">{rotation.before_v6 ?? '—'}</span> → <span class="mono">{rotation.after_v6 ?? '—'}</span>
            </p>
          </div>
        {/if}

        <div class="group-row">
          <span class="info-label">Auto rotation</span>
          <div class="rotate">
            <Select id="warp-rotate-{node}" bind:value={every} label="Automatic new IP" onchange={settings} options={[{ value: 'off', label: 'Off' }, { value: '21600', label: 'Every 6 h' }, { value: '43200', label: 'Every 12 h' }, { value: '86400', label: 'Every 24 h' }, { value: '604800', label: 'Every 7 d' }]} />
          </div>
        </div>
      {/if}

      {#if warp.error}
        <div class="group-body diff-box"><p class="note warn">{warp.error}</p></div>
      {/if}

      {#if warp.registered && !warp.rule_count}
        <div class="group-footer end">
          <Button size="compact" kind="danger" onclick={() => (deleting = true)}>Delete account</Button>
        </div>
      {/if}
    {:else}
      <div class="group-row"><p class="muted small">Loading…</p></div>
    {/if}
  </div>
</section>

<Confirm bind:open={deleting} title="Delete the WARP account" action="Delete account" onconfirm={async () => { warp = await api.del<WarpView>(`/nodes/${node}/warp`); toast('WARP account deleted'); }} />

<style>
  .rotate { width: 138px; }
  .diff-box { border-top: 1px solid var(--line); padding: 12px 14px; }
</style>
