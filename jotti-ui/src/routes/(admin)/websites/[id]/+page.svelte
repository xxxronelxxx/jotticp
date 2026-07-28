<script lang="ts">
  import { onMount } from 'svelte';
  import { page } from '$app/stores';
  import { browser } from '$app/environment';
  import { api } from '$api/client';
  import type { Site } from '$api/client';
  import StatusBadge from '$components/ui/StatusBadge.svelte';
  import FileManager from '$lib/components/ui/FileManager.svelte';

  // Tab components
  import OverviewTab  from '$lib/components/site/OverviewTab.svelte';
  import DatabasesTab from '$lib/components/site/DatabasesTab.svelte';
  import EmailTab     from '$lib/components/site/EmailTab.svelte';
  import DnsTab       from '$lib/components/site/DnsTab.svelte';
  import SslTab       from '$lib/components/site/SslTab.svelte';
  import PhpTab       from '$lib/components/site/PhpTab.svelte';
  import CacheTab     from '$lib/components/site/CacheTab.svelte';
  import CronTab      from '$lib/components/site/CronTab.svelte';
  import LogsTab      from '$lib/components/site/LogsTab.svelte';
  import BackupsTab   from '$lib/components/site/BackupsTab.svelte';
  import GitDeployTab from '$lib/components/site/GitDeployTab.svelte';
  import StagingTab   from '$lib/components/site/StagingTab.svelte';
  import AccessTab    from '$lib/components/site/AccessTab.svelte';
  import AppsTab      from '$lib/components/site/AppsTab.svelte';
  import RuntimeTab   from '$lib/components/site/RuntimeTab.svelte';

  // ── State ──────────────────────────────────────────────────────────────────
  let site: Site | null = null;
  let loading = true;
  let activeTab = 'overview';
  let previewUrl: string | null = null;

  // Lazy-mount: tabs mount on first visit, then stay in DOM hidden to preserve state.
  let loadedTabs = new Set<string>(['overview', 'files']);

  $: siteId = $page.params.id;

  // ── Navigation structure ───────────────────────────────────────────────────
  // 6 top-level groups → sub-tabs appear below only for the active group.
  // Reduces 16 individual tabs → max 6 group pills + max 4 sub-tabs visible.
  const groups = [
    {
      key:   'overview',
      label: 'Overview',
      icon:  'M3 12l2-2m0 0l7-7 7 7M5 10v10a1 1 0 001 1h3m10-11l2 2m-2-2v10a1 1 0 01-1 1h-3m-6 0a1 1 0 001-1v-4a1 1 0 011-1h2a1 1 0 011 1v4a1 1 0 001 1m-6 0h6',
      tabs: [
        { key: 'overview', label: 'Overview' },
      ],
    },
    {
      key:   'files',
      label: 'Files',
      icon:  'M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z',
      tabs: [
        { key: 'files',  label: 'File Manager' },
        { key: 'access', label: 'Access'       },
      ],
    },
    {
      key:   'hosting',
      label: 'Hosting',
      icon:  'M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2',
      tabs: [
        { key: 'databases', label: 'Databases' },
        { key: 'email',     label: 'Email'     },
        { key: 'dns',       label: 'DNS'       },
        { key: 'ssl',       label: 'SSL'       },
      ],
    },
    {
      key:   'config',
      label: 'Config',
      icon:  'M12 6V4m0 2a2 2 0 100 4m0-4a2 2 0 110 4m-6 8a2 2 0 100-4m0 4a2 2 0 110-4m0 4v2m0-6V4m6 6v10m6-2a2 2 0 100-4m0 4a2 2 0 110-4m0 4v2m0-6V4',
      tabs: [
        { key: 'php',   label: 'PHP'   },
        { key: 'cache', label: 'Cache' },
        { key: 'cron',  label: 'Cron'  },
      ],
    },
    {
      key:   'ops',
      label: 'Ops',
      icon:  'M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z',
      tabs: [
        { key: 'backups', label: 'Backups' },
        { key: 'logs',    label: 'Logs'    },
        { key: 'apps',    label: 'Apps'    },
      ],
    },
    {
      key:   'dev',
      label: 'Dev',
      icon:  'M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4',
      tabs: [
        { key: 'git-deploy', label: 'Git'     },
        { key: 'staging',    label: 'Staging' },
        { key: 'runtime',    label: 'Runtime' },
      ],
    },
  ] as const;

  type GroupKey = typeof groups[number]['key'];
  type TabKey   = typeof groups[number]['tabs'][number]['key'];
  const allTabs = groups.flatMap(g => g.tabs);

  // Derive active group reactively from activeTab — single source of truth.
  $: activeGroup = (groups.find(g => g.tabs.some(t => t.key === activeTab))?.key ?? 'overview') as GroupKey;
  $: currentGroupTabs = groups.find(g => g.key === activeGroup)?.tabs ?? [];

  // ── Lifecycle ───────────────────────────────────────────────────────────────
  onMount(async () => {
    if (browser) {
      const hash = window.location.hash.replace('#', '');
      if (hash && allTabs.some(t => t.key === hash)) activeTab = hash as TabKey;
    }
    await loadSite();
  });

  async function loadSite() {
    loading = true;
    try {
      site = await api.sites.get(siteId);
    } catch {
      // site stays null → template shows error state
    } finally {
      loading = false;
    }
  }

  function handleTabChange(key: string) {
    loadedTabs = new Set([...loadedTabs, key]);
    activeTab = key;
    if (browser) window.location.hash = key;
  }

  function handleGroupClick(group: typeof groups[number]) {
    handleTabChange(group.tabs[0].key);
  }

  function handleChildTabChange(event: CustomEvent<string>) {
    handleTabChange(event.detail);
  }

  function handleSiteUpdated() {
    void loadSite();
  }

  async function loadPreviewUrl() {
    try {
      const res = await api.sites.previewUrl(siteId);
      previewUrl = res.preview_url;
    } catch { /* preview URL not available */ }
  }
</script>

<svelte:head>
  <title>{site ? `${site.domain} — JottiCP` : 'Site — JottiCP'}</title>
</svelte:head>

{#if loading}
  <div class="space-y-4 animate-pulse">
    <div class="h-8 bg-muted rounded-lg w-64"></div>
    <div class="h-10 bg-muted rounded-lg"></div>
    <div class="h-64 bg-muted rounded-xl"></div>
  </div>

{:else if !site}
  <div class="text-center py-20">
    <p class="text-muted-foreground">Site not found.</p>
    <a href="/websites" class="mt-4 inline-block text-sm text-primary hover:underline">← Back to websites</a>
  </div>

{:else}
  <div class="space-y-4">

    <!-- ── Header: breadcrumb + title + actions ─────────────────────────────── -->
    <div class="flex flex-col sm:flex-row sm:items-start sm:justify-between gap-3">
      <div>
        <div class="flex items-center gap-1.5 text-sm text-muted-foreground mb-1">
          <a href="/websites" class="hover:text-foreground transition-colors">Websites</a>
          <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7"/>
          </svg>
          <span class="text-foreground font-medium truncate max-w-[200px]">{site.domain}</span>
        </div>
        <div class="flex items-center gap-2.5 flex-wrap">
          <h1 class="text-xl font-bold text-foreground tracking-tight">{site.domain}</h1>
          <StatusBadge status={site.status} />
          {#if site.ssl_status === 'active'}
            <span class="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium bg-green-500/10 text-green-400 border border-green-500/20">
              <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z"/>
              </svg>
              HTTPS
            </span>
          {/if}
        </div>
        <p class="text-xs text-muted-foreground mt-1">
          PHP {site.php_version} · {site.web_server.toUpperCase()} · {site.server_label ?? 'Local'}
        </p>
      </div>

      <div class="flex items-center gap-2 shrink-0">
        {#if previewUrl}
          <a
            href={previewUrl}
            target="_blank"
            rel="noopener noreferrer"
            class="h-8 px-3 rounded-lg border border-primary/30 bg-primary/5 text-xs font-medium text-primary
                   hover:bg-primary/10 transition-colors inline-flex items-center gap-1.5"
          >
            <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"/>
              <path stroke-linecap="round" stroke-linejoin="round" d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z"/>
            </svg>
            Preview
          </a>
        {:else}
          <button
            on:click={loadPreviewUrl}
            class="h-8 px-3 rounded-lg border border-border text-xs font-medium text-muted-foreground
                   hover:bg-muted hover:text-foreground transition-colors inline-flex items-center gap-1.5"
          >
            <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"/>
              <path stroke-linecap="round" stroke-linejoin="round" d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z"/>
            </svg>
            Preview
          </button>
        {/if}
        <a
          href="https://{site.domain}"
          target="_blank"
          rel="noopener noreferrer"
          class="h-8 px-3 rounded-lg border border-border text-xs font-medium text-muted-foreground
                 hover:bg-muted hover:text-foreground transition-colors inline-flex items-center gap-1.5"
        >
          Visit
          <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14"/>
          </svg>
        </a>
      </div>
    </div>

    <!-- ── Two-tier navigation ──────────────────────────────────────────────── -->
    <!--
      Tier 1: 6 group pills — always fits in one row, no overflow.
      Tier 2: 2–4 sub-tab pills — only the active group's tabs, shown below.
      Total visible: 6 + max 4 = replaces 16 flat tabs.
    -->
    <div class="bg-card border border-border rounded-xl overflow-hidden">

      <!-- Group pills row -->
      <div class="flex items-center gap-1 p-1.5 bg-muted/40 border-b border-border overflow-x-auto scrollbar-hide">
        {#each groups as group}
          <button
            type="button"
            on:click={() => handleGroupClick(group)}
            aria-selected={activeGroup === group.key}
            class="flex items-center gap-1.5 px-3 h-8 rounded-lg text-sm font-medium whitespace-nowrap transition-all
                   {activeGroup === group.key
                     ? 'bg-background text-foreground shadow-sm border border-border/80'
                     : 'text-muted-foreground hover:text-foreground hover:bg-background/60'}"
          >
            <svg class="w-3.5 h-3.5 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
              <path stroke-linecap="round" stroke-linejoin="round" d={group.icon}/>
            </svg>
            {group.label}
          </button>
        {/each}
      </div>

      <!-- Sub-tab row — only renders when active group has multiple tabs -->
      {#if currentGroupTabs.length > 1}
        <div class="flex items-center gap-0 px-3 border-b border-border bg-background/50">
          {#each currentGroupTabs as tab}
            <button
              type="button"
              on:click={() => handleTabChange(tab.key)}
              aria-selected={activeTab === tab.key}
              class="h-9 px-3.5 text-xs font-medium border-b-2 transition-all whitespace-nowrap
                     {activeTab === tab.key
                       ? 'border-primary text-foreground'
                       : 'border-transparent text-muted-foreground hover:text-foreground hover:border-border'}"
            >
              {tab.label}
            </button>
          {/each}
        </div>
      {/if}

      <!-- ── Tab content panels ─────────────────────────────────────────────── -->
      <div class="p-4 md:p-5 min-h-[420px]" role="tabpanel">

        <!-- Always-rendered (never unmounted) -->
        <div class:hidden={activeTab !== 'overview'}>
          <OverviewTab {siteId} {site} on:siteUpdated={handleSiteUpdated} />
        </div>
        <div class:hidden={activeTab !== 'files'}>
          <FileManager {siteId} unixUser={site.unix_user} embedded={true} />
        </div>

        <!-- Lazy-mounted: rendered only after first visit, then kept in DOM -->
        {#if loadedTabs.has('access')}
          <div class:hidden={activeTab !== 'access'}>
            <AccessTab {siteId} {site} />
          </div>
        {/if}
        {#if loadedTabs.has('databases')}
          <div class:hidden={activeTab !== 'databases'}>
            <DatabasesTab {siteId} {site} />
          </div>
        {/if}
        {#if loadedTabs.has('email')}
          <div class:hidden={activeTab !== 'email'}>
            <EmailTab {siteId} {site} on:tabChange={handleChildTabChange} />
          </div>
        {/if}
        {#if loadedTabs.has('dns')}
          <div class:hidden={activeTab !== 'dns'}>
            <DnsTab {siteId} {site} />
          </div>
        {/if}
        {#if loadedTabs.has('ssl')}
          <div class:hidden={activeTab !== 'ssl'}>
            <SslTab {siteId} {site} />
          </div>
        {/if}
        {#if loadedTabs.has('php')}
          <div class:hidden={activeTab !== 'php'}>
            <PhpTab {siteId} {site} />
          </div>
        {/if}
        {#if loadedTabs.has('cache')}
          <div class:hidden={activeTab !== 'cache'}>
            <CacheTab {siteId} {site} />
          </div>
        {/if}
        {#if loadedTabs.has('cron')}
          <div class:hidden={activeTab !== 'cron'}>
            <CronTab {siteId} {site} />
          </div>
        {/if}
        {#if loadedTabs.has('backups')}
          <div class:hidden={activeTab !== 'backups'}>
            <BackupsTab {siteId} {site} />
          </div>
        {/if}
        {#if loadedTabs.has('logs')}
          <div class:hidden={activeTab !== 'logs'}>
            <LogsTab {siteId} {site} />
          </div>
        {/if}
        {#if loadedTabs.has('apps')}
          <div class:hidden={activeTab !== 'apps'}>
            <AppsTab {siteId} {site} />
          </div>
        {/if}
        {#if loadedTabs.has('git-deploy')}
          <div class:hidden={activeTab !== 'git-deploy'}>
            <GitDeployTab {siteId} {site} />
          </div>
        {/if}
        {#if loadedTabs.has('staging')}
          <div class:hidden={activeTab !== 'staging'}>
            <StagingTab {siteId} {site} />
          </div>
        {/if}
        {#if loadedTabs.has('runtime')}
          <div class:hidden={activeTab !== 'runtime'}>
            <RuntimeTab {siteId} {site} />
          </div>
        {/if}

      </div>
    </div>

  </div>
{/if}
