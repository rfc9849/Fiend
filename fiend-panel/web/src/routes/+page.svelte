<script lang="ts">
  import serverIcon from '@icons/server.svg?raw';
</script>

<svelte:head>
  <title>fiend — server panel</title>
  <meta name="description" content="Private fiend server panel" />
</svelte:head>

<main>
  <header><span class="wordmark">fiend</span><span class="label">server panel</span></header>
  <section aria-labelledby="title">
    <span class="icon">{@html serverIcon}</span>
    <h1 id="title">Panel package installed</h1>
    <p>This page checks the private panel connection.</p>
    <p>Node management and account setup are not available in this build.</p>
    <div class="note">Keep this panel on loopback. Use an SSH tunnel to open it.</div>
  </section>
</main>

<style>
  :global(:root) { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; color: #242b30; background: #e3e5e6; font-size: 13px; color-scheme: light dark; }
  :global(body) { margin: 0; }
  :global(*) { box-sizing: border-box; }
  main { min-height: 100dvh; max-width: 960px; margin: auto; background: #fafbfb; padding: 32px; }
  header { display: flex; gap: 16px; align-items: baseline; }
  .wordmark { font-size: 26px; font-weight: 600; letter-spacing: -1px; }
  .label, p { color: #677179; }
  section { margin-top: 48px; padding: 24px; background: #fff; border: 1px solid #d9dfe3; border-radius: 9px; }
  h1 { font-size: 24px; font-weight: 600; letter-spacing: -.6px; }
  p { line-height: 1.6; }
  .icon { display: block; width: 24px; height: 24px; }
  .icon :global(svg) { width: 24px; height: 24px; }
  .note { margin-top: 24px; padding: 12px; border-radius: 6px; background: #e6edf3; color: #365f87; line-height: 1.6; }
  @media (prefers-color-scheme: dark) {
    :global(:root) { color: #e2e7eb; background: #101214; }
    main { background: #1b1e21; }
    section { background: #22262a; border-color: #353d43; }
    .label, p { color: #a0aab3; }
    .note { background: #293946; color: #a0bdd6; }
  }
  @media (max-width: 720px) { main { padding: 20px; } section { padding: 20px; margin-top: 32px; } }
</style>
