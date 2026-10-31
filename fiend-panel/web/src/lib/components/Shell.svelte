<script lang="ts">
  import type { Snippet } from 'svelte';
  import { page } from '$app/state';
  import Brand from './Brand.svelte';
  import Icon from './Icon.svelte';
  let { children, admin }: { children: Snippet; admin: string } = $props();
  let menu = $state(false);
  const items = [
    { href: '/', label: 'Dashboard', icon: 'home' },
    { href: '/nodes', label: 'Nodes', icon: 'server' },
    { href: '/users', label: 'Users', icon: 'key' },
    { href: '/disguises', label: 'Disguises', icon: 'globe' },
    { href: '/routing', label: 'Routing', icon: 'route' }
  ];
  const current = (href: string) => (href === '/' ? page.url.pathname === '/' : page.url.pathname.startsWith(href));
</script>

<a class="skip" href="#content">Skip to content</a>
<div class="shell">
  <aside class="sidebar" class:open={menu}>
    <div class="top">
      <a class="home" href="/" aria-label="fiend dashboard"><Brand /></a>
      <button type="button" class="menu" aria-expanded={menu} aria-controls="main-nav" aria-label={menu ? 'Close menu' : 'Open menu'} onclick={() => (menu = !menu)}>
        <Icon name={menu ? 'close' : 'log'} size={18} />
      </button>
    </div>
    <nav id="main-nav" aria-label="Main navigation">
      {#each items as item (item.href)}
        <a class="item" href={item.href} aria-current={current(item.href) ? 'page' : undefined} onclick={() => (menu = false)}>
          <Icon name={item.icon} size={16} /><span>{item.label}</span>
        </a>
      {/each}
      <div class="bottom">
        <a class="item" href="/settings" aria-current={current('/settings') ? 'page' : undefined} onclick={() => (menu = false)}>
          <Icon name="settings" size={16} /><span>Settings</span>
        </a>
        <p class="who">Signed in as {admin}</p>
      </div>
    </nav>
  </aside>
  <main id="content" tabindex="-1">
    <div class="content">{@render children()}</div>
  </main>
</div>

<style>
  .skip { position: absolute; left: 8px; top: -40px; z-index: 100; background: var(--panel); padding: 6px 10px; border-radius: 6px; }
  .skip:focus { top: 8px; }
  .shell { display: grid; grid-template-columns: 208px minmax(0, 1fr); min-height: 100dvh; background: linear-gradient(to right, var(--sidebar) 208px, var(--surface) 208px); }
  .sidebar { background: var(--sidebar); padding: 14px 10px; display: flex; flex-direction: column; position: sticky; top: 0; height: 100dvh; }
  .top { display: flex; align-items: center; justify-content: space-between; padding: 4px 10px 26px; }
  .home { text-decoration: none; }
  .menu { display: none; width: 34px; height: 34px; border-radius: 6px; align-items: center; justify-content: center; color: var(--muted); }
  .menu:hover { background: var(--hover); }
  nav { display: flex; flex-direction: column; gap: 3px; flex: 1; }
  .item { display: flex; align-items: center; gap: 10px; height: 36px; padding: 0 12px; border-radius: 4px; color: var(--muted); text-decoration: none; font-size: 13px; }
  .item:hover { background: var(--hover); color: var(--text); }
  .item[aria-current=page] { background: var(--panel); color: var(--text); font-weight: 500; box-shadow: inset 0 0 0 1px var(--line); }
  .bottom { margin-top: auto; display: flex; flex-direction: column; gap: 6px; }
  .who { font-size: 11px; padding: 4px 12px 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  main { background: var(--surface); min-width: 0; outline: none; }
  .content { max-width: 1280px; margin: 0 auto; padding: 26px 32px 48px; }

  @media (max-width: 1023px) {
    .shell { grid-template-columns: 64px minmax(0, 1fr); background: linear-gradient(to right, var(--sidebar) 64px, var(--surface) 64px); }
    .sidebar { padding: 14px 8px; }
    .top { padding: 4px 8px 26px; justify-content: center; }
    .top :global(.word) { display: none; }
    .item { justify-content: center; padding: 0; }
    .item span, .who { display: none; }
    .content { padding: 22px 24px 40px; }
  }

  @media (max-width: 719px) {
    .shell { grid-template-columns: minmax(0, 1fr); grid-template-rows: auto 1fr; background: var(--surface); }
    .sidebar { position: sticky; top: 0; z-index: 40; height: auto; padding: 8px 12px; }
    .top { padding: 0; justify-content: space-between; }
    .top :global(.word) { display: inline; }
    .menu { display: inline-flex; }
    nav { display: none; padding-top: 10px; }
    .sidebar.open nav { display: flex; }
    .item { justify-content: flex-start; padding: 0 12px; }
    .item span, .who { display: inline; }
    .bottom { margin-top: 6px; }
    .content { padding: 18px 14px 36px; }
  }
</style>
