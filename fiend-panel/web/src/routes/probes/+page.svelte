<script lang="ts">
  import { onMount } from 'svelte';
  import { page } from '$app/state';
  import TopBar from '#lib/components/TopBar.svelte';
  import Tabs from '#lib/components/Tabs.svelte';
  import Button from '#lib/components/Button.svelte';
  import EmptyState from '#lib/components/EmptyState.svelte';
  import Icon from '#lib/components/Icon.svelte';
  import { api, fieldErrors } from '#lib/api/client.ts';
  import { onEvent } from '#lib/stores/events.ts';
  import { ago, bytes, date } from '#lib/format.ts';
  import type { ProbeReport, ProbeView } from '#lib/api/generated/index.ts';

  const reasons: Record<string, string> = {
    not_tls: 'Not TLS',
    bad_record: 'Bad TLS record',
    truncated: 'Incomplete data',
    not_client_hello: 'Not a ClientHello',
    unknown_key: 'Unknown key',
    replay: 'Replay',
    bad_client_hello: 'Bad ClientHello',
    bad_early_data: 'Bad early data',
    key_share: 'Unsupported key share'
  };
  const rangeLabels: Record<string, string> = { '1h': '1 h', '24h': '24 h', '7d': '7 d', '30d': '30 d' };
  const tone = (reason: string) => (reason === 'replay' ? 'danger' : reason === 'unknown_key' ? '' : 'warn');

  let report = $state<ProbeReport | null>(null);
  let range = $state('24h');
  let peer = $state('');
  let open = $state<number | null>(null);
  let copiedId = $state<number | null>(null);
  let error = $state('');
  let timer: ReturnType<typeof setTimeout> | undefined;

  const topReason = $derived.by(() => {
    if (!report?.probes.length) return null;
    const counts: Record<string, number> = {};
    for (const item of report.probes) counts[item.reason] = (counts[item.reason] ?? 0) + 1;
    let best = '';
    let max = 0;
    for (const [key, count] of Object.entries(counts)) {
      if (count > max) {
        best = key;
        max = count;
      }
    }
    return best ? { key: best, label: reasons[best] ?? best, count: max } : null;
  });

  async function load() {
    try {
      const query = new URLSearchParams({ range });
      if (peer) query.set('peer', peer);
      report = await api.get<ProbeReport>(`/probes?${query}`);
      error = '';
    } catch (failure) {
      error = fieldErrors(failure).form ?? '';
    }
  }

  function pick(value: string) {
    peer = value;
    open = null;
    load();
  }

  onMount(() => {
    const initial = page.url.searchParams.get('range');
    if (initial && ['1h', '24h', '7d', '30d'].includes(initial)) range = initial;
    load();
    const stop = onEvent((kind) => {
      if (kind === 'stats') {
        clearTimeout(timer);
        timer = setTimeout(load, 2000);
      }
    });
    return () => {
      stop();
      clearTimeout(timer);
    };
  });

  function raw(hex: string): number[] {
    const out: number[] = [];
    for (let index = 0; index + 1 < hex.length; index += 2) out.push(parseInt(hex.slice(index, index + 2), 16));
    return out;
  }

  const printable = (byte: number) => byte >= 0x20 && byte < 0x7f;

  function summary(probe: ProbeView): string {
    const data = raw(probe.head);
    if (!data.length) return 'No data';
    if (data[0] === 0x16) return probe.sni ? `TLS ClientHello for ${probe.sni}` : 'TLS ClientHello without SNI';
    const text = data.slice(0, 120);
    const visible = text.filter((byte) => printable(byte) || byte === 0x0d || byte === 0x0a || byte === 0x09).length;
    if (visible / text.length > 0.9) {
      const line = String.fromCharCode(...text).split(/\r?\n/)[0];
      return line.length > 80 ? `${line.slice(0, 80)}…` : line;
    }
    return `Binary, starts with ${data.slice(0, 8).map((byte) => byte.toString(16).padStart(2, '0')).join(' ')}`;
  }

  function dump(hex: string): string {
    const data = raw(hex);
    const lines: string[] = [];
    for (let offset = 0; offset < data.length; offset += 16) {
      const row = data.slice(offset, offset + 16);
      const hexPart = row.map((byte) => byte.toString(16).padStart(2, '0')).join(' ').padEnd(47, ' ');
      const text = row.map((byte) => (printable(byte) ? String.fromCharCode(byte) : '.')).join('');
      lines.push(`${offset.toString(16).padStart(4, '0')}  ${hexPart}  ${text}`);
    }
    return lines.join('\n');
  }

  async function copyHex(probe: ProbeView) {
    try {
      await navigator.clipboard.writeText(dump(probe.head));
      copiedId = probe.id;
      setTimeout(() => {
        if (copiedId === probe.id) copiedId = null;
      }, 2000);
    } catch {
      copiedId = null;
    }
  }
</script>

<svelte:head><title>Probes · fiend</title></svelte:head>

<TopBar title="Probes" subtitle="Unauthenticated connections forwarded to the origin.">
  {#snippet actions()}
    <Tabs label="Probe range" bind:value={range} onchange={load} tabs={[{ value: '1h', label: '1 h' }, { value: '24h', label: '24 h' }, { value: '7d', label: '7 d' }, { value: '30d', label: '30 d' }]} />
  {/snippet}
</TopBar>

{#if error}<p class="note error banner" role="alert">{error}</p>{/if}

{#if peer}
  <div class="filter-bar">
    <span class="badge accent mono">Source: {peer}</span>
    <Button size="compact" kind="ghost" icon="close" onclick={() => pick('')}>Show all addresses</Button>
  </div>
{/if}

{#if report}
  <div class="kpi-grid">
    <section class="kpi" aria-label="Total probes">
      <div class="kpi-head"><span>Total probes</span><Icon name="shield" size={15} /></div>
      <div class="kpi-val">
        <span class="kpi-num">{report.total}</span>
        <span class="kpi-unit">in {rangeLabels[range] ?? range}</span>
      </div>
      <div class="kpi-foot">{peer ? `Filtered to ${peer}` : 'Forwarded to origin'}</div>
    </section>

    <section class="kpi" aria-label="Unique sources">
      <div class="kpi-head"><span>Unique sources</span><Icon name="globe" size={15} /></div>
      <div class="kpi-val">
        <span class="kpi-num">{report.peers.length}</span>
        <span class="kpi-unit">address{report.peers.length === 1 ? '' : 'es'}</span>
      </div>
      <div class="kpi-foot">Distinct client IPs</div>
    </section>

    <section class="kpi" aria-label="Top reason">
      <div class="kpi-head"><span>Top reason</span><Icon name="info" size={15} /></div>
      <div class="kpi-val">
        <span class="kpi-num reason-num">{topReason ? topReason.label : '—'}</span>
      </div>
      <div class="kpi-foot">{topReason ? `${topReason.count} in recent samples` : 'No rejections recorded'}</div>
    </section>

    <section class="kpi" aria-label="Captured samples">
      <div class="kpi-head"><span>Recent samples</span><Icon name="log" size={15} /></div>
      <div class="kpi-val">
        <span class="kpi-num">{report.probes.length}</span>
        <span class="kpi-unit">inspectable</span>
      </div>
      <div class="kpi-foot">Select any row to inspect payload</div>
    </section>
  </div>

  <section class="section" aria-labelledby="peers-title">
    <div class="group-head">
      <h2 id="peers-title" class="group-title">
        Sources
        <span class="badge">{report.peers.length}</span>
      </h2>
      <span class="muted small">{report.total} probe{report.total === 1 ? '' : 's'} in the last {rangeLabels[range] ?? range}</span>
    </div>
    {#if report.peers.length}
      <div class="table-wrap peers">
        <table>
          <thead><tr><th>Address</th><th class="num">Probes</th><th>First</th><th>Last</th><th>Reasons</th><th><span class="visually-hidden">External</span></th></tr></thead>
          <tbody>
            {#each report.peers as row (row.peer)}
              <tr class:selected={peer === row.peer}>
                <td><button type="button" class="link mono" onclick={() => pick(peer === row.peer ? '' : row.peer)}>{row.peer}</button></td>
                <td class="num">{row.count}</td>
                <td>{date(row.first)}</td>
                <td class="muted">{ago(row.last)}</td>
                <td><div class="reasons">{#each row.reasons as reason (reason)}<span class="badge {tone(reason)}">{reasons[reason] ?? reason}</span>{/each}</div></td>
                <td class="ext"><a class="small" href="https://ipinfo.io/{row.peer}" target="_blank" rel="noopener noreferrer">IP info</a></td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {:else}
      <EmptyState icon="shield" title="No probes in this range" message="No unauthenticated connections were forwarded to the origin during the last {rangeLabels[range] ?? range}." />
    {/if}
  </section>

  {#if report.probes.length}
    <section class="section" aria-labelledby="recent-title">
      <div class="group-head">
        <h2 id="recent-title" class="group-title">
          Recent probes
          <span class="badge">{report.probes.length}</span>
        </h2>
        <span class="muted small">Select a row to inspect captured bytes</span>
      </div>
      <div class="table-wrap">
        <table>
          <thead><tr><th>Time</th><th>Node</th><th>Address</th><th>Reason</th><th>Data</th><th class="num">Size</th></tr></thead>
          <tbody>
            {#each report.probes as probe (probe.id)}
              <tr class="clickable" class:selected={open === probe.id} onclick={() => (open = open === probe.id ? null : probe.id)}>
                <td>{date(probe.ts)}</td>
                <td>{probe.node || '—'}</td>
                <td class="mono">{probe.peer}</td>
                <td><span class="badge {tone(probe.reason)}">{reasons[probe.reason] ?? probe.reason}</span></td>
                <td class="data"><button type="button" class="link" aria-expanded={open === probe.id}>{summary(probe)}</button></td>
                <td class="num">{bytes(probe.size)}</td>
              </tr>
              {#if open === probe.id}
                <tr class="dump-row">
                  <td colspan="6">
                    {#if probe.head}
                      <div class="dump-box">
                        <div class="dump-head">
                          <span class="small muted">
                            {#if probe.size > probe.head.length / 2}
                              First {bytes(probe.head.length / 2)} of {bytes(probe.size)}
                            {:else}
                              Captured payload · {bytes(probe.size)}
                            {/if}
                          </span>
                          <Button size="compact" kind="ghost" icon={copiedId === probe.id ? 'check' : 'copy'} onclick={() => copyHex(probe)}>{copiedId === probe.id ? 'Copied' : 'Copy hex'}</Button>
                        </div>
                        <pre class="dump">{dump(probe.head)}</pre>
                      </div>
                    {:else}
                      <p class="muted small empty-dump">The peer sent no data before the node forwarded the connection.</p>
                    {/if}
                  </td>
                </tr>
              {/if}
            {/each}
          </tbody>
        </table>
      </div>
    </section>
  {/if}
{:else if !error}
  <p class="muted">Loading…</p>
{/if}

<style>
  .banner { margin-bottom: 16px; }
  .filter-bar { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; margin-bottom: 14px; }
  .section { margin-top: 24px; }
  .reason-num { font-size: 16px; letter-spacing: -.2px; }
  .peers { max-height: 420px; overflow: auto; }
  .link { color: var(--text); text-align: left; }
  .link:hover { text-decoration: underline; }
  .reasons { display: flex; gap: 6px; align-items: center; flex-wrap: wrap; padding: 6px 0; }
  .ext { text-align: right; width: 1%; }
  .data { max-width: 420px; overflow: hidden; text-overflow: ellipsis; }
  .data .link { font-family: var(--mono); font-size: 12px; }
  td.num { text-align: right; font-variant-numeric: tabular-nums; }
  .dump-row td { height: auto; padding: 12px 14px; white-space: normal; background: var(--soft); }
  .dump-row:hover { background: var(--soft); }
  .dump-box { background: var(--panel); border: 1px solid var(--line); border-radius: 6px; overflow: hidden; }
  .dump-head { display: flex; align-items: center; justify-content: space-between; gap: 10px; padding: 6px 10px 6px 12px; border-bottom: 1px solid var(--line); background: var(--soft); }
  .dump { font-family: var(--mono); font-size: 11.5px; line-height: 1.55; overflow-x: auto; margin: 0; padding: 12px; white-space: pre; color: var(--text); }
  .empty-dump { margin: 0; }
</style>
