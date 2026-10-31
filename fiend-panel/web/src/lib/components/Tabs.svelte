<script lang="ts">
  let { value = $bindable(), tabs, label, onchange }: { value: string; tabs: { value: string; label: string; count?: number }[]; label: string; onchange?: (value: string) => void } = $props();
  let list: HTMLDivElement;
  function key(event: KeyboardEvent, index: number) {
    let next = index;
    if (event.key === 'ArrowRight') next = (index + 1) % tabs.length;
    else if (event.key === 'ArrowLeft') next = (index - 1 + tabs.length) % tabs.length;
    else if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = tabs.length - 1;
    else return;
    event.preventDefault();
    value = tabs[next].value;
    onchange?.(value);
    (list.querySelectorAll('[role=tab]')[next] as HTMLElement).focus();
  }
</script>

<div class="tabs" role="tablist" aria-label={label} bind:this={list}>
  {#each tabs as tab, index (tab.value)}
    <button type="button" role="tab" aria-selected={tab.value === value} tabindex={tab.value === value ? 0 : -1} onclick={() => { value = tab.value; onchange?.(value); }} onkeydown={(event) => key(event, index)}>
      {tab.label}{#if tab.count !== undefined}<span class="count">{tab.count}</span>{/if}
    </button>
  {/each}
</div>

<style>
  .tabs { display: flex; gap: 2px; padding: 2px; border-radius: 7px; background: var(--soft); border: 1px solid var(--line); width: max-content; max-width: 100%; overflow-x: auto; }
  button { padding: 4px 10px; border-radius: 5px; color: var(--muted); font-size: 12px; white-space: nowrap; display: inline-flex; gap: 6px; align-items: center; }
  button[aria-selected=true] { background: var(--panel); color: var(--text); box-shadow: inset 0 0 0 1px var(--line); }
  .count { font-size: 10px; color: var(--muted); }
</style>
