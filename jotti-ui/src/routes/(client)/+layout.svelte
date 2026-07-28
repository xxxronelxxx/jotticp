<script lang="ts">
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import { onMount } from 'svelte';
  import { auth, isAuthenticated, currentUser } from '$stores/auth';
  import { theme, toggleTheme } from '$stores/theme';
  import OrbitIcon from '$lib/components/ui/OrbitIcon.svelte';

  // ── Nav items (client-only, no admin items) ───────────────────────────────
  const navItems = [
    { href: '/client/dashboard', label: 'Dashboard',  icon: 'home'     },
    { href: '/sites',     label: 'My Sites',   icon: 'globe'    },
    { href: '/email',            label: 'Email',      icon: 'mail'     },
    { href: '/databases',        label: 'Databases',  icon: 'database' },
    { href: '/ssl',              label: 'SSL',        icon: 'lock'     },
    { href: '/backups',          label: 'Backups',    icon: 'archive'  },
    { href: '/cron',             label: 'Cron',       icon: 'clock'    },
    { href: '/profile',   label: 'Profile',    icon: 'user'     },
  ] as const;

  let sidebarOpen = false;

  onMount(() => {
    auth.init();
  });

  $: if ($auth.is_initialized && !$isAuthenticated) {
    goto('/login');
  }

  function isActive(href: string): boolean {
    const path = $page.url.pathname;
    if (href === '/client/dashboard') return path === '/client/dashboard';
    return path.startsWith(href);
  }

  function getIconPath(icon: string): string {
    const paths: Record<string, string> = {
      home:     'M3 9l9-7 9 7v11a2 2 0 01-2 2H5a2 2 0 01-2-2V9zM9 22V12h6v10',
      globe:    'M12 2a10 10 0 100 20A10 10 0 0012 2zm0 0c-2.5 2.5-4 6.2-4 10s1.5 7.5 4 10m0-20c2.5 2.5 4 6.2 4 10s-1.5 7.5-4 10M2 12h20',
      mail:     'M4 4h16c1.1 0 2 .9 2 2v12c0 1.1-.9 2-2 2H4c-1.1 0-2-.9-2-2V6c0-1.1.9-2 2-2zm0 2l8 5 8-5',
      database: 'M12 2C8.13 2 5 3.34 5 5v14c0 1.66 3.13 3 7 3s7-1.34 7-3V5c0-1.66-3.13-3-7-3zm0 2c3.31 0 5 1.01 5 1.5S15.31 7 12 7 7 5.99 7 5.5 8.69 4 12 4zM7 8.67c1.21.65 2.96 1.02 5 1.02s3.79-.37 5-1.02V11c0 .5-1.69 1.5-5 1.5S7 11.5 7 11V8.67zm0 5c1.21.65 2.96 1.02 5 1.02s3.79-.37 5-1.02V16c0 .5-1.69 1.5-5 1.5S7 16.5 7 16v-2.33z',
      lock:     'M12 1a3 3 0 00-3 3v8a6 6 0 1012 0V4a3 3 0 00-3-3H12z M12 15v2',
      archive:  'M21 8a2 2 0 00-1-1.73l-7-4a2 2 0 00-2 0l-7 4A2 2 0 002 8v8a2 2 0 001 1.73l7 4a2 2 0 002 0l7-4A2 2 0 0021 16V8z',
      clock:    'M12 2a10 10 0 100 20A10 10 0 0012 2zm0 5v5l3 3',
      user:     'M20 21v-2a4 4 0 00-4-4H8a4 4 0 00-4 4v2M12 11a4 4 0 100-8 4 4 0 000 8z',
      sun:      'M12 17a5 5 0 100-10 5 5 0 000 10zm0-13v2M12 19v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M2 12h2M19 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42',
      moon:     'M21 12.79A9 9 0 1111.21 3 7 7 0 0021 12.79z',
      'log-out':'M9 21H5a2 2 0 01-2-2V5a2 2 0 012-2h4M16 17l5-5-5-5M21 12H9',
    };
    return paths[icon] ?? 'M12 12h.01';
  }

  async function handleLogout() {
    await auth.logout();
  }
</script>

<svelte:window on:click={() => { sidebarOpen = false; }} />

{#if $auth.is_initialized && $isAuthenticated}
  <div class="flex h-screen bg-[var(--bg-page)] overflow-hidden">

    <!-- Mobile sidebar overlay -->
    {#if sidebarOpen}
      <div
        class="fixed inset-0 z-30 bg-black/50 lg:hidden"
        on:click|stopPropagation={() => sidebarOpen = false}
        role="presentation"
        aria-hidden="true"
      ></div>
    {/if}

    <!-- Sidebar -->
    <aside
      class="fixed lg:static inset-y-0 left-0 z-40 flex flex-col w-[220px]
             bg-[var(--bg-surface)] border-r border-[var(--border)] transition-transform duration-300
             {sidebarOpen ? 'translate-x-0' : '-translate-x-full lg:translate-x-0'}"
    >
      <!-- Logo + client label -->
      <div class="flex items-center gap-2.5 px-4 py-4 border-b border-[var(--border)] shrink-0">
        <div class="w-8 h-8 rounded-lg bg-[var(--accent)] flex items-center justify-center shrink-0 shadow-sm shadow-indigo-500/30">
          <OrbitIcon size={18} className="text-white" />
        </div>
        <div class="leading-tight min-w-0">
          <span class="font-bold text-[var(--text-primary)] text-sm tracking-tight block">JottiCP</span>
          <span class="text-[10px] text-[var(--text-muted)] font-medium uppercase tracking-wider leading-none block truncate">
            Client Portal
          </span>
        </div>
      </div>

      <!-- Username badge -->
      <div class="px-3 py-2.5 border-b border-[var(--border)] bg-[var(--accent-light)]/40">
        <p class="text-xs font-medium text-[var(--accent)] truncate">
          {$currentUser?.email ?? 'User'}
        </p>
      </div>

      <!-- Nav -->
      <nav class="flex-1 overflow-y-auto py-2 px-2">
        {#each navItems as item}
          <a
            href={item.href}
            class="flex items-center gap-2.5 px-3 py-2 rounded-lg text-sm font-medium transition-colors mb-0.5
                   {isActive(item.href)
                     ? 'bg-[var(--accent-light)] text-[var(--accent)]'
                     : 'text-[var(--text-muted)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)]'}"
            on:click={() => sidebarOpen = false}
          >
            <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
              <path stroke-linecap="round" stroke-linejoin="round" d={getIconPath(item.icon)} />
            </svg>
            <span>{item.label}</span>
          </a>
        {/each}
      </nav>

      <!-- Dark mode toggle + logout -->
      <div class="shrink-0 px-2 py-2 border-t border-[var(--border)] space-y-0.5">
        <button
          type="button"
          on:click={toggleTheme}
          class="w-full flex items-center gap-2.5 px-3 py-2 rounded-lg text-sm font-medium
                 text-[var(--text-muted)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)] transition-colors"
        >
          <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
            <path stroke-linecap="round" stroke-linejoin="round"
                  d={$theme === 'dark' ? getIconPath('sun') : getIconPath('moon')} />
          </svg>
          <span>{$theme === 'dark' ? 'Light mode' : 'Dark mode'}</span>
        </button>

        <button
          type="button"
          on:click={handleLogout}
          class="w-full flex items-center gap-2.5 px-3 py-2 rounded-lg text-sm font-medium
                 text-[var(--error)] hover:bg-[var(--bg-hover)] transition-colors"
        >
          <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
            <path stroke-linecap="round" stroke-linejoin="round" d={getIconPath('log-out')} />
          </svg>
          <span>Logout</span>
        </button>
      </div>
    </aside>

    <!-- Main content -->
    <div class="flex flex-col flex-1 min-w-0 overflow-hidden">

      <!-- Top bar (mobile) -->
      <header class="flex items-center h-12 px-4 border-b border-[var(--border)] bg-[var(--bg-page)] shrink-0 gap-3 lg:hidden">
        <button
          type="button"
          on:click|stopPropagation={() => sidebarOpen = !sidebarOpen}
          class="w-9 h-9 inline-flex items-center justify-center rounded-lg text-[var(--text-muted)]
                 hover:bg-[var(--bg-hover)] transition-colors"
          aria-label="Toggle sidebar"
        >
          <svg class="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M4 6h16M4 12h16M4 18h16" />
          </svg>
        </button>
        <span class="text-sm font-medium text-[var(--text-primary)]">Client Portal</span>
      </header>

      <!-- Scrollable page content -->
      <main class="flex-1 overflow-y-auto bg-[var(--bg-page)]">
        <slot />
      </main>
    </div>
  </div>

{:else if !$auth.is_initialized}
  <div class="min-h-screen flex items-center justify-center bg-[var(--bg-page)]">
    <div class="flex flex-col items-center gap-3">
      <svg class="w-8 h-8 animate-spin text-[var(--accent)]" fill="none" viewBox="0 0 24 24">
        <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4" />
        <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
      </svg>
      <p class="text-sm text-[var(--text-muted)]">Loading...</p>
    </div>
  </div>
{/if}
