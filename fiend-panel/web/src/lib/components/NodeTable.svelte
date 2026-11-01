<script lang="ts">
  import type { NodeView } from '#lib/api/generated/index.ts';
  import { ago, bytes, rate, stateLabel } from '#lib/format.ts';
  let { nodes }: { nodes: NodeView[] } = $props();
</script>

{#if nodes.length === 0}
  <p class="empty">No node yet. Finish the setup wizard to add the local node.</p>
{:else}
  <div class="table-wrap">
    <table>
      <thead>
        <tr>
          <th>Name</th><th>Address</th><th>Version</th><th class="num">CPU</th><th class="num">Memory</th><th class="num">Traffic</th><th>Last seen</th>
        </tr>
      </thead>
      <tbody>
        {#each nodes as node (node.id)}
          <tr>
            <td>
              <span class="name-cell">
                <span class="dot {node.status}" title={stateLabel(node.status)} aria-hidden="true"></span>
                <a class="row-link" href="/nodes/{node.id}">{node.name}<span class="visually-hidden">, {stateLabel(node.status)}</span></a>
                {#if node.local}<span class="badge">local</span>{/if}
              </span>
            </td>
            <td class="mono">{node.address}:{node.port}</td>
            <td>{#if node.version}<span class="mono">{node.version}</span>{#if node.arch}<span class="muted arch">{node.arch}</span>{/if}{:else}—{/if}</td>
            <td class="num">{node.cpu === null ? '—' : `${node.cpu.toFixed(0)} %`}</td>
            <td class="num">{node.memory_used === null ? '—' : `${bytes(node.memory_used)} / ${bytes(node.memory_total)}`}</td>
            <td class="num">{rate(node.traffic_rate)}</td>
            <td>{#if node.status !== 'online'}<span class="state">{stateLabel(node.status)} · </span>{/if}{ago(node.last_seen)}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
{/if}

<style>
  .empty { padding: 18px; border: 1px dashed var(--line); border-radius: 9px; text-align: center; }
  .state { color: var(--danger); }
  .arch { margin-left: 6px; }
</style>
