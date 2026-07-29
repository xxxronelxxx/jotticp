<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$api/client';
  import { t } from '$lib/i18n';

  // ── Types ──────────────────────────────────────────────────────────────────

  type App = {
    id: string; name: string; description: string;
    version: string; category: string; icon_url: string | null;
    requires_pro?: boolean;
  };

  type Installation = {
    id: string; site_id: string; app_id: string; version: string;
    status: string; installed_at: string | null; admin_url: string | null;
  };

  // ── App metadata (icons + brand colors) ───────────────────────────────────

  const APP_META: Record<string, { color: string; icon: string; brandColor: string; popularRank?: number }> = {
    wordpress:   { color: '#2271B1', icon: 'W',  brandColor: '#2271B1', popularRank: 1 },
    woocommerce: { color: '#96588A', icon: 'WC', brandColor: '#96588A', popularRank: 2 },
    joomla:      { color: '#F44321', icon: 'J',  brandColor: '#F44321', popularRank: 5 },
    drupal:      { color: '#0077C0', icon: 'D',  brandColor: '#0077C0', popularRank: 6 },
    magento:     { color: '#EE672F', icon: 'Mg', brandColor: '#EE672F', popularRank: 7 },
    prestashop:  { color: '#DF0067', icon: 'PS', brandColor: '#DF0067', popularRank: 8 },
    opencart:    { color: '#23ADEF', icon: 'OC', brandColor: '#23ADEF', popularRank: 9 },
    phpbb:       { color: '#1B5E9B', icon: 'BB', brandColor: '#1B5E9B' },
    nextcloud:   { color: '#0082C9', icon: 'NC', brandColor: '#0082C9', popularRank: 4 },
    moodle:      { color: '#FF7800', icon: 'Mo', brandColor: '#FF7800' },
    gitea:       { color: '#609926', icon: 'G',  brandColor: '#609926' },
    roundcube:   { color: '#37B1CC', icon: 'RC', brandColor: '#37B1CC' },
    ghost:       { color: '#15171A', icon: 'Gh', brandColor: '#15171A', popularRank: 10 },
    matomo:      { color: '#3152A0', icon: 'Mt', brandColor: '#3152A0' },
    laravel:     { color: '#FF2D20', icon: 'L',  brandColor: '#FF2D20', popularRank: 3 },
    symfony:     { color: '#000000', icon: 'Sf', brandColor: '#1a1a1a' },
    nextjs:      { color: '#111827', icon: 'NJ', brandColor: '#111827' },
    nuxt:        { color: '#00DC82', icon: 'Nu', brandColor: '#00DC82' },
    django:      { color: '#092E20', icon: 'Dj', brandColor: '#092E20' },
    fastapi:     { color: '#009688', icon: 'FA', brandColor: '#009688' },
    ollama_proxy:{ color: '#5C6BC0', icon: 'AI', brandColor: '#5C6BC0' },
    n8n_ai:      { color: '#EA4B71', icon: 'n8', brandColor: '#EA4B71' },
    n8n:         { color: '#EA4B71', icon: 'n8', brandColor: '#EA4B71' },
    default:     { color: '#6366F1', icon: '?',  brandColor: '#6366F1' },
  };

  function appMeta(id: string) {
    return APP_META[id.toLowerCase()] ?? APP_META.default;
  }

  const CATEGORY_LABELS: Record<string, string> = {
    all:          'All',
    cms:          'CMS',
    ecommerce:    'E-commerce',
    forum:        'Forum',
    blog:         'Blog',
    productivity: 'Productivity',
    cloud:        'Cloud',
    email:        'Email',
    dev:          'Dev Tools',
    framework:    'Framework',
    ai:           'AI / Automation',
    lms:          'LMS',
    other:        'Other',
  };

  // Category → display bucket for filter pills
  const CATEGORY_BUCKETS: Record<string, string> = {
    cms: 'cms', ecommerce: 'ecommerce', forum: 'forum', blog: 'cms',
    productivity: 'cloud', cloud: 'cloud', email: 'cloud',
    dev: 'dev', framework: 'dev', ai: 'dev',
    lms: 'lms',
  };

  // Filter pills shown in UI
  const FILTER_PILLS = [
    { key: 'all',       label: 'All' },
    { key: 'cms',       label: 'CMS' },
    { key: 'ecommerce', label: 'E-commerce' },
    { key: 'forum',     label: 'Forum' },
    { key: 'cloud',     label: 'Cloud' },
    { key: 'lms',       label: 'LMS' },
    { key: 'dev',       label: 'Dev Tools' },
    { key: 'other',     label: 'Other' },
  ];

  // ── State ──────────────────────────────────────────────────────────────────

  let apps: App[] = [];
  let installations: Installation[] = [];
  let sites: Array<{id: string; domain: string}> = [];
  let loading = true;

  let searchQuery    = '';
  let activeCategory = 'all';
  let sortMode: 'popular' | 'alpha' = 'popular';

  // Install modal
  let showInstallModal = false;
  let selectedApp: App | null = null;
  let installSiteId    = '';
  let installSubdir    = '';
  let installEmail     = '';
  let installPassword  = '';
  let installTitle     = '';
  let installing       = false;
  let installProgress  = 0;
  let installStep      = '';
  let installError     = '';

  // Update modal
  let showUpdateModal = false;
  let updatingInst: Installation | null = null;
  let updateProgress = 0;
  let updateStep = '';
  let updating = false;

  // Simulated update versions (in a real app the backend supplies these)
  const UPDATE_VERSIONS: Record<string, string> = {
    wordpress:  '6.5.3',
    woocommerce: '8.9.1',
    joomla:     '5.1.2',
    drupal:     '10.2.5',
    magento:    '2.4.7',
    nextcloud:  '29.0.2',
    ghost:      '5.82.2',
  };

  // Inline confirm state
  let confirmUninstallId: string | null = null;
  let confirmUpdateAll = false;

  // Toast
  let toastMessage = '';
  let toastType: 'success' | 'error' = 'success';

  // ── Lifecycle ──────────────────────────────────────────────────────────────

  onMount(async () => {
    await Promise.all([loadApps(), loadInstallations(), loadSites()]);
    loading = false;
  });

  async function loadApps()          { try { apps = await api.apps.list(); } catch { apps = []; } }
  async function loadInstallations() { try { installations = await api.apps.listInstalled(); } catch { installations = []; } }
  async function loadSites() {
    try {
      const resp = await api.sites.list();
      sites = resp.map((s: any) => ({ id: s.id, domain: s.domain }));
      if (sites.length > 0) installSiteId = sites[0].id;
    } catch { sites = []; }
  }

  // ── Derived ────────────────────────────────────────────────────────────────

  $: filteredApps = (() => {
    let list = apps.filter(app => {
      // Category filter — match on bucket
      if (activeCategory !== 'all') {
        const appBucket = CATEGORY_BUCKETS[app.category] ?? 'other';
        if (appBucket !== activeCategory && app.category !== activeCategory) return false;
      }
      const q = searchQuery.toLowerCase();
      return !q || app.name.toLowerCase().includes(q) || app.description.toLowerCase().includes(q);
    });

    if (sortMode === 'popular') {
      list = [...list].sort((a, b) => {
        const ra = APP_META[a.id.toLowerCase()]?.popularRank ?? 999;
        const rb = APP_META[b.id.toLowerCase()]?.popularRank ?? 999;
        return ra - rb;
      });
    } else {
      list = [...list].sort((a, b) => a.name.localeCompare(b.name));
    }
    return list;
  })();

  // Installs that have a newer version available
  $: updatableInstalls = installations.filter(i => {
    const newVer = UPDATE_VERSIONS[i.app_id];
    return newVer && i.status === 'active' && newVer !== i.version;
  });

  function getUpdateVersion(inst: Installation): string | null {
    return UPDATE_VERSIONS[inst.app_id] ?? null;
  }

  function timeAgo(s: string | null): string {
    if (!s) return '—';
    const ms = Date.now() - new Date(s).getTime();
    const days = Math.floor(ms / 86400000);
    if (days === 0) return 'today';
    if (days === 1) return 'yesterday';
    if (days < 30) return `${days}d ago`;
    const months = Math.floor(days / 30);
    if (months < 12) return `${months}mo ago`;
    return `${Math.floor(months / 12)}y ago`;
  }

  // ── Actions ────────────────────────────────────────────────────────────────

  function openInstall(app: App) {
    selectedApp     = app;
    installSubdir   = '';
    installEmail    = '';
    installPassword = '';
    installTitle    = app.name + ' Site';
    installError    = '';
    installProgress = 0;
    installStep     = '';
    showInstallModal = true;
  }

  async function confirmInstall() {
    if (!selectedApp || !installSiteId) return;
    installing    = true;
    installError  = '';

    const params: Record<string, string> = {};
    if (installSubdir)   params.subdir         = installSubdir;
    if (installEmail)    params.admin_email     = installEmail;
    if (installPassword) params.admin_password  = installPassword;
    if (installTitle)    params.site_title      = installTitle;

    try {
      installStep = 'Preparing…'; installProgress = 10;
      await new Promise(r => setTimeout(r, 300));
      installStep = 'Downloading…'; installProgress = 35;
      await api.apps.install({ app_id: selectedApp.id, site_id: installSiteId, params });
      installStep = 'Installing…'; installProgress = 70;
      await new Promise(r => setTimeout(r, 300));
      installStep = 'Finalising…'; installProgress = 90;
      await new Promise(r => setTimeout(r, 200));
      installProgress = 100;
      installStep = 'Done';
      showToast(`${selectedApp.name} installation queued`, 'success');
      showInstallModal = false;
      await loadInstallations();
    } catch (e: any) {
      installError = e.message || 'Installation failed';
    } finally {
      installing = false;
    }
  }

  async function uninstall(id: string, name: string) {
    try {
      await api.apps.uninstall(id);
      installations = installations.filter(i => i.id !== id);
      showToast('App removed', 'success');
    } catch {
      showToast('Failed to remove app', 'error');
    }
  }

  function openUpdateModal(inst: Installation) {
    updatingInst = inst;
    updateProgress = 0;
    updateStep = '';
    updating = false;
    showUpdateModal = true;
  }

  async function confirmUpdate() {
    if (!updatingInst) return;
    updating = true;
    try {
      updateStep = 'Backing up…'; updateProgress = 15;
      await new Promise(r => setTimeout(r, 400));
      updateStep = 'Downloading update…'; updateProgress = 40;
      await api.apps.update(updatingInst.id);
      updateStep = 'Installing…'; updateProgress = 70;
      await new Promise(r => setTimeout(r, 400));
      updateStep = 'Verifying…'; updateProgress = 90;
      await new Promise(r => setTimeout(r, 300));
      updateProgress = 100;
      updateStep = 'Done';
      showToast('Update queued', 'success');
      showUpdateModal = false;
      await loadInstallations();
    } catch {
      showToast('Failed to queue update', 'error');
    } finally {
      updating = false;
    }
  }

  async function updateAll() {
    confirmUpdateAll = false;
    for (const inst of updatableInstalls) {
      try {
        await api.apps.update(inst.id);
      } catch {}
    }
    showToast(`${updatableInstalls.length} update${updatableInstalls.length !== 1 ? 's' : ''} queued`, 'success');
    await loadInstallations();
  }

  function showToast(msg: string, type: 'success' | 'error') {
    toastMessage = msg; toastType = type;
    setTimeout(() => { toastMessage = ''; }, 4000);
  }

  function fmtDate(s: string | null) {
    return s ? new Date(s).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' }) : '—';
  }

  function statusBadgeClass(status: string): string {
    const map: Record<string, string> = {
      active:     'bg-green-500/10 text-green-400 border border-green-500/20',
      installing: 'bg-amber-500/10 text-amber-400 border border-amber-500/20',
      pending:    'bg-blue-500/10 text-blue-400 border border-blue-500/20',
      failed:     'bg-red-500/10 text-red-400 border border-red-500/20',
      updating:   'bg-purple-500/10 text-purple-400 border border-purple-500/20',
    };
    return map[status] ?? 'bg-muted text-muted-foreground border border-border';
  }
</script>

<style>
  @keyframes fadeUp {
    from { opacity: 0; transform: translateY(6px); }
    to   { opacity: 1; transform: none; }
  }
  .fade-up { animation: fadeUp 0.25s ease-out both; }
</style>

<svelte:head><title>{$t('apps.title')} — JottiCP</title></svelte:head>

<!-- Toast -->
{#if toastMessage}
  <div class="fixed bottom-4 right-4 z-50 bg-card border border-border rounded-xl px-4 py-3 text-sm shadow-lg flex items-center gap-2"
       role="status" aria-live="polite">
    {#if toastType === 'success'}
      <svg class="w-4 h-4 text-green-400 shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>
      </svg>
    {:else}
      <svg class="w-4 h-4 text-red-400 shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
      </svg>
    {/if}
    <span class="text-foreground">{toastMessage}</span>
  </div>
{/if}

<div class="p-4 lg:p-6 space-y-6 max-w-[1400px] mx-auto">

  <!-- ── Header ──────────────────────────────────────────────────────────── -->
  <div class="flex items-center justify-between gap-4 flex-wrap">
    <div>
      <h1 class="text-2xl font-semibold text-foreground">{$t('apps.title')}</h1>
      <p class="text-sm text-muted-foreground mt-0.5">{$t('apps.subtitle')}</p>
    </div>
    <!-- Update All button -->
    {#if !loading && updatableInstalls.length > 0}
      {#if confirmUpdateAll}
        <div class="flex items-center gap-1.5 fade-up">
          <span class="text-xs text-amber-400">Update all {updatableInstalls.length} app{updatableInstalls.length !== 1 ? 's' : ''}?</span>
          <button class="text-xs px-2 py-1 rounded bg-amber-500 text-white hover:bg-amber-500/90" on:click={updateAll}>Yes</button>
          <button class="text-xs px-2 py-1 rounded bg-muted" on:click={() => confirmUpdateAll = false}>No</button>
        </div>
      {:else}
        <button
          class="h-9 px-4 rounded-lg bg-amber-500/10 text-amber-400 border border-amber-500/20 text-sm font-medium hover:bg-amber-500/20 inline-flex items-center gap-2 transition-colors duration-200 fade-up"
          on:click={() => confirmUpdateAll = true}
        >
          <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/>
          </svg>
          Update All ({updatableInstalls.length})
        </button>
      {/if}
    {/if}
  </div>

  {#if loading}

    <!-- Skeleton grid -->
    <div class="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-4 gap-4">
      {#each Array(8) as _}
        <div class="h-52 bg-muted rounded-xl animate-pulse"></div>
      {/each}
    </div>

  {:else}

    <!-- ── Installed Apps ─────────────────────────────────────────────── -->
    {#if installations.length > 0}
      <div class="space-y-3 fade-up">
        <h2 class="text-sm font-semibold text-foreground">Installed Applications</h2>
        <div class="bg-card border border-border rounded-xl overflow-hidden">
          <div class="overflow-x-auto">
            <table class="w-full">
              <thead class="bg-muted/50">
                <tr>
                  <th class="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase">App</th>
                  <th class="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase hidden sm:table-cell">Domain</th>
                  <th class="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase hidden md:table-cell">Version</th>
                  <th class="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase hidden lg:table-cell">Health</th>
                  <th class="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase">Status</th>
                  <th class="px-4 py-3 text-right text-xs font-medium text-muted-foreground uppercase">Actions</th>
                </tr>
              </thead>
              <tbody>
                {#each installations as inst (inst.id)}
                  {@const app = apps.find(a => a.id === inst.app_id)}
                  {@const meta = appMeta(inst.app_id)}
                  {@const newVer = getUpdateVersion(inst)}
                  {@const hasUpdate = !!newVer && newVer !== inst.version}
                  <tr class="border-t border-border hover:bg-muted/30 transition-colors duration-200">
                    <!-- App name + icon -->
                    <td class="px-4 py-3">
                      <div class="flex items-center gap-3">
                        <div class="w-9 h-9 rounded-xl flex items-center justify-center text-white text-xs font-bold shrink-0 shadow-sm transition-all duration-200 hover:scale-110"
                             style="background: {meta.brandColor};">
                          {meta.icon}
                        </div>
                        <div>
                          <span class="font-medium text-foreground text-sm">{app?.name ?? inst.app_id}</span>
                          {#if hasUpdate}
                            <span class="ml-2 text-[10px] px-1.5 py-0.5 rounded-full bg-amber-500/10 text-amber-400 border border-amber-500/20 font-semibold">
                              Update available
                            </span>
                          {/if}
                        </div>
                      </div>
                    </td>
                    <!-- Domain -->
                    <td class="px-4 py-3 text-xs text-muted-foreground hidden sm:table-cell">
                      {sites.find(s => s.id === inst.site_id)?.domain ?? '—'}
                    </td>
                    <!-- Version -->
                    <td class="px-4 py-3 hidden md:table-cell">
                      <span class="font-mono text-xs text-muted-foreground">{inst.version}</span>
                      {#if hasUpdate}
                        <span class="ml-1 text-xs text-amber-400">→ {newVer}</span>
                      {:else}
                        <span class="ml-1 text-[10px] text-green-400 font-medium">Up to date</span>
                      {/if}
                    </td>
                    <!-- Health column -->
                    <td class="px-4 py-3 hidden lg:table-cell">
                      <div class="space-y-1">
                        <!-- Security status (simulated) -->
                        <span class="inline-flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded-full bg-green-500/10 text-green-400 border border-green-500/20 font-medium">
                          <svg class="w-2.5 h-2.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z"/>
                          </svg>
                          No vulnerabilities
                        </span>
                        <!-- Last updated -->
                        {#if inst.installed_at}
                          <p class="text-[10px] text-muted-foreground">Updated {timeAgo(inst.installed_at)}</p>
                        {/if}
                      </div>
                    </td>
                    <!-- Status -->
                    <td class="px-4 py-3">
                      <span class="px-2 py-0.5 rounded-full text-xs font-medium capitalize {statusBadgeClass(inst.status)}">
                        {inst.status}
                      </span>
                    </td>
                    <!-- Actions -->
                    <td class="px-4 py-3">
                      <div class="flex gap-1.5 justify-end items-center flex-wrap">
                        {#if inst.admin_url}
                          <a href={inst.admin_url} target="_blank" rel="noopener"
                             class="h-8 px-3 rounded-lg border border-border text-xs text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-1 transition-colors duration-200">
                            Open
                            <svg class="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                                d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14"/>
                            </svg>
                          </a>
                        {/if}
                        {#if inst.status === 'active' && hasUpdate}
                          <button
                            class="h-8 px-3 rounded-lg bg-amber-500/10 text-amber-400 border border-amber-500/20 text-xs font-medium hover:bg-amber-500/20 transition-colors duration-200 inline-flex items-center gap-1"
                            on:click={() => openUpdateModal(inst)}
                          >
                            <svg class="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/>
                            </svg>
                            Update
                          </button>
                        {:else if inst.status === 'active'}
                          <button
                            class="h-8 px-3 rounded-lg border border-border text-xs text-muted-foreground hover:bg-muted hover:text-foreground transition-colors duration-200"
                            on:click={() => openUpdateModal(inst)}
                          >Update</button>
                        {/if}
                        {#if confirmUninstallId === inst.id}
                          <div class="flex items-center gap-1.5">
                            <span class="text-xs text-destructive">Remove {app?.name ?? inst.app_id}?</span>
                            <button class="text-xs px-2 py-1 rounded bg-destructive text-white hover:bg-destructive/90" on:click={() => { uninstall(inst.id, app?.name ?? inst.app_id); confirmUninstallId = null; }}>Yes</button>
                            <button class="text-xs px-2 py-1 rounded bg-muted" on:click={() => confirmUninstallId = null}>No</button>
                          </div>
                        {:else}
                          <button
                            class="h-8 px-3 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-xs font-medium hover:bg-red-500/20 transition-colors duration-200"
                            on:click={() => confirmUninstallId = inst.id}
                          >Uninstall</button>
                        {/if}
                      </div>
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        </div>
      </div>
    {/if}

    <!-- ── Search + Category filter pills + Sort ──────────────────────── -->
    <div class="space-y-3 fade-up">
      <div class="flex flex-col sm:flex-row gap-3 items-start sm:items-center">
        <!-- Search -->
        <div class="relative flex-1 max-w-sm">
          <svg class="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground pointer-events-none"
               fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"/>
          </svg>
          <input
            type="text" bind:value={searchQuery}
            placeholder="Search apps…"
            class="w-full h-9 pl-9 pr-4 rounded-lg border border-border bg-background text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
          />
        </div>

        <!-- Sort toggle -->
        <div class="flex rounded-lg border border-border overflow-hidden text-sm">
          <button
            class="h-9 px-3 transition-colors duration-200 inline-flex items-center gap-1.5
                   {sortMode === 'popular' ? 'bg-primary text-primary-foreground' : 'bg-background text-muted-foreground hover:bg-muted hover:text-foreground'}"
            on:click={() => sortMode = 'popular'}
          >
            <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 7h8m0 0v8m0-8l-8 8-4-4-6 6"/>
            </svg>
            Popular
          </button>
          <button
            class="h-9 px-3 transition-colors duration-200 inline-flex items-center gap-1.5
                   {sortMode === 'alpha' ? 'bg-primary text-primary-foreground' : 'bg-background text-muted-foreground hover:bg-muted hover:text-foreground'}"
            on:click={() => sortMode = 'alpha'}
          >
            <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M3 4h13M3 8h9m-9 4h6m4 0l4-4m0 0l4 4m-4-4v12"/>
            </svg>
            A–Z
          </button>
        </div>
      </div>

      <!-- Category filter pills -->
      <div class="flex gap-2 flex-wrap">
        {#each FILTER_PILLS as pill}
          <button
            type="button"
            on:click={() => activeCategory = pill.key}
            class="h-8 px-3 rounded-full text-xs font-medium transition-colors duration-200
                   {activeCategory === pill.key
                     ? 'bg-primary text-primary-foreground'
                     : 'border border-border text-muted-foreground hover:bg-muted hover:text-foreground'}"
          >
            {pill.label}
          </button>
        {/each}
      </div>
    </div>

    <!-- Stats strip -->
    <div class="flex gap-5 text-sm text-muted-foreground">
      <span><strong class="text-foreground">{apps.length}</strong> apps available</span>
      {#if installations.length > 0}
        <span><strong class="text-foreground">{installations.length}</strong> installed</span>
      {/if}
      {#if searchQuery || activeCategory !== 'all'}
        <span><strong class="text-foreground">{filteredApps.length}</strong> matching</span>
      {/if}
      {#if updatableInstalls.length > 0}
        <span class="text-amber-400"><strong>{updatableInstalls.length}</strong> update{updatableInstalls.length !== 1 ? 's' : ''} available</span>
      {/if}
    </div>

    <!-- ── App grid ───────────────────────────────────────────────────── -->
    {#if filteredApps.length === 0}
      <div class="text-center py-16">
        <svg class="w-12 h-12 text-muted-foreground mx-auto mb-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
            d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"/>
        </svg>
        <p class="font-medium text-foreground">No apps found</p>
        <p class="text-sm text-muted-foreground mt-1">Try a different search or category.</p>
      </div>
    {:else}
      <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-4">
        {#each filteredApps as app (app.id)}
          {@const meta = appMeta(app.id)}
          {@const installed = installations.find(i => i.app_id === app.id)}
          {@const newVer = installed ? getUpdateVersion(installed) : null}
          {@const hasUpdate = !!newVer && installed && newVer !== installed.version}
          <div class="bg-card border border-border rounded-xl p-5 flex flex-col
                      transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20 hover:border-primary/30 cursor-pointer group fade-up">

            <!-- App logo + status badge -->
            <div class="flex items-center justify-between mb-4">
              <div class="w-12 h-12 rounded-xl flex items-center justify-center text-white font-bold text-xl shadow-sm transition-transform duration-200 group-hover:scale-105"
                   style="background: {meta.brandColor};">
                {meta.icon}
              </div>
              <div class="flex flex-col items-end gap-1">
                {#if installed}
                  <span class="bg-green-500/10 text-green-400 border border-green-500/20 px-2 py-0.5 rounded-full text-xs font-medium">
                    Installed
                  </span>
                  {#if hasUpdate}
                    <span class="bg-amber-500/10 text-amber-400 border border-amber-500/20 px-2 py-0.5 rounded-full text-[10px] font-medium">
                      {installed.version} → {newVer}
                    </span>
                  {/if}
                {/if}
              </div>
            </div>

            <!-- App info -->
            <div class="flex-1 min-w-0 mb-4">
              <h3 class="font-semibold text-foreground text-sm">{app.name}</h3>
              <p class="text-xs text-muted-foreground mt-1 line-clamp-2 leading-relaxed">{app.description}</p>
            </div>

            <!-- Version + Category -->
            <div class="flex items-center gap-2 mb-4 flex-wrap">
              <span class="bg-muted text-muted-foreground border border-border px-2 py-0.5 rounded-full text-xs font-medium">
                v{app.version}
              </span>
              <span class="bg-primary/10 text-primary border border-primary/20 px-2 py-0.5 rounded-full text-xs font-medium capitalize">
                {CATEGORY_LABELS[app.category] ?? app.category}
              </span>
            </div>

            <!-- Install / Update button -->
            {#if installed && hasUpdate}
              <button
                class="w-full h-9 rounded-lg text-sm font-medium transition-colors duration-200 inline-flex items-center justify-center gap-2
                       bg-amber-500/10 text-amber-400 border border-amber-500/20 hover:bg-amber-500/20"
                on:click={() => openUpdateModal(installed)}
              >
                <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/>
                </svg>
                Update to {newVer}
              </button>
            {:else}
              <button
                class="w-full h-9 rounded-lg text-sm font-medium transition-colors duration-200 inline-flex items-center justify-center gap-2
                       {installed
                         ? 'border border-border text-muted-foreground hover:bg-muted hover:text-foreground'
                         : 'bg-primary text-primary-foreground hover:bg-primary/90'}"
                on:click={() => openInstall(app)}
              >
                {#if installed}
                  Reinstall
                {:else}
                  <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4"/>
                  </svg>
                  Install
                {/if}
              </button>
            {/if}
          </div>
        {/each}
      </div>
    {/if}

  {/if}
</div>

<!-- ── Install Modal ────────────────────────────────────────────────────── -->
{#if showInstallModal && selectedApp}
  {@const meta = appMeta(selectedApp.id)}
  <div class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
       on:click|self={() => { if (!installing) showInstallModal = false; }}>
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-lg shadow-2xl space-y-4 fade-up">

      <!-- Header -->
      <div class="flex items-center gap-4">
        <div class="w-12 h-12 rounded-xl flex items-center justify-center text-white font-bold text-xl shadow shrink-0"
             style="background: {meta.brandColor};">
          {meta.icon}
        </div>
        <div>
          <h3 class="text-base font-bold text-foreground">Install {selectedApp.name}</h3>
          <p class="text-sm text-muted-foreground">
            <span class="bg-muted text-muted-foreground border border-border px-1.5 py-0.5 rounded text-xs font-mono">v{selectedApp.version}</span>
            &nbsp;{CATEGORY_LABELS[selectedApp.category] ?? selectedApp.category}
          </p>
        </div>
      </div>

      <!-- Site + path -->
      <div class="grid grid-cols-2 gap-3">
        <div class="space-y-1">
          <label class="text-xs font-medium text-muted-foreground">Deploy to site</label>
          <select bind:value={installSiteId}
                  class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary">
            {#each sites as s}<option value={s.id}>{s.domain}</option>{/each}
          </select>
        </div>
        <div class="space-y-1">
          <label class="text-xs font-medium text-muted-foreground">Subdirectory (optional)</label>
          <input bind:value={installSubdir} placeholder="/"
                 class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary" />
        </div>
      </div>

      <!-- Common config fields -->
      <div class="space-y-1">
        <label class="text-xs font-medium text-muted-foreground">Site title</label>
        <input bind:value={installTitle} placeholder="My {selectedApp.name} Site"
               class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary" />
      </div>

      <div class="grid grid-cols-2 gap-3">
        <div class="space-y-1">
          <label class="text-xs font-medium text-muted-foreground">Admin email</label>
          <input type="email" bind:value={installEmail} placeholder="admin@example.com"
                 class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary" />
        </div>
        <div class="space-y-1">
          <label class="text-xs font-medium text-muted-foreground">Admin password</label>
          <input type="password" bind:value={installPassword} placeholder="Strong password"
                 class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary" />
        </div>
      </div>

      <!-- Progress bar (shown while installing) -->
      {#if installing}
        <div class="space-y-1.5">
          <div class="flex items-center justify-between">
            <p class="text-xs text-muted-foreground">{installStep}</p>
            <p class="text-xs font-mono text-foreground">{installProgress}%</p>
          </div>
          <div class="bg-muted rounded-full h-2">
            <div class="bg-primary h-2 rounded-full transition-all duration-500" style="width:{installProgress}%"></div>
          </div>
          <!-- Step indicators -->
          <div class="flex justify-between text-[10px] text-muted-foreground mt-1">
            {#each ['Backup', 'Download', 'Install', 'Verify'] as step, i}
              <span class:text-primary={installProgress > i * 25}>{step}</span>
            {/each}
          </div>
        </div>
      {/if}

      {#if installError}
        <div class="rounded-lg bg-red-500/10 border border-red-500/20 px-3 py-2 text-sm text-red-400">
          {installError}
        </div>
      {/if}

      <div class="flex gap-2 pt-1">
        <button
          class="flex-1 h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center justify-center gap-2 disabled:opacity-50 transition-colors duration-200"
          on:click={confirmInstall}
          disabled={installing || !installSiteId}
        >
          {#if installing}
            <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
              <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/>
              <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"/>
            </svg>
            Installing…
          {:else}
            Install Now
          {/if}
        </button>
        <button
          class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground transition-colors duration-200"
          on:click={() => showInstallModal = false}
          disabled={installing}
        >
          Cancel
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- ── Update Modal ──────────────────────────────────────────────────────── -->
{#if showUpdateModal && updatingInst}
  {@const app = apps.find(a => a.id === updatingInst!.app_id)}
  {@const meta = appMeta(updatingInst.app_id)}
  {@const newVer = getUpdateVersion(updatingInst)}
  <div class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
       on:click|self={() => { if (!updating) showUpdateModal = false; }}>
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-md shadow-2xl space-y-4 fade-up">

      <!-- Header -->
      <div class="flex items-center gap-4">
        <div class="w-12 h-12 rounded-xl flex items-center justify-center text-white font-bold text-xl shadow shrink-0"
             style="background: {meta.brandColor};">
          {meta.icon}
        </div>
        <div>
          <h3 class="text-base font-bold text-foreground">Update {app?.name ?? updatingInst.app_id}</h3>
          <p class="text-sm text-muted-foreground">
            <span class="font-mono">{updatingInst.version}</span>
            <span class="mx-1 text-amber-400">→</span>
            <span class="font-mono text-foreground">{newVer ?? 'latest'}</span>
          </p>
        </div>
      </div>

      <!-- Info -->
      <div class="rounded-lg bg-muted/40 border border-border p-3 text-xs text-muted-foreground">
        The update process will: <strong class="text-foreground">Back up</strong> your files, then <strong class="text-foreground">Download</strong> the new version, <strong class="text-foreground">Install</strong> it, and <strong class="text-foreground">Verify</strong> the result.
      </div>

      <!-- Progress (shown while updating) -->
      {#if updating}
        <div class="space-y-1.5">
          <div class="flex items-center justify-between">
            <p class="text-xs text-muted-foreground">{updateStep}</p>
            <p class="text-xs font-mono text-foreground">{updateProgress}%</p>
          </div>
          <div class="bg-muted rounded-full h-2">
            <div class="bg-amber-400 h-2 rounded-full transition-all duration-500" style="width:{updateProgress}%"></div>
          </div>
          <!-- Step indicators -->
          <div class="flex justify-between text-[10px] text-muted-foreground mt-1">
            {#each ['Backup', 'Download', 'Install', 'Verify'] as step, i}
              <span class:text-amber-400={updateProgress > i * 25}>{step}</span>
            {/each}
          </div>
        </div>
      {/if}

      <div class="flex gap-2 pt-1">
        <button
          class="flex-1 h-9 px-4 rounded-lg bg-amber-500/10 text-amber-400 border border-amber-500/20 text-sm font-medium hover:bg-amber-500/20 inline-flex items-center justify-center gap-2 disabled:opacity-50 transition-colors duration-200"
          on:click={confirmUpdate}
          disabled={updating}
        >
          {#if updating}
            <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
              <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/>
              <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"/>
            </svg>
            Updating…
          {:else}
            Confirm Update
          {/if}
        </button>
        <button
          class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground transition-colors duration-200"
          on:click={() => showUpdateModal = false}
          disabled={updating}
        >
          Cancel
        </button>
      </div>
    </div>
  </div>
{/if}
