<script lang="ts">
  import Icon from './Icon.svelte';
  let { icon, label, onclick, pressed, disabled = false }: { icon: string; label: string; onclick?: (event: MouseEvent) => void; pressed?: boolean; disabled?: boolean } = $props();
</script>

<button type="button" class="icon-button" aria-label={label} title={label} aria-pressed={pressed} {disabled} {onclick}><Icon name={icon} size={16} /></button>

<style>
  .icon-button { width: 30px; height: 30px; display: inline-flex; align-items: center; justify-content: center; border-radius: 6px; color: var(--muted); flex: none; }
  .icon-button:hover:not(:disabled) { background: var(--hover); color: var(--text); }
</style>
