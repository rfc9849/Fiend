<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import Centered from '#lib/components/Centered.svelte';
  import Field from '#lib/components/Field.svelte';
  import Button from '#lib/components/Button.svelte';
  import { api, fieldErrors, loadSession } from '#lib/api/client.ts';
  import { session } from '#lib/stores/session.svelte.ts';

  let name = $state('');
  let password = $state('');
  let code = $state('');
  let busy = $state(false);
  let error = $state('');
  let fields = $state<Record<string, string>>({});

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    fields = {};
    error = '';
    if (!name) fields.name = 'Enter your admin name.';
    if (!password) fields.password = 'Enter your password.';
    if (Object.keys(fields).length) return;
    busy = true;
    try {
      await api.post('/login', { name, password, totp_code: code || null });
      session.info = await loadSession();
      const next = page.url.searchParams.get('next');
      await goto(next && next.startsWith('/') && !next.startsWith('//') ? next : '/');
    } catch (failure) {
      const result = fieldErrors(failure);
      error = result.form ?? '';
      fields = result.fields;
    } finally {
      busy = false;
    }
  }
</script>

<svelte:head><title>Sign in · fiend</title></svelte:head>

<Centered>
  <h1>Sign in</h1>
  <p class="lead">Use the admin account of this panel.</p>
  <form novalidate onsubmit={submit} class="stack">
    {#if error}<p class="note error" role="alert">{error}</p>{/if}
    <Field label="Name" id="name" error={fields.name}><input id="name" autocomplete="username" bind:value={name} aria-invalid={!!fields.name} /></Field>
    <Field label="Password" id="password" error={fields.password}><input id="password" type="password" autocomplete="current-password" bind:value={password} aria-invalid={!!fields.password} /></Field>
    <Field label="Two-factor code" id="code" help="Leave empty if two-factor sign-in is off." error={fields.totp_code}><input id="code" inputmode="numeric" autocomplete="one-time-code" maxlength="8" bind:value={code} /></Field>
    <Button kind="primary" type="submit" {busy}>Sign in</Button>
  </form>
</Centered>

<style>
  .lead { margin: 6px 0 18px; }
</style>
