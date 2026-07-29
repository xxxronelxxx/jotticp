<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { api } from '$api/client';
  import { auth } from '$stores/auth';
  import { t } from '$lib/i18n';

  // ── Types ──────────────────────────────────────────────────────────────────
  interface ResellerClient {
    id: string;
    email: string;
    display_name: string | null;
    sites_count: number;
    disk_used_gb?: number;
    package_name: string | null;
    status: 'active' | 'suspended';
    created_at: string;
  }

  interface ResellerPackage {
    id: string;
    name: string;
    max_sites: number;
    max_disk_gb: number;
    max_email_accounts: number;
    max_databases: number;
    price_monthly: number;
    clients_count: number;
    features: Record<string, boolean>;
    _isTemplate?: boolean;
  }

  interface ResellerStats {
    total_clients: number;
    active_clients: number;
    total_sites: number;
    total_disk_used_gb: number;
    total_revenue_monthly: number;
  }

  // Built-in package templates (used to pre-fill new package modal)
  const PACKAGE_TEMPLATES: Omit<ResellerPackage, 'id' | 'clients_count'>[] = [
    {
      name: 'Starter',
      max_sites: 5,
      max_disk_gb: 10,
      max_email_accounts: 10,
      max_databases: 5,
      price_monthly: 9,
      features: { ssl: true, backups: false, git_deploy: false, waf: false, staging: false },
      _isTemplate: true,
    },
    {
      name: 'Professional',
      max_sites: 20,
      max_disk_gb: 50,
      max_email_accounts: 50,
      max_databases: 20,
      price_monthly: 29,
      features: { ssl: true, backups: true, git_deploy: true, waf: false, staging: false },
      _isTemplate: true,
    },
    {
      name: 'Business',
      max_sites: 9999,
      max_disk_gb: 200,
      max_email_accounts: 200,
      max_databases: 100,
      price_monthly: 79,
      features: { ssl: true, backups: true, git_deploy: true, waf: true, staging: true },
      _isTemplate: true,
    },
  ];

  const TEMPLATE_COLORS = [
    { band: 'from-blue-500/30 to-blue-600/10', icon: 'text-blue-400', badge: 'bg-blue-500/10 text-blue-400 border border-blue-500/20' },
    { band: 'from-violet-500/30 to-violet-600/10', icon: 'text-violet-400', badge: 'bg-violet-500/10 text-violet-400 border border-violet-500/20' },
    { band: 'from-emerald-500/30 to-emerald-600/10', icon: 'text-emerald-400', badge: 'bg-emerald-500/10 text-emerald-400 border border-emerald-500/20' },
  ];

  // ── State ──────────────────────────────────────────────────────────────────
  let activeTab: 'clients' | 'packages' | 'revenue' = 'clients';
  let clients: ResellerClient[] = [];
  let packages: ResellerPackage[] = [];
  let stats: ResellerStats | null = null;
  let statsLoading = true;
  let clientsLoading = false;
  let packagesLoading = false;
  let loadedTabs = new Set<string>();

  // Impersonation banner
  let impersonatingEmail: string | null = null;

  // Inline confirm state
  let confirmRemoveClientId: string | null = null;
  let confirmDeletePkgId: string | null = null;

  // Create/Edit Reseller package modal
  let showCreateModal = false;
  let editingPackageId: string | null = null;
  let pkgForm = {
    name: '',
    max_sites: 5,
    max_disk_gb: 10,
    max_email_accounts: 20,
    max_databases: 5,
    price_monthly: 0,
    features: {
      ssl:        true,
      backups:    true,
      git_deploy: false,
      waf:        false,
      staging:    false,
    },
  };
  let pkgLoading = false;
  let pkgError = '';

  // Toast
  let toastMessage = '';
  let toastType: 'success' | 'error' = 'success';
  let loadError = '';
  let toastTimer: ReturnType<typeof setTimeout> | null = null;

  onMount(async () => {
    // Check if currently impersonating
    if (typeof localStorage !== 'undefined') {
      impersonatingEmail = localStorage.getItem('orbit_impersonating');
    }
    await Promise.all([loadTab('clients'), loadStats()]);
  });

  async function loadStats() {
    statsLoading = true;
    loadError = '';
    try {
      stats = await api.reseller.stats();
    } catch {
      // non-fatal
    } finally {
      statsLoading = false;
    }
  }

  async function loadTab(tab: string) {
    if (loadedTabs.has(tab)) return;
    if (tab === 'clients')  clientsLoading = true;
    if (tab === 'packages') packagesLoading = true;
    try {
      if (tab === 'clients') {
        clients = await api.reseller.clients();
      } else if (tab === 'packages') {
        packages = await api.reseller.packages();
      }
      loadedTabs.add(tab);
    } catch (err: unknown) {
      loadError = (err as { message?: string })?.message ?? 'Failed to load data';
    } finally {
      if (tab === 'clients')  clientsLoading = false;
      if (tab === 'packages') packagesLoading = false;
    }
  }

  async function handleTabChange(tab: typeof activeTab) {
    activeTab = tab;
    if (tab !== 'revenue') await loadTab(tab);
  }

  // ── Derived summary ─────────────────────────────────────────────────────────

  $: topClientBySites = clients.reduce<ResellerClient | null>((best, c) =>
    !best || c.sites_count > best.sites_count ? c : best, null);

  $: avgResourcePct = stats && stats.total_clients > 0
    ? Math.min(100, Math.round((stats.total_sites / (stats.total_clients * 5)) * 100))
    : 0;

  // ── Client actions ─────────────────────────────────────────────────────────

  async function suspendClient(client: ResellerClient) {
    try {
      if (client.status === 'active') {
        await api.reseller.suspendClient(client.id);
        clients = clients.map(c => c.id === client.id ? { ...c, status: 'suspended' } : c);
        showToast(`${client.email} suspended`, 'success');
      } else {
        await api.reseller.unsuspendClient(client.id);
        clients = clients.map(c => c.id === client.id ? { ...c, status: 'active' } : c);
        showToast(`${client.email} activated`, 'success');
      }
    } catch {
      showToast('Failed to update client status', 'error');
    }
  }

  async function removeClient(client: ResellerClient) {
    confirmRemoveClientId = null;
    try {
      await api.reseller.removeClient(client.id);
      clients = clients.filter(c => c.id !== client.id);
      if (stats) stats = {
        ...stats,
        total_clients: stats.total_clients - 1,
        active_clients: stats.active_clients - (client.status === 'active' ? 1 : 0),
      };
      showToast(`${client.email} removed`, 'success');
    } catch {
      showToast('Failed to remove client', 'error');
    }
  }

  async function loginAsClient(client: ResellerClient) {
    try {
      const resp = await api.reseller.loginAsClient(client.id);
      localStorage.setItem('orbit_original_token',   localStorage.getItem('orbit_access_token') ?? '');
      localStorage.setItem('orbit_original_user',    localStorage.getItem('orbit_user') ?? '');
      localStorage.setItem('orbit_original_expires', localStorage.getItem('orbit_token_expires') ?? '');
      localStorage.setItem('orbit_impersonating',    client.email || resp.email);
      impersonatingEmail = client.email || resp.email;
      auth.setSession({
        access_token:    resp.access_token,
        expires_in:      resp.expires_in,
        user_id:         resp.user_id,
        email:           resp.email,
        role:            resp.role,
        wizard_complete: resp.wizard_complete,
      });
      await goto('/dashboard', { invalidateAll: true });
    } catch {
      showToast('Failed to login as client', 'error');
    }
  }

  function returnToAdmin() {
    const original = localStorage.getItem('orbit_original_token');
    if (!original) return;
    api.setToken(original);
    localStorage.setItem('orbit_access_token', original);
    localStorage.removeItem('orbit_impersonating');
    localStorage.removeItem('orbit_original_token');
    localStorage.removeItem('orbit_original_user');
    localStorage.removeItem('orbit_original_expires');
    impersonatingEmail = null;
    window.location.reload();
  }

  // ── Package modal ───────────────────────────────────────────────────────────

  function openNewPackageModal(template?: typeof PACKAGE_TEMPLATES[0]) {
    editingPackageId = null;
    if (template) {
      pkgForm = {
        name:               template.name,
        max_sites:          template.max_sites,
        max_disk_gb:        template.max_disk_gb,
        max_email_accounts: template.max_email_accounts,
        max_databases:      template.max_databases,
        price_monthly:      template.price_monthly,
        features: { ...template.features },
      };
    } else {
      pkgForm = {
        name: '',
        max_sites: 5,
        max_disk_gb: 10,
        max_email_accounts: 20,
        max_databases: 5,
        price_monthly: 0,
        features: { ssl: true, backups: true, git_deploy: false, waf: false, staging: false },
      };
    }
    pkgError = '';
    showCreateModal = true;
  }

  function openEditPackageModal(pkg: ResellerPackage) {
    editingPackageId = pkg.id;
    pkgForm = {
      name:               pkg.name,
      max_sites:          pkg.max_sites,
      max_disk_gb:        pkg.max_disk_gb,
      max_email_accounts: pkg.max_email_accounts,
      max_databases:      pkg.max_databases,
      price_monthly:      pkg.price_monthly,
      features: { ssl: true, backups: true, git_deploy: false, waf: false, staging: false, ...pkg.features },
    };
    pkgError = '';
    showCreateModal = true;
  }

  function clonePackage(pkg: ResellerPackage) {
    editingPackageId = null;
    pkgForm = {
      name:               `${pkg.name} (Copy)`,
      max_sites:          pkg.max_sites,
      max_disk_gb:        pkg.max_disk_gb,
      max_email_accounts: pkg.max_email_accounts,
      max_databases:      pkg.max_databases,
      price_monthly:      pkg.price_monthly,
      features: { ssl: true, backups: true, git_deploy: false, waf: false, staging: false, ...pkg.features },
    };
    pkgError = '';
    showCreateModal = true;
  }

  async function savePackage(e: SubmitEvent) {
    e.preventDefault();
    pkgLoading = true;
    pkgError = '';
    try {
      if (editingPackageId) {
        await api.reseller.updatePackage(editingPackageId, pkgForm as Record<string, unknown>);
        packages = packages.map(p => p.id === editingPackageId ? { ...p, ...pkgForm } : p);
        showToast('Package updated', 'success');
      } else {
        const { id } = await api.reseller.createPackage(pkgForm);
        packages = [...packages, { id, ...pkgForm, clients_count: 0 }];
        showToast('Package created', 'success');
      }
      showCreateModal = false;
    } catch (err: unknown) {
      const e = err as { message?: string };
      pkgError = e.message ?? 'Failed to save package';
    } finally {
      pkgLoading = false;
    }
  }

  async function deletePackage(pkg: ResellerPackage) {
    confirmDeletePkgId = null;
    try {
      await api.reseller.deletePackage(pkg.id);
      packages = packages.filter(p => p.id !== pkg.id);
      showToast('Package deleted', 'success');
    } catch {
      showToast('Failed to delete package', 'error');
    }
  }

  // ── Helpers ─────────────────────────────────────────────────────────────────

  function showToast(msg: string, type: 'success' | 'error') {
    if (toastTimer) clearTimeout(toastTimer);
    toastMessage = msg;
    toastType = type;
    toastTimer = setTimeout(() => { toastMessage = ''; }, 4000);
  }

  function avatarInitials(client: ResellerClient): string {
    const name = client.display_name ?? client.email;
    const parts = name.split(/[\s@]/);
    return parts.length >= 2
      ? (parts[0][0] + parts[1][0]).toUpperCase()
      : name.slice(0, 2).toUpperCase();
  }

  function avatarColor(id: string): string {
    const colors = [
      'bg-violet-500/20 text-violet-400',
      'bg-blue-500/20 text-blue-400',
      'bg-emerald-500/20 text-emerald-400',
      'bg-amber-500/20 text-amber-400',
      'bg-rose-500/20 text-rose-400',
      'bg-cyan-500/20 text-cyan-400',
    ];
    let hash = 0;
    for (const c of id) hash = (hash * 31 + c.charCodeAt(0)) & 0xffffff;
    return colors[hash % colors.length];
  }

  function formatDate(dateStr: string): string {
    return new Date(dateStr).toLocaleDateString();
  }

  function utilizationPct(used: number, max: number): number {
    if (max <= 0) return 0;
    return Math.min(100, Math.round((used / max) * 100));
  }

  function utilizationColor(pct: number): string {
    if (pct >= 90) return 'bg-red-500';
    if (pct >= 70) return 'bg-amber-500';
    return 'bg-emerald-500';
  }

  /** SVG donut circumference helper */
  function donutDash(pct: number, r = 14): string {
    const circ = 2 * Math.PI * r;
    const filled = (pct / 100) * circ;
    return `${filled} ${circ - filled}`;
  }

  const featureLabels: Record<string, string> = {
    ssl:        'SSL Certificates',
    backups:    'Automated Backups',
    git_deploy: 'Git Deploy',
    waf:        'WAF Protection',
    staging:    'Staging Sites',
  };

  // Placeholder revenue data for the chart (7 months)
  const revenueMonths = ['Nov', 'Dec', 'Jan', 'Feb', 'Mar', 'Apr', 'May'];
  $: revenueData = stats && stats.total_revenue_monthly > 0
    ? [0.6, 0.7, 0.75, 0.8, 0.85, 0.9, 1.0].map(m => Math.round(stats!.total_revenue_monthly * m))
    : Array(7).fill(0);
  $: revenueMax = Math.max(...revenueData, 1);
</script>

<style>
  @keyframes fadeUp {
    from { opacity: 0; transform: translateY(6px); }
    to   { opacity: 1; transform: none; }
  }
  .fade-up { animation: fadeUp 0.25s ease-out both; }
</style>

<svelte:head>
  <title>{$t('reseller.title')} — JottiCP</title>
</svelte:head>

<!-- ── Impersonation Banner ───────────────────────────────────────────────── -->
{#if impersonatingEmail}
  <div class="sticky top-0 z-40 bg-amber-500/15 border-b border-amber-500/30 px-4 py-2.5
              flex items-center justify-between gap-3 text-sm fade-up">
    <div class="flex items-center gap-2">
      <svg class="w-4 h-4 text-amber-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
        <path stroke-linecap="round" stroke-linejoin="round" d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z" />
      </svg>
      <span class="text-amber-300">{$t('reseller.viewing_as')}<strong class="text-amber-200">{impersonatingEmail}</strong></span>
    </div>
    <button
      on:click={returnToAdmin}
      class="h-7 px-3 rounded-lg bg-amber-500/20 border border-amber-500/30 text-amber-300 text-xs font-medium
             hover:bg-amber-500/30 transition-all duration-150 active:scale-95"
    >{$t('reseller.return_to_admin')}</button>
  </div>
{/if}

<div class="p-4 lg:p-6 space-y-5">
  <!-- ── Load error banner ──────────────────────────────────────────────── -->
  {#if loadError}
    <div class="flex items-center gap-3 px-4 py-3 rounded-xl
                bg-red-500/10 border border-red-500/20 text-sm text-red-400">
      <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round"
          d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z" />
      </svg>
      <span>{loadError}</span>
      <button on:click={() => void loadStats()}
              class="ml-auto text-xs font-semibold underline underline-offset-2 hover:no-underline">
        Retry
      </button>
    </div>
  {/if}

  <!-- Header -->
  <div class="flex items-center justify-between gap-3 flex-wrap">
    <div>
      <h1 class="text-xl font-semibold text-foreground">{$t('reseller.title')}</h1>
      <p class="text-sm text-muted-foreground mt-0.5">{$t('reseller.subtitle')}</p>
    </div>
    <div class="flex gap-2">
      {#if activeTab === 'packages'}
        <button
          on:click={() => openNewPackageModal()}
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                 hover:bg-primary/90 inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
        >
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4" />
          </svg>{$t('reseller.new_package')}</button>
      {/if}
    </div>
  </div>

  <!-- ── Dashboard Summary Bar ───────────────────────────────────────────── -->
  {#if statsLoading}
    <div class="grid grid-cols-2 lg:grid-cols-4 gap-3">
      {#each [1,2,3,4] as _}
        <div class="h-20 bg-muted rounded-xl animate-pulse"></div>
      {/each}
    </div>
  {:else if stats}
    <div class="grid grid-cols-2 lg:grid-cols-4 gap-3">
      <!-- Total clients -->
      <div class="bg-card border border-border rounded-xl p-4
                  transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
        <p class="text-xs text-muted-foreground uppercase tracking-wider mb-1">{$t('reseller.total_clients')}</p>
        <div class="flex items-end justify-between">
          <p class="text-2xl font-bold text-foreground">{stats.total_clients}</p>
          {#if stats.active_clients < stats.total_clients}
            <span class="text-xs text-amber-400 mb-0.5">{stats.total_clients - stats.active_clients} suspended</span>
          {:else}
            <span class="text-xs text-emerald-400 mb-0.5">{$t('reseller.all_active')}</span>
          {/if}
        </div>
        <p class="text-xs text-muted-foreground mt-1">{stats.active_clients} active</p>
      </div>

      <!-- Revenue -->
      <div class="bg-card border border-border rounded-xl p-4
                  transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
        <p class="text-xs text-muted-foreground uppercase tracking-wider mb-1">{$t('reseller.monthly_revenue')}</p>
        <p class="text-2xl font-bold text-emerald-400">
          {stats.total_revenue_monthly > 0 ? `$${Number(stats.total_revenue_monthly).toFixed(0)}` : 'N/A'}
        </p>
        <p class="text-xs text-muted-foreground mt-1">{$t('reseller.recurring_month')}</p>
      </div>

      <!-- Resource utilization -->
      <div class="bg-card border border-border rounded-xl p-4
                  transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
        <p class="text-xs text-muted-foreground uppercase tracking-wider mb-1">{$t('reseller.avg_utilization')}</p>
        <p class="text-2xl font-bold {avgResourcePct >= 80 ? 'text-red-400' : avgResourcePct >= 60 ? 'text-amber-400' : 'text-foreground'}">
          {avgResourcePct}%
        </p>
        <div class="mt-1 h-1 bg-muted rounded-full overflow-hidden">
          <div class="h-full rounded-full {utilizationColor(avgResourcePct)} transition-all"
               style="width: {avgResourcePct}%"></div>
        </div>
      </div>

      <!-- Top client -->
      <div class="bg-card border border-border rounded-xl p-4
                  transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
        <p class="text-xs text-muted-foreground uppercase tracking-wider mb-1">{$t('reseller.top_client')}</p>
        {#if topClientBySites}
          <p class="text-sm font-semibold text-foreground truncate">{topClientBySites.display_name ?? topClientBySites.email.split('@')[0]}</p>
          <p class="text-xs text-muted-foreground mt-1">{topClientBySites.sites_count} sites</p>
        {:else}
          <p class="text-sm text-muted-foreground">—</p>
        {/if}
      </div>
    </div>
  {/if}

  <!-- Tabs -->
  <div class="border-b border-border">
    <nav class="flex gap-0 -mb-px">
      {#each [
        { key: 'clients',  label: 'Clients'  },
        { key: 'packages', label: 'Packages' },
        { key: 'revenue',  label: 'Revenue'  },
      ] as tab}
        <button
          type="button"
          on:click={() => handleTabChange(tab.key as typeof activeTab)}
          class="px-5 py-2.5 text-sm font-medium border-b-2 whitespace-nowrap transition-colors
                 {activeTab === tab.key
                   ? 'border-primary text-primary'
                   : 'border-transparent text-muted-foreground hover:text-foreground hover:border-border'}"
        >
          {tab.label}
          {#if tab.key === 'clients' && clients.length > 0}
            <span class="ml-1.5 px-1.5 py-0.5 rounded-full text-xs bg-muted text-muted-foreground">{clients.length}</span>
          {:else if tab.key === 'packages' && packages.length > 0}
            <span class="ml-1.5 px-1.5 py-0.5 rounded-full text-xs bg-muted text-muted-foreground">{packages.length}</span>
          {/if}
        </button>
      {/each}
    </nav>
  </div>

  <!-- ── Clients Tab ──────────────────────────────────────────────────────── -->
  {#if activeTab === 'clients'}
    {#if clientsLoading}
      <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4">
        {#each [1,2,3,4,5,6] as _}
          <div class="h-52 bg-muted rounded-xl animate-pulse"></div>
        {/each}
      </div>
    {:else if clients.length === 0}
      <div class="bg-card border border-border rounded-xl p-16 text-center">
        <div class="w-12 h-12 rounded-full bg-muted flex items-center justify-center mx-auto mb-4">
          <svg class="w-6 h-6 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
              d="M19 21V5a2 2 0 00-2-2H7a2 2 0 00-2 2v16m14 0h2m-2 0h-5m-9 0H3m2 0h5M9 7h1m-1 4h1m4-4h1m-1 4h1m-5 10v-5a1 1 0 011-1h2a1 1 0 011 1v5m-4 0h4" />
          </svg>
        </div>
        <h3 class="text-sm font-medium text-foreground mb-1">{$t('reseller.no_clients')}</h3>
        <p class="text-xs text-muted-foreground">{$t('reseller.no_clients_desc')}</p>
      </div>
    {:else}
      <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4">
        {#each clients as client (client.id)}
          {@const sitesPct = utilizationPct(client.sites_count, 10)}
          {@const diskPct  = utilizationPct(client.disk_used_gb ?? 0, 20)}
          <div class="bg-card border border-border rounded-xl p-5 flex flex-col gap-4
                      transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
            <!-- Client header -->
            <div class="flex items-start gap-3">
              <div class="w-10 h-10 rounded-full {avatarColor(client.id)} text-sm font-medium flex items-center justify-center shrink-0">
                {avatarInitials(client)}
              </div>
              <div class="min-w-0 flex-1">
                <p class="font-medium text-foreground truncate">
                  {client.display_name ?? client.email}
                </p>
                {#if client.display_name}
                  <p class="text-xs text-muted-foreground truncate">{client.email}</p>
                {/if}
                <div class="flex items-center gap-2 mt-1">
                  {#if client.package_name}
                    <span class="text-xs text-muted-foreground bg-muted px-1.5 py-0.5 rounded">{client.package_name}</span>
                  {/if}
                  {#if client.status === 'suspended'}
                    <span class="inline-flex items-center gap-1 text-xs font-medium text-amber-400">
                      <span class="w-1.5 h-1.5 rounded-full bg-amber-400"></span>{$t('sites.suspended')}</span>
                  {:else}
                    <span class="inline-flex items-center gap-1 text-xs font-medium text-emerald-400">
                      <span class="w-1.5 h-1.5 rounded-full bg-emerald-400"></span>{$t('sites.active')}</span>
                  {/if}
                </div>
              </div>
            </div>

            <!-- Resource donuts row -->
            <div class="flex items-center justify-around py-1">
              <!-- Sites donut -->
              <div class="flex flex-col items-center gap-1">
                <div class="relative w-10 h-10">
                  <svg class="w-10 h-10 -rotate-90" viewBox="0 0 36 36">
                    <circle cx="18" cy="18" r="14" fill="none" stroke="currentColor" class="text-muted" stroke-width="3.5" />
                    <circle cx="18" cy="18" r="14" fill="none"
                            stroke="currentColor"
                            class="{sitesPct >= 90 ? 'text-red-400' : sitesPct >= 70 ? 'text-amber-400' : 'text-emerald-400'}"
                            stroke-width="3.5"
                            stroke-dasharray="{donutDash(sitesPct)}"
                            stroke-linecap="round" />
                  </svg>
                  <span class="absolute inset-0 flex items-center justify-center text-[9px] font-bold text-foreground">
                    {client.sites_count}
                  </span>
                </div>
                <p class="text-[10px] text-muted-foreground">{$t('reseller.sites')}</p>
              </div>

              <!-- Disk donut -->
              <div class="flex flex-col items-center gap-1">
                <div class="relative w-10 h-10">
                  <svg class="w-10 h-10 -rotate-90" viewBox="0 0 36 36">
                    <circle cx="18" cy="18" r="14" fill="none" stroke="currentColor" class="text-muted" stroke-width="3.5" />
                    <circle cx="18" cy="18" r="14" fill="none"
                            stroke="currentColor"
                            class="{diskPct >= 90 ? 'text-red-400' : diskPct >= 70 ? 'text-amber-400' : 'text-blue-400'}"
                            stroke-width="3.5"
                            stroke-dasharray="{donutDash(diskPct)}"
                            stroke-linecap="round" />
                  </svg>
                  <span class="absolute inset-0 flex items-center justify-center text-[9px] font-bold text-foreground">
                    {diskPct}%
                  </span>
                </div>
                <p class="text-[10px] text-muted-foreground">{$t('reseller.disk')}</p>
              </div>

              <!-- Member since -->
              <div class="flex flex-col items-center gap-1 text-center">
                <p class="text-sm font-semibold text-foreground">{new Date(client.created_at).getFullYear()}</p>
                <p class="text-[10px] text-muted-foreground">Since {new Date(client.created_at).toLocaleString('default', { month: 'short' })}</p>
              </div>
            </div>

            <!-- Actions -->
            <div class="flex items-center gap-2 pt-1 border-t border-border">
              <button on:click={() => loginAsClient(client)}
                      class="h-8 px-3 rounded-lg text-xs font-medium text-primary bg-primary/10
                             hover:bg-primary/20 transition-all duration-150 active:scale-95 flex-1">
                Impersonate
              </button>
              <button on:click={() => suspendClient(client)}
                      class="h-8 px-3 rounded-lg text-xs font-medium border border-border
                             hover:bg-muted transition-all duration-150 active:scale-95
                             {client.status === 'active' ? 'text-amber-400' : 'text-emerald-400'}">
                {client.status === 'active' ? 'Suspend' : 'Activate'}
              </button>
              {#if confirmRemoveClientId === client.id}
                <div class="flex items-center gap-1.5">
                  <span class="text-xs text-destructive">{$t('servers.remove')}</span>
                  <button class="text-xs px-2 py-1 rounded bg-destructive text-white hover:bg-destructive/90" on:click={() => removeClient(client)}>Yes</button>
                  <button class="text-xs px-2 py-1 rounded bg-muted" on:click={() => confirmRemoveClientId = null}>No</button>
                </div>
              {:else}
                <button on:click={() => confirmRemoveClientId = client.id}
                        class="h-8 w-8 rounded-lg border border-red-500/20 flex items-center justify-center
                               text-red-400 hover:bg-red-500/10 transition-all duration-150 active:scale-95 shrink-0"
                        title="Remove client">
                  <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                      d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
                  </svg>
                </button>
              {/if}
            </div>
          </div>
        {/each}
      </div>
    {/if}

  <!-- ── Packages Tab ─────────────────────────────────────────────────────── -->
  {:else if activeTab === 'packages'}

    <!-- Package templates section -->
    <div class="space-y-3">
      <div class="flex items-center justify-between">
        <h2 class="text-sm font-semibold text-foreground">{$t('reseller.default_templates')}</h2>
        <span class="text-xs text-muted-foreground">{$t('reseller.click_use_template_to_pre_fill_a_new_pac')}</span>
      </div>
      <div class="grid grid-cols-1 sm:grid-cols-3 gap-3">
        {#each PACKAGE_TEMPLATES as tpl, i}
          {@const tc = TEMPLATE_COLORS[i % TEMPLATE_COLORS.length]}
          <div class="bg-card border border-border rounded-xl overflow-hidden
                      transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
            <!-- Coloured header band -->
            <div class="h-1.5 bg-gradient-to-r {tc.band}"></div>
            <div class="p-4">
              <div class="flex items-start justify-between mb-3">
                <div>
                  <p class="font-semibold text-foreground">{tpl.name}</p>
                  <p class="text-xs text-muted-foreground mt-0.5">{$t('reseller.template')}</p>
                </div>
                <span class="text-base font-bold {tc.icon}">
                  ${tpl.price_monthly}/mo
                </span>
              </div>
              <!-- Limits grid -->
              <div class="grid grid-cols-2 gap-1.5 mb-3">
                {#each [
                  { label: 'Sites',   value: tpl.max_sites === 9999 ? '∞' : String(tpl.max_sites) },
                  { label: 'Disk',    value: `${tpl.max_disk_gb} GB` },
                  { label: 'Emails',  value: String(tpl.max_email_accounts) },
                  { label: 'DBs',     value: String(tpl.max_databases) },
                ] as item}
                  <div class="bg-muted/30 rounded px-2 py-1.5">
                    <p class="text-[10px] text-muted-foreground">{item.label}</p>
                    <p class="text-xs font-semibold text-foreground">{item.value}</p>
                  </div>
                {/each}
              </div>
              <button
                on:click={() => openNewPackageModal(tpl)}
                class="w-full h-8 rounded-lg text-xs font-medium border border-border text-muted-foreground
                       hover:bg-muted hover:text-foreground transition-all duration-150 active:scale-95"
              >
                Use Template
              </button>
            </div>
          </div>
        {/each}
      </div>
    </div>

    <!-- Existing packages -->
    {#if packagesLoading}
      <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4">
        {#each [1,2,3] as _}
          <div class="h-52 bg-muted rounded-xl animate-pulse"></div>
        {/each}
      </div>
    {:else if packages.length === 0}
      <div class="bg-card border border-border rounded-xl p-10 text-center">
        <div class="w-12 h-12 rounded-full bg-muted flex items-center justify-center mx-auto mb-4">
          <svg class="w-6 h-6 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
              d="M20 7l-8-4-8 4m16 0l-8 4m8-4v10l-8 4m0-10L4 7m8 4v10M4 7v10l8 4" />
          </svg>
        </div>
        <h3 class="text-sm font-medium text-foreground mb-1">{$t('reseller.no_custom_packages_yet')}</h3>
        <p class="text-xs text-muted-foreground mb-4">{$t('reseller.use_a_template_above_or_create_from_scra')}</p>
        <button on:click={() => openNewPackageModal()}
                class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                       hover:bg-primary/90 inline-flex items-center gap-2 transition-all duration-150 active:scale-95">
          New Package
        </button>
      </div>
    {:else}
      <div>
        <h2 class="text-sm font-semibold text-foreground mb-3">{$t('reseller.your_packages')}</h2>
        <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4">
          {#each packages as pkg (pkg.id)}
            <div class="bg-card border border-border rounded-xl p-5 flex flex-col gap-4
                        transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
              <!-- Package header -->
              <div class="flex items-start justify-between gap-2">
                <div>
                  <h3 class="font-semibold text-foreground">{pkg.name}</h3>
                  <p class="text-xs text-muted-foreground mt-0.5">
                    {pkg.clients_count} client{pkg.clients_count !== 1 ? 's' : ''}
                  </p>
                </div>
                <div class="text-right shrink-0">
                  <p class="text-lg font-bold text-foreground">
                    {Number(pkg.price_monthly) > 0 ? `$${Number(pkg.price_monthly).toFixed(0)}` : 'Free'}
                  </p>
                  {#if Number(pkg.price_monthly) > 0}
                    <p class="text-xs text-muted-foreground">{$t('reseller.per_month')}</p>
                  {/if}
                </div>
              </div>

              <!-- Limits grid -->
              <div class="grid grid-cols-2 gap-2">
                {#each [
                  { label: 'Sites',     value: pkg.max_sites === 9999 ? '∞' : String(pkg.max_sites)  },
                  { label: 'Disk',      value: `${pkg.max_disk_gb} GB`                               },
                  { label: 'Emails',    value: String(pkg.max_email_accounts)                        },
                  { label: 'Databases', value: String(pkg.max_databases)                             },
                ] as item}
                  <div class="bg-muted/30 rounded-lg px-3 py-2">
                    <p class="text-xs text-muted-foreground">{item.label}</p>
                    <p class="text-sm font-semibold text-foreground">{item.value}</p>
                  </div>
                {/each}
              </div>

              <!-- Features -->
              <div class="flex flex-wrap gap-1">
                {#each Object.entries(pkg.features ?? {}).filter(([, v]) => v) as [key]}
                  <span class="text-xs bg-primary/10 text-primary px-2 py-0.5 rounded-full">
                    {featureLabels[key] ?? key}
                  </span>
                {/each}
              </div>

              <!-- Actions -->
              <div class="flex gap-2 pt-1 border-t border-border">
                <button on:click={() => openEditPackageModal(pkg)}
                        class="flex-1 h-8 rounded-lg border border-border text-xs font-medium text-muted-foreground
                               hover:bg-muted hover:text-foreground transition-all duration-150 active:scale-95">
                  Edit
                </button>
                <button on:click={() => clonePackage(pkg)}
                        class="h-8 px-3 rounded-lg border border-border text-xs font-medium text-muted-foreground
                               hover:bg-muted hover:text-foreground transition-all duration-150 active:scale-95"
                        title="Clone package">
                  Clone
                </button>
                {#if confirmDeletePkgId === pkg.id}
                  <div class="flex items-center gap-1.5">
                    <span class="text-xs text-destructive">{$t('common.delete_confirm')}</span>
                    <button class="text-xs px-2 py-1 rounded bg-destructive text-white hover:bg-destructive/90" on:click={() => deletePackage(pkg)}>Yes</button>
                    <button class="text-xs px-2 py-1 rounded bg-muted" on:click={() => confirmDeletePkgId = null}>No</button>
                  </div>
                {:else}
                  <button on:click={() => confirmDeletePkgId = pkg.id}
                          class="h-8 px-3 rounded-lg border border-red-500/20 text-xs font-medium text-red-400
                                 hover:bg-red-500/10 transition-all duration-150 active:scale-95"
                          disabled={pkg.clients_count > 0}
                          title={pkg.clients_count > 0 ? 'Cannot delete package with active clients' : 'Delete package'}>
                    Delete
                  </button>
                {/if}
              </div>
            </div>
          {/each}
        </div>
      </div>
    {/if}

  <!-- ── Revenue Tab ─────────────────────────────────────────────────────── -->
  {:else if activeTab === 'revenue'}
    <div class="grid grid-cols-1 lg:grid-cols-3 gap-5">
      <!-- Revenue overview chart -->
      <div class="lg:col-span-2 bg-card border border-border rounded-xl p-5">
        <div class="flex items-start justify-between mb-4">
          <div>
            <h3 class="text-sm font-semibold text-foreground">{$t('reseller.revenue_overview')}</h3>
            <p class="text-xs text-muted-foreground mt-0.5">{$t('reseller.last_7_months')}</p>
          </div>
          {#if !stats || stats.total_revenue_monthly === 0}
            <span class="text-xs bg-muted text-muted-foreground px-2 py-1 rounded-lg">{$t('reseller.no_billing_data')}</span>
          {:else}
            <span class="text-xs text-emerald-400 font-medium">${Number(stats.total_revenue_monthly).toFixed(0)}/mo</span>
          {/if}
        </div>

        <!-- Bar chart -->
        <div class="flex items-end gap-2 h-32">
          {#each revenueData as val, i}
            <div class="flex-1 flex flex-col items-center gap-1">
              <div class="w-full rounded-t-sm transition-all duration-300 relative group"
                   style="height: {val > 0 ? Math.max(4, Math.round((val / revenueMax) * 112)) : 4}px;
                          background: {val > 0 ? 'rgb(var(--color-primary) / 0.6)' : 'rgb(var(--muted) / 0.5)'}">
                {#if val > 0}
                  <div class="absolute -top-7 left-1/2 -translate-x-1/2 bg-popover border border-border
                              rounded px-1.5 py-0.5 text-[10px] text-foreground whitespace-nowrap
                              opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none z-10">
                    ${val}
                  </div>
                {/if}
              </div>
              <span class="text-[10px] text-muted-foreground">{revenueMonths[i]}</span>
            </div>
          {/each}
        </div>

        {#if !stats || stats.total_revenue_monthly === 0}
          <div class="mt-4 p-3 bg-muted/30 border border-border rounded-lg flex items-center gap-3">
            <svg class="w-5 h-5 text-muted-foreground shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
              <path stroke-linecap="round" stroke-linejoin="round" d="M13 10V3L4 14h7v7l9-11h-7z" />
            </svg>
            <div>
              <p class="text-xs font-medium text-foreground">{$t('reseller.connect_billing')}</p>
              <p class="text-xs text-muted-foreground mt-0.5">{$t('reseller.connect_billing_desc')}</p>
            </div>
          </div>
        {/if}
      </div>

      <!-- Revenue summary cards -->
      <div class="space-y-3">
        <div class="bg-card border border-border rounded-xl p-4
                    transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
          <p class="text-xs text-muted-foreground uppercase tracking-wider mb-1">{$t('reseller.monthly_recurring_rev')}</p>
          <p class="text-2xl font-bold text-emerald-400">
            {stats && stats.total_revenue_monthly > 0 ? `$${Number(stats.total_revenue_monthly).toFixed(2)}` : 'N/A'}
          </p>
        </div>
        <div class="bg-card border border-border rounded-xl p-4
                    transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
          <p class="text-xs text-muted-foreground uppercase tracking-wider mb-1">{$t('reseller.paying_clients')}</p>
          <p class="text-2xl font-bold text-foreground">
            {stats ? stats.active_clients : '—'}
          </p>
        </div>
        <div class="bg-card border border-border rounded-xl p-4
                    transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
          <p class="text-xs text-muted-foreground uppercase tracking-wider mb-1">{$t('reseller.avg_per_client')}</p>
          <p class="text-2xl font-bold text-foreground">
            {#if stats && stats.active_clients > 0 && stats.total_revenue_monthly > 0}
              ${(Number(stats.total_revenue_monthly) / stats.active_clients).toFixed(2)}
            {:else}
              N/A
            {/if}
          </p>
        </div>
      </div>
    </div>
  {/if}
</div>

<!-- ── Create/Edit Package Modal ─────────────────────────────────────────── -->
{#if showCreateModal}
  <!-- svelte-ignore a11y-click-events-have-key-events -->
  <!-- svelte-ignore a11y-no-static-element-interactions -->
  <div class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
       on:click|self={() => showCreateModal = false}
       role="dialog" aria-modal="true" aria-label="Package editor">
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-lg shadow-2xl max-h-[90vh] overflow-y-auto">
      <div class="flex items-center justify-between mb-5">
        <h2 class="text-base font-semibold text-foreground">
          {editingPackageId ? 'Edit Package' : 'New Package'}
        </h2>
        <button on:click={() => showCreateModal = false}
                class="w-8 h-8 rounded-lg flex items-center justify-center text-muted-foreground hover:bg-muted hover:text-foreground transition-colors">
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      <form on:submit={savePackage} class="space-y-5">
        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5">{$t('reseller.package_name')}</label>
          <input type="text" bind:value={pkgForm.name} required placeholder="e.g. Starter, Pro, Business"
                 class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary" />
        </div>

        <div>
          <p class="text-sm font-medium text-foreground mb-3">{$t('reseller.resource_limits')}</p>
          <div class="grid grid-cols-2 gap-3">
            {#each [
              { key: 'max_sites',          label: 'Max Sites',      unit: 'sites'    },
              { key: 'max_disk_gb',        label: 'Disk Quota',     unit: 'GB'       },
              { key: 'max_email_accounts', label: 'Email Accounts', unit: 'accounts' },
              { key: 'max_databases',      label: 'Databases',      unit: 'dbs'      },
              { key: 'price_monthly',      label: 'Price / Month',  unit: 'USD'      },
            ] as field}
              <div class="bg-muted/30 rounded-lg p-3 border border-border">
                <label class="block text-xs text-muted-foreground mb-1.5">{field.label}</label>
                <div class="flex items-center gap-2">
                  <input type="number" min="0" step={field.key === 'price_monthly' ? '0.01' : '1'}
                         bind:value={pkgForm[field.key as keyof typeof pkgForm]}
                         class="w-20 h-7 px-2 rounded border border-border bg-background text-sm text-foreground font-mono focus:outline-none focus:ring-1 focus:ring-primary/50" />
                  <span class="text-xs text-muted-foreground">{field.unit}</span>
                </div>
              </div>
            {/each}
          </div>
        </div>

        <!-- Feature toggles -->
        <div>
          <p class="text-sm font-medium text-foreground mb-2">{$t('reseller.features')}</p>
          <div class="bg-muted/20 rounded-xl border border-border divide-y divide-border">
            {#each Object.entries(featureLabels) as [key, label]}
              <label class="flex items-center justify-between px-4 py-3 cursor-pointer hover:bg-muted/30 transition-colors">
                <span class="text-sm text-foreground">{label}</span>
                <button type="button"
                        on:click={() => pkgForm.features[key as keyof typeof pkgForm.features] = !pkgForm.features[key as keyof typeof pkgForm.features]}
                        class="relative inline-flex h-5 w-9 items-center rounded-full transition-colors shrink-0
                               {pkgForm.features[key as keyof typeof pkgForm.features] ? 'bg-primary' : 'bg-muted-foreground/30'}">
                  <span class="inline-block h-3.5 w-3.5 transform rounded-full bg-background shadow transition-transform
                               {pkgForm.features[key as keyof typeof pkgForm.features] ? 'translate-x-4' : 'translate-x-0.5'}"></span>
                </button>
              </label>
            {/each}
          </div>
        </div>

        {#if pkgError}
          <div role="alert" class="text-sm text-red-400 bg-red-500/10 border border-red-500/20 rounded-lg px-3 py-2.5">
            {pkgError}
          </div>
        {/if}

        <div class="flex gap-2 pt-1">
          <button type="button" on:click={() => showCreateModal = false}
                  class="flex-1 h-9 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center justify-center gap-2 transition-colors">
            Cancel
          </button>
          <button type="submit" disabled={pkgLoading}
                  class="flex-1 h-9 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center justify-center gap-2 transition-all duration-150 active:scale-95 disabled:opacity-50">
            {pkgLoading ? 'Saving…' : (editingPackageId ? 'Update Package' : 'Create Package')}
          </button>
        </div>
      </form>
    </div>
  </div>
{/if}

<!-- ── Toast ─────────────────────────────────────────────────────────────── -->
{#if toastMessage}
  <div class="fixed bottom-4 right-4 z-50 bg-card border border-border rounded-xl px-4 py-3 text-sm shadow-lg
              flex items-center gap-2 fade-up"
       role="status" aria-live="polite">
    {#if toastType === 'success'}
      <svg class="w-4 h-4 text-emerald-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7" />
      </svg>
    {:else}
      <svg class="w-4 h-4 text-red-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
      </svg>
    {/if}
    <span class="text-foreground">{toastMessage}</span>
  </div>
{/if}
