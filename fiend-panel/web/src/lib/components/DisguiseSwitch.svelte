<script lang="ts">
  import { onMount } from 'svelte';
  import Button from './Button.svelte';
  import Dialog from './Dialog.svelte';
  import Field from './Field.svelte';
  import Select from './Select.svelte';
  import TemplatePicker from './TemplatePicker.svelte';
  import Icon from './Icon.svelte';
  import { api, fieldErrors } from '#lib/api/client.ts';
  import type { Created, DisguiseCheck, DisguiseView, NodeView, PresetView } from '#lib/api/generated/index.ts';

  let { open = $bindable(), node, onsaved }: { open: boolean; node: NodeView; onsaved: (value: NodeView) => void } = $props();
  let saved = $state<DisguiseView[]>([]);
  let presets = $state<PresetView[]>([]);
  let choice = $state('');
  let preset = $state('file');
  let file = $state<string | null>(null);
  let result = $state<DisguiseCheck | null>(null);
  let loading = $state(true);
  let busy = $state('');
  let error = $state('');
  let templateError = $state('');
  const availablePresets = $derived(presets.filter((item) => !saved.some((entry) => entry.name === item.name)));
  const options = $derived([
    ...saved.filter((item) => item.id !== node.disguise_id).map((item) => ({ value: String(item.id), label: item.name, detail: item.origin_host })),
    { value: 'new', label: 'Choose a preset or import a template' }
  ]);

  onMount(async () => {
    try {
      [saved, presets] = await Promise.all([api.get<DisguiseView[]>('/disguises'), api.get<PresetView[]>('/disguises/presets')]);
      choice = options[0].value;
      preset = availablePresets[0]?.id ?? 'file';
    } catch (failure) {
      error = fieldErrors(failure).form ?? 'Could not load the disguises.';
    } finally {
      loading = false;
    }
  });

  async function check(event: SubmitEvent) {
    event.preventDefault();
    if (busy) return;
    error = '';
    templateError = '';
    result = null;
    if (choice === 'new' && preset === 'file' && !file) {
      templateError = 'Choose a template file.';
      return;
    }
    busy = 'check';
    try {
      if (choice === 'new') {
        const created = await api.post<Created>('/disguises', {
          preset: preset === 'file' ? null : preset,
          template_base64: preset === 'file' ? file : null
        });
        choice = String(created.id);
        saved = await api.get<DisguiseView[]>('/disguises');
      }
      result = await api.post<DisguiseCheck>(`/nodes/${node.id}/disguise/check`, { disguise_id: Number(choice) });
    } catch (failure) {
      const outcome = fieldErrors(failure);
      templateError = outcome.fields.template ?? '';
      error = outcome.form ?? '';
    } finally {
      busy = '';
    }
  }

  async function apply() {
    if (!result?.ready || busy) return;
    busy = 'apply';
    error = '';
    try {
      const updated = await api.put<NodeView>(`/nodes/${node.id}/disguise`, { disguise_id: Number(choice) });
      onsaved(updated);
      open = false;
    } catch (failure) {
      error = fieldErrors(failure).form ?? 'Could not apply the disguise.';
    } finally {
      busy = '';
    }
  }
</script>

<Dialog bind:open title={node.disguise_id === null ? 'Set TLS disguise' : 'Change TLS disguise'}>
  {#if error}<p class="note error banner" role="alert">{error}</p>{/if}
  {#if loading}
    <p class="muted">Loading saved disguises…</p>
  {:else if choice}
    <form id="check-disguise" class="stack" onsubmit={check}>
      <p class="lead">The disguise sets the TLS template in new Fiend links and the site the server uses for incoming connections without Fiend access.</p>
      <Field label="TLS disguise" id="disguise-choice">
        <Select id="disguise-choice" bind:value={choice} {options} label="TLS disguise" disabled={!!busy} onchange={() => { result = null; error = ''; templateError = ''; }} />
      </Field>
      {#if choice === 'new'}
        <TemplatePicker presets={availablePresets} bind:preset bind:file error={templateError} />
      {/if}
      {#if result}
        <div class="group" role="status">
          {#each result.checks as item (item.name)}
            <div class="group-row check-row">
              <span class="check-icon" class:ok={item.ok}><Icon name={item.ok ? 'check' : 'info'} size={15} /></span>
              <div class="setting-copy">
                <div class="setting-title">{item.name}</div>
                <p class="setting-desc">{item.detail}</p>
              </div>
            </div>
          {/each}
        </div>
        {#if result.ready && node.disguise_id !== null}
          <p class="note soft">Existing profiles keep their old TLS template. Share updated links after the server applies the change.</p>
        {/if}
      {/if}
    </form>
  {/if}
  {#snippet actions()}
    <Button kind="ghost" disabled={!!busy} onclick={() => (open = false)}>Cancel</Button>
    {#if choice}
      {#if result?.ready}
        <Button kind="primary" busy={busy === 'apply'} onclick={apply}>Apply disguise</Button>
      {:else}
        <Button kind="primary" type="submit" form="check-disguise" busy={busy === 'check'}>Check site from server</Button>
      {/if}
    {/if}
  {/snippet}
</Dialog>

<style>
  .banner { margin-bottom: 12px; }
  .lead { font-size: 12.5px; line-height: 1.5; }
  .check-row { justify-content: flex-start; align-items: flex-start; gap: 10px; min-height: 0; padding: 10px 14px; }
  .check-icon { margin-top: 2px; color: var(--warn); display: inline-flex; flex: none; }
  .check-icon.ok { color: var(--green); }
</style>
