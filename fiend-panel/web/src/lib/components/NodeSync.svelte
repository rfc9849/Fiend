<script lang="ts">
  import type { NodeView } from '#lib/api/generated/index.ts';

  let { node }: { node: NodeView } = $props();

  const state = $derived.by(() => {
    if (node.disguise_id === null) return { label: 'Needs disguise', tone: 'warn' };
    if (node.status === 'error') return { label: 'Sync failed', tone: 'danger' };
    if (node.status !== 'online') return { label: 'Waiting for node', tone: 'warn' };
    if (node.applied_version !== node.desired_version) return { label: 'Waiting for sync', tone: 'warn' };
    return { label: 'Up to date', tone: 'green' };
  });
</script>

<span class="badge {state.tone}" title={node.last_error ?? undefined}>{state.label}</span>
