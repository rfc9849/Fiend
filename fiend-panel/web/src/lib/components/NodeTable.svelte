<script lang="ts">
  import { goto } from '$app/navigation';
  import NodeSync from './NodeSync.svelte';
  import EmptyState from './EmptyState.svelte';
  import Button from './Button.svelte';
  import type { NodeView } from '#lib/api/generated/index.ts';
  import { ago, bytes, rate, stateLabel } from '#lib/format.ts';
  let { nodes }: { nodes: NodeView[] } = $props();
</script>

{#if nodes.length === 0}
  <EmptyState icon="server" title="No server connected" message="Connect the local server to start monitoring traffic and managing access." compact>
    {#snippet actions()}
      <Button kind="primary" size="compact" onclick={() => goto('/welcome?next=%2Fnodes')}>Connect local server</Button>
    {/snippet}
  </EmptyState>
{:else}
  <div class="table-wrap">
    <table>
      <thead>
        <tr>
          <th>Name</th><th>Address</th><th>Version</th><th>Settings</th><th class="num">CPU</th><th class="num">Memory</th><th class="num">Traffic</th><th>Last seen</th>
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
            <td><NodeSync {node} /></td>
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
  .state { color: var(--danger); }
  .arch { margin-left: 6px; font-size: 11px; }
</style>
