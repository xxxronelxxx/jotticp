<script lang="ts">
  import { onMount } from 'svelte';
  import { get } from 'svelte/store';
  import { auth } from '$lib/stores/auth';
  import { t } from '$lib/i18n';

  // ── Types ─────────────────────────────────────────────────────────────────────

  interface Plugin {
    slug:           string;
    name:           string;
    description:    string;
    version:        string;
    author:         string;
    homepage:       string | null;
    icon_url:       string | null;
    tags:           string[];
    installed:      boolean;
    enabled:        boolean;
    install_count?: number;
    requires_plan?: 'community' | 'pro';
  }

  type Tab = 'marketplace' | 'installed' | 'upload';
  type Toast = { message: string; type: 'success' | 'error' };

  // ── State ─────────────────────────────────────────────────────────────────────

  let activeTab: Tab = 'marketplace';
  let allPlugins: Plugin[] = [];
  let loading = true;
  let searchQuery = '';
  let activeTag = '';
  let actionInProgress: string | null = null; // slug of plugin being acted on

  // Upload state
  let uploadFile: File | null = null;
  let uploadInput: HTMLInputElement;
  let uploading = false;
  let uploadError = '';

  // Detail modal
  let detailPlugin: Plugin | null = null;

  // Inline confirm state
  let confirmUninstallSlug: string | null = null;

  // Toast
  let toast: Toast | null = null;
  let toastTimer: ReturnType<typeof setTimeout> | null = null;

  // ── Built-in marketplace catalog (curated list shown before API loads) ────────
  const catalog: Plugin[] = [
    {
      slug: 'orbit-seo',
      name: 'OrbitSEO',
      description: 'Automatic XML sitemaps, robots.txt management, Open Graph meta injection, and Google Search Console verification for every site.',
      version: '1.2.0',
      author: 'Development SPB',
      homepage: null,
      icon_url: null,
      tags: ['SEO', 'Performance'],
      installed: false,
      enabled: false,
      install_count: 2140,
      requires_plan: 'community',
    },
    {
      slug: 'orbit-waf-rules',
      name: 'ModSecurity Rules Pack',
      description: 'OWASP Core Rule Set 4.x + additional JottiCP curated rules. Requires ModSecurity to be enabled on the web server.',
      version: '4.3.1',
      author: 'Development SPB',
      homepage: null,
      icon_url: null,
      tags: ['Security', 'WAF'],
      installed: false,
      enabled: false,
      install_count: 1820,
      requires_plan: 'community',
    },
    {
      slug: 'orbit-metrics',
      name: 'Advanced Metrics',
      description: 'Per-site request analytics dashboard with visitor geo-distribution, top pages, referrers, and bandwidth usage charts powered by uPlot.',
      version: '2.0.4',
      author: 'Development SPB',
      homepage: null,
      icon_url: null,
      tags: ['Analytics', 'Dashboard'],
      installed: false,
      enabled: false,
      install_count: 3550,
      requires_plan: 'community',
    },
    {
      slug: 'orbit-redis-ui',
      name: 'Valkey/Redis UI',
      description: 'Web-based key browser, memory profiler, pub/sub monitor, and slow log viewer for your Valkey per-site instances.',
      version: '1.0.7',
      author: 'Development SPB',
      homepage: null,
      icon_url: null,
      tags: ['Database', 'Developer'],
      installed: false,
      enabled: false,
      install_count: 980,
      requires_plan: 'community',
    },
    {
      slug: 'orbit-2fa-enforcer',
      name: '2FA Enforcer',
      description: 'Enforce TOTP two-factor authentication for all sub-accounts, reseller clients, and admin logins. Reports on compliance.',
      version: '1.1.2',
      author: 'Development SPB',
      homepage: null,
      icon_url: null,
      tags: ['Security', 'Compliance'],
      installed: false,
      enabled: false,
      install_count: 760,
      requires_plan: 'community',
    },
    {
      slug: 'orbit-cdn',
      name: 'CDN Integration',
      description: 'Connect BunnyCDN, CloudFront, or Fastly to your sites. One-click cache purge, zone management, and automatic origin configuration.',
      version: '1.5.0',
      author: 'Development SPB',
      homepage: null,
      icon_url: null,
      tags: ['CDN', 'Performance'],
      installed: false,
      enabled: false,
      install_count: 1230,
      requires_plan: 'pro',
    },
    {
      slug: 'orbit-whmcs',
      name: 'WHMCS Provisioning',
      description: 'Automated site provisioning from WHMCS orders. Supports all WHMCS module standard actions: create, suspend, unsuspend, terminate, change package.',
      version: '3.0.0',
      author: 'Development SPB',
      homepage: null,
      icon_url: null,
      tags: ['Billing', 'Reseller'],
      installed: false,
      enabled: false,
      install_count: 890,
      requires_plan: 'pro',
    },
    {
      slug: 'orbit-slack-notify',
      name: 'Slack Notifications',
      description: 'Send site events — backup results, SSL renewals, downtime alerts, disk warnings — to a Slack channel via Incoming Webhooks.',
      version: '1.3.0',
      author: 'Community',
      homepage: null,
      icon_url: null,
      tags: ['Notifications', 'Integration'],
      installed: false,
      enabled: false,
      install_count: 620,
      requires_plan: 'community',
    },
  ];

  // All tags across catalog
  $: allTags = [...new Set(catalog.flatMap(p => p.tags))].sort();

  // ── Auth helpers ──────────────────────────────────────────────────────────────

  function authH(): Record<string, string> {
    const token = get(auth).token;
    return token ? { Authorization: 'Bearer ' + token } : {};
  }

  // ── Lifecycle ─────────────────────────────────────────────────────────────────

  onMount(async () => {
    await loadPlugins();
  });

  // ── Computed ──────────────────────────────────────────────────────────────────

  $: displayedPlugins = (() => {
    const base = allPlugins.length > 0 ? allPlugins : catalog;
    const q = searchQuery.toLowerCase().trim();
    return base.filter(p => {
      const matchesSearch = !q || p.name.toLowerCase().includes(q) || p.description.toLowerCase().includes(q) || p.tags.some(t => t.toLowerCase().includes(q));
      const matchesTag    = !activeTag || p.tags.includes(activeTag);
      if (activeTab === 'installed') return p.installed && matchesSearch && matchesTag;
      return matchesSearch && matchesTag;
    });
  })();

  $: installedCount = (allPlugins.length > 0 ? allPlugins : catalog).filter(p => p.installed).length;

  // ── Helpers ───────────────────────────────────────────────────────────────────

  function showToast(message: string, type: 'success' | 'error') {
    if (toastTimer) clearTimeout(toastTimer);
    toast = { message, type };
    toastTimer = setTimeout(() => { toast = null; }, 4500);
  }

  function tagColor(tag: string): string {
    const colors: Record<string, string> = {
      Security:     'bg-red-500/10 text-red-400 border-red-500/20',
      Performance:  'bg-green-500/10 text-green-400 border-green-500/20',
      Analytics:    'bg-blue-500/10 text-blue-400 border-blue-500/20',
      SEO:          'bg-purple-500/10 text-purple-400 border-purple-500/20',
      Database:     'bg-orange-500/10 text-orange-400 border-orange-500/20',
      Developer:    'bg-cyan-500/10 text-cyan-400 border-cyan-500/20',
      CDN:          'bg-sky-500/10 text-sky-400 border-sky-500/20',
      Billing:      'bg-yellow-500/10 text-yellow-400 border-yellow-500/20',
      Reseller:     'bg-yellow-500/10 text-yellow-400 border-yellow-500/20',
      Notifications:'bg-pink-500/10 text-pink-400 border-pink-500/20',
      Integration:  'bg-indigo-500/10 text-indigo-400 border-indigo-500/20',
      WAF:          'bg-red-500/10 text-red-400 border-red-500/20',
      Compliance:   'bg-green-500/10 text-green-400 border-green-500/20',
    };
    return colors[tag] ?? 'bg-muted text-muted-foreground border-border';
  }

  function pluginInitials(name: string): string {
    return name.split(' ').map(w => w[0]).join('').slice(0, 2).toUpperCase();
  }

  // ── Data loading ──────────────────────────────────────────────────────────────

  async function loadPlugins() {
    loading = true;
    try {
      const res = await fetch('/api/v1/plugins', { headers: authH() });
      if (res.ok) allPlugins = await res.json() as Plugin[];
      else allPlugins = []; // fallback to catalog
    } catch {
      allPlugins = []; // API not ready yet — use catalog
    } finally {
      loading = false;
    }
  }

  // ── Actions ───────────────────────────────────────────────────────────────────

  async function installPlugin(slug: string) {
    actionInProgress = slug;
    try {
      const res = await fetch(`/api/v1/plugins/${slug}/install`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', ...authH() },
        body: '{}',
      });
      if (!res.ok) {
        const body = await res.json() as { message?: string };
        throw new Error(body.message ?? `HTTP ${res.status}`);
      }
      showToast(`${slug} installed successfully`, 'success');
      await loadPlugins();
    } catch (err: unknown) {
      showToast((err as { message?: string }).message ?? 'Install failed', 'error');
    } finally {
      actionInProgress = null;
    }
  }

  async function uninstallPlugin(slug: string) {
    confirmUninstallSlug = null;
    actionInProgress = slug;
    try {
      const res = await fetch(`/api/v1/plugins/${slug}`, {
        method: 'DELETE',
        headers: authH(),
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      showToast(`${slug} uninstalled`, 'success');
      await loadPlugins();
    } catch (err: unknown) {
      showToast((err as { message?: string }).message ?? 'Uninstall failed', 'error');
    } finally {
      actionInProgress = null;
    }
  }

  async function togglePlugin(slug: string, enable: boolean) {
    actionInProgress = slug;
    try {
      const endpoint = enable ? 'enable' : 'disable';
      const res = await fetch(`/api/v1/plugins/${slug}/${endpoint}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', ...authH() },
        body: '{}',
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      showToast(`${slug} ${enable ? 'enabled' : 'disabled'}`, 'success');
      await loadPlugins();
    } catch (err: unknown) {
      showToast((err as { message?: string }).message ?? 'Action failed', 'error');
    } finally {
      actionInProgress = null;
    }
  }

  function onUploadChange(e: Event) {
    const input = e.target as HTMLInputElement;
    uploadFile = input.files?.[0] ?? null;
    uploadError = '';
  }

  async function uploadPlugin() {
    if (!uploadFile) return;
    uploading = true;
    uploadError = '';
    const fd = new FormData();
    fd.append('plugin', uploadFile);
    try {
      const res = await fetch('/api/v1/plugins/upload', {
        method: 'POST',
        headers: authH(),
        body: fd,
      });
      if (!res.ok) {
        const body = await res.json() as { message?: string };
        throw new Error(body.message ?? `HTTP ${res.status}`);
      }
      showToast('Plugin uploaded and installed', 'success');
      uploadFile = null;
      uploadInput.value = '';
      activeTab = 'installed';
      await loadPlugins();
    } catch (err: unknown) {
      uploadError = (err as { message?: string }).message ?? 'Upload failed';
    } finally {
      uploading = false;
    }
  }
</script>

<svelte:head>
  <title>{$t('plugins.title')} — JottiCP</title>
</svelte:head>

<!-- ── Detail modal ───────────────────────────────────────────────────────────── -->
{#if detailPlugin}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
    on:click|self={() => detailPlugin = null}
  >
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-lg mx-4 shadow-2xl">
      <div class="flex items-start gap-4 mb-5">
        <!-- Icon -->
        <div class="w-14 h-14 rounded-xl bg-primary/10 flex items-center justify-center text-lg font-bold text-primary shrink-0">
          {pluginInitials(detailPlugin.name)}
        </div>
        <div class="flex-1 min-w-0">
          <div class="flex items-center gap-2 flex-wrap">
            <h3 class="text-base font-semibold text-foreground">{detailPlugin.name}</h3>
            <span class="text-xs text-muted-foreground font-mono">v{detailPlugin.version}</span>
            {#if detailPlugin.requires_plan === 'pro'}
              <span class="inline-flex items-center px-1.5 py-0.5 rounded text-xs font-medium bg-yellow-500/10 text-yellow-400 border border-yellow-500/20">{$t('plugins.pro')}</span>
            {/if}
          </div>
          <p class="text-xs text-muted-foreground mt-0.5">By {detailPlugin.author}</p>
          <div class="flex flex-wrap gap-1 mt-2">
            {#each detailPlugin.tags as tag}
              <span class="inline-flex items-center px-1.5 py-0.5 rounded text-xs border {tagColor(tag)}">{tag}</span>
            {/each}
          </div>
        </div>
        <button on:click={() => detailPlugin = null} class="text-muted-foreground hover:text-foreground">
          <svg class="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12"/>
          </svg>
        </button>
      </div>

      <p class="text-sm text-muted-foreground leading-relaxed mb-5">{detailPlugin.description}</p>

      {#if detailPlugin.install_count !== undefined}
        <div class="flex items-center gap-1.5 text-xs text-muted-foreground mb-5">
          <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4"/>
          </svg>
          {detailPlugin.install_count.toLocaleString()} installs
        </div>
      {/if}

      <div class="flex gap-2">
        {#if !detailPlugin.installed}
          <button
            on:click={async () => { await installPlugin(detailPlugin!.slug); detailPlugin = null; }}
            disabled={actionInProgress === detailPlugin.slug}
            class="flex-1 h-9 rounded-xl bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 disabled:opacity-50 inline-flex items-center justify-center gap-2"
          >
            {#if actionInProgress === detailPlugin.slug}
              <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
                <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
              </svg>
              Installing…
            {:else}
              Install
            {/if}
          </button>
        {:else}
          <button
            on:click={() => togglePlugin(detailPlugin!.slug, !detailPlugin!.enabled)}
            disabled={actionInProgress === detailPlugin.slug}
            class="flex-1 h-9 rounded-xl border border-border text-sm font-medium hover:bg-muted disabled:opacity-50 inline-flex items-center justify-center gap-2 text-foreground"
          >
            {detailPlugin.enabled ? 'Disable' : 'Enable'}
          </button>
          {#if confirmUninstallSlug === detailPlugin.slug}
            <div class="flex items-center gap-1.5">
              <span class="text-xs text-destructive">{$t('plugins.uninstall')}</span>
              <button class="text-xs px-2 py-1 rounded bg-destructive text-white hover:bg-destructive/90" on:click={() => { uninstallPlugin(detailPlugin!.slug); detailPlugin = null; }}>{$t("common.yes")}</button>
              <button class="text-xs px-2 py-1 rounded bg-muted" on:click={() => confirmUninstallSlug = null}>{$t("common.no")}</button>
            </div>
          {:else}
            <button
              on:click={() => confirmUninstallSlug = detailPlugin!.slug}
              disabled={actionInProgress === detailPlugin.slug}
              class="h-9 px-4 rounded-xl border border-red-500/30 text-red-400 text-sm font-medium hover:bg-red-500/10 disabled:opacity-50"
            >
              Uninstall
            </button>
          {/if}
        {/if}
        <button on:click={() => detailPlugin = null} class="h-9 px-4 rounded-xl border border-border text-sm font-medium text-muted-foreground hover:bg-muted">
          Close
        </button>
      </div>
    </div>
  </div>
{/if}

<div class="space-y-6 page-content">

  <!-- Header ───────────────────────────────────────────────────────────────── -->
  <div class="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-3">
    <div>
      <h1 class="text-2xl font-bold text-foreground tracking-tight">{$t('plugins.title')}</h1>
      <p class="text-sm text-muted-foreground mt-0.5">{$t('plugins.subtitle')}</p>
    </div>
    {#if installedCount > 0}
      <span class="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-primary/10 text-primary text-sm font-medium border border-primary/20">
        <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
          <path stroke-linecap="round" stroke-linejoin="round" d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z"/>
        </svg>
        {installedCount} installed
      </span>
    {/if}
  </div>

  <!-- Tabs + search ────────────────────────────────────────────────────────── -->
  <div class="flex flex-col sm:flex-row sm:items-center gap-3">
    <!-- Tabs -->
    <div class="inline-flex items-center gap-1 bg-muted rounded-xl p-1 shrink-0">
      {#each [
        { key: 'marketplace', label: 'Marketplace' },
        { key: 'installed',   label: `Installed${installedCount > 0 ? ` (${installedCount})` : ''}` },
        { key: 'upload',      label: 'Upload' },
      ] as tab}
        <button
          type="button"
          on:click={() => { activeTab = tab.key as Tab; searchQuery = ''; activeTag = ''; }}
          class="px-4 py-2 rounded-lg text-sm font-medium transition-all
                 {activeTab === tab.key
                   ? 'bg-card text-foreground shadow-sm border border-border'
                   : 'text-muted-foreground hover:text-foreground'}"
        >
          {tab.label}
        </button>
      {/each}
    </div>

    <!-- Search (not on upload tab) -->
    {#if activeTab !== 'upload'}
      <div class="relative flex-1 max-w-sm">
        <svg class="w-4 h-4 text-muted-foreground absolute left-3 top-1/2 -translate-y-1/2" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
          <path stroke-linecap="round" stroke-linejoin="round" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"/>
        </svg>
        <input
          type="search"
          bind:value={searchQuery}
          placeholder="Search plugins…"
          class="w-full h-9 pl-9 pr-3 rounded-lg border border-border bg-background text-sm text-foreground
                 placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-ring"
        />
      </div>
    {/if}
  </div>

  <!-- Tag filter bar ───────────────────────────────────────────────────────── -->
  {#if activeTab !== 'upload'}
    <div class="flex flex-wrap gap-1.5">
      <button
        type="button"
        on:click={() => activeTag = ''}
        class="px-3 py-1 rounded-full text-xs font-medium border transition-all
               {activeTag === ''
                 ? 'bg-primary text-primary-foreground border-primary'
                 : 'bg-transparent text-muted-foreground border-border hover:border-primary/40 hover:text-foreground'}"
      >
        All
      </button>
      {#each allTags as tag}
        <button
          type="button"
          on:click={() => activeTag = activeTag === tag ? '' : tag}
          class="px-3 py-1 rounded-full text-xs font-medium border transition-all
                 {activeTag === tag
                   ? tagColor(tag) + ' ring-1 ring-current'
                   : 'bg-transparent text-muted-foreground border-border hover:text-foreground hover:border-primary/40'}"
        >
          {tag}
        </button>
      {/each}
    </div>
  {/if}

  <!-- ── Marketplace / Installed content ────────────────────────────────────── -->
  {#if activeTab !== 'upload'}
    {#if loading}
      <div class="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4">
        {#each [0,1,2,3,4,5] as _}
          <div class="bg-card border border-border rounded-xl p-5 animate-pulse">
            <div class="flex gap-3 mb-3">
              <div class="w-11 h-11 rounded-xl bg-muted shrink-0"></div>
              <div class="flex-1 space-y-2">
                <div class="h-4 bg-muted rounded w-32"></div>
                <div class="h-3 bg-muted rounded w-20"></div>
              </div>
            </div>
            <div class="space-y-1.5">
              <div class="h-3 bg-muted rounded w-full"></div>
              <div class="h-3 bg-muted rounded w-3/4"></div>
            </div>
          </div>
        {/each}
      </div>
    {:else if displayedPlugins.length === 0}
      <div class="text-center py-16">
        <div class="w-14 h-14 mx-auto mb-4 rounded-full bg-muted flex items-center justify-center">
          <svg class="w-7 h-7 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
            <path stroke-linecap="round" stroke-linejoin="round" d="M20.25 6.375c0 2.278-3.694 4.125-8.25 4.125S3.75 8.653 3.75 6.375m16.5 0c0-2.278-3.694-4.125-8.25-4.125S3.75 4.097 3.75 6.375m16.5 0v11.25c0 2.278-3.694 4.125-8.25 4.125s-8.25-1.847-8.25-4.125V6.375m16.5 2.25c0 2.278-3.694 4.125-8.25 4.125s-8.25-1.847-8.25-4.125m16.5 2.25c0 2.278-3.694 4.125-8.25 4.125s-8.25-1.847-8.25-4.125"/>
          </svg>
        </div>
        <p class="text-sm font-medium text-foreground mb-1">
          {activeTab === 'installed' ? 'No plugins installed' : 'No plugins match your search'}
        </p>
        <p class="text-xs text-muted-foreground">
          {activeTab === 'installed'
            ? 'Browse the Marketplace tab to discover and install plugins'
            : 'Try a different search term or clear the tag filter'}
        </p>
      </div>
    {:else}
      <div class="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4">
        {#each displayedPlugins as plugin (plugin.slug)}
          <div
            class="group bg-card border border-border rounded-xl p-5 hover:border-primary/30 transition-all duration-200 cursor-pointer card-hover flex flex-col"
            role="button"
            tabindex="0"
            on:click={() => detailPlugin = plugin}
            on:keydown={(e) => { if (e.key === 'Enter' || e.key === ' ') detailPlugin = plugin; }}
          >
            <!-- Card header -->
            <div class="flex items-start gap-3 mb-3">
              <div class="w-11 h-11 rounded-xl bg-primary/10 flex items-center justify-center text-sm font-bold text-primary shrink-0">
                {pluginInitials(plugin.name)}
              </div>
              <div class="flex-1 min-w-0">
                <div class="flex items-center gap-2 flex-wrap">
                  <span class="text-sm font-semibold text-foreground truncate">{plugin.name}</span>
                  <span class="text-xs text-muted-foreground font-mono shrink-0">v{plugin.version}</span>
                  {#if plugin.requires_plan === 'pro'}
                    <span class="inline-flex items-center px-1 py-0.5 rounded text-[10px] font-bold bg-yellow-500/10 text-yellow-400 border border-yellow-500/20 shrink-0">{$t('plugins.pro')}</span>
                  {/if}
                </div>
                <p class="text-xs text-muted-foreground mt-0.5 truncate">{plugin.author}</p>
              </div>

              <!-- Status indicator -->
              {#if plugin.installed}
                <span class="inline-flex items-center gap-1 px-1.5 py-0.5 rounded-full text-xs font-medium border shrink-0
                             {plugin.enabled
                               ? 'bg-green-500/10 text-green-400 border-green-500/20'
                               : 'bg-muted text-muted-foreground border-border'}">
                  <span class="w-1.5 h-1.5 rounded-full {plugin.enabled ? 'bg-green-400' : 'bg-muted-foreground'}"></span>
                  {plugin.enabled ? 'Active' : 'Disabled'}
                </span>
              {/if}
            </div>

            <!-- Description -->
            <p class="text-xs text-muted-foreground leading-relaxed flex-1 line-clamp-3 mb-3">
              {plugin.description}
            </p>

            <!-- Tags + install count -->
            <div class="flex items-center justify-between gap-2">
              <div class="flex flex-wrap gap-1">
                {#each plugin.tags.slice(0, 2) as tag}
                  <span class="inline-flex items-center px-1.5 py-0.5 rounded text-xs border {tagColor(tag)}">{tag}</span>
                {/each}
              </div>
              {#if plugin.install_count !== undefined}
                <span class="text-xs text-muted-foreground flex items-center gap-1 shrink-0">
                  <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4"/>
                  </svg>
                  {plugin.install_count >= 1000 ? (plugin.install_count / 1000).toFixed(1) + 'k' : plugin.install_count}
                </span>
              {/if}
            </div>

            <!-- Action row (shown on hover) -->
            <div
              class="mt-3 pt-3 border-t border-border flex items-center gap-2 opacity-0 group-hover:opacity-100 transition-opacity"
              on:click|stopPropagation
              role="none"
            >
              {#if !plugin.installed}
                <button
                  on:click={() => installPlugin(plugin.slug)}
                  disabled={actionInProgress === plugin.slug}
                  class="flex-1 h-8 rounded-lg bg-primary text-primary-foreground text-xs font-medium
                         hover:bg-primary/90 transition-colors disabled:opacity-50
                         inline-flex items-center justify-center gap-1.5"
                >
                  {#if actionInProgress === plugin.slug}
                    <svg class="w-3 h-3 animate-spin" fill="none" viewBox="0 0 24 24">
                      <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                      <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
                    </svg>
                    Installing
                  {:else}
                    <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                      <path stroke-linecap="round" stroke-linejoin="round" d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4"/>
                    </svg>
                    Install
                  {/if}
                </button>
              {:else}
                <button
                  on:click={() => togglePlugin(plugin.slug, !plugin.enabled)}
                  disabled={actionInProgress === plugin.slug}
                  class="flex-1 h-8 rounded-lg border border-border text-foreground text-xs font-medium hover:bg-muted transition-colors disabled:opacity-50"
                >
                  {plugin.enabled ? 'Disable' : 'Enable'}
                </button>
                {#if confirmUninstallSlug === plugin.slug}
                  <div class="flex items-center gap-1.5">
                    <span class="text-xs text-destructive">{$t('servers.remove')}</span>
                    <button class="text-xs px-2 py-1 rounded bg-destructive text-white hover:bg-destructive/90" on:click={() => uninstallPlugin(plugin.slug)}>{$t("common.yes")}</button>
                    <button class="text-xs px-2 py-1 rounded bg-muted" on:click={() => confirmUninstallSlug = null}>{$t("common.no")}</button>
                  </div>
                {:else}
                  <button
                    on:click={() => confirmUninstallSlug = plugin.slug}
                    disabled={actionInProgress === plugin.slug}
                    class="h-8 px-3 rounded-lg border border-red-500/30 text-red-400 text-xs font-medium hover:bg-red-500/10 transition-colors disabled:opacity-50"
                  >
                    Remove
                  </button>
                {/if}
              {/if}
              <button
                on:click={() => detailPlugin = plugin}
                class="h-8 px-3 rounded-lg border border-border text-muted-foreground text-xs hover:bg-muted hover:text-foreground transition-colors"
              >
                Info
              </button>
            </div>
          </div>
        {/each}
      </div>
    {/if}

  <!-- ── Upload tab ──────────────────────────────────────────────────────────── -->
  {:else}
    <div class="grid grid-cols-1 lg:grid-cols-5 gap-6">

      <!-- Upload form -->
      <div class="lg:col-span-3 bg-card border border-border rounded-xl p-6 space-y-5">
        <div class="flex items-center gap-3">
          <div class="w-9 h-9 rounded-xl bg-primary/10 flex items-center justify-center shrink-0">
            <svg class="w-5 h-5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
              <path stroke-linecap="round" stroke-linejoin="round" d="M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M15 13l-3-3m0 0l-3 3m3-3v12"/>
            </svg>
          </div>
          <div>
            <h2 class="text-base font-semibold text-foreground">{$t('plugins.upload_plugin')}</h2>
            <p class="text-xs text-muted-foreground">{$t('plugins.upload_desc')}</p>
          </div>
        </div>

        <!-- Drop zone -->
        <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
        <div
          class="border-2 border-dashed rounded-xl p-8 text-center cursor-pointer transition-colors
                 {uploadFile ? 'border-primary/50 bg-primary/5' : 'border-border hover:border-primary/30 hover:bg-muted/30'}"
          on:click={() => uploadInput.click()}
          on:dragover|preventDefault
          on:drop|preventDefault={(e) => { const f = e.dataTransfer?.files[0]; if (f) uploadFile = f; }}
        >
          <input
            bind:this={uploadInput}
            type="file"
            accept=".tar.gz,.tgz"
            class="hidden"
            on:change={onUploadChange}
          />
          {#if uploadFile}
            <div class="flex flex-col items-center gap-2">
              <svg class="w-10 h-10 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
                <path stroke-linecap="round" stroke-linejoin="round" d="M20.25 7.5l-.625 10.632a2.25 2.25 0 01-2.247 2.118H6.622a2.25 2.25 0 01-2.247-2.118L3.75 7.5M10 11.25h4M3.375 7.5h17.25c.621 0 1.125-.504 1.125-1.125v-1.5c0-.621-.504-1.125-1.125-1.125H3.375c-.621 0-1.125.504-1.125 1.125v1.5c0 .621.504 1.125 1.125 1.125z"/>
              </svg>
              <p class="text-sm font-medium text-foreground">{uploadFile.name}</p>
              <button
                type="button"
                class="text-xs text-muted-foreground hover:text-foreground underline"
                on:click|stopPropagation={() => { uploadFile = null; uploadInput.value = ''; }}
              >
                Remove
              </button>
            </div>
          {:else}
            <div class="flex flex-col items-center gap-3">
              <svg class="w-10 h-10 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
                <path stroke-linecap="round" stroke-linejoin="round" d="M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M15 13l-3-3m0 0l-3 3m3-3v12"/>
              </svg>
              <p class="text-sm font-medium text-foreground">{$t('plugins.drop_zone')}</p>
              <p class="text-xs text-muted-foreground">{$t('plugins.drop_zone_hint')}</p>
            </div>
          {/if}
        </div>

        {#if uploadError}
          <p class="text-sm text-red-400">{uploadError}</p>
        {/if}

        <button
          type="button"
          disabled={!uploadFile || uploading}
          on:click={uploadPlugin}
          class="w-full h-10 rounded-xl bg-primary text-primary-foreground text-sm font-medium
                 hover:bg-primary/90 transition-all active:scale-[0.98]
                 disabled:opacity-50 disabled:cursor-not-allowed
                 inline-flex items-center justify-center gap-2"
        >
          {#if uploading}
            <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
              <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
              <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
            </svg>
            Installing…
          {:else}
            Install Plugin
          {/if}
        </button>
      </div>

      <!-- Plugin manifest docs -->
      <div class="lg:col-span-2 space-y-4">
        <div class="bg-card border border-border rounded-xl p-5">
          <h3 class="text-sm font-semibold text-foreground mb-3">{$t('plugins.plugin_manifest')}</h3>
          <p class="text-xs text-muted-foreground mb-3">{$t('plugins.every_plugin_must_include_an')}<code class="font-mono bg-muted px-1 rounded">{$t('plugins.orbit_plugintoml')}</code>{$t('plugins.file_in_its_root')}</p>
          <pre class="text-xs font-mono bg-muted rounded-lg p-3 overflow-x-auto text-foreground leading-relaxed">{$t('plugins.pluginslug_my_pluginname_my_pluginversio')}</pre>
        </div>

        <div class="bg-amber-500/10 border border-amber-500/20 rounded-xl p-4">
          <div class="flex items-start gap-2.5">
            <svg class="w-4 h-4 text-amber-400 shrink-0 mt-0.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M12 9v4m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"/>
            </svg>
            <div>
              <p class="text-xs font-semibold text-amber-400 mb-1">{$t('plugins.security_note_title')}</p>
              <p class="text-xs text-amber-300/80 leading-relaxed">{$t('plugins.security_note_desc')}</p>
            </div>
          </div>
        </div>
      </div>
    </div>
  {/if}

</div>

<style>
  @keyframes fadeUp { from { opacity:0; transform:translateY(6px) } to { opacity:1; transform:none } }
  .page-content { animation: fadeUp 0.2s ease-out both }
  .card-hover { transition: all 0.2s ease; }
  .card-hover:hover { box-shadow: 0 4px 20px rgba(0,0,0,0.15); }
  .line-clamp-3 {
    display: -webkit-box;
    -webkit-line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
</style>

<!-- ── Toast ─────────────────────────────────────────────────────────────────── -->
{#if toast}
  <div
    class="fixed bottom-4 right-4 z-50 flex items-center gap-3 px-4 py-3 rounded-xl shadow-2xl border
      {toast.type === 'success'
        ? 'bg-green-950/90 border-green-800 text-green-300'
        : 'bg-red-950/90 border-red-800 text-red-300'}"
    role="alert"
    aria-live="polite"
  >
    {#if toast.type === 'success'}
      <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round" d="M5 13l4 4L19 7"/>
      </svg>
    {:else}
      <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"/>
      </svg>
    {/if}
    <span class="text-sm font-medium">{toast.message}</span>
    <button on:click={() => toast = null} class="text-current opacity-60 hover:opacity-100 transition-opacity" aria-label="Dismiss">
      <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12"/>
      </svg>
    </button>
  </div>
{/if}
