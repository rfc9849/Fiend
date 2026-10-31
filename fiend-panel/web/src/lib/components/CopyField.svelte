<script lang="ts">
  import Icon from './Icon.svelte';
  let { value, label, secret = false, onreveal }: { value: string; label: string; secret?: boolean; onreveal?: () => Promise<string> } = $props();
  let copied = $state(false);
  let shown = $state(false);
  let revealed = $state('');
  const text = $derived(secret ? (shown ? revealed || value : '•'.repeat(24)) : value);

  async function full(): Promise<string> {
    if (onreveal && !revealed) revealed = await onreveal();
    return revealed || value;
  }

  async function copy() {
    try {
      await navigator.clipboard.writeText(await full());
      copied = true;
      setTimeout(() => (copied = false), 2000);
    } catch {
      copied = false;
    }
  }

  async function toggle() {
    if (!shown) await full();
    shown = !shown;
  }
</script>

<div class="copy">
  <span class="value mono" aria-label={label}>{text}</span>
  {#if secret}
    <button type="button" class="action" aria-label={shown ? `Hide ${label}` : `Show ${label}`} aria-pressed={shown} onclick={toggle}><Icon name="eye" size={16} /></button>
  {/if}
  <button type="button" class="action" aria-label="Copy {label}" onclick={copy}>
    {#if copied}<span class="copied">Copied</span>{:else}<Icon name="copy" size={16} />{/if}
  </button>
</div>

<style>
  .copy { display: flex; align-items: center; gap: 4px; min-height: 34px; padding: 3px 4px 3px 10px; border: 1px solid var(--line); border-radius: 6px; background: var(--soft); min-width: 0; }
  .value { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .action { min-width: 28px; height: 28px; display: inline-flex; align-items: center; justify-content: center; border-radius: 5px; color: var(--muted); padding: 0 4px; }
  .action:hover { background: var(--hover); color: var(--text); }
  .copied { font-size: 11px; color: var(--green); font-weight: 500; }
</style>
