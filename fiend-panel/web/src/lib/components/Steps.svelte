<script lang="ts">
  import type { JobStep } from '#lib/api/generated/index.ts';
  import Icon from './Icon.svelte';
  let { steps }: { steps: JobStep[] } = $props();
</script>

<ol class="steps">
  {#each steps as step (step.name)}
    <li class={step.state}>
      <span class="marker" aria-hidden="true">{#if step.state === 'done'}<Icon name="check" size={14} />{:else if step.state === 'failed'}<Icon name="close" size={14} />{:else if step.state === 'running'}<span class="spin"></span>{/if}</span>
      <span class="text"><span class="name">{step.name}</span><span class="visually-hidden">: {step.state}</span>{#if step.detail}<span class="detail">{step.detail}</span>{/if}</span>
    </li>
  {/each}
</ol>

<style>
  .steps { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 10px; }
  li { display: flex; gap: 10px; align-items: flex-start; color: var(--muted); }
  li.running, li.done { color: var(--text); }
  li.failed { color: var(--danger); }
  .marker { width: 20px; height: 20px; border-radius: 50%; border: 1px solid var(--line); display: inline-flex; align-items: center; justify-content: center; flex: none; }
  .done .marker { background: var(--green-soft); color: var(--green); border-color: transparent; }
  .failed .marker { background: var(--danger-soft); border-color: transparent; }
  .spin { width: 10px; height: 10px; border: 2px solid var(--accent); border-right-color: transparent; border-radius: 50%; animation: spin .8s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .text { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .detail { font-size: 11px; overflow-wrap: anywhere; }
</style>
