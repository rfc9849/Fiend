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

  let { open = $bindable(), nodes, selected, ondone }: { open: boolean; nodes: NodeView[]; selected: number[]; ondone: () => void } = $props();
  let bundles = $state<BundleView[]>([]);
  let bundle = $state('');
  let job = $state<JobView | null>(null);
  let busy = $state(false);
  let error = $state('');
  let stop: (() => void) | null = null;
  const chosen = $derived(nodes.filter((node) => selected.includes(node.id)));

  $effect(() => {
    if (open) {
      job = null;
      error = '';
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
      const started = await api.post<JobStarted>('/nodes/update', { node_ids: selected, bundle_id: Number(bundle) });
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
    {#if job.error}<p class="note error result">{job.error}</p>{:else if job.done}<p class="note result">The update is complete.</p>{/if}
  {:else}
    <div class="stack">
      <p>The panel sends the node binary through the control session. The node checks it, restarts, and rolls back after three failed starts.</p>
      <p><strong>Nodes:</strong> {chosen.map((node) => node.name).join(', ')}</p>
      {#if bundles.length}
        <Field label="Bundle" id="bundle"><Select id="bundle" bind:value={bundle} label="Bundle" options={bundles.map((item) => ({ value: String(item.id), label: `${item.version} · ${item.arch}`, detail: day(item.uploaded_at) }))} /></Field>
      {:else}
        <p class="note warn">No bundle is available. Upload a bundle in Settings.</p>
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
</style>
