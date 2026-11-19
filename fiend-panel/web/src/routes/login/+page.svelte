<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { tick } from 'svelte';
  import Centered from '#lib/components/Centered.svelte';
  import Field from '#lib/components/Field.svelte';
  import Button from '#lib/components/Button.svelte';
  import { ApiError, api, fieldErrors, loadSession } from '#lib/api/client.ts';
  import type { LoginChallenge, LoginTotp } from '#lib/api/generated/index.ts';
  import { session } from '#lib/stores/session.svelte.ts';

  let name = $state('');
  let password = $state('');
  let challenge = $state('');
  let code = $state('');
  let busy = $state(false);
  let error = $state('');
  let fields = $state<Record<string, string>>({});
  let passwordInput = $state<HTMLInputElement>();
  let codeInput = $state<HTMLInputElement>();

  async function finish() {
    const info = await loadSession();
    if (!info) throw new ApiError(401, 'Sign in again to continue.');
    session.info = info;
    const next = page.url.searchParams.get('next');
    await goto(next && next.startsWith('/') && !next.startsWith('//') ? next : '/');
  }

  async function submitPassword(event: SubmitEvent) {
    event.preventDefault();
    if (busy) return;
    fields = {};
    error = '';
    if (!name) fields.name = 'Enter your admin name.';
    if (!password) fields.password = 'Enter your password.';
    if (Object.keys(fields).length) return;

    busy = true;
    try {
      const result = await api.post<LoginChallenge | undefined>('/login', { name, password });
      if (result) {
        challenge = result.challenge;
      } else {
        await finish();
      }
    } catch (failure) {
      const result = fieldErrors(failure);
      error = result.form ?? '';
      fields = result.fields;
    } finally {
      password = '';
      busy = false;
    }
    if (challenge) {
      await tick();
      codeInput?.focus();
    }
  }

  async function startOver(message = '') {
    challenge = '';
    code = '';
    fields = {};
    error = message;
    await tick();
    passwordInput?.focus();
  }

  async function submitCode(event: SubmitEvent) {
    event.preventDefault();
    if (busy) return;
    fields = {};
    error = '';
    const value = code.replace(/\s/g, '');
    if (!/^[0-9]{6}$/.test(value)) {
      fields.code = 'Enter the six-digit code.';
      codeInput?.focus();
      return;
    }

    busy = true;
    try {
      const verification: LoginTotp = { challenge, code: value };
      await api.post('/login/totp', verification);
      await finish();
    } catch (failure) {
      busy = false;
      if (failure instanceof ApiError && failure.status === 401) {
        await startOver(failure.message);
      } else {
        const result = fieldErrors(failure);
        error = result.form ?? '';
        fields = result.fields;
        if (fields.code) {
          await tick();
          codeInput?.focus();
          codeInput?.select();
        }
      }
    } finally {
      busy = false;
    }
  }
</script>

<svelte:head><title>{challenge ? 'Two-factor verification' : 'Sign in'} · fiend</title></svelte:head>

<Centered>
  {#if challenge}
    <h1>Two-factor verification</h1>
    <p class="intro help">Enter the six-digit code from your authenticator app.</p>
    <form novalidate onsubmit={submitCode} class="stack">
      {#if error}<p class="note error" role="alert">{error}</p>{/if}
      <Field label="Verification code" id="code" error={fields.code}>
        <input id="code" class="code" inputmode="numeric" autocomplete="one-time-code" spellcheck="false" bind:this={codeInput} bind:value={code} readonly={busy} aria-invalid={!!fields.code} aria-describedby={fields.code ? 'code-error' : undefined} />
      </Field>
      <Button kind="primary" type="submit" {busy}>Verify and sign in</Button>
      <Button kind="ghost" disabled={busy} onclick={() => startOver()}>Back to sign in</Button>
    </form>
  {:else}
    <h1>Sign in</h1>
    <form novalidate onsubmit={submitPassword} class="stack">
      {#if error}<p class="note error" role="alert">{error}</p>{/if}
      <Field label="Name" id="name" error={fields.name}><input id="name" autocomplete="username" bind:value={name} readonly={busy} aria-invalid={!!fields.name} /></Field>
      <Field label="Password" id="password" error={fields.password}><input id="password" type="password" autocomplete="current-password" bind:this={passwordInput} bind:value={password} readonly={busy} aria-invalid={!!fields.password} /></Field>
      <Button kind="primary" type="submit" {busy}>Sign in</Button>
    </form>
  {/if}
</Centered>

<style>
  h1 { margin-bottom: 18px; }
  .intro { margin: -8px 0 18px; }
  .code { font: 500 20px var(--mono); letter-spacing: .24em; }
</style>
