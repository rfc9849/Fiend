<script lang="ts">
  import Dialog from './Dialog.svelte';
  import Button from './Button.svelte';
  let { open = $bindable(), title, message = '', action = 'Confirm', danger = true, onconfirm }: { open: boolean; title: string; message?: string; action?: string; danger?: boolean; onconfirm: () => Promise<void> | void } = $props();
  let busy = $state(false);
  let error = $state('');
  async function confirm() {
    busy = true;
    error = '';
    try {
      await onconfirm();
      open = false;
    } catch (failure) {
      error = failure instanceof Error ? failure.message : 'The action failed.';
    } finally {
      busy = false;
    }
  }
</script>

<Dialog bind:open {title}>
  {#if message}<p class="message">{message}</p>{/if}
  {#if error}<p class="note error" role="alert">{error}</p>{/if}
  {#snippet actions()}
    <Button kind="ghost" onclick={() => (open = false)}>Cancel</Button>
    <Button kind={danger ? 'danger' : 'primary'} {busy} onclick={confirm}>{action}</Button>
  {/snippet}
</Dialog>

<style>
  .message { color: var(--text); line-height: 1.6; margin-bottom: 10px; }
</style>
