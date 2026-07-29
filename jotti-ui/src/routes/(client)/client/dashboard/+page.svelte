<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$api/client';
  import OrbitIcon from '$lib/components/ui/OrbitIcon.svelte';
  import type { Site, EmailAccount } from '$api/client';
  import { currentUser } from '$stores/auth';
  import StatusBadge from '$components/ui/StatusBadge.svelte';
  import { t } from '$lib/i18n';

  // ── State ──────────────────────────────────────────────────────────────────
  let sites: Site[] = [];
  let emailAccounts: EmailAccount[] = [];
  let loading = true;

  // Sidebar nav for client portal (fewer items)
  const clientNav = [
    { href: '/client/dashboard', label: 'Overview' },
    { href: '/websites',         label: 'My Sites' },
    { href: '/email',            label: 'Email' },
    { href: '/databases',        label: 'Databases' },
    { href: '/backups',          label: 'Backups' },
  ];

  onMount(async () => {
    loading = true;
    try {
      [sites, emailAccounts] = await Promise.all([
        api.sites.list(),
        api.email.accounts.list(),
      ]);
    } catch {
      console.error('Failed to load client dashboard');
    } finally {
      loading = false;
    }
  });

  function formatDisk(mb: number): string {
    if (mb >= 1024) return `${(mb / 1024).toFixed(1)} GB`;
    return `${mb} MB`;
  }

  function sslColor(status: string): string {
    return status === 'active' ? 'text-green-600' : status === 'expired' ? 'text-red-500' : 'text-amber-500';
  }
</script>

<svelte:head>
  <title>{$t('client.dashboard')} — JottiCP</title>
</svelte:head>

<!-- Client portal layout (simpler than admin) -->
<div class="flex h-screen bg-[var(--bg-page)] overflow-hidden">

  <!-- Simple sidebar -->
  <aside class="hidden md:flex flex-col w-[220px] bg-[var(--bg-surface)] border-r border-[var(--border)] shrink-0">
    <!-- Logo -->
    <div class="flex items-center gap-2 px-4 py-4 border-b border-[var(--border)]">
      <div class="w-7 h-7 rounded-lg bg-[var(--accent)] flex items-center justify-center shadow-sm shadow-indigo-500/30">
        <OrbitIcon size={16} className="text-white" />
      </div>
      <span class="font-bold text-sm text-[var(--text-primary)]">JottiCP</span>
    </div>

    <!-- Nav -->
    <nav class="flex-1 py-2 px-2">
      {#each clientNav as item}
        <a href={item.href}
           class="flex items-center gap-2 px-3 py-2 rounded-lg text-sm font-medium
                  text-[var(--text-muted)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)]
                  transition-colors mb-0.5">
          {item.label}
        </a>
      {/each}
    </nav>

    <!-- User info -->
    <div class="px-4 py-3 border-t border-[var(--border)]">
      <p class="text-xs font-medium text-[var(--text-primary)] truncate">{$currentUser?.email ?? ''}</p>
      <p class="text-xs text-[var(--text-muted)] capitalize">{$currentUser?.role}</p>
    </div>
  </aside>

  <!-- Main content -->
  <div class="flex-1 overflow-y-auto">
    <div class="p-4 lg:p-6 space-y-5">

      <!-- Welcome header -->
      <div>
        <h1 class="text-2xl font-semibold text-[var(--text-primary)]">
          {$t('client.welcome_back', { name: $currentUser?.email?.split('@')[0] ?? 'User' })}
        </h1>
        <p class="text-sm text-[var(--text-muted)] mt-0.5">{$t('client.overview')}</p>
      </div>

      <!-- Summary cards -->
      {#if loading}
        <div class="grid grid-cols-2 sm:grid-cols-3 gap-3">
          {#each [1,2,3] as _}
            <div class="h-20 bg-muted rounded-xl animate-pulse"></div>
          {/each}
        </div>
      {:else}
        <div class="grid grid-cols-2 sm:grid-cols-3 gap-3">
          <a href="/websites" class="bg-card border border-border rounded-xl p-4 hover:shadow-sm transition-shadow">
            <p class="text-xs text-muted-foreground uppercase tracking-wider mb-1">{$t('client.websites')}</p>
            <p class="text-3xl font-bold text-foreground">{sites.length}</p>
          </a>
          <a href="/email" class="bg-card border border-border rounded-xl p-4 hover:shadow-sm transition-shadow">
            <p class="text-xs text-muted-foreground uppercase tracking-wider mb-1">{$t('client.email_accounts')}</p>
            <p class="text-3xl font-bold text-foreground">{emailAccounts.length}</p>
          </a>
          <a href="/websites?filter=ssl_expiring"
             class="bg-card border border-border rounded-xl p-4 hover:shadow-sm transition-shadow">
            <p class="text-xs text-muted-foreground uppercase tracking-wider mb-1">{$t('client.ssl_expiring')}</p>
            <p class="text-3xl font-bold {sites.filter(s => {
              const days = s.ssl_expires_at ? Math.ceil((new Date(s.ssl_expires_at).getTime() - Date.now()) / 86400000) : null;
              return days !== null && days < 30;
            }).length > 0 ? 'text-amber-500' : 'text-foreground'}">
              {sites.filter(s => {
                const days = s.ssl_expires_at ? Math.ceil((new Date(s.ssl_expires_at).getTime() - Date.now()) / 86400000) : null;
                return days !== null && days < 30;
              }).length}
            </p>
          </a>
        </div>
      {/if}

      <!-- My Sites -->
      <div>
        <div class="flex items-center justify-between mb-3">
          <h2 class="text-base font-semibold text-[var(--text-primary)]">{$t('client.my_websites')}</h2>
          <a href="/websites" class="text-xs text-[var(--accent)] hover:underline">{$t('client.view_all')}</a>
        </div>

        {#if loading}
          <div class="space-y-2">
            {#each [1,2] as _}
              <div class="h-16 bg-muted rounded-xl animate-pulse"></div>
            {/each}
          </div>
        {:else if sites.length === 0}
          <div class="bg-card border border-border rounded-xl p-8 text-center">
            <p class="text-sm text-muted-foreground mb-3">{$t('client.no_sites_yet')}</p>
            <a href="/websites/add"
               class="inline-flex h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                      hover:bg-primary/90 transition-colors items-center">
              {$t('client.add_first_site')}
            </a>
          </div>
        {:else}
          <div class="space-y-2">
            {#each sites.slice(0, 5) as site}
              <a href="/websites/{site.id}"
                 class="flex items-center gap-4 p-3 bg-card border border-border rounded-xl
                        hover:shadow-sm transition-shadow">
                <div class="flex-1 min-w-0">
                  <div class="flex items-center gap-2">
                    <span class="font-medium text-foreground text-sm truncate">{site.domain}</span>
                    <StatusBadge status={site.status} />
                  </div>
                  <p class="text-xs text-muted-foreground mt-0.5">
                    PHP {site.php_version} · {formatDisk(site.disk_mb)}
                  </p>
                </div>
                <span class="text-xs {sslColor(site.ssl_status)} shrink-0">
                  {site.ssl_status === 'active' ? 'SSL ✓' : site.ssl_status === 'expired' ? 'SSL Expired' : 'No SSL'}
                </span>
              </a>
            {/each}
          </div>
        {/if}
      </div>

      <!-- Support button -->
      <div class="bg-primary/5 border border-primary/20 rounded-xl p-4">
        <div class="flex items-center justify-between">
          <div>
            <p class="text-sm font-semibold text-foreground">{$t('client.need_help')}</p>
            <p class="text-xs text-muted-foreground mt-0.5">{$t('client.need_help_desc')}</p>
          </div>
          <a href="mailto:support@jotticp.io"
             class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                    hover:bg-primary/90 transition-colors inline-flex items-center">
            {$t('client.contact_support')}
          </a>
        </div>
      </div>

    </div>
  </div>
</div>
