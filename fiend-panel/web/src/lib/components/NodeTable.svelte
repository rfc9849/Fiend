<script lang="ts">
  import type { NodeView } from '#lib/api/generated/index.ts';
  import HealthRing from './HealthRing.svelte';
  import { ago, bytes, rate, stateLabel } from '#lib/format.ts';
  let { nodes, selectable = false, selected = $bindable([]) }: { nodes: NodeView[]; selectable?: boolean; selected?: number[] } = $props();
  function toggle(id: number) {
    selected = selected.includes(id) ? selected.filter((item) => item !== id) : [...selected, id];
  }
</script>

{#if nodes.length === 0}
  <p class="empty">No nodes yet.</p>
{:else}
  <div class="table-wrap">
    <table>
      <thead>
        <tr>
          {#if selectable}<th><span class="visually-hidden">Select</span></th>{/if}
          <th>Name</th><th>Address</th><th>Version</th><th class="num">CPU</th><th class="num">Memory</th><th class="num">Traffic</th><th>Last seen</th>
        </tr>
      </thead>
      <tbody>
        {#each nodes as node (node.id)}
          <tr class:selected={selected.includes(node.id)}>
            {#if selectable}<td><input type="checkbox" aria-label="Select {node.name}" checked={selected.includes(node.id)} onchange={() => toggle(node.id)} /></td>{/if}
            <td>
              <a class="row-link" href="/nodes/{node.id}">
                <HealthRing status={node.status} size={18} />
                <span>{node.name}</span>
                <span class="visually-hidden">, {stateLabel(node.status)}</span>
              </a>
              {#if node.local}<span class="badge">local</span>{/if}
            </td>
            <td class="mono">{node.address}:{node.port}</td>
            <td class="mono">{node.version ?? '—'}{#if node.arch} · {node.arch}{/if}</td>
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
  input[type=checkbox] { width: 16px; height: 16px; accent-color: var(--accent); }
</style>
