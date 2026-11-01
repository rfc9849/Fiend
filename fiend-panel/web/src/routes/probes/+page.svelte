<script lang="ts">
  import { onMount } from 'svelte';
  import TopBar from '#lib/components/TopBar.svelte';
  import Tabs from '#lib/components/Tabs.svelte';
  import Button from '#lib/components/Button.svelte';
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
  const tone = (reason: string) => (reason === 'replay' ? 'danger' : reason === 'unknown_key' ? '' : 'warn');

  let report = $state<ProbeReport | null>(null);
  let range = $state('24h');
  let peer = $state('');
  let open = $state<number | null>(null);
  let error = $state('');
  let timer: ReturnType<typeof setTimeout> | undefined;

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
</script>

<svelte:head><title>Probes · fiend</title></svelte:head>

<TopBar title="Probes" subtitle="Connections that did not authenticate. The node sent them to the origin." />
{#if error}<p class="note error" role="alert">{error}</p>{/if}
<div class="filters">
  <Tabs label="Probe range" bind:value={range} onchange={load} tabs={[{ value: '1h', label: '1 h' }, { value: '24h', label: '24 h' }, { value: '7d', label: '7 d' }, { value: '30d', label: '30 d' }]} />
  {#if peer}<Button kind="ghost" icon="close" onclick={() => pick('')}>Show all addresses (now {peer})</Button>{/if}
</div>
{#if report}
  <section class="card" aria-labelledby="peers-title">
    <div class="card-header"><h2 id="peers-title">Sources</h2><span class="muted small">{report.total} probe{report.total === 1 ? '' : 's'} from {report.peers.length} address{report.peers.length === 1 ? '' : 'es'}</span></div>
    {#if report.peers.length}
      <div class="table-wrap peers">
        <table>
          <thead><tr><th>Address</th><th class="num">Probes</th><th>First</th><th>Last</th><th>Reasons</th><th></th></tr></thead>
          <tbody>
            {#each report.peers as row (row.peer)}
              <tr>
                <td><button type="button" class="link mono" onclick={() => pick(row.peer)}>{row.peer}</button></td>
                <td class="num">{row.count}</td>
                <td>{date(row.first)}</td>
                <td>{ago(row.last)}</td>
                <td class="reasons">{#each row.reasons as reason (reason)}<span class="badge {tone(reason)}">{reasons[reason] ?? reason}</span>{/each}</td>
                <td><a class="small" href="https://ipinfo.io/{row.peer}" target="_blank" rel="noopener noreferrer">IP info</a></td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {:else}
      <p class="empty">No probes in this range.</p>
    {/if}
  </section>
  {#if report.probes.length}
    <section class="card" aria-labelledby="recent-title">
      <div class="card-header"><h2 id="recent-title">Recent probes</h2><span class="muted small">Newest {report.probes.length}. Select a row to see the data.</span></div>
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
                      <pre class="dump">{dump(probe.head)}</pre>
                      {#if probe.size > probe.head.length / 2}<p class="muted small">Shows the first {bytes(probe.head.length / 2)} of {bytes(probe.size)}.</p>{/if}
                    {:else}
                      <p class="muted small">The peer sent no data before the node forwarded the connection.</p>
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
  .filters { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; margin-bottom: 14px; }
  .peers { max-height: 420px; overflow: auto; }
  .link { color: var(--text); text-align: left; }
  .link:hover { text-decoration: underline; }
  .reasons { display: flex; gap: 6px; align-items: center; height: 36px; }
  .data { max-width: 420px; overflow: hidden; text-overflow: ellipsis; }
  .data .link { font-family: var(--mono); font-size: 12px; }
  td.num { text-align: right; font-variant-numeric: tabular-nums; }
  .dump-row td { height: auto; padding: 10px 12px; white-space: normal; background: var(--soft); }
  .dump-row:hover { background: none; }
  .dump { font-family: var(--mono); font-size: 12px; line-height: 1.5; overflow-x: auto; margin: 0 0 6px; white-space: pre; }
  .empty { padding: 18px; border: 1px dashed var(--line); border-radius: 9px; text-align: center; }
</style>
