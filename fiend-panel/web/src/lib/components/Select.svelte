<script lang="ts">
  import Icon from './Icon.svelte';
  type Option = { value: string; label: string; detail?: string };
  let {
    id,
    value = $bindable(),
    options,
    label = '',
    placeholder = 'Choose',
    disabled = false,
    onchange
  }: { id: string; value: string; options: Option[]; label?: string; placeholder?: string; disabled?: boolean; onchange?: (value: string) => void } = $props();

  let open = $state(false);
  let active = $state(0);
  let trigger: HTMLDivElement;
  let typed = '';
  let typedAt = 0;
  const selected = $derived(options.find((option) => option.value === value));

  function show() {
    if (disabled) return;
    open = true;
    active = Math.max(0, options.findIndex((option) => option.value === value));
  }

  function choose(index: number) {
    const option = options[index];
    if (!option) return;
    value = option.value;
    open = false;
    onchange?.(option.value);
    trigger.focus();
  }

  function keydown(event: KeyboardEvent) {
    if (disabled) return;
    const last = options.length - 1;
    if (!open) {
      if (['ArrowDown', 'ArrowUp', 'Enter', ' ', 'Home', 'End'].includes(event.key)) {
        event.preventDefault();
        show();
        if (event.key === 'Home') active = 0;
        if (event.key === 'End') active = last;
      } else if (event.key.length === 1) {
        search(event.key);
      }
      return;
    }
    switch (event.key) {
      case 'ArrowDown': event.preventDefault(); active = Math.min(last, active + 1); break;
      case 'ArrowUp': event.preventDefault(); active = Math.max(0, active - 1); break;
      case 'Home': event.preventDefault(); active = 0; break;
      case 'End': event.preventDefault(); active = last; break;
      case 'PageDown': event.preventDefault(); active = Math.min(last, active + 10); break;
      case 'PageUp': event.preventDefault(); active = Math.max(0, active - 10); break;
      case 'Enter': case ' ': event.preventDefault(); choose(active); break;
      case 'Escape': event.preventDefault(); open = false; break;
      case 'Tab': choose(active); break;
      default: if (event.key.length === 1) search(event.key);
    }
  }

  function search(key: string) {
    const now = Date.now();
    typed = now - typedAt > 600 ? key.toLowerCase() : typed + key.toLowerCase();
    typedAt = now;
    const index = options.findIndex((option) => option.label.toLowerCase().startsWith(typed));
    if (index >= 0) {
      if (open) active = index;
      else choose(index);
    }
  }
</script>

<div class="select" class:open>
  <div
    bind:this={trigger}
    {id}
    class="trigger"
    role="combobox"
    tabindex={disabled ? -1 : 0}
    aria-controls="{id}-list"
    aria-expanded={open}
    aria-haspopup="listbox"
    aria-label={label || undefined}
    aria-disabled={disabled || undefined}
    aria-activedescendant={open ? `${id}-option-${active}` : undefined}
    onclick={() => (open ? (open = false) : show())}
    onkeydown={keydown}
    onblur={() => (open = false)}
  >
    <span class:placeholder={!selected}>{selected?.label ?? placeholder}</span>
    <Icon name="chevron" size={14} />
  </div>
  {#if open}
    <ul id="{id}-list" role="listbox" tabindex="-1" aria-label={label || undefined}>
      {#each options as option, index (option.value)}
        <li
          id="{id}-option-{index}"
          role="option"
          aria-selected={option.value === value}
          class:active={index === active}
          onmousedown={(event) => { event.preventDefault(); choose(index); }}
          onmousemove={() => (active = index)}
        >
          <span>{option.label}</span>
          {#if option.detail}<span class="detail">{option.detail}</span>{/if}
          {#if option.value === value}<Icon name="check" size={14} />{/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .select { position: relative; min-width: 0; }
  .trigger { display: flex; align-items: center; justify-content: space-between; gap: 8px; min-height: 34px; padding: 6px 10px; border: 1px solid var(--line); border-radius: 6px; background: var(--panel); cursor: pointer; user-select: none; }
  .trigger[aria-disabled=true] { opacity: .5; cursor: not-allowed; }
  .trigger > span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .placeholder { color: var(--muted); }
  ul { position: absolute; z-index: 30; left: 0; right: 0; top: calc(100% + 4px); margin: 0; padding: 4px; list-style: none; background: var(--panel); border: 1px solid var(--line); border-radius: 8px; box-shadow: var(--shadow); max-height: 280px; overflow: auto; }
  li { display: flex; align-items: center; gap: 8px; padding: 6px 8px; border-radius: 4px; cursor: pointer; }
  li > span:first-child { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; }
  li.active { background: var(--hover); }
  .detail { color: var(--muted); font-size: 11px; font-family: var(--mono); }
</style>
