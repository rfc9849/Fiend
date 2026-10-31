<script lang="ts">
  import Field from './Field.svelte';
  import Select from './Select.svelte';
  import type { PresetView } from '#lib/api/generated/index.ts';
  let {
    presets,
    preset = $bindable(),
    file = $bindable(),
    error = ''
  }: { presets: PresetView[]; preset: string; file: string | null; error?: string } = $props();
  let fileName = $state('');
  let fileError = $state('');
  const options = $derived([...presets.map((item) => ({ value: item.id, label: item.name, detail: item.domain })), { value: 'file', label: 'Import a file', detail: '.bin or .json' }]);

  async function pick(event: Event) {
    fileError = '';
    const input = event.currentTarget as HTMLInputElement;
    const chosen = input.files?.[0];
    if (!chosen) return;
    if (chosen.size > 128 * 1024) {
      fileError = 'The file is too large. A template is at most 128 KB.';
      return;
    }
    const data = new Uint8Array(await chosen.arrayBuffer());
    let text = '';
    for (const byte of data) text += String.fromCharCode(byte);
    file = btoa(text);
    fileName = chosen.name;
  }
</script>

<Field label="TLS template" id="template" error={error || fileError} help="A preset, a ClientHello capture (.bin), or a template file (.json).">
  <Select id="template" bind:value={preset} {options} label="TLS template" />
</Field>
{#if preset === 'file'}
  <div class="file">
    <label class="pick" for="template-file">Choose a file</label>
    <input id="template-file" class="visually-hidden" type="file" accept=".bin,.json,application/json,application/octet-stream" onchange={pick} />
    <span class="muted">{fileName || 'No file selected'}</span>
  </div>
{/if}

<style>
  .file { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .pick { display: inline-flex; align-items: center; min-height: 32px; padding: 6px 12px; border: 1px solid var(--line); border-radius: 6px; background: var(--panel); cursor: pointer; font-size: 12px; font-weight: 500; }
  .pick:hover { background: var(--hover); }
  .file:focus-within .pick { outline: 2px solid var(--accent); outline-offset: 2px; }
</style>
