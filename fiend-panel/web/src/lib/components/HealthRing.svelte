<script lang="ts">
  let { status, size = 22 }: { status: string; size?: number } = $props();
  const tone = $derived(status === 'online' ? 'green' : status === 'setup' || status === 'unknown' ? 'warn' : 'danger');
  const arc = $derived(status === 'online' ? 100 : status === 'setup' || status === 'unknown' ? 60 : 30);
</script>

<svg class="ring {tone}" width={size} height={size} viewBox="0 0 36 36" aria-hidden="true">
  <circle cx="18" cy="18" r="15" fill="none" stroke="var(--dial-edge)" stroke-width="3" />
  <circle class="arc" cx="18" cy="18" r="15" fill="none" stroke-width="3" stroke-linecap="round" pathLength="100" stroke-dasharray="{arc} 100" transform="rotate(-90 18 18)" />
</svg>

<style>
  .ring { flex: none; }
  .green .arc { stroke: var(--green); }
  .warn .arc { stroke: var(--warn); }
  .danger .arc { stroke: var(--danger); }
  .arc { transition: stroke-dasharray .4s; }
</style>
