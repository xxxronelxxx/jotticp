<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { api } from '$api/client';

  // ── Types ──────────────────────────────────────────────────────────────────

  type App = {
    id: string; name: string; description: string;
    version: string; category: string; icon_url: string | null;
    requires_pro?: boolean;
  };

  type Site = { id: string; domain: string; status: string; };

  // ── App metadata ───────────────────────────────────────────────────────────

  const APP_META: Record<string, { color: string; icon: string }> = {
    wordpress:   { color: '#2271B1', icon: 'W' },
    woocommerce: { color: '#96588A', icon: 'WC' },
    joomla:      { color: '#F44321', icon: 'J' },
    drupal:      { color: '#0077C0', icon: 'D' },
    magento:     { color: '#EE672F', icon: 'Mg' },
    prestashop:  { color: '#DF0067', icon: 'PS' },
    phpbb:       { color: '#1B5E9B', icon: 'BB' },
    nextcloud:   { color: '#0082C9', icon: 'NC' },
    moodle:      { color: '#FF7800', icon: 'Mo' },
    gitea:       { color: '#609926', icon: 'G' },
    ghost:       { color: '#15171A', icon: 'Gh' },
    laravel:     { color: '#FF2D20', icon: 'L' },
    django:      { color: '#092E20', icon: 'Dj' },
    fastapi:     { color: '#009688', icon: 'FA' },
    n8n_ai:      { color: '#EA4B71', icon: 'n8' },
    default:     { color: '#6366F1', icon: '?' },
  };

  function appMeta(id: string) {
    return APP_META[id.toLowerCase()] ?? APP_META.default;
  }

  const CATEGORY_LABELS: Record<string, string> = {
    cms: 'CMS', ecommerce: 'E-commerce', forum: 'Forum', blog: 'Blog',
    productivity: 'Productivity', email: 'Email', dev: 'Dev Tools',
    framework: 'Framework', ai: 'AI / Automation', all: 'All',
  };

  // ── Wizard state ───────────────────────────────────────────────────────────

  // Steps: 1=Choose app, 2=Choose site+path, 3=Configure, 4=Installing, 5=Done
  let step = 1;

  // Step 1
  let apps: App[] = [];
  let appsLoading  = true;
  let searchQuery  = '';
  let activeCategory = 'all';
  let selectedApp: App | null = null;

  // Step 2
  let sites: Site[]    = [];
  let sitesLoading     = true;
  let selectedSiteId   = '';
  let createNewSite    = false;
  let newDomain        = '';
  let installSubdir    = '';

  // Step 3
  let siteTitle       = '';
  let adminEmail      = '';
  let adminPassword   = '';
  let showPassword    = false;

  // Step 4 — installation progress
  let installId    = '';
  let progressPct  = 0;
  let progressSteps: { label: string; done: boolean; active: boolean }[] = [];
  let progressLog: string[] = [];
  let installing   = false;
  let installError = '';
  let ws: WebSocket | null = null;

  // Step 5
  let adminUrl    = '';
  let siteUrl     = '';

  // Toast
  let toastMessage = '';
  let toastType: 'success' | 'error' = 'success';
  let loadError = '';

  // ── Lifecycle ──────────────────────────────────────────────────────────────

  onMount(async () => {
    // Load apps and sites in parallel
    try {
      const [appList, siteList] = await Promise.all([
        api.apps.list(),
        api.sites.list(),
      ]);
      apps  = appList;
      sites = (siteList as Site[]).filter(s => s.status === 'active');
      if (sites.length > 0) selectedSiteId = sites[0].id;
    } catch (err: unknown) {
      loadError = (err as { message?: string })?.message ?? 'Failed to load data';
    } finally {
      appsLoading  = false;
      sitesLoading = false;
    }
  });

  onDestroy(() => { ws?.close(); });

  // ── Derived ────────────────────────────────────────────────────────────────

  $: categories = ['all', ...Array.from(new Set(apps.map(a => a.category)))];

  $: filteredApps = apps.filter(app => {
    const matchCat    = activeCategory === 'all' || app.category === activeCategory;
    const q           = searchQuery.toLowerCase();
    const matchSearch = !q || app.name.toLowerCase().includes(q) || app.description.toLowerCase().includes(q);
    return matchCat && matchSearch;
  });

  $: canProceed1 = selectedApp !== null;
  $: canProceed2 = createNewSite ? newDomain.trim().length > 0 : selectedSiteId.length > 0;
  $: canProceed3 = adminEmail.trim().length > 0 && adminPassword.trim().length > 0;

  $: selectedSiteDomain = createNewSite
    ? newDomain
    : (sites.find(s => s.id === selectedSiteId)?.domain ?? '');

  // ── Navigation ─────────────────────────────────────────────────────────────

  function goNext() { step = Math.min(step + 1, 5); }
  function goBack() { step = Math.max(step - 1, 1); }

  function selectApp(app: App) {
    selectedApp = app;
    siteTitle   = app.name + ' Site';
  }

  // ── Install ────────────────────────────────────────────────────────────────

  const INSTALL_STEPS = [
    { label: 'Preparing site', key: 'prepare' },
    { label: 'Downloading package', key: 'download' },
    { label: 'Extracting files', key: 'extract' },
    { label: 'Configuring database', key: 'database' },
    { label: 'Running installer', key: 'install' },
    { label: 'Finalising', key: 'finalise' },
  ];

  function buildProgressSteps(currentKey: string | null = null) {
    const currentIdx = currentKey ? INSTALL_STEPS.findIndex(s => s.key === currentKey) : -1;
    return INSTALL_STEPS.map((s, i) => ({
      label:  s.label,
      done:   i < currentIdx,
      active: i === currentIdx,
    }));
  }

  async function startInstall() {
    if (!selectedApp || !selectedSiteId && !createNewSite) return;
    installing    = true;
    installError  = '';
    progressPct   = 5;
    progressLog   = [];
    progressSteps = buildProgressSteps('prepare');
    step          = 4;

    let siteId = selectedSiteId;

    // Create new site if requested
    if (createNewSite) {
      progressLog = [...progressLog, `Creating site: ${newDomain}…`];
      try {
        const site = await api.sites.create({
          domain: newDomain, php_version: '8.3', web_server: 'nginx',
        }) as Site;
        siteId = site.id;
        progressLog = [...progressLog, `Site created (${site.id})`];
      } catch (e: any) {
        installError = `Failed to create site: ${e.message}`;
        installing   = false;
        return;
      }
    }

    // Queue installation
    const params: Record<string, string> = {};
    if (installSubdir)  params.subdir         = installSubdir;
    if (adminEmail)     params.admin_email     = adminEmail;
    if (adminPassword)  params.admin_password  = adminPassword;
    if (siteTitle)      params.site_title      = siteTitle;

    let newInstallId: string;
    try {
      progressLog = [...progressLog, `Queuing ${selectedApp.name} install…`];
      const resp = await api.apps.install({
        app_id:  selectedApp.id,
        site_id: siteId,
        params,
      }) as { id?: string; job_id?: string };
      // API may return { id } or { job_id }
      newInstallId = resp.id ?? resp.job_id ?? '';
      installId    = newInstallId;
      progressPct  = 15;
      progressLog  = [...progressLog, `Install job queued (${newInstallId}). Connecting to progress stream…`];
    } catch (e: any) {
      installError = e.message ?? 'Failed to queue installation';
      installing   = false;
      return;
    }

    // Connect WebSocket for live progress
    // Correct WS endpoint: /api/v1/apps/installed/{id}/progress
    const proto = location.protocol === 'https:' ? 'wss' : 'ws';
    const wsUrl = `${proto}://${location.host}/api/v1/apps/installed/${newInstallId}/progress`;
    ws = new WebSocket(wsUrl);

    ws.onmessage = (e) => {
      try {
        const msg = JSON.parse(e.data) as {
          step?: string; pct?: number; msg?: string; status?: string;
          admin_url?: string; error?: string;
        };

        if (msg.msg)  progressLog = [...progressLog, msg.msg];
        if (msg.pct)  progressPct = msg.pct;
        if (msg.step) progressSteps = buildProgressSteps(msg.step);

        if (msg.step === 'done' || msg.status === 'active') {
          progressPct   = 100;
          progressSteps = buildProgressSteps(null).map(s => ({ ...s, done: true, active: false }));
          adminUrl      = msg.admin_url ?? '';
          siteUrl       = `https://${selectedSiteDomain}${installSubdir ? '/' + installSubdir.replace(/^\//, '') : ''}`;
          installing    = false;
          step          = 5;
          ws?.close();
        }

        if (msg.step === 'error' || msg.error) {
          installError = msg.msg ?? msg.error ?? 'Installation failed';
          installing   = false;
          ws?.close();
        }
      } catch {
        progressLog = [...progressLog, e.data];
      }
    };

    ws.onerror = () => {
      // WS failed — check installed list manually
      installError = 'Live progress unavailable. Check "Installed Apps" for status.';
      installing   = false;
    };

    ws.onclose = () => {
      if (installing) {
        installing = false;
      }
    };
  }

  function reset() {
    step           = 1;
    selectedApp    = null;
    searchQuery    = '';
    activeCategory = 'all';
    selectedSiteId = sites[0]?.id ?? '';
    createNewSite  = false;
    newDomain      = '';
    installSubdir  = '';
    siteTitle      = '';
    adminEmail     = '';
    adminPassword  = '';
    progressPct    = 0;
    progressSteps  = [];
    progressLog    = [];
    installError   = '';
    installing     = false;
    adminUrl       = '';
    siteUrl        = '';
    ws?.close();
    ws = null;
  }

  function showToast(msg: string, type: 'success' | 'error') {
    toastMessage = msg; toastType = type;
    setTimeout(() => { toastMessage = ''; }, 4000);
  }
</script>

<svelte:head><title>Install App Wizard — JottiCP</title></svelte:head>

<!-- Toast -->
{#if toastMessage}
  <div class="fixed bottom-4 right-4 z-50 bg-card border border-border rounded-xl px-4 py-3 text-sm shadow-lg flex items-center gap-2"
       role="status" aria-live="polite">
    <span class="text-foreground">{toastMessage}</span>
  </div>
{/if}

<div class="p-4 lg:p-6 space-y-6 max-w-4xl">
  <!-- ── Load error banner ──────────────────────────────────────────────── -->
  {#if loadError}
    <div class="flex items-center gap-3 px-4 py-3 rounded-xl
                bg-red-500/10 border border-red-500/20 text-sm text-red-400">
      <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round"
          d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z" />
      </svg>
      <span>{loadError}</span>
      <button on:click={() => window.location.reload()}
              class="ml-auto text-xs font-semibold underline underline-offset-2 hover:no-underline">
        Retry
      </button>
    </div>
  {/if}



  <!-- ── Header ──────────────────────────────────────────────────────────── -->
  <div class="flex items-center gap-3">
    <a href="/apps" class="text-muted-foreground hover:text-foreground transition-colors">
      <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7"/>
      </svg>
    </a>
    <div>
      <h1 class="text-xl font-bold text-foreground">Install App Wizard</h1>
      <p class="text-sm text-muted-foreground">
        {#if selectedApp && step > 1}
          Installing {selectedApp.name}
        {:else}
          Choose and configure an application
        {/if}
      </p>
    </div>
  </div>

  <!-- ── Step progress bar ───────────────────────────────────────────────── -->
  <div class="flex items-center" aria-label="Wizard progress">
    {#each [
      { n: 1, label: 'Choose App' },
      { n: 2, label: 'Choose Site' },
      { n: 3, label: 'Configure' },
      { n: 4, label: 'Installing' },
      { n: 5, label: 'Done' },
    ] as s}
      <div class="flex flex-col items-center gap-1">
        <div class="w-8 h-8 rounded-full flex items-center justify-center text-sm font-semibold border-2 transition-all
                    {step > s.n
                      ? 'border-green-500 bg-green-500 text-white'
                      : step === s.n
                        ? 'border-primary bg-primary text-primary-foreground'
                        : 'border-border bg-background text-muted-foreground'}">
          {#if step > s.n}
            <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2.5" d="M5 13l4 4L19 7"/>
            </svg>
          {:else}
            {s.n}
          {/if}
        </div>
        <span class="text-xs hidden sm:block transition-colors
                     {step === s.n ? 'text-foreground font-medium' : 'text-muted-foreground'}">
          {s.label}
        </span>
      </div>
      {#if s.n < 5}
        <div class="flex-1 h-0.5 mx-2 transition-colors
                    {step > s.n ? 'bg-green-500' : 'bg-border'}"></div>
      {/if}
    {/each}
  </div>

  <!-- ══════════════════════════════════════════════════════════════════════
       Step 1: Choose App
  ══════════════════════════════════════════════════════════════════════════ -->
  {#if step === 1}
    <div class="space-y-4">
      <h2 class="text-base font-semibold text-foreground">Choose an application</h2>

      <!-- Search + category filter -->
      <div class="flex flex-col sm:flex-row gap-3">
        <div class="relative flex-1 max-w-xs">
          <svg class="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground pointer-events-none"
               fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"/>
          </svg>
          <input type="text" bind:value={searchQuery} placeholder="Search…"
                 class="w-full h-9 pl-9 pr-3 rounded-lg border border-border bg-background text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary" />
        </div>
        <div class="flex gap-2 flex-wrap">
          {#each categories as cat}
            <button type="button" on:click={() => activeCategory = cat}
                    class="h-9 px-3 rounded-lg text-sm font-medium transition-colors
                           {activeCategory === cat
                             ? 'bg-primary text-primary-foreground'
                             : 'border border-border text-muted-foreground hover:bg-muted hover:text-foreground'}">
              {CATEGORY_LABELS[cat] ?? cat}
            </button>
          {/each}
        </div>
      </div>

      {#if appsLoading}
        <div class="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-4 gap-4">
          {#each Array(8) as _}
            <div class="h-32 bg-muted rounded-xl animate-pulse"></div>
          {/each}
        </div>
      {:else if filteredApps.length === 0}
        <div class="text-center py-12 text-muted-foreground">
          <p class="font-medium text-foreground">No apps found</p>
          <p class="text-sm mt-1">Try a different search.</p>
        </div>
      {:else}
        <div class="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-4 gap-3">
          {#each filteredApps as app (app.id)}
            {@const meta = appMeta(app.id)}
            <button
              type="button"
              on:click={() => selectApp(app)}
              class="relative flex flex-col items-center gap-2 p-4 rounded-xl border-2 text-left transition-all
                     {selectedApp?.id === app.id
                       ? 'border-primary bg-primary/5 shadow-sm'
                       : 'border-border bg-card hover:border-primary/50 hover:bg-muted/30'}"
            >
              {#if selectedApp?.id === app.id}
                <div class="absolute top-2 right-2 w-5 h-5 rounded-full bg-primary flex items-center justify-center">
                  <svg class="w-3 h-3 text-primary-foreground" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="3" d="M5 13l4 4L19 7"/>
                  </svg>
                </div>
              {/if}
              <div class="w-10 h-10 rounded-xl flex items-center justify-center text-white text-xs font-bold"
                   style="background: {meta.color};">
                {meta.icon}
              </div>
              <div class="text-center min-w-0 w-full">
                <p class="text-sm font-semibold text-foreground truncate">{app.name}</p>
                <p class="text-xs text-muted-foreground mt-0.5 line-clamp-2 leading-relaxed">{app.description}</p>
                <span class="mt-1 inline-block bg-muted text-muted-foreground border border-border px-1.5 py-0.5 rounded text-xs">
                  v{app.version}
                </span>
              </div>
            </button>
          {/each}
        </div>
      {/if}

      <div class="flex justify-end pt-2">
        <button
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 disabled:opacity-50"
          on:click={goNext}
          disabled={!canProceed1}
        >
          Next: Choose Site
          <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7"/>
          </svg>
        </button>
      </div>
    </div>

  <!-- ══════════════════════════════════════════════════════════════════════
       Step 2: Choose Site + Path
  ══════════════════════════════════════════════════════════════════════════ -->
  {:else if step === 2}
    <div class="space-y-4">
      <h2 class="text-base font-semibold text-foreground">Choose deployment site</h2>

      <!-- Existing / New toggle -->
      <div class="flex rounded-lg border border-border overflow-hidden w-fit">
        <button
          type="button"
          on:click={() => createNewSite = false}
          class="px-4 py-2 text-sm font-medium transition-colors
                 {!createNewSite ? 'bg-primary text-primary-foreground' : 'text-muted-foreground hover:bg-muted hover:text-foreground'}"
        >
          Existing site
        </button>
        <button
          type="button"
          on:click={() => createNewSite = true}
          class="px-4 py-2 text-sm font-medium transition-colors
                 {createNewSite ? 'bg-primary text-primary-foreground' : 'text-muted-foreground hover:bg-muted hover:text-foreground'}"
        >
          Create new site
        </button>
      </div>

      <div class="bg-card border border-border rounded-xl p-5 space-y-4">
        {#if !createNewSite}
          {#if sitesLoading}
            <div class="h-9 bg-muted rounded-lg animate-pulse"></div>
          {:else if sites.length === 0}
            <div class="rounded-lg bg-amber-500/10 border border-amber-500/20 px-3 py-2 text-sm text-amber-400">
              No active sites found. Use "Create new site" or add a site from the Sites page first.
            </div>
          {:else}
            <div class="space-y-1">
              <label class="text-xs font-medium text-muted-foreground">Select site</label>
              <select bind:value={selectedSiteId}
                      class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary">
                {#each sites as s}<option value={s.id}>{s.domain}</option>{/each}
              </select>
            </div>
          {/if}
        {:else}
          <div class="space-y-1">
            <label class="text-xs font-medium text-muted-foreground">New domain</label>
            <input bind:value={newDomain} placeholder="app.example.com" type="text"
                   class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary" />
            <p class="text-xs text-muted-foreground">DNS must already point to this server. SSL will be provisioned automatically.</p>
          </div>
        {/if}

        <div class="space-y-1">
          <label class="text-xs font-medium text-muted-foreground">Subdirectory (optional, default /)</label>
          <input bind:value={installSubdir} placeholder="/"
                 class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary" />
        </div>

        <!-- Summary -->
        {#if selectedSiteDomain}
          <div class="rounded-lg bg-muted/50 border border-border p-3">
            <p class="text-xs text-muted-foreground mb-1">Install URL preview</p>
            <p class="text-sm font-mono text-foreground">
              https://{selectedSiteDomain}{installSubdir ? '/' + installSubdir.replace(/^\//, '') : ''}
            </p>
          </div>
        {/if}
      </div>

      <div class="flex items-center justify-between pt-2">
        <button class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2"
                on:click={goBack}>
          <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7"/>
          </svg>
          Back
        </button>
        <button
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 disabled:opacity-50"
          on:click={goNext}
          disabled={!canProceed2}
        >
          Next: Configure
          <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7"/>
          </svg>
        </button>
      </div>
    </div>

  <!-- ══════════════════════════════════════════════════════════════════════
       Step 3: Configure
  ══════════════════════════════════════════════════════════════════════════ -->
  {:else if step === 3}
    <div class="space-y-4">
      <h2 class="text-base font-semibold text-foreground">Configure {selectedApp?.name}</h2>

      <div class="bg-card border border-border rounded-xl p-5 space-y-4">

        <div class="space-y-1">
          <label class="text-xs font-medium text-muted-foreground" for="site-title">Site title</label>
          <input id="site-title" bind:value={siteTitle} placeholder="My {selectedApp?.name} Site"
                 class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary" />
        </div>

        <div class="space-y-1">
          <label class="text-xs font-medium text-muted-foreground" for="admin-email">Admin email</label>
          <input id="admin-email" type="email" bind:value={adminEmail} placeholder="admin@example.com"
                 class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary" />
        </div>

        <div class="space-y-1">
          <label class="text-xs font-medium text-muted-foreground" for="admin-pw">Admin password</label>
          <div class="relative">
            <input id="admin-pw" type={showPassword ? 'text' : 'password'} bind:value={adminPassword}
                   placeholder="Strong password"
                   class="h-9 w-full rounded-lg border border-border bg-background px-3 pr-10 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary" />
            <button type="button" class="absolute right-3 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                    on:click={() => showPassword = !showPassword}>
              {#if showPassword}
                <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                    d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97
                       9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242
                       4.242M9.88 9.88l-3.29-3.29m7.532 7.532l3.29 3.29M3 3l3.59 3.59m0
                       0A9.953 9.953 0 0112 5c4.478 0 8.268 2.943 9.543 7a10.025 10.025 0
                       01-4.132 5.411m0 0L21 21"/>
                </svg>
              {:else}
                <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                    d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"/>
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                    d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274
                       4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z"/>
                </svg>
              {/if}
            </button>
          </div>
        </div>

        <!-- Summary box -->
        <div class="rounded-lg bg-muted/50 border border-border p-3 space-y-1.5 text-sm">
          <p class="text-xs font-medium text-muted-foreground uppercase tracking-wide">Deployment summary</p>
          <div class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1">
            <span class="text-muted-foreground">App</span>
            <span class="text-foreground">{selectedApp?.name} v{selectedApp?.version}</span>
            <span class="text-muted-foreground">Site</span>
            <span class="text-foreground font-mono text-xs">{selectedSiteDomain}</span>
            {#if installSubdir}
              <span class="text-muted-foreground">Path</span>
              <span class="text-foreground font-mono text-xs">/{installSubdir.replace(/^\//, '')}</span>
            {/if}
          </div>
        </div>
      </div>

      <div class="flex items-center justify-between pt-2">
        <button class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2"
                on:click={goBack}>
          <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7"/>
          </svg>
          Back
        </button>
        <button
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 disabled:opacity-50"
          on:click={startInstall}
          disabled={!canProceed3}
        >
          <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M13 10V3L4 14h7v7l9-11h-7z"/>
          </svg>
          Install Now
        </button>
      </div>
    </div>

  <!-- ══════════════════════════════════════════════════════════════════════
       Step 4: Installing
  ══════════════════════════════════════════════════════════════════════════ -->
  {:else if step === 4}
    <div class="space-y-5">
      <div>
        <h2 class="text-base font-semibold text-foreground">
          {#if installing}
            Installing {selectedApp?.name}…
          {:else if installError}
            Installation failed
          {:else}
            Installation complete
          {/if}
        </h2>
        <p class="text-sm text-muted-foreground mt-0.5">
          {#if installing}
            Do not close this page.
          {:else if installError}
            An error occurred during installation.
          {/if}
        </p>
      </div>

      <!-- Progress bar -->
      <div class="bg-card border border-border rounded-xl p-5 space-y-4">
        <div class="space-y-1.5">
          <div class="flex items-center justify-between text-xs text-muted-foreground">
            <span>Overall progress</span>
            <span>{progressPct}%</span>
          </div>
          <div class="bg-muted rounded-full h-2">
            <div class="bg-primary h-2 rounded-full transition-all duration-500" style="width:{progressPct}%"></div>
          </div>
        </div>

        <!-- Step checklist -->
        {#if progressSteps.length > 0}
          <ul class="space-y-2">
            {#each progressSteps as s}
              <li class="flex items-center gap-2.5 text-sm">
                {#if s.done}
                  <div class="w-5 h-5 rounded-full bg-green-500/10 border border-green-500/20 flex items-center justify-center shrink-0">
                    <svg class="w-3 h-3 text-green-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2.5" d="M5 13l4 4L19 7"/>
                    </svg>
                  </div>
                  <span class="text-muted-foreground line-through">{s.label}</span>
                {:else if s.active}
                  <div class="w-5 h-5 rounded-full bg-amber-500/10 border border-amber-500/20 flex items-center justify-center shrink-0">
                    <svg class="w-3 h-3 text-amber-400 animate-spin" fill="none" viewBox="0 0 24 24">
                      <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/>
                      <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"/>
                    </svg>
                  </div>
                  <span class="text-foreground font-medium">{s.label}</span>
                {:else}
                  <div class="w-5 h-5 rounded-full bg-muted border border-border flex items-center justify-center shrink-0"></div>
                  <span class="text-muted-foreground">{s.label}</span>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
      </div>

      <!-- Log panel -->
      {#if progressLog.length > 0}
        <div class="bg-zinc-950 border border-zinc-800 rounded-xl p-4 h-48 overflow-y-auto font-mono text-xs leading-relaxed"
             aria-live="polite" aria-atomic="false">
          {#each progressLog as line}
            <p class="text-green-400 whitespace-pre-wrap break-all">{line}</p>
          {/each}
          {#if installing}
            <p class="text-green-400 animate-pulse">▋</p>
          {/if}
        </div>
      {/if}

      {#if installError}
        <div class="rounded-lg bg-red-500/10 border border-red-500/20 px-4 py-3 text-sm text-red-400">
          <strong>Error:</strong> {installError}
        </div>
      {/if}

      {#if !installing}
        <div class="flex gap-2">
          <button class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2"
                  on:click={reset}>
            Start over
          </button>
          <a href="/apps" class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2">
            View installed apps
          </a>
        </div>
      {:else}
        <p class="text-sm text-muted-foreground">Installation in progress — please wait.</p>
      {/if}
    </div>

  <!-- ══════════════════════════════════════════════════════════════════════
       Step 5: Done
  ══════════════════════════════════════════════════════════════════════════ -->
  {:else if step === 5}
    {@const meta = selectedApp ? appMeta(selectedApp.id) : APP_META.default}
    <div class="space-y-5">
      <div class="bg-card border border-border rounded-xl p-6 text-center space-y-4">
        <!-- Success icon -->
        <div class="w-14 h-14 rounded-full bg-green-500/10 border border-green-500/20 flex items-center justify-center mx-auto">
          <svg class="w-7 h-7 text-green-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>
          </svg>
        </div>

        <!-- App icon + name -->
        <div class="flex items-center justify-center gap-3">
          <div class="w-10 h-10 rounded-xl flex items-center justify-center text-white text-sm font-bold"
               style="background: {meta.color};">
            {meta.icon}
          </div>
          <div class="text-left">
            <p class="font-bold text-foreground">{selectedApp?.name} installed successfully</p>
            <p class="text-sm text-muted-foreground">{selectedSiteDomain}</p>
          </div>
        </div>

        <!-- Links -->
        <div class="flex flex-col sm:flex-row gap-3 justify-center pt-2">
          {#if siteUrl}
            <a href={siteUrl} target="_blank" rel="noopener"
               class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center justify-center gap-2">
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                  d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14"/>
              </svg>
              Open site
            </a>
          {/if}
          {#if adminUrl}
            <a href={adminUrl} target="_blank" rel="noopener"
               class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center justify-center gap-2">
              Admin panel
            </a>
          {/if}
          <a href="/apps"
             class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center justify-center gap-2">
            All installed apps
          </a>
        </div>
      </div>

      <div class="text-center">
        <button class="text-sm text-muted-foreground hover:text-foreground underline underline-offset-2 transition-colors"
                on:click={reset}>
          Install another app
        </button>
      </div>
    </div>
  {/if}

</div>
