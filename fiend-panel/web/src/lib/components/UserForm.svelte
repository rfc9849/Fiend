<script lang="ts">
  import Field from './Field.svelte';
  import Select from './Select.svelte';
  import Switch from './Switch.svelte';
  import type { NodeView } from '#lib/api/generated/index.ts';

  let { draft = $bindable(), nodes, fields }: { draft: UserDraft; nodes: NodeView[]; fields: Record<string, string> } = $props();
</script>

<div class="stack">
  <Field label="Name" id="user-name" error={fields.name}><input id="user-name" bind:value={draft.name} /></Field>
  <Switch id="user-enabled" bind:checked={draft.enabled} label="Enabled" />
  <div class="grid-2">
    <Field label="Traffic limit" id="user-limit" error={fields.traffic_limit} help="For example 50 GB. Empty means no limit."><input id="user-limit" bind:value={draft.limit} placeholder="No limit" /></Field>
    <Field label="Reset usage" id="user-reset" error={fields.reset_period}>
      <Select id="user-reset" bind:value={draft.reset} label="Reset usage" options={[{ value: 'none', label: 'Never' }, { value: 'day', label: 'Every day' }, { value: 'week', label: 'Every week' }, { value: 'month', label: 'Every month' }]} />
    </Field>
  </div>
  <Field label="Expiry" id="user-start" error={fields.hold_days}>
    <Select id="user-start" bind:value={draft.start} label="Expiry" options={[{ value: 'never', label: 'Never expires' }, { value: 'date', label: 'On a date' }, { value: 'hold', label: 'Days after the first connect' }]} />
  </Field>
  {#if draft.start === 'date'}
    <Field label="Expiry date" id="user-expiry" error={fields.expire_at}><input id="user-expiry" type="date" bind:value={draft.expiry} /></Field>
  {:else if draft.start === 'hold'}
    <Field label="Days after the first connect" id="user-hold" error={fields.hold_days}><input id="user-hold" inputmode="numeric" bind:value={draft.holdDays} /></Field>
  {/if}
  <fieldset class="nodes">
    <legend>Nodes</legend>
    <p class="small muted">Select none to allow all nodes, also nodes that you add later.</p>
    {#each nodes as node (node.id)}
      <label class="check"><input type="checkbox" checked={draft.nodes.includes(node.id)} onchange={(event) => { const on = (event.currentTarget as HTMLInputElement).checked; draft.nodes = on ? [...draft.nodes, node.id] : draft.nodes.filter((item) => item !== node.id); }} />{node.name}</label>
    {/each}
    {#if fields.nodes}<p class="error">{fields.nodes}</p>{/if}
  </fieldset>
  <Field label="Note" id="user-note" error={fields.note}><textarea id="user-note" bind:value={draft.note}></textarea></Field>
</div>

<script lang="ts" module>
  import type { UserView as View } from '#lib/api/generated/index.ts';
  import { parseBytes } from '#lib/format.ts';

  export type UserDraft = { name: string; enabled: boolean; note: string; limit: string; expiry: string; holdDays: string; reset: string; nodes: number[]; start: string };

  export function toDraft(user: View | null): UserDraft {
    return {
      name: user?.name ?? '',
      enabled: user?.enabled ?? true,
      note: user?.note ?? '',
      limit: user?.traffic_limit ? `${+(user.traffic_limit / 1e9).toFixed(3)} GB` : '',
      expiry: user?.expire_at && !user.hold_days ? new Date(user.expire_at * 1000).toISOString().slice(0, 10) : '',
      holdDays: user?.hold_days ? String(user.hold_days) : '30',
      reset: user?.reset_period ?? 'none',
      nodes: user?.nodes ?? [],
      start: user?.hold_days ? 'hold' : user?.expire_at ? 'date' : 'never'
    };
  }

  export function toInput(draft: UserDraft): { body: Record<string, unknown> | null; fields: Record<string, string> } {
    const fields: Record<string, string> = {};
    if (!draft.name.trim()) fields.name = 'Enter a name.';
    let limit: number | null = null;
    if (draft.limit.trim()) {
      limit = parseBytes(draft.limit);
      if (!limit) fields.traffic_limit = 'Enter a size such as 50 GB.';
    }
    let expire: number | null = null;
    if (draft.start === 'date') {
      const time = Date.parse(`${draft.expiry}T23:59:59`);
      if (Number.isNaN(time)) fields.expire_at = 'Choose a date.';
      else expire = Math.floor(time / 1000);
    }
    let hold: number | null = null;
    if (draft.start === 'hold') {
      hold = Number(draft.holdDays);
      if (!Number.isInteger(hold) || hold < 1 || hold > 3650) fields.hold_days = 'Enter 1 to 3650 days.';
    }
    if (Object.keys(fields).length) return { body: null, fields };
    return { body: { name: draft.name.trim(), enabled: draft.enabled, note: draft.note, traffic_limit: limit, expire_at: expire, hold_days: hold, reset_period: draft.reset, nodes: draft.nodes }, fields };
  }
</script>

<style>
  .nodes { border: 1px solid var(--line); border-radius: 6px; padding: 10px 12px; margin: 0; display: flex; flex-direction: column; gap: 6px; }
  legend { font-weight: 500; font-size: 12px; padding: 0 4px; }
  .check { display: flex; gap: 8px; align-items: center; }
  .check input { width: 16px; height: 16px; accent-color: var(--accent); }
  .error { color: var(--danger); font-size: 11px; }
</style>
