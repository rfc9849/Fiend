<script lang="ts">
  let { id, checked = $bindable(), label, disabled = false, onchange }: { id: string; checked: boolean; label: string; disabled?: boolean; onchange?: (value: boolean) => void } = $props();
</script>

<button {id} type="button" class="switch" role="switch" aria-checked={checked} {disabled} onclick={() => { checked = !checked; onchange?.(checked); }}>
  <span class="track" aria-hidden="true"><span class="thumb"></span></span>
  <span>{label}</span>
</button>

<style>
  .switch { display: inline-flex; align-items: center; gap: 10px; color: var(--text); min-height: 28px; text-align: left; }
  .track { width: 32px; height: 18px; border-radius: 9px; background: var(--line); position: relative; flex: none; transition: background .15s; }
  .thumb { position: absolute; top: 2px; left: 2px; width: 14px; height: 14px; border-radius: 50%; background: var(--panel); transition: transform .15s; }
  .switch[aria-checked=true] .track { background: var(--accent); }
  .switch[aria-checked=true] .thumb { transform: translateX(14px); }
</style>
