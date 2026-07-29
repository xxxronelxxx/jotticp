<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$api/client';
  import { t } from '$lib/i18n';

  // ── Types ──────────────────────────────────────────────────────────────────
  type PhpExtension = {
    name: string;
    enabled: boolean;
    description: string;
    dev_only?: boolean;
    category?: string;
    special_note?: string;
  };

  type Site = {
    id: string;
    domain: string;
    php_version: string;
  };

  // ── State ──────────────────────────────────────────────────────────────────
  let sites: Site[] = [];
  let selectedSiteId = '';
  let loading = true;
  let loadError = '';
  let extLoading = false;
  let saving = false;
  let applyPending = false;

  let allExtensions: PhpExtension[] = [];
  let enabledExtensions: string[] = [];
  let originalEnabled: string[] = [];
  let extSearch = '';

  // Global settings state
  let settings = {
    memory_limit: '256M',
    max_execution_time: '30',
    upload_max_filesize: '64M',
    post_max_size: '64M',
    max_input_vars: '1000',
    error_mode: 'production' as 'production' | 'development',
    display_errors: false,
    opcache_enable: true,
    opcache_memory: '128',
  };
  let originalSettings = { ...settings };

  // Per-version tab
  let versionTab = '';
  $: activeVer = versionTab || installedVersions[0] || '';
  $: activeVerCount = versionSiteCount[activeVer] ?? 0;

  // Version comparison table
  let showVersionCompare = false;

  // Extension health check
  let healthCheckResult: null | { issues: HealthIssue[] } = null;
  let healthChecking = false;

  type HealthIssue = {
    severity: 'error' | 'warning' | 'info';
    title: string;
    detail: string;
    fix?: string;
  };

  // Preset confirmation
  let pendingPreset: null | { label: string; apply: () => void } = null;

  // Toast
  let toast = '';
  let toastType: 'success' | 'error' = 'success';
  let toastTimer: ReturnType<typeof setTimeout>;

  // PHP version metadata
  const versionMeta: Record<string, { label: string; color: string; textColor: string; bgColor: string; borderColor: string }> = {
    '8.4': { label: '8.4',  color: 'green',  textColor: 'text-green-400',  bgColor: 'bg-green-400/10',  borderColor: 'border-green-400/20' },
    '8.3': { label: '8.3',  color: 'green',  textColor: 'text-green-400',  bgColor: 'bg-green-400/10',  borderColor: 'border-green-400/20' },
    '8.2': { label: '8.2',  color: 'blue',   textColor: 'text-blue-400',   bgColor: 'bg-blue-400/10',   borderColor: 'border-blue-400/20' },
    '8.1': { label: '8.1',  color: 'amber',  textColor: 'text-amber-400',  bgColor: 'bg-amber-400/10',  borderColor: 'border-amber-400/20' },
    '8.0': { label: '8.0',  color: 'muted',  textColor: 'text-muted-foreground',  bgColor: 'bg-muted/60',  borderColor: 'border-border' },
    '7.4': { label: '7.4',  color: 'muted',  textColor: 'text-muted-foreground',  bgColor: 'bg-muted/60',  borderColor: 'border-border' },
  };

  const allVersions = ['8.4', '8.3', '8.2', '8.1', '8.0', '7.4'];

  // Version feature comparison data
  const versionFeatures: { feature: string; versions: Record<string, boolean> }[] = [
    { feature: 'Named arguments',    versions: { '7.4': false, '8.0': true,  '8.1': true,  '8.2': true,  '8.3': true,  '8.4': true  } },
    { feature: 'Match expression',   versions: { '7.4': false, '8.0': true,  '8.1': true,  '8.2': true,  '8.3': true,  '8.4': true  } },
    { feature: 'Nullsafe operator',  versions: { '7.4': false, '8.0': true,  '8.1': true,  '8.2': true,  '8.3': true,  '8.4': true  } },
    { feature: 'Union types',        versions: { '7.4': false, '8.0': true,  '8.1': true,  '8.2': true,  '8.3': true,  '8.4': true  } },
    { feature: 'Fibers',             versions: { '7.4': false, '8.0': false, '8.1': true,  '8.2': true,  '8.3': true,  '8.4': true  } },
    { feature: 'Enums',              versions: { '7.4': false, '8.0': false, '8.1': true,  '8.2': true,  '8.3': true,  '8.4': true  } },
    { feature: 'Readonly properties',versions: { '7.4': false, '8.0': false, '8.1': true,  '8.2': true,  '8.3': true,  '8.4': true  } },
    { feature: 'Intersection types', versions: { '7.4': false, '8.0': false, '8.1': true,  '8.2': true,  '8.3': true,  '8.4': true  } },
    { feature: 'Readonly classes',   versions: { '7.4': false, '8.0': false, '8.1': false, '8.2': true,  '8.3': true,  '8.4': true  } },
    { feature: 'DNF types',          versions: { '7.4': false, '8.0': false, '8.1': false, '8.2': true,  '8.3': true,  '8.4': true  } },
    { feature: 'Typed constants',    versions: { '7.4': false, '8.0': false, '8.1': false, '8.2': false, '8.3': true,  '8.4': true  } },
    { feature: 'Dynamic class const',versions: { '7.4': false, '8.0': false, '8.1': false, '8.2': false, '8.3': true,  '8.4': true  } },
    { feature: 'Property hooks',     versions: { '7.4': false, '8.0': false, '8.1': false, '8.2': false, '8.3': false, '8.4': true  } },
    { feature: 'Asymmetric visibility',versions: { '7.4': false, '8.0': false, '8.1': false, '8.2': false, '8.3': false, '8.4': true } },
  ];

  // Category display config
  const categoryOrder = ['core', 'database', 'networking', 'image', 'math', 'compression', 'i18n', 'caching', 'system', 'auth', 'mail', 'debug', 'special', 'other'];
  const categoryLabels: Record<string, string> = {
    core: 'Core', database: 'Database', networking: 'Networking',
    image: 'Image', math: 'Math / Crypto', compression: 'Compression',
    i18n: 'Internationalisation', caching: 'Caching', system: 'System',
    auth: 'Auth / LDAP', mail: 'Mail', debug: 'Debug', special: 'Special', other: 'Other',
  };

  // ── INI Presets ────────────────────────────────────────────────────────────
  const iniPresets = [
    {
      id: 'production',
      label: 'Production',
      description: 'Hides errors, enables OPcache, 256M memory',
      apply(): void {
        settings.memory_limit = '256M';
        settings.max_execution_time = '30';
        settings.upload_max_filesize = '64M';
        settings.post_max_size = '64M';
        settings.max_input_vars = '1000';
        settings.error_mode = 'production';
        settings.display_errors = false;
        settings.opcache_enable = true;
        settings.opcache_memory = '128';
        settings = { ...settings };
      },
    },
    {
      id: 'development',
      label: 'Development',
      description: 'Shows all errors, 512M memory',
      apply(): void {
        settings.memory_limit = '512M';
        settings.max_execution_time = '120';
        settings.upload_max_filesize = '128M';
        settings.post_max_size = '128M';
        settings.max_input_vars = '5000';
        settings.error_mode = 'development';
        settings.display_errors = true;
        settings.opcache_enable = false;
        settings.opcache_memory = '64';
        settings = { ...settings };
      },
    },
    {
      id: 'wordpress',
      label: 'WordPress',
      description: '256M memory, 64M upload, 300s execution',
      apply(): void {
        settings.memory_limit = '256M';
        settings.max_execution_time = '300';
        settings.upload_max_filesize = '64M';
        settings.post_max_size = '64M';
        settings.max_input_vars = '3000';
        settings.error_mode = 'production';
        settings.display_errors = false;
        settings.opcache_enable = true;
        settings.opcache_memory = '128';
        settings = { ...settings };
      },
    },
    {
      id: 'highperf',
      label: 'High Performance',
      description: '512M memory, aggressive OPcache tuning',
      apply(): void {
        settings.memory_limit = '512M';
        settings.max_execution_time = '60';
        settings.upload_max_filesize = '64M';
        settings.post_max_size = '64M';
        settings.max_input_vars = '1000';
        settings.error_mode = 'production';
        settings.display_errors = false;
        settings.opcache_enable = true;
        settings.opcache_memory = '256';
        settings = { ...settings };
      },
    },
  ];

  // ── Lifecycle ──────────────────────────────────────────────────────────────
  onMount(async () => {
    try {
      const resp = await api.sites.list();
      sites = resp.map((s: any) => ({ id: s.id, domain: s.domain, php_version: s.php_version }));
      if (sites.length > 0) {
        selectedSiteId = sites[0].id;
        await loadExtensions();
      }
    } catch (err: unknown) {
      loadError = (err as { message?: string })?.message ?? 'Failed to load sites';
    } finally {
      loading = false;
    }
  });

  // ── Derived ────────────────────────────────────────────────────────────────
  $: selectedSite = sites.find(s => s.id === selectedSiteId);

  $: installedVersions = (() => {
    const seen = new Set<string>();
    for (const s of sites) if (s.php_version) seen.add(s.php_version);
    return allVersions.filter(v => seen.has(v));
  })();

  $: versionSiteCount = (() => {
    const map: Record<string, number> = {};
    for (const s of sites) {
      map[s.php_version] = (map[s.php_version] ?? 0) + 1;
    }
    return map;
  })();

  $: groupedExtensions = (() => {
    const filtered = extSearch
      ? allExtensions.filter(e =>
          e.name.toLowerCase().includes(extSearch.toLowerCase()) ||
          (e.description ?? '').toLowerCase().includes(extSearch.toLowerCase()))
      : allExtensions;

    return categoryOrder.reduce((acc, cat) => {
      const group = filtered.filter(e => (e.category ?? 'other') === cat);
      if (group.length) acc[cat] = group;
      return acc;
    }, {} as Record<string, PhpExtension[]>);
  })();

  $: hasChanges = (() => {
    if (JSON.stringify(enabledExtensions.slice().sort()) !== JSON.stringify(originalEnabled.slice().sort())) return true;
    for (const k of Object.keys(settings) as (keyof typeof settings)[]) {
      if (settings[k] !== originalSettings[k]) return true;
    }
    return false;
  })();

  $: ioncubeExt = allExtensions.find(e => e.name === 'ioncube_loader');

  // Visible versions for compare table (installed + 8.3 fallback)
  $: compareVersions = installedVersions.length > 0 ? installedVersions : ['8.0', '8.1', '8.2', '8.3'];

  // ── Data ───────────────────────────────────────────────────────────────────
  async function loadExtensions() {
    extLoading = true;
    try {
      const exts = await api.php.extensions(selectedSiteId) as PhpExtension[];
      allExtensions = exts;
      enabledExtensions = exts.filter(e => e.enabled).map(e => e.name);
      originalEnabled = [...enabledExtensions];
    } catch {
      showToast('Failed to load extensions', 'error');
    } finally {
      extLoading = false;
    }
  }

  // ── Actions ────────────────────────────────────────────────────────────────
  function toggleExtension(name: string) {
    if (enabledExtensions.includes(name)) {
      enabledExtensions = enabledExtensions.filter(e => e !== name);
    } else {
      enabledExtensions = [...enabledExtensions, name];
    }
  }

  async function applyChanges() {
    saving = true;
    try {
      const extMap: Record<string, boolean> = {};
      for (const e of allExtensions) {
        extMap[e.name] = enabledExtensions.includes(e.name);
      }
      await api.php.updateExtensions(selectedSiteId, { extensions: extMap });
      originalEnabled = [...enabledExtensions];
      originalSettings = { ...settings };
      showToast('PHP configuration applied', 'success');
      applyPending = false;
    } catch (e: any) {
      showToast(e.message || 'Failed to apply changes', 'error');
    } finally {
      saving = false;
    }
  }

  async function flushOpcache() {
    try {
      await api.php.flushOpcache(selectedSiteId);
      showToast('OPcache flushed', 'success');
    } catch {
      showToast('Failed to flush OPcache', 'error');
    }
  }

  async function handleSiteChange() {
    await loadExtensions();
    settings = { ...originalSettings };
  }

  // Preset handling
  function requestPreset(preset: typeof iniPresets[0]) {
    pendingPreset = { label: preset.label, apply: preset.apply };
  }

  function confirmPreset() {
    if (pendingPreset) {
      pendingPreset.apply();
      showToast(`${pendingPreset.label} preset applied`, 'success');
      pendingPreset = null;
    }
  }

  // Extension health check
  function runHealthCheck() {
    healthChecking = true;
    const issues: HealthIssue[] = [];

    const recommended = ['intl', 'gd', 'zip', 'mbstring', 'curl'];
    for (const ext of recommended) {
      if (!enabledExtensions.includes(ext)) {
        issues.push({
          severity: 'warning',
          title: `${ext} not enabled`,
          detail: `The ${ext} extension is commonly needed by CMS applications and frameworks.`,
          fix: `Enable the ${ext} extension in the Extensions panel above.`,
        });
      }
    }

    const xdebugEnabled = enabledExtensions.includes('xdebug');
    if (xdebugEnabled && settings.error_mode === 'production') {
      issues.push({
        severity: 'error',
        title: 'xdebug enabled in production mode',
        detail: 'xdebug significantly degrades performance and may expose sensitive data in a production environment.',
        fix: 'Disable xdebug or switch to the Development preset if this is a dev environment.',
      });
    }

    if (settings.display_errors && settings.error_mode === 'production') {
      issues.push({
        severity: 'warning',
        title: 'display_errors is On in production mode',
        detail: 'Showing errors in browser output can leak sensitive server information.',
        fix: 'Turn off display_errors or switch to the Production preset.',
      });
    }

    if (parseInt(settings.opcache_memory) < 64 && settings.opcache_enable) {
      issues.push({
        severity: 'info',
        title: 'OPcache memory is low',
        detail: `${settings.opcache_memory}MB may be insufficient for large applications.`,
        fix: 'Increase opcache.memory_consumption to at least 128MB.',
      });
    }

    if (issues.length === 0) {
      issues.push({
        severity: 'info',
        title: 'No issues found',
        detail: 'All recommended extensions are enabled and no security risks were detected.',
      });
    }

    healthCheckResult = { issues };
    healthChecking = false;
  }

  function healthSeverityClass(s: HealthIssue['severity']): string {
    return s === 'error'
      ? 'bg-red-500/10 border-red-500/20 text-red-400'
      : s === 'warning'
      ? 'bg-amber-500/10 border-amber-500/20 text-amber-400'
      : 'bg-blue-500/10 border-blue-500/20 text-blue-400';
  }

  // ── Helpers ────────────────────────────────────────────────────────────────
  function showToast(msg: string, type: 'success' | 'error') {
    clearTimeout(toastTimer);
    toast = msg;
    toastType = type;
    toastTimer = setTimeout(() => (toast = ''), 4000);
  }
</script>

<style>
  @keyframes fadeUp {
    from { opacity: 0; transform: translateY(6px); }
    to   { opacity: 1; transform: none; }
  }
  .fade-up { animation: fadeUp 0.25s ease-out both; }
</style>

<svelte:head><title>{$t('php.title')} — JottiCP</title></svelte:head>

<!-- ── Toast ──────────────────────────────────────────────────────────────── -->
{#if toast}
  <div class="fixed bottom-4 right-4 z-50 bg-card border border-border rounded-xl px-4 py-3 text-sm shadow-lg flex items-center gap-2"
       class:text-green-400={toastType === 'success'}
       class:text-red-400={toastType === 'error'}>
    {#if toastType === 'success'}
      <svg class="w-4 h-4 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>
      </svg>
    {:else}
      <svg class="w-4 h-4 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
      </svg>
    {/if}
    {toast}
  </div>
{/if}

<!-- ── Preset Confirm Dialog ───────────────────────────────────────────────── -->
{#if pendingPreset}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
       on:click|self={() => pendingPreset = null}>
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-sm shadow-2xl fade-up">
      <div class="flex items-start gap-3 mb-4">
        <div class="w-10 h-10 rounded-full bg-amber-500/10 flex items-center justify-center flex-shrink-0">
          <svg class="w-5 h-5 text-amber-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"/>
          </svg>
        </div>
        <div>
          <h3 class="text-base font-semibold text-foreground">Apply {pendingPreset.label} preset?</h3>
          <p class="text-sm text-muted-foreground mt-0.5">This will override the current settings form values. You can still cancel before applying.</p>
        </div>
      </div>
      <div class="flex gap-3">
        <button
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors duration-200"
          on:click={confirmPreset}
        >Apply Preset</button>
        <button
          class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground transition-colors duration-200"
          on:click={() => pendingPreset = null}
        >Cancel</button>
      </div>
    </div>
  </div>
{/if}

<div class="p-6 space-y-6 max-w-[1400px] mx-auto">
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

  <!-- ── Header ─────────────────────────────────────────────────────────── -->
  <div class="flex items-center justify-between gap-4 flex-wrap">
    <div>
      <h1 class="text-2xl font-semibold text-foreground">{$t('php.title')}</h1>
      <p class="text-sm text-muted-foreground mt-0.5">{$t('php.subtitle')}</p>
    </div>
    <button
      class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 disabled:opacity-40 transition-colors duration-200"
      disabled={!hasChanges || saving}
      on:click={applyChanges}
    >
      {#if saving}
        <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
          <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/>
          <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"/>
        </svg>
        Applying…
      {:else}
        <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>
        </svg>
        {$t('php.apply_changes')}
      {/if}
    </button>
  </div>

  {#if loading}
    <!-- Skeleton -->
    <div class="space-y-4 animate-pulse">
      <div class="h-28 bg-card border border-border rounded-xl"></div>
      <div class="h-64 bg-card border border-border rounded-xl"></div>
    </div>
  {:else}

    <!-- ── PHP Version Management ────────────────────────────────────────── -->
    <div class="bg-card border border-border rounded-xl p-6 fade-up">
      <div class="flex items-center justify-between mb-4">
        <div>
          <h2 class="text-base font-semibold text-foreground">{$t('php.version_mgmt')}</h2>
          <p class="text-sm text-muted-foreground mt-0.5">{$t('php.version_mgmt_desc')}</p>
        </div>
      </div>

      <div class="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-6 gap-3">
        {#each allVersions as ver}
          {@const meta = versionMeta[ver] ?? { textColor: 'text-muted-foreground', bgColor: 'bg-muted/60', borderColor: 'border-border' }}
          {@const count = versionSiteCount[ver] ?? 0}
          {@const installed = count > 0}
          <div class="rounded-xl border p-4 flex flex-col gap-2 transition-all duration-200
            {installed
              ? 'border-border bg-background hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20'
              : 'border-border/50 bg-muted/20 opacity-60'}">
            <!-- Version badge -->
            <div class="flex items-center justify-between">
              <span class="text-xs font-semibold px-2 py-0.5 rounded-full border {meta.bgColor} {meta.textColor} {meta.borderColor}">
                PHP {ver}
              </span>
              {#if ver === '8.3' || ver === '8.4'}
                <span class="text-xs bg-green-400/10 text-green-400 border border-green-400/20 px-1.5 py-0.5 rounded-full font-medium">Latest</span>
              {/if}
            </div>
            <!-- Status -->
            <p class="text-xs text-muted-foreground">
              {#if installed}
                <span class="text-foreground font-medium">{count}</span> site{count !== 1 ? 's' : ''}
              {:else}
                {$t('php.not_in_use')}
              {/if}
            </p>
            <!-- Status badge -->
            <span class="text-xs font-medium {installed ? 'text-green-400' : 'text-muted-foreground'}">
              {installed ? $t('php.active') : $t('php.available')}
            </span>
          </div>
        {/each}
      </div>

      <!-- Version Compare toggle -->
      <div class="mt-4 border-t border-border pt-4">
        <button
          class="text-sm font-medium text-muted-foreground hover:text-foreground inline-flex items-center gap-1.5 transition-colors duration-200"
          on:click={() => showVersionCompare = !showVersionCompare}
        >
          <svg class="w-4 h-4 transition-transform duration-200 {showVersionCompare ? 'rotate-180' : ''}" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7"/>
          </svg>
          {(showVersionCompare ? $t('php.hide_compare') : $t('php.show_compare'))}
        </button>

        {#if showVersionCompare}
          <div class="mt-4 overflow-x-auto fade-up">
            <table class="w-full text-sm border border-border rounded-xl overflow-hidden">
              <thead class="bg-muted/50">
                <tr>
                  <th class="px-4 py-2.5 text-left text-xs font-medium text-muted-foreground uppercase tracking-wide">{$t('php.feature_col')}</th>
                  {#each compareVersions as ver}
                    {@const meta = versionMeta[ver]}
                    <th class="px-3 py-2.5 text-center text-xs font-medium {meta?.textColor ?? 'text-muted-foreground'} uppercase tracking-wide">
                      PHP {ver}
                    </th>
                  {/each}
                </tr>
              </thead>
              <tbody>
                {#each versionFeatures as row}
                  <tr class="border-t border-border hover:bg-muted/20 transition-colors duration-200">
                    <td class="px-4 py-2 text-sm text-foreground">{row.feature}</td>
                    {#each compareVersions as ver}
                      <td class="px-3 py-2 text-center">
                        {#if row.versions[ver]}
                          <span class="text-green-400 text-base leading-none" title="Supported">✓</span>
                        {:else}
                          <span class="text-muted-foreground/40 text-base leading-none" title="Not supported">✗</span>
                        {/if}
                      </td>
                    {/each}
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        {/if}
      </div>
    </div>

    <!-- ── Site selector + flush ─────────────────────────────────────────── -->
    <div class="bg-card border border-border rounded-xl p-4 flex flex-wrap gap-3 items-center">
      <div class="flex-1 min-w-48">
          <label class="block text-xs font-medium text-muted-foreground mb-1">{$t('php.configuring_site')}</label>
        <div class="relative">
          <select
            class="w-full h-9 rounded-lg border border-border bg-background px-3 pr-8 text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 appearance-none"
            bind:value={selectedSiteId}
            on:change={handleSiteChange}
          >
            {#each sites as site}
              <option value={site.id}>{site.domain} — PHP {site.php_version}</option>
            {/each}
          </select>
          <svg class="pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7"/>
          </svg>
        </div>
      </div>

      {#if selectedSite}
        <div class="flex items-end gap-2 mt-4 sm:mt-0">
          <div class="text-right hidden sm:block">
            <p class="text-xs text-muted-foreground">{$t('php.active_version')}</p>
            <p class="text-lg font-bold text-foreground">{selectedSite.php_version}</p>
          </div>
          <button
            class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2 transition-colors duration-200"
            on:click={flushOpcache}
          >
            <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/>
            </svg>
            {$t('php.flush_opcache')}
          </button>
          <a
            href="/websites/{selectedSite.id}"
            class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2 transition-colors duration-200"
          >
            <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14"/>
            </svg>
            Site settings
          </a>
        </div>
      {/if}
    </div>

    <!-- ── Two-column layout: settings + extensions ───────────────────────── -->
    <div class="grid grid-cols-1 xl:grid-cols-3 gap-6">

      <!-- Global PHP Settings card -->
      <div class="bg-card border border-border rounded-xl p-6">
        <h2 class="text-base font-semibold text-foreground mb-3">Global PHP Settings</h2>

        <!-- INI Presets -->
        <div class="mb-4">
          <p class="text-xs font-medium text-muted-foreground mb-2">Quick Presets</p>
          <div class="grid grid-cols-2 gap-2">
            {#each iniPresets as preset}
              <button
                class="text-left px-3 py-2 rounded-lg border border-border bg-background hover:bg-muted hover:border-primary/30 transition-colors duration-200 group"
                on:click={() => requestPreset(preset)}
              >
                <p class="text-xs font-semibold text-foreground group-hover:text-primary transition-colors duration-200">{preset.label}</p>
                <p class="text-[10px] text-muted-foreground mt-0.5 leading-snug">{preset.description}</p>
              </button>
            {/each}
          </div>
        </div>

        {#if ioncubeExt}
          <div class="mb-4 p-3 rounded-lg border border-amber-500/30 bg-amber-500/10 text-xs text-amber-300">
            <strong class="font-semibold">IonCube Loader</strong> requires manual installation.
            Download from <a href="https://ioncube.com" target="_blank" rel="noopener" class="underline hover:text-amber-200">ioncube.com</a>,
            place <code class="font-mono bg-amber-500/10 px-1 py-0.5 rounded">ioncube_loader.so</code> in the PHP extension dir, then restart PHP-FPM.
          </div>
        {/if}

        <dl class="divide-y divide-border">

          <!-- memory_limit -->
          <div class="flex items-center justify-between py-3 gap-3">
            <div class="min-w-0">
              <dt class="text-sm font-medium text-foreground">memory_limit</dt>
              <dd class="text-xs text-muted-foreground mt-0.5">Max memory per script</dd>
            </div>
            <input
              class="w-24 h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 text-right font-mono"
              bind:value={settings.memory_limit}
              placeholder="256M"
            />
          </div>

          <!-- max_execution_time -->
          <div class="flex items-center justify-between py-3 gap-3">
            <div class="min-w-0">
              <dt class="text-sm font-medium text-foreground">max_execution_time</dt>
              <dd class="text-xs text-muted-foreground mt-0.5">Seconds before timeout</dd>
            </div>
            <input
              class="w-24 h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 text-right font-mono"
              bind:value={settings.max_execution_time}
              placeholder="30"
            />
          </div>

          <!-- upload_max_filesize -->
          <div class="flex items-center justify-between py-3 gap-3">
            <div class="min-w-0">
              <dt class="text-sm font-medium text-foreground">upload_max_filesize</dt>
              <dd class="text-xs text-muted-foreground mt-0.5">Max uploaded file size</dd>
            </div>
            <input
              class="w-24 h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 text-right font-mono"
              bind:value={settings.upload_max_filesize}
              placeholder="64M"
            />
          </div>

          <!-- post_max_size -->
          <div class="flex items-center justify-between py-3 gap-3">
            <div class="min-w-0">
              <dt class="text-sm font-medium text-foreground">post_max_size</dt>
              <dd class="text-xs text-muted-foreground mt-0.5">Max POST body size</dd>
            </div>
            <input
              class="w-24 h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 text-right font-mono"
              bind:value={settings.post_max_size}
              placeholder="64M"
            />
          </div>

          <!-- max_input_vars -->
          <div class="flex items-center justify-between py-3 gap-3">
            <div class="min-w-0">
              <dt class="text-sm font-medium text-foreground">max_input_vars</dt>
              <dd class="text-xs text-muted-foreground mt-0.5">Max form input variables</dd>
            </div>
            <input
              class="w-24 h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 text-right font-mono"
              bind:value={settings.max_input_vars}
              placeholder="1000"
            />
          </div>

          <!-- error_reporting mode -->
          <div class="flex items-center justify-between py-3 gap-3">
            <div class="min-w-0">
              <dt class="text-sm font-medium text-foreground">error_reporting</dt>
              <dd class="text-xs text-muted-foreground mt-0.5">Production hides errors; dev shows all</dd>
            </div>
            <div class="flex rounded-lg border border-border overflow-hidden text-xs">
              <button
                class="px-2.5 h-8 transition-colors duration-200 {settings.error_mode === 'production' ? 'bg-primary text-primary-foreground' : 'bg-background text-muted-foreground hover:bg-muted'}"
                on:click={() => settings.error_mode = 'production'}
              >Prod</button>
              <button
                class="px-2.5 h-8 transition-colors duration-200 {settings.error_mode === 'development' ? 'bg-primary text-primary-foreground' : 'bg-background text-muted-foreground hover:bg-muted'}"
                on:click={() => settings.error_mode = 'development'}
              >Dev</button>
            </div>
          </div>

          <!-- display_errors -->
          <div class="flex items-center justify-between py-3 gap-3">
            <div class="min-w-0">
              <dt class="text-sm font-medium text-foreground">display_errors</dt>
              <dd class="text-xs text-muted-foreground mt-0.5">Show errors in browser output</dd>
            </div>
            <button
              class="inline-flex h-5 w-9 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 focus:outline-none focus:ring-2 focus:ring-primary/50"
              class:bg-primary={settings.display_errors}
              class:bg-muted={!settings.display_errors}
              on:click={() => settings.display_errors = !settings.display_errors}
              aria-label="Toggle display_errors"
            >
              <span
                class="block h-4 w-4 rounded-full bg-background shadow transform transition-transform"
                class:translate-x-4={settings.display_errors}
                class:translate-x-0={!settings.display_errors}
              ></span>
            </button>
          </div>

          <!-- opcache.enable -->
          <div class="flex items-center justify-between py-3 gap-3">
            <div class="min-w-0">
              <dt class="text-sm font-medium text-foreground">opcache.enable</dt>
              <dd class="text-xs text-muted-foreground mt-0.5">Opcode cache for performance</dd>
            </div>
            <button
              class="inline-flex h-5 w-9 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 focus:outline-none focus:ring-2 focus:ring-primary/50"
              class:bg-primary={settings.opcache_enable}
              class:bg-muted={!settings.opcache_enable}
              on:click={() => settings.opcache_enable = !settings.opcache_enable}
              aria-label="Toggle opcache.enable"
            >
              <span
                class="block h-4 w-4 rounded-full bg-background shadow transform transition-transform"
                class:translate-x-4={settings.opcache_enable}
                class:translate-x-0={!settings.opcache_enable}
              ></span>
            </button>
          </div>

          <!-- opcache.memory_consumption -->
          <div class="flex items-center justify-between py-3 gap-3">
            <div class="min-w-0">
              <dt class="text-sm font-medium text-foreground">opcache.memory_consumption</dt>
              <dd class="text-xs text-muted-foreground mt-0.5">OPcache memory in MB</dd>
            </div>
            <input
              class="w-24 h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 text-right font-mono disabled:opacity-50"
              bind:value={settings.opcache_memory}
              disabled={!settings.opcache_enable}
              placeholder="128"
            />
          </div>

        </dl>

        {#if hasChanges}
          <div class="mt-4 pt-4 border-t border-border">
            <p class="text-xs text-amber-400 flex items-center gap-1.5">
              <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <circle cx="12" cy="12" r="10"/><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01"/>
              </svg>
              Unsaved changes — click "Apply Changes" to save.
            </p>
          </div>
        {/if}
      </div>

      <!-- Extensions panel -->
      <div class="xl:col-span-2 bg-card border border-border rounded-xl p-6">
        <div class="flex items-center justify-between gap-3 mb-4 flex-wrap">
          <div>
            <h2 class="text-base font-semibold text-foreground">PHP Extensions</h2>
            <p class="text-sm text-muted-foreground mt-0.5">
              {enabledExtensions.length} enabled · {allExtensions.length - enabledExtensions.length} disabled
            </p>
          </div>
          <div class="flex items-center gap-2 flex-wrap">
            <!-- Health check button -->
            <button
              class="h-9 px-3 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-1.5 transition-colors duration-200 disabled:opacity-50"
              on:click={runHealthCheck}
              disabled={healthChecking || allExtensions.length === 0}
            >
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z"/>
              </svg>
              Health Check
            </button>
            <!-- Extension search -->
            <div class="relative min-w-48">
              <svg class="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <circle cx="11" cy="11" r="8"/><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 21l-4.35-4.35"/>
              </svg>
              <input
                class="w-full h-9 rounded-lg border border-border bg-background pl-8 pr-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50"
                placeholder="Search extensions…"
                bind:value={extSearch}
              />
            </div>
          </div>
        </div>

        <!-- Health check results -->
        {#if healthCheckResult}
          <div class="mb-5 space-y-2 fade-up">
            <div class="flex items-center justify-between mb-2">
              <p class="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Health Check Results</p>
              <button
                class="text-xs text-muted-foreground hover:text-foreground transition-colors duration-200"
                on:click={() => healthCheckResult = null}
              >Dismiss</button>
            </div>
            {#each healthCheckResult.issues as issue}
              <div class="flex items-start gap-3 p-3 rounded-lg border {healthSeverityClass(issue.severity)}">
                <div class="text-sm font-semibold shrink-0">
                  {#if issue.severity === 'error'}
                    <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <circle cx="12" cy="12" r="10"/><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 9l-6 6M9 9l6 6"/>
                    </svg>
                  {:else if issue.severity === 'warning'}
                    <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"/>
                    </svg>
                  {:else}
                    <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <circle cx="12" cy="12" r="10"/><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01"/>
                    </svg>
                  {/if}
                </div>
                <div class="min-w-0">
                  <p class="text-xs font-semibold">{issue.title}</p>
                  <p class="text-xs mt-0.5 opacity-80">{issue.detail}</p>
                  {#if issue.fix}
                    <p class="text-xs mt-1 font-medium opacity-90">Fix: {issue.fix}</p>
                  {/if}
                </div>
              </div>
            {/each}
          </div>
        {/if}

        {#if extLoading}
          <div class="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 gap-2">
            {#each Array(12) as _}
              <div class="h-14 rounded-lg bg-muted/40 animate-pulse"></div>
            {/each}
          </div>
        {:else if allExtensions.length === 0}
          <div class="py-12 flex flex-col items-center gap-2 text-center">
            <p class="text-sm font-medium text-foreground">No extensions found</p>
            <p class="text-xs text-muted-foreground">Select a site above to load its PHP extensions.</p>
          </div>
        {:else}
          <div class="space-y-5">
            {#each Object.entries(groupedExtensions) as [cat, exts]}
              <div>
                <p class="text-xs font-semibold uppercase tracking-wide text-muted-foreground mb-2">
                  {categoryLabels[cat] ?? cat}
                </p>
                <div class="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 gap-2">
                  {#each exts as ext}
                    {@const enabled = enabledExtensions.includes(ext.name)}
                    <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
                    <div
                      class="relative flex flex-col gap-1.5 px-3 py-2.5 rounded-lg border cursor-pointer select-none transition-all duration-200
                        {enabled
                          ? 'bg-primary/10 border-primary/30 text-foreground hover:-translate-y-0.5 hover:shadow-lg hover:shadow-black/20'
                          : 'bg-background border-border text-muted-foreground hover:border-border/80 hover:bg-muted/30'}"
                      on:click={() => toggleExtension(ext.name)}
                      title={ext.description || ext.name}
                    >
                      <div class="flex items-center justify-between gap-1">
                        <code class="font-mono text-xs font-medium truncate">{ext.name}</code>
                        <div class="flex items-center gap-1 flex-shrink-0">
                          {#if ext.dev_only}
                            <span class="text-[10px] font-semibold px-1 py-0.5 rounded bg-orange-500/10 text-orange-400 border border-orange-500/20 leading-none">Dev</span>
                          {/if}
                          <!-- Mini toggle indicator -->
                          <div
                            class="h-3.5 w-6 rounded-full border border-transparent transition-colors duration-200 flex-shrink-0"
                            class:bg-primary={enabled}
                            class:bg-muted={!enabled}
                          >
                            <div
                              class="h-3 w-3 rounded-full bg-background shadow-sm transform transition-transform mt-px"
                              class:translate-x-2.5={enabled}
                              class:translate-x-px={!enabled}
                            ></div>
                          </div>
                        </div>
                      </div>
                      {#if ext.description}
                        <p class="text-[10px] leading-tight text-muted-foreground truncate">{ext.description}</p>
                      {/if}
                      {#if ext.special_note}
                        <p class="text-[10px] leading-tight text-amber-400 truncate">{ext.special_note}</p>
                      {/if}
                    </div>
                  {/each}
                </div>
              </div>
            {/each}
          </div>
        {/if}
      </div>
    </div>

    <!-- ── Per-version config tabs ────────────────────────────────────────── -->
    {#if installedVersions.length > 0}
      <div class="bg-card border border-border rounded-xl overflow-hidden">
        <div class="border-b border-border px-6 pt-4">
          <h2 class="text-base font-semibold text-foreground mb-3">Per-Version INI Settings</h2>
          <!-- Tab bar -->
          <div class="flex gap-1 -mb-px">
            {#each installedVersions as ver}
              {@const meta = versionMeta[ver] ?? { textColor: 'text-muted-foreground' }}
              <button
                class="px-4 py-2 text-sm font-medium border-b-2 transition-colors duration-200
                  {(versionTab || installedVersions[0]) === ver
                    ? `border-primary text-foreground`
                    : `border-transparent text-muted-foreground hover:text-foreground hover:border-border`}"
                on:click={() => versionTab = ver}
              >
                <span class="font-mono {meta.textColor}">PHP {ver}</span>
                <span class="ml-1.5 text-xs text-muted-foreground">({versionSiteCount[ver] ?? 0} sites)</span>
              </button>
            {/each}
          </div>
        </div>

        <div class="p-6">
          <div class="flex items-start gap-4 mb-4">
            <div>
              <p class="text-sm text-muted-foreground">
                Showing INI overrides applied to all <span class="font-medium text-foreground">{activeVerCount}</span> site{activeVerCount !== 1 ? 's' : ''} running PHP <span class="font-mono font-medium text-foreground">{activeVer}</span>.
                Per-site settings can be adjusted from each site's PHP tab.
              </p>
            </div>
          </div>

          <!-- Version-specific settings table (informational) -->
          <div class="rounded-xl border border-border overflow-hidden">
            <table class="w-full text-sm">
              <thead class="bg-muted/50">
                <tr>
                  <th class="px-4 py-2.5 text-left text-xs font-medium text-muted-foreground uppercase tracking-wide">INI Directive</th>
                  <th class="px-4 py-2.5 text-left text-xs font-medium text-muted-foreground uppercase tracking-wide">Value (this form)</th>
                  <th class="px-4 py-2.5 text-left text-xs font-medium text-muted-foreground uppercase tracking-wide">Notes</th>
                </tr>
              </thead>
              <tbody>
                {#each [
                  { key: 'memory_limit',             val: settings.memory_limit,         note: 'Per-worker memory cap' },
                  { key: 'max_execution_time',        val: settings.max_execution_time + 's', note: 'Script timeout in seconds' },
                  { key: 'upload_max_filesize',       val: settings.upload_max_filesize,  note: 'Single file upload limit' },
                  { key: 'post_max_size',             val: settings.post_max_size,        note: 'Must be ≥ upload_max_filesize' },
                  { key: 'max_input_vars',            val: settings.max_input_vars,       note: 'Form field limit' },
                  { key: 'display_errors',            val: settings.display_errors ? 'On' : 'Off', note: settings.display_errors ? 'Errors shown in output' : 'Errors hidden from output' },
                  { key: 'opcache.enable',            val: settings.opcache_enable ? '1' : '0', note: 'Bytecode cache' },
                  { key: 'opcache.memory_consumption', val: settings.opcache_memory + 'M', note: 'OPcache memory pool' },
                ] as row}
                  <tr class="border-t border-border hover:bg-muted/30 transition-colors duration-200">
                    <td class="px-4 py-3"><code class="font-mono text-xs bg-muted px-1.5 py-0.5 rounded text-foreground">{row.key}</code></td>
                    <td class="px-4 py-3"><span class="font-mono text-sm text-foreground">{row.val}</span></td>
                    <td class="px-4 py-3 text-xs text-muted-foreground">{row.note}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        </div>
      </div>
    {/if}

  {/if}
</div>
