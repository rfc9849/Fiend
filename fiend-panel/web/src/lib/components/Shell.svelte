<script lang="ts">
  import type { Snippet } from 'svelte';
  import { page } from '$app/state';
  import Brand from './Brand.svelte';
  import Icon from './Icon.svelte';
  import { api } from '#lib/api/client.ts';
  import { initials } from '#lib/format.ts';
  let { children, admin, version = '' }: { children: Snippet; admin: string; version?: string } = $props();
  let menu = $state(false);
  const items = [
    { href: '/', label: 'Dashboard', icon: 'home' },
    { href: '/nodes', label: 'Server', icon: 'server' },
    { href: '/users', label: 'Users', icon: 'users' },
    { href: '/routing', label: 'Routing', icon: 'route' }
  ];
  async function signOut() {
    try {
      await api.post('/logout');
    } finally {
      location.assign('/login');
    }
  }
  const current = (href: string) => (href === '/' ? page.url.pathname === '/' : page.url.pathname.startsWith(href));
</script>

<a class="skip" href="#content">Skip to content</a>
<div class="shell">
  <aside class="sidebar" class:open={menu}>
    <div class="top">
      <a class="home" href="/" aria-label="fiend dashboard"><Brand /></a>
      <button type="button" class="menu" aria-expanded={menu} aria-controls="main-nav" aria-label={menu ? 'Close menu' : 'Open menu'} onclick={() => (menu = !menu)}>
        <Icon name={menu ? 'close' : 'menu'} size={18} />
      </button>
    </div>
    <nav id="main-nav" aria-label="Main navigation">
      <div class="group-nav">
        {#each items as item (item.href)}
          <a class="item" href={item.href} aria-current={current(item.href) ? 'page' : undefined} onclick={() => (menu = false)}>
            <Icon name={item.icon} size={16} /><span>{item.label}</span>
          </a>
        {/each}
      </div>
      <div class="bottom">
        <a class="item" href="/settings" aria-current={current('/settings') ? 'page' : undefined} onclick={() => (menu = false)}>
          <Icon name="settings" size={16} /><span>Settings</span>
        </a>
        <button type="button" class="item" aria-label="Sign out" onclick={signOut}>
          <Icon name="power" size={16} /><span>Sign out</span>
        </button>
        <div class="account">
          <span class="avatar sm" aria-hidden="true">{initials(admin)}</span>
          <div class="account-meta">
            <p class="who">{admin}</p>
            {#if version}<p class="ver">fiend {version}</p>{/if}
          </div>
        </div>
      </div>
    </nav>
  </aside>
  <main id="content" tabindex="-1">
    <div class="content">{@render children()}</div>
  </main>
</div>

<style>
  .skip { position: absolute; left: 8px; top: -40px; z-index: 100; background: var(--panel); padding: 6px 10px; border-radius: 6px; border: 1px solid var(--line); }
  .skip:focus { top: 8px; }
  .shell { display: grid; grid-template-columns: 212px minmax(0, 1fr); min-height: 100dvh; background: var(--surface); }
  .sidebar { background: var(--sidebar); border-right: 1px solid var(--line); padding: 14px 10px 12px; display: flex; flex-direction: column; position: sticky; top: 0; height: 100dvh; }
  .top { display: flex; align-items: center; justify-content: space-between; padding: 4px 10px 24px; }
  .home { text-decoration: none; }
  .menu { display: none; width: 34px; height: 34px; border-radius: 6px; align-items: center; justify-content: center; color: var(--muted); }
  .menu:hover { background: var(--hover); }
  nav { display: flex; flex-direction: column; flex: 1; min-height: 0; }
  .group-nav { display: flex; flex-direction: column; gap: 3px; }
  .item { display: flex; align-items: center; gap: 10px; height: 36px; padding: 0 11px; border-radius: 5px; color: var(--muted); text-decoration: none; font-size: 13px; width: 100%; text-align: left; transition: background .12s, color .12s; }
  .item:hover { background: var(--hover); color: var(--text); }
  .item[aria-current=page] { background: var(--panel); color: var(--text); font-weight: 500; box-shadow: inset 0 0 0 1px var(--line); }
  .bottom { margin-top: auto; padding-top: 10px; border-top: 1px solid var(--line); display: flex; flex-direction: column; gap: 3px; }
  .account { display: flex; align-items: center; gap: 9px; margin-top: 8px; padding: 8px 9px 2px; border-top: 1px solid var(--line); min-width: 0; }
  .avatar.sm { width: 26px; height: 26px; font-size: 10px; border-radius: 4px; background: var(--panel); }
  .account-meta { min-width: 0; flex: 1; }
  .who { font-size: 12px; font-weight: 500; color: var(--text); line-height: 1.25; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ver { font-size: 10.5px; color: var(--muted); line-height: 1.3; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  main { background: var(--surface); min-width: 0; outline: none; }
  .content { max-width: 1240px; margin: 0 auto; padding: 26px 32px 52px; }

  @media (max-width: 1023px) {
    .shell { grid-template-columns: 64px minmax(0, 1fr); }
    .sidebar { padding: 14px 8px; }
    .top { padding: 4px 8px 24px; justify-content: center; }
    .top :global(.word) { display: none; }
    .item { justify-content: center; padding: 0; }
    .item span, .account-meta { display: none; }
    .account { justify-content: center; padding: 8px 0 2px; }
    .content { padding: 22px 24px 40px; }
  }

  @media (max-width: 719px) {
    .shell { grid-template-columns: minmax(0, 1fr); grid-template-rows: auto 1fr; }
    .sidebar { position: sticky; top: 0; z-index: 40; height: auto; padding: 8px 12px; border-right: 0; border-bottom: 1px solid var(--line); }
    .top { padding: 0; justify-content: space-between; }
    .top :global(.word) { display: inline; }
    .menu { display: inline-flex; }
    nav { display: none; padding-top: 10px; }
    .sidebar.open nav { display: flex; }
    .item { justify-content: flex-start; padding: 0 11px; }
    .item span, .account-meta { display: block; }
    .account { justify-content: flex-start; padding: 8px 9px 2px; }
    .bottom { margin-top: 8px; }
    .content { padding: 18px 14px 36px; }
  }
</style>
