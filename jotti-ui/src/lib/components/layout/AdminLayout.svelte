<script lang="ts">
  import { page } from '$app/stores';
  import { goto, afterNavigate } from '$app/navigation';
  import { navigating } from '$app/stores';
  import { auth, currentUser } from '$stores/auth';
  import OrbitIcon from '$lib/components/ui/OrbitIcon.svelte';
  import { theme, toggleTheme } from '$stores/theme';
  import { onMount } from 'svelte';
  import { api } from '$api/client';
  import type { Notification } from '$api/client';
  import { currentLang, setLanguage } from '$lib/i18n';

  // ── Page content fade-up on navigation ───────────────────────────────────────
  let contentKey = 0;
  afterNavigate(() => { contentKey++; });

  // ── Navigation groups ─────────────────────────────────────────────────────
  const navGroups = [
    {
      label: 'Overview',
      color: 'text-blue-500',
      items: [
        { href: '/dashboard',   label: 'Dashboard', icon: 'home'      },
        { href: '/websites',    label: 'Websites',  icon: 'globe'     },
        { href: '/servers',     label: 'Servers',   icon: 'server'    },
      ],
    },
    {
      label: 'Hosting',
      color: 'text-emerald-500',
      items: [
        { href: '/databases',   label: 'Databases', icon: 'database'  },
        { href: '/email',       label: 'Email',     icon: 'mail'      },
        { href: '/dns',         label: 'DNS',       icon: 'globe2'    },
        { href: '/ssl',         label: 'SSL',       icon: 'lock'      },
        { href: '/filemanager', label: 'Files',     icon: 'folder'    },
      ],
    },
    {
      label: 'Tools',
      color: 'text-violet-500',
      items: [
        { href: '/backups',     label: 'Backups',   icon: 'cloud-up'  },
        { href: '/apps',        label: 'Apps',      icon: 'grid'      },
        { href: '/cron',        label: 'Cron',      icon: 'clock'     },
        { href: '/php',         label: 'PHP',       icon: 'code'      },
        { href: '/cache',       label: 'Cache',     icon: 'zap'       },
        { href: '/webhooks',    label: 'Webhooks',  icon: 'webhook'   },
        { href: '/cloudflare',  label: 'Cloudflare',icon: 'cloud'     },
        { href: '/migration',   label: 'Migration', icon: 'import'    },
        { href: '/plugins',     label: 'Plugins',   icon: 'puzzle'    },
      ],
    },
    {
      label: 'Security',
      color: 'text-rose-500',
      items: [
        { href: '/security',    label: 'Security',  icon: 'shield'    },
        { href: '/firewall',    label: 'Firewall',  icon: 'shield-lock'},
      ],
    },
    {
      label: 'System',
      color: 'text-slate-400',
      items: [
        { href: '/logs',        label: 'Logs',      icon: 'doc-text'  },
        { href: '/audit-log',   label: 'Audit Log', icon: 'audit'     },
        { href: '/dbmanager',   label: 'DB Manager',icon: 'table'     },
        { href: '/users',       label: 'Users',     icon: 'users'     },
        { href: '/reseller',    label: 'Reseller',  icon: 'briefcase' },
        { href: '/settings',    label: 'Settings',  icon: 'cog'       },
      ],
    },
  ] as const;

  // ── State ─────────────────────────────────────────────────────────────────
  let notifications: Notification[] = [];
  let notifOpen     = false;
  let userMenuOpen  = false;
  let sidebarOpen   = false; // mobile

  // Impersonation — detect from localStorage
  let impersonationEmail: string | null = null;

  onMount(async () => {
    impersonationEmail = localStorage.getItem('orbit_impersonate_email');
    try {
      notifications = await api.notifications.list({ unread_only: true });
    } catch { /* ignore */ }
  });

  $: unreadCount = notifications.filter(n => !n.read_at).length;

  // ── Active route check ────────────────────────────────────────────────────
  // `currentPath` is a reactive dependency so the markup re-evaluates isActive()
  // on client-side (SPA) navigation, not just on full page reload.
  $: currentPath = $page.url.pathname;
  function isActive(href: string, path: string): boolean {
    if (href === '/dashboard') return path === '/dashboard';
    return path.startsWith(href);
  }

  // ── Breadcrumb ────────────────────────────────────────────────────────────
  $: breadcrumbs = $page.url.pathname
    .split('/')
    .filter(Boolean)
    .map(s => s.replace(/-/g, ' ').replace(/\b\w/g, c => c.toUpperCase()));

  // ── User initials ─────────────────────────────────────────────────────────
  $: userInitials = (() => {
    const email = $currentUser?.email ?? '';
    const parts  = email.split('@')[0].split(/[._-]/);
    if (parts.length >= 2) return (parts[0][0] + parts[1][0]).toUpperCase();
    return email.slice(0, 2).toUpperCase() || 'U';
  })();

  // ── Handlers ─────────────────────────────────────────────────────────────
  async function handleLogout() {
    userMenuOpen = false;
    await auth.logout();
  }

  function returnToAdmin() {
    localStorage.removeItem('orbit_impersonate_email');
    localStorage.removeItem('orbit_impersonate_token');
    impersonationEmail = null;
    const originalToken = localStorage.getItem('orbit_original_token');
    if (originalToken) {
      api.setToken(originalToken);
      localStorage.removeItem('orbit_original_token');
    }
    goto('/users');
  }

  function closeMenus() {
    notifOpen    = false;
    userMenuOpen = false;
  }

  function closeSidebar() {
    sidebarOpen = false;
  }

  // ── SVG icon paths ────────────────────────────────────────────────────────
  function icon(name: string): string {
    const p: Record<string, string> = {
      // navigation
      home:         'M3 9l9-7 9 7v11a2 2 0 01-2 2H5a2 2 0 01-2-2V9zM9 22V12h6v10',
      globe:        'M12 2a10 10 0 100 20A10 10 0 0012 2zm0 0c-2.5 2.5-4 6.2-4 10s1.5 7.5 4 10m0-20c2.5 2.5 4 6.2 4 10s-1.5 7.5-4 10M2 12h20',
      server:       'M2 3h20v6H2V3zm0 12h20v6H2v-6zM6 7h.01M6 19h.01',
      database:     'M12 3C7.58 3 4 4.12 4 5.5v13C4 19.88 7.58 21 12 21s8-1.12 8-2.5v-13C20 4.12 16.42 3 12 3zm6 15.5c0 .83-2.69 1.5-6 1.5s-6-.67-6-1.5V16c1.47.83 3.61 1.5 6 1.5s4.53-.67 6-1.5v2.5zm0-5c0 .83-2.69 1.5-6 1.5s-6-.67-6-1.5V11c1.47.83 3.61 1.5 6 1.5S18 11.83 18 11v2.5zM12 9c-3.31 0-6-.67-6-1.5S8.69 6 12 6s6 .67 6 1.5S15.31 9 12 9z',
      mail:         'M4 4h16c1.1 0 2 .9 2 2v12c0 1.1-.9 2-2 2H4c-1.1 0-2-.9-2-2V6c0-1.1.9-2 2-2zm0 2l8 5 8-5',
      globe2:       'M12 2a10 10 0 100 20A10 10 0 0012 2zM2 12h20M12 2c-4 4.5-4 11.5 0 20M12 2c4 4.5 4 11.5 0 20',
      lock:         'M19 11H5a2 2 0 00-2 2v7a2 2 0 002 2h14a2 2 0 002-2v-7a2 2 0 00-2-2zM7 11V7a5 5 0 0110 0v4',
      folder:       'M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2v11z',
      'cloud-up':   'M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4M17 8l-5-5-5 5M12 3v12',
      grid:         'M3 3h7v7H3zM14 3h7v7h-7zM14 14h7v7h-7zM3 14h7v7H3z',
      clock:        'M12 2a10 10 0 100 20A10 10 0 0012 2zm0 5v5l3 3',
      code:         'M16 18l6-6-6-6M8 6l-6 6 6 6',
      zap:          'M13 2L3 14h9l-1 8 10-12h-9l1-8z',
      shield:       'M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z',
      'shield-lock':'M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10zM12 8v4M12 16h.01',
      'doc-text':   'M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8L14 2zm0 0v6h6M8 13h8M8 17h5',
      table:        'M3 6a1 1 0 011-1h16a1 1 0 011 1v12a1 1 0 01-1 1H4a1 1 0 01-1-1V6zm0 4h18M10 6v14',
      users:        'M17 21v-2a4 4 0 00-4-4H5a4 4 0 00-4 4v2M9 11a4 4 0 100-8 4 4 0 000 8zm13 10v-2a4 4 0 00-3-3.87M16 3.13a4 4 0 010 7.75',
      briefcase:    'M20 7H4a2 2 0 00-2 2v10a2 2 0 002 2h16a2 2 0 002-2V9a2 2 0 00-2-2zM16 7V5a2 2 0 00-2-2h-4a2 2 0 00-2 2v2',
      cog:          'M12 15a3 3 0 100-6 3 3 0 000 6zm7.4 0a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 010 2.83 2 2 0 01-2.83 0l-.06-.06A1.65 1.65 0 0015 19.4a1.65 1.65 0 00-1 1.51V21a2 2 0 01-4 0v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83-2.83l.06-.06A1.65 1.65 0 004.68 15a1.65 1.65 0 00-1.51-1H3a2 2 0 010-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 012.83-2.83l.06.06A1.65 1.65 0 009 4.68a1.65 1.65 0 001-1.51V3a2 2 0 014 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 012.83 2.83l-.06.06A1.65 1.65 0 0019.4 9a1.65 1.65 0 001.51 1H21a2 2 0 010 4h-.09a1.65 1.65 0 00-1.51 1z',
      // ui
      bell:         'M18 8a6 6 0 00-12 0c0 7-3 9-3 9h18s-3-2-3-9zm-4.73 11a2 2 0 01-2.54 0',
      sun:          'M12 17a5 5 0 100-10 5 5 0 000 10zm0-13v2M12 19v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M2 12h2M19 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42',
      moon:         'M21 12.79A9 9 0 1111.21 3 7 7 0 0021 12.79z',
      hamburger:    'M4 6h16M4 12h16M4 18h16',
      'chevron-right': 'M9 18l6-6-6-6',
      'log-out':    'M9 21H5a2 2 0 01-2-2V5a2 2 0 012-2h4M16 17l5-5-5-5M21 12H9',
      user:         'M20 21v-2a4 4 0 00-4-4H8a4 4 0 00-4 4v2M12 11a4 4 0 100-8 4 4 0 000 8z',
      webhook:      'M18 20V10M12 20V4M6 20v-6',
      cloud:        'M18 10h-1.26A8 8 0 109 20h9a5 5 0 000-10z',
      import:       'M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2M16 16l-4 4m0 0l-4-4m4 4V10',
      puzzle:       'M11 4a2 2 0 114 0v1a1 1 0 001 1h3a1 1 0 011 1v3a1 1 0 01-1 1h-1a2 2 0 100 4h1a1 1 0 011 1v3a1 1 0 01-1 1h-3a1 1 0 01-1-1v-1a2 2 0 10-4 0v1a1 1 0 01-1 1H7a1 1 0 01-1-1v-3a1 1 0 00-1-1H4a2 2 0 110-4h1a1 1 0 001-1V7a1 1 0 011-1h3a1 1 0 001-1V4z',
      audit:        'M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2m-6 9l2 2 4-4',
    };
    return p[name] ?? 'M12 12h.01';
  }
</script>

<!-- Click-outside to close menus -->
<svelte:window on:click={closeMenus} />

<div class="flex h-screen bg-background overflow-hidden">

  <!-- ── Mobile overlay ──────────────────────────────────────────────────────── -->
  {#if sidebarOpen}
    <div
      class="fixed inset-0 z-40 bg-black/50 lg:hidden"
      on:click={closeSidebar}
      role="presentation"
      aria-hidden="true"
    ></div>
  {/if}

  <!-- ── Sidebar ──────────────────────────────────────────────────────────────── -->
  <aside
    class="fixed lg:static inset-y-0 left-0 z-50 flex flex-col w-60 bg-card border-r border-border
           transition-transform duration-300 ease-in-out
           {sidebarOpen ? 'translate-x-0' : '-translate-x-full lg:translate-x-0'}"
    aria-label="Sidebar navigation"
  >

    <!-- Logo -->
    <div class="flex items-center gap-3 px-4 py-4 border-b border-border shrink-0">
      <div class="w-8 h-8 rounded-lg bg-primary flex items-center justify-center shrink-0 shadow-sm shadow-primary/30">
        <OrbitIcon size={18} className="text-primary-foreground" />
      </div>
      <div class="leading-tight">
        <span class="font-bold text-foreground text-sm tracking-tight">JottiCP</span>
        <span class="block text-muted-foreground text-[10px] font-medium">v1.0</span>
      </div>
    </div>

    <!-- Navigation (scrollable) -->
    <nav
      class="flex-1 overflow-y-auto py-3 px-2
             scrollbar-thin scrollbar-thumb-border scrollbar-track-transparent"
      aria-label="Main navigation"
    >
      {#each navGroups as group}
        <p class="px-3 py-1 text-[10px] font-semibold uppercase tracking-widest mt-4 mb-1 first:mt-0 flex items-center gap-1.5 {group.color}">
          {group.label}
        </p>
        {#each group.items as item}
          <a
            href={item.href}
            class="relative flex items-center gap-3 px-3 py-2 rounded-lg text-sm transition-all duration-150 mb-0.5
                   {isActive(item.href, currentPath)
                     ? 'bg-primary/10 text-primary font-medium border-l-2 border-primary pl-[10px]'
                     : 'text-muted-foreground hover:bg-muted/50 hover:text-foreground border-l-2 border-transparent pl-[10px]'}"
            aria-current={isActive(item.href, currentPath) ? 'page' : undefined}
            on:click={closeSidebar}
          >
            <svg
              class="w-4 h-4 shrink-0 {isActive(item.href, currentPath) ? 'text-primary' : ''}"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              stroke-width="1.75"
              aria-hidden="true"
            >
              <path stroke-linecap="round" stroke-linejoin="round" d={icon(item.icon)} />
            </svg>
            <span>{item.label}</span>
          </a>
        {/each}
      {/each}
    </nav>

    <!-- Bottom: user info + logout -->
    <div class="shrink-0 border-t border-border">

      <!-- Theme + Language row -->
      <div class="px-2 pt-2 pb-1 flex items-center gap-1">
        <button
          type="button"
          on:click={toggleTheme}
          class="flex-1 flex items-center gap-2 px-3 py-1.5 rounded-lg text-xs
                 text-muted-foreground hover:bg-muted/50 hover:text-foreground transition-colors"
          aria-label="Toggle theme"
        >
          {#if $theme === 'dark'}
            <svg class="w-3.5 h-3.5 shrink-0" fill="none" viewBox="0 0 24 24"
                 stroke="currentColor" stroke-width="1.75" aria-hidden="true">
              <path stroke-linecap="round" stroke-linejoin="round" d={icon('sun')} />
            </svg>
            <span>Light</span>
          {:else}
            <svg class="w-3.5 h-3.5 shrink-0" fill="none" viewBox="0 0 24 24"
                 stroke="currentColor" stroke-width="1.75" aria-hidden="true">
              <path stroke-linecap="round" stroke-linejoin="round" d={icon('moon')} />
            </svg>
            <span>Dark</span>
          {/if}
        </button>

        <select
          on:change={e => setLanguage((e.target as HTMLSelectElement).value as Parameters<typeof setLanguage>[0])}
          value={$currentLang}
          class="text-xs bg-transparent border border-border rounded px-1.5 py-1
                 text-muted-foreground focus:outline-none focus:ring-1 focus:ring-primary"
          aria-label="Select language"
        >
          <option value="en">EN</option>
          <option value="ar">AR</option>
          <option value="fr">FR</option>
          <option value="es">ES</option>
          <option value="zh-CN">ZH</option>
        </select>
      </div>

      <!-- User identity card -->
      <div class="p-3 flex items-center gap-3">
        <div class="w-8 h-8 rounded-full flex items-center justify-center text-xs font-semibold text-white shrink-0"
             style="background: linear-gradient(135deg, hsl(var(--primary)), hsl(var(--primary)/0.6))">
          {userInitials}
        </div>
        <div class="flex-1 min-w-0">
          <p class="text-sm font-medium text-foreground truncate">
            {$currentUser?.email?.split('@')[0] ?? 'User'}
          </p>
          <p class="text-xs text-muted-foreground truncate">
            {$currentUser?.email ?? ''}
          </p>
        </div>
        <button
          type="button"
          on:click={handleLogout}
          class="text-muted-foreground hover:text-foreground transition-colors p-1 rounded shrink-0"
          title="Log out"
          aria-label="Log out"
        >
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24"
               stroke="currentColor" stroke-width="1.75" aria-hidden="true">
            <path stroke-linecap="round" stroke-linejoin="round" d={icon('log-out')} />
          </svg>
        </button>
      </div>
    </div>
  </aside>

  <!-- ── Main area ────────────────────────────────────────────────────────────── -->
  <div class="flex flex-col flex-1 min-w-0 overflow-hidden">

    <!-- Impersonation banner -->
    {#if impersonationEmail}
      <div class="flex items-center justify-between px-4 py-2 bg-amber-400 text-amber-900 text-sm font-medium shrink-0">
        <span>You are logged in as <strong>{impersonationEmail}</strong></span>
        <button
          type="button"
          on:click={returnToAdmin}
          class="ml-4 px-3 py-1 rounded-md bg-amber-900/15 hover:bg-amber-900/25 text-amber-900
                 text-xs font-semibold transition-colors"
        >
          Return to Admin
        </button>
      </div>
    {/if}

    <!-- Navigation loading bar -->
    {#if $navigating}
      <div class="fixed top-0 left-0 right-0 h-0.5 bg-primary z-[100] origin-left"
           style="animation: loading 1s ease-in-out infinite">
      </div>
    {/if}

    <!-- Topbar -->
    <header
      class="sticky top-0 z-30 flex items-center h-14 px-4 gap-3
             bg-card/80 backdrop-blur-sm border-b border-border shrink-0"
    >
      <!-- Mobile hamburger -->
      <button
        type="button"
        on:click|stopPropagation={() => (sidebarOpen = !sidebarOpen)}
        class="lg:hidden w-9 h-9 inline-flex items-center justify-center rounded-lg
               text-muted-foreground hover:bg-muted/50 transition-colors"
        aria-label="Toggle sidebar"
        aria-expanded={sidebarOpen}
      >
        <svg class="w-5 h-5" fill="none" viewBox="0 0 24 24"
             stroke="currentColor" stroke-width="2" aria-hidden="true">
          <path stroke-linecap="round" stroke-linejoin="round" d={icon('hamburger')} />
        </svg>
      </button>

      <!-- Breadcrumb -->
      <nav aria-label="Breadcrumb" class="hidden sm:flex items-center gap-1 text-sm min-w-0 flex-1">
        <span class="text-muted-foreground shrink-0">JottiCP</span>
        {#each breadcrumbs as crumb, i}
          <svg class="w-3.5 h-3.5 text-muted-foreground/50 shrink-0" fill="none" viewBox="0 0 24 24"
               stroke="currentColor" stroke-width="2" aria-hidden="true">
            <path stroke-linecap="round" stroke-linejoin="round" d={icon('chevron-right')} />
          </svg>
          <span
            class="truncate {i === breadcrumbs.length - 1
              ? 'text-foreground font-medium'
              : 'text-muted-foreground'}"
          >
            {crumb}
          </span>
        {/each}
      </nav>

      <!-- Spacer (visible on mobile where breadcrumb is hidden) -->
      <div class="flex-1 sm:hidden"></div>

      <!-- Right controls -->
      <div class="flex items-center gap-1 shrink-0">

        <!-- Notifications -->
        <div class="relative">
          <button
            type="button"
            on:click|stopPropagation={() => { notifOpen = !notifOpen; userMenuOpen = false; }}
            class="relative w-9 h-9 inline-flex items-center justify-center rounded-lg
                   text-muted-foreground hover:bg-muted/50 hover:text-foreground transition-colors"
            aria-label="Notifications{unreadCount > 0 ? ` (${unreadCount} unread)` : ''}"
            aria-expanded={notifOpen}
          >
            <svg class="w-5 h-5" fill="none" viewBox="0 0 24 24"
                 stroke="currentColor" stroke-width="1.75" aria-hidden="true">
              <path stroke-linecap="round" stroke-linejoin="round" d={icon('bell')} />
            </svg>
            {#if unreadCount > 0}
              <span
                class="absolute -top-1 -right-1 w-4 h-4 bg-red-500 rounded-full text-[9px] font-bold
                       text-white flex items-center justify-center animate-bounce"
                style="animation-iteration-count: 3"
                aria-hidden="true"
              >
                {unreadCount > 9 ? '9+' : unreadCount}
              </span>
            {/if}
          </button>

          {#if notifOpen}
            <div
              class="absolute right-0 top-11 z-50 w-80 bg-card border border-border
                     rounded-xl shadow-xl overflow-hidden"
              on:click|stopPropagation
              role="dialog"
              aria-label="Notifications panel"
            >
              <div class="flex items-center justify-between px-4 py-3 border-b border-border">
                <span class="text-sm font-semibold text-foreground">Notifications</span>
                {#if unreadCount > 0}
                  <button
                    type="button"
                    on:click={() => { api.notifications.markAllRead(); notifications = []; }}
                    class="text-xs text-primary hover:underline"
                  >
                    Mark all read
                  </button>
                {/if}
              </div>
              <div class="max-h-80 overflow-y-auto divide-y divide-border">
                {#if notifications.length === 0}
                  <p class="text-sm text-muted-foreground text-center py-6">No unread notifications</p>
                {:else}
                  {#each notifications as n}
                    <div class="px-4 py-3 hover:bg-muted/30 {!n.read_at ? 'bg-primary/5' : ''}">
                      <p class="text-sm font-medium text-foreground truncate">{n.title}</p>
                      <p class="text-xs text-muted-foreground mt-0.5 line-clamp-2">{n.body}</p>
                    </div>
                  {/each}
                {/if}
              </div>
              <div class="px-4 py-2 border-t border-border">
                <a href="/logs" class="text-xs text-primary hover:underline">View audit log</a>
              </div>
            </div>
          {/if}
        </div>

        <!-- User avatar -->
        <div class="relative">
          <button
            type="button"
            on:click|stopPropagation={() => { userMenuOpen = !userMenuOpen; notifOpen = false; }}
            class="flex items-center gap-2 h-9 px-2 rounded-lg
                   text-muted-foreground hover:bg-muted/50 hover:text-foreground transition-colors"
            aria-label="User menu"
            aria-expanded={userMenuOpen}
          >
            <div class="w-7 h-7 rounded-full flex items-center justify-center text-xs font-semibold text-white shrink-0"
                 style="background: linear-gradient(135deg, hsl(var(--primary)), hsl(var(--primary)/0.6))">
              {userInitials}
            </div>
            <span class="hidden sm:block text-sm font-medium text-foreground max-w-[120px] truncate">
              {$currentUser?.email?.split('@')[0] ?? 'User'}
            </span>
          </button>

          {#if userMenuOpen}
            <div
              class="absolute right-0 top-11 z-50 w-48 bg-card border border-border
                     rounded-xl shadow-xl overflow-hidden"
              on:click|stopPropagation
              role="menu"
            >
              <div class="px-3 py-2.5 border-b border-border">
                <p class="text-xs font-semibold text-foreground truncate">{$currentUser?.email}</p>
                <p class="text-xs text-muted-foreground capitalize">{$currentUser?.role}</p>
              </div>

              <div class="py-1">
                <a
                  href="/settings"
                  role="menuitem"
                  class="flex items-center gap-2.5 px-3 py-2 text-sm text-muted-foreground
                         hover:bg-muted/50 hover:text-foreground transition-colors"
                  on:click={() => (userMenuOpen = false)}
                >
                  <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24"
                       stroke="currentColor" stroke-width="1.75" aria-hidden="true">
                    <path stroke-linecap="round" stroke-linejoin="round" d={icon('user')} />
                  </svg>
                  Profile
                </a>

                <button
                  type="button"
                  role="menuitem"
                  on:click={handleLogout}
                  class="w-full flex items-center gap-2.5 px-3 py-2 text-sm text-red-500
                         hover:bg-muted/50 transition-colors"
                >
                  <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24"
                       stroke="currentColor" stroke-width="1.75" aria-hidden="true">
                    <path stroke-linecap="round" stroke-linejoin="round" d={icon('log-out')} />
                  </svg>
                  Logout
                </button>
              </div>
            </div>
          {/if}
        </div>
      </div>
    </header>

    <!-- Scrollable page content -->
    <main class="flex-1 overflow-y-auto bg-background">
      {#key contentKey}
        <div class="fade-up">
          <slot />
        </div>
      {/key}
    </main>
  </div>
</div>

<style>
  .fade-up {
    animation: fadeUp 0.2s ease-out both;
  }
  @keyframes fadeUp {
    from { opacity: 0; transform: translateY(4px); }
    to   { opacity: 1; transform: none; }
  }
  @keyframes loading {
    0%   { transform: scaleX(0); }
    50%  { transform: scaleX(0.7); }
    100% { transform: scaleX(1); }
  }
</style>
