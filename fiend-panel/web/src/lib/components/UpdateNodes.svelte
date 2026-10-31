<script lang="ts">
  import Dialog from './Dialog.svelte';
  import Field from './Field.svelte';
  import Button from './Button.svelte';
  import Select from './Select.svelte';
  import Steps from './Steps.svelte';
  import { api, fieldErrors } from '#lib/api/client.ts';
  import { onEvent } from '#lib/stores/events.ts';
  import { day } from '#lib/format.ts';
  import type { BundleView, JobStarted, JobView, NodeView } from '#lib/api/generated/index.ts';

  let { open = $bindable(), nodes, selected = [], ondone }: { open: boolean; nodes: NodeView[]; selected?: number[]; ondone: () => void } = $props();
  let bundles = $state<BundleView[]>([]);
  let bundle = $state('');
  let job = $state<JobView | null>(null);
  let busy = $state(false);
  let error = $state('');
  let picked = $state<number[]>([]);
  let stop: (() => void) | null = null;
  const chosen = $derived(nodes.filter((node) => picked.includes(node.id)));

  $effect(() => {
    if (open) {
      job = null;
      error = '';
      picked = selected.length ? [...selected] : nodes.map((node) => node.id);
      api.get<BundleView[]>('/bundles').then((items) => {
        bundles = items;
        bundle = items[0] ? String(items[0].id) : '';
      }).catch((failure) => (error = fieldErrors(failure).form ?? ''));
    } else {
      stop?.();
      stop = null;
    }
  });

  async function start() {
    busy = true;
    error = '';
    try {
      const started = await api.post<JobStarted>('/nodes/update', { node_ids: picked, bundle_id: Number(bundle) });
      const refresh = async () => {
        job = await api.get<JobView>(`/jobs/${started.job_id}`);
        if (job.done) ondone();
      };
      stop = onEvent((kind, id) => {
        if (kind === 'job' && id === started.job_id) refresh();
      });
      await refresh();
    } catch (failure) {
      error = fieldErrors(failure).form ?? '';
    } finally {
      busy = false;
    }
  }
</script>

<Dialog bind:open title="Update nodes">
  {#if error}<p class="note error" role="alert">{error}</p>{/if}
  {#if job}
    <Steps steps={job.steps} />
    {#if job.error}<p class="note error result">{job.error}</p>{:else if job.done}<p class="note result">Done.</p>{/if}
  {:else}
    <div class="stack">
      {#if nodes.length > 1}
        <fieldset class="nodes">
          <legend>Nodes</legend>
          {#each nodes as node (node.id)}
            <label class="check"><input type="checkbox" checked={picked.includes(node.id)} onchange={(event) => { const on = (event.currentTarget as HTMLInputElement).checked; picked = on ? [...picked, node.id] : picked.filter((item) => item !== node.id); }} />{node.name}<span class="muted small">{node.version ?? 'unknown version'}</span></label>
          {/each}
        </fieldset>
      {:else}
        <p><strong>Node:</strong> {chosen.map((node) => node.name).join(', ')}</p>
      {/if}
      {#if bundles.length}
        <Field label="Version" id="bundle"><Select id="bundle" bind:value={bundle} label="Version" options={bundles.map((item) => ({ value: String(item.id), label: `${item.version} · ${item.arch}`, detail: day(item.uploaded_at) }))} /></Field>
      {:else}
        <p class="note warn">No bundle. Upload one in Settings.</p>
      {/if}
    </div>
  {/if}
  {#snippet actions()}
    {#if job}
      <Button kind="primary" disabled={!job.done} onclick={() => (open = false)}>Close</Button>
    {:else}
      <Button kind="ghost" onclick={() => (open = false)}>Cancel</Button>
      <Button kind="primary" {busy} disabled={!bundle || !chosen.length} onclick={start}>Update</Button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .result { margin-top: 14px; }
  .nodes { border: 1px solid var(--line); border-radius: 6px; padding: 10px 12px; margin: 0; display: flex; flex-direction: column; gap: 6px; }
  legend { font-weight: 500; font-size: 12px; padding: 0 4px; }
  .check { display: flex; gap: 8px; align-items: center; }
  .check input { width: 16px; height: 16px; accent-color: var(--accent); }
</style>
