<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { api } from '$api/client';
  import type { Site, CreateSitePayload } from '$api/client';
  import { debounce } from '$lib/utils';

  // ── State ──────────────────────────────────────────────────────────────────
  let sites:        Site[]   = [];
  let isLoading              = true;
  let searchQuery            = '';   // committed (debounced) value used for filtering
  let searchInput            = '';   // raw input value — bound to the input element
  let filterStatus           = '';
  let sortBy                 = 'domain';
  let filteredSites: Site[]  = [];

  // Debounced handler — updates searchQuery 250 ms after the user stops typing
  const onSearchInput = debounce((e: Event) => {
    searchQuery = (e.target as HTMLInputElement).value;
  }, 250);

  // Delete modal
  let showDeleteModal        = false;
  let siteToDelete: Site | null = null;
  let confirmDeleteDomain    = '';

  // Create modal
  let showCreateModal        = false;
  let createLoading          = false;
  let createError            = '';
  let quickSetup             = true;
  let diskQuota              = '';
  let phpPreset              = 'none';
  let newSite: CreateSitePayload = {
    domain:     '',
    php_version: '8.3',
    web_server: 'nginx',
    plan:       'static',
  };

  // View mode: 'list' | 'grid'
  let viewMode: 'list' | 'grid' = 'list';

  // Bulk actions
  let selectedIds         = new Set<string>();
  let bulkDeleteConfirm   = '';
  let showBulkDeleteModal = false;
  let bulkLoading         = false;

  // Kebab menu
  let openMenuId: string | null = null;

  // Webserver switch
  let switchingWebserverId = '';

  // Toast
  let toastMessage = '';
  let toastType: 'success' | 'error' = 'success';
  let loadError = '';

  // ── Computed stats ─────────────────────────────────────────────────────────
  $: totalSites    = sites.length;
  $: activeSites   = sites.filter(s => s.status === 'active').length;
  $: suspendedSites = sites.filter(s => s.status === 'suspended').length;
  $: allSelected   = filteredSites.length > 0 && filteredSites.every(s => selectedIds.has(s.id));
  $: someSelected  = selectedIds.size > 0;

  // ── Lifecycle ──────────────────────────────────────────────────────────────
  onMount(async () => {
    if (typeof location !== "undefined" && new URLSearchParams(location.search).get("new") === "1") showCreateModal = true;
    await loadSites();
    document.addEventListener('click', handleGlobalClick);
    return () => document.removeEventListener('click', handleGlobalClick);
  });

  function handleGlobalClick(e: MouseEvent) {
    const target = e.target as HTMLElement;
    if (!target.closest('[data-kebab]')) openMenuId = null;
  }

  // ── Data loading ───────────────────────────────────────────────────────────
  async function loadSites() {
    isLoading = true;
    loadError = '';
    try {
      sites = await api.sites.list();
      applyFilters();
    } catch (err: unknown) {
      loadError = (err as { message?: string })?.message ?? 'Failed to load websites';
    } finally {
      isLoading = false;
    }
  }

  function applyFilters() {
    let result = [...sites];
    if (searchQuery) {
      const q = searchQuery.toLowerCase();
      result = result.filter(s => s.domain.toLowerCase().includes(q));
    }
    if (filterStatus) {
      result = result.filter(s => s.status === filterStatus);
    }
    // Sort
    result.sort((a, b) => {
      if (sortBy === 'domain')   return a.domain.localeCompare(b.domain);
      if (sortBy === 'created')  return new Date(b.created_at).getTime() - new Date(a.created_at).getTime();
      if (sortBy === 'php')      return a.php_version.localeCompare(b.php_version);
      return 0;
    });
    filteredSites = result;
  }

  $: { void searchQuery; void filterStatus; void sortBy; if (!isLoading) applyFilters(); }

  // ── Domain validation ──────────────────────────────────────────────────────
  function validateDomain(d: string): string {
    // Normalize first
    d = d.trim().toLowerCase().replace(/^https?:\/\//, '').replace(/\/$/, '');
    if (!d) return 'Domain is required';
    if (d.length > 253) return 'Domain too long (max 253 characters)';
    if (!/^[a-z0-9]([a-z0-9-]*[a-z0-9])?(\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)+$/.test(d))
      return 'Invalid domain format (e.g. example.com)';
    // Validate each label (max 63 chars, no leading/trailing hyphen)
    for (const label of d.split('.')) {
      if (label.length > 63) return 'Domain label too long (max 63 characters per segment)';
    }
    return '';
  }

  // Normalize the domain value before submitting
  function normalizeDomain(d: string): string {
    return d.trim().toLowerCase().replace(/^https?:\/\//, '').replace(/\/$/, '');
  }

  // ── Create ─────────────────────────────────────────────────────────────────
  function openCreateModal() {
    newSite = { domain: '', php_version: '8.3', web_server: 'nginx', plan: 'static' };
    createError = '';
    quickSetup = true;
    diskQuota = '';
    phpPreset = 'none';
    showCreateModal = true;
  }

  async function handleCreate() {
    // Normalize domain before validation and submission
    newSite.domain = normalizeDomain(newSite.domain);
    createError = validateDomain(newSite.domain);
    if (createError) return;
    createLoading = true;
    createError   = '';
    try {
      const created = await api.sites.create(newSite);
      sites = [created, ...sites];
      applyFilters();
      showCreateModal = false;
      showToast(`${created.domain} created`, 'success');
    } catch (err: unknown) {
      const e = err as { message?: string };
      createError = e.message ?? 'Failed to create website';
    } finally {
      createLoading = false;
    }
  }

  // ── Suspend / Resume ───────────────────────────────────────────────────────
  async function handleSuspend(site: Site) {
    openMenuId = null;
    updateSiteInList(site.id, { status: 'suspended', suspended_at: new Date().toISOString() });
    try {
      await api.sites.suspend(site.id);
      showToast(`${site.domain} suspended`, 'success');
    } catch {
      updateSiteInList(site.id, { status: 'active', suspended_at: null });
      showToast('Failed to suspend site', 'error');
    }
  }

  async function handleUnsuspend(site: Site) {
    openMenuId = null;
    updateSiteInList(site.id, { status: 'active', suspended_at: null });
    try {
      await api.sites.unsuspend(site.id);
      showToast(`${site.domain} resumed`, 'success');
    } catch {
      updateSiteInList(site.id, { status: 'suspended', suspended_at: site.suspended_at });
      showToast('Failed to resume site', 'error');
    }
  }

  // ── Delete ─────────────────────────────────────────────────────────────────
  function openDeleteModal(site: Site) {
    openMenuId = null;
    siteToDelete       = site;
    confirmDeleteDomain = '';
    showDeleteModal    = true;
  }

  async function handleDelete() {
    if (!siteToDelete || confirmDeleteDomain !== siteToDelete.domain) return;
    const { id, domain } = siteToDelete;
    showDeleteModal = false;
    siteToDelete    = null;
    sites          = sites.filter(s => s.id !== id);
    selectedIds.delete(id);
    selectedIds = new Set(selectedIds);
    applyFilters();
    try {
      await api.sites.delete(id);
      showToast(`${domain} deleted`, 'success');
    } catch {
      await loadSites();
      showToast('Failed to delete site', 'error');
    }
  }

  // ── Bulk actions ───────────────────────────────────────────────────────────
  function toggleSelectAll() {
    if (allSelected) {
      filteredSites.forEach(s => selectedIds.delete(s.id));
    } else {
      filteredSites.forEach(s => selectedIds.add(s.id));
    }
    selectedIds = new Set(selectedIds);
  }

  function toggleSelect(id: string) {
    if (selectedIds.has(id)) selectedIds.delete(id);
    else selectedIds.add(id);
    selectedIds = new Set(selectedIds);
  }

  async function bulkSuspend() {
    bulkLoading = true;
    const ids = [...selectedIds];
    const failed: string[] = [];
    for (const id of ids) {
      const site = sites.find(s => s.id === id);
      if (site?.status === 'active') {
        updateSiteInList(id, { status: 'suspended', suspended_at: new Date().toISOString() });
        try {
          await api.sites.suspend(id);
        } catch (err) {
          console.error(`suspend ${site.domain} failed:`, err);
          failed.push(site.domain);
          updateSiteInList(id, { status: 'active', suspended_at: null });
        }
      }
    }
    selectedIds = new Set();
    bulkLoading = false;
    if (failed.length === 0) {
      showToast(`${ids.length} site(s) suspended`, 'success');
    } else {
      showToast(`Suspended ${ids.length - failed.length}/${ids.length}. Failed: ${failed.join(', ')}`, 'error');
    }
  }

  async function bulkResume() {
    bulkLoading = true;
    const ids = [...selectedIds];
    const failed: string[] = [];
    for (const id of ids) {
      const site = sites.find(s => s.id === id);
      if (site?.status === 'suspended') {
        updateSiteInList(id, { status: 'active', suspended_at: null });
        try {
          await api.sites.unsuspend(id);
        } catch (err) {
          console.error(`resume ${site.domain} failed:`, err);
          failed.push(site.domain);
          updateSiteInList(id, { status: 'suspended', suspended_at: new Date().toISOString() });
        }
      }
    }
    selectedIds = new Set();
    bulkLoading = false;
    if (failed.length === 0) {
      showToast(`${ids.length} site(s) resumed`, 'success');
    } else {
      showToast(`Resumed ${ids.length - failed.length}/${ids.length}. Failed: ${failed.join(', ')}`, 'error');
    }
  }

  function openBulkDeleteModal() {
    bulkDeleteConfirm = '';
    showBulkDeleteModal = true;
  }

  async function handleBulkDelete() {
    if (bulkDeleteConfirm !== 'DELETE') return;
    bulkLoading = true;
    showBulkDeleteModal = false;
    const ids = [...selectedIds];
    const snapshot = sites;
    const failed: string[] = [];
    for (const id of ids) {
      const site = sites.find(s => s.id === id);
      sites = sites.filter(s => s.id !== id);
      try {
        await api.sites.delete(id);
      } catch (err) {
        console.error(`delete ${site?.domain ?? id} failed:`, err);
        if (site) failed.push(site.domain);
      }
    }
    if (failed.length > 0) {
      // Restore failed sites so the user can retry
      sites = snapshot.filter(s => !ids.includes(s.id) || failed.includes(s.domain));
    }
    selectedIds = new Set();
    applyFilters();
    bulkLoading = false;
    if (failed.length === 0) {
      showToast(`${ids.length} site(s) deleted`, 'success');
    } else {
      showToast(`Deleted ${ids.length - failed.length}/${ids.length}. Failed: ${failed.join(', ')}`, 'error');
    }
  }

  // ── Helpers ────────────────────────────────────────────────────────────────
  function updateSiteInList(id: string, updates: Partial<Site>) {
    sites = sites.map(s => s.id === id ? { ...s, ...updates } : s);
    applyFilters();
  }

  function showToast(message: string, type: 'success' | 'error') {
    toastMessage = message;
    toastType    = type;
    setTimeout(() => { toastMessage = ''; }, 4000);
  }

  function formatDate(dateStr: string): string {
    return new Date(dateStr).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' });
  }

  async function switchWebserver(site: Site, ws: string) {
    openMenuId = null;
    if (site.web_server === ws) return;
    switchingWebserverId = site.id;
    const token = typeof localStorage !== 'undefined' ? localStorage.getItem('orbit_access_token') : null;
    try {
      const res = await fetch(`/api/v1/sites/${site.id}/webserver`, {
        method: 'PUT',
        headers: {
          'Content-Type': 'application/json',
          ...(token ? { 'Authorization': `Bearer ${token}` } : {}),
        },
        body: JSON.stringify({ web_server: ws }),
      });
      if (!res.ok) {
        const err = await res.json().catch(() => ({}));
        throw new Error((err as { message?: string }).message ?? `HTTP ${res.status}`);
      }
      updateSiteInList(site.id, { web_server: ws });
      showToast(`${site.domain} → ${ws.toUpperCase()} switched`, 'success');
    } catch (e: unknown) {
      showToast((e as Error).message ?? 'Switch failed', 'error');
    } finally {
      switchingWebserverId = '';
    }
  }
</script>

<style>
  @keyframes fadeUp {
    from { opacity: 0; transform: translateY(6px); }
    to   { opacity: 1; transform: none; }
  }
  .fade-up { animation: fadeUp 0.25s ease-out both; }
</style>

<svelte:head>
  <title>Websites — OrbitCP</title>
</svelte:head>

<div class="p-4 lg:p-6 space-y-6">
  <!-- ── Load error banner ──────────────────────────────────────────────── -->
  {#if loadError}
    <div class="flex items-center gap-3 px-4 py-3 rounded-xl
                bg-red-500/10 border border-red-500/20 text-sm text-red-400">
      <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round"
          d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z" />
      </svg>
      <span>{loadError}</span>
      <button on:click={() => void loadSites()}
              class="ml-auto text-xs font-semibold underline underline-offset-2 hover:no-underline">
        Retry
      </button>
    </div>
  {/if}



  <!-- ── Page header ──────────────────────────────────────────────────────── -->
  <div class="flex items-start justify-between gap-4">
    <div>
      <h1 class="text-2xl font-bold text-foreground">Websites</h1>
      <p class="text-muted-foreground text-sm mt-1">Manage all your hosted sites</p>
    </div>
    <div class="flex items-center gap-3 shrink-0">
      <!-- Search -->
      <div class="relative hidden sm:block">
        <svg class="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground pointer-events-none"
             fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
            d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
        </svg>
        <input
          type="search"
          value={searchInput}
          on:input={(e) => { searchInput = (e.target as HTMLInputElement).value; onSearchInput(e); }}
          placeholder="Search domains…"
          class="h-9 pl-9 pr-3 rounded-lg border border-border bg-background text-sm text-foreground
                 placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50
                 focus:border-primary w-56"
        />
      </div>
      <!-- Grid / List view toggle -->
      <div class="hidden sm:flex items-center border border-border rounded-lg overflow-hidden">
        <button
          on:click={() => viewMode = 'list'}
          class="h-9 w-9 flex items-center justify-center transition-colors
                 {viewMode === 'list' ? 'bg-muted text-foreground' : 'text-muted-foreground hover:bg-muted/50'}"
          title="List view"
        >
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M4 6h16M4 12h16M4 18h16" />
          </svg>
        </button>
        <button
          on:click={() => viewMode = 'grid'}
          class="h-9 w-9 flex items-center justify-center transition-colors
                 {viewMode === 'grid' ? 'bg-muted text-foreground' : 'text-muted-foreground hover:bg-muted/50'}"
          title="Grid view"
        >
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <rect x="3" y="3" width="7" height="7" rx="1" /><rect x="14" y="3" width="7" height="7" rx="1" />
            <rect x="3" y="14" width="7" height="7" rx="1" /><rect x="14" y="14" width="7" height="7" rx="1" />
          </svg>
        </button>
      </div>

      <!-- Create button -->
      <button
        on:click={openCreateModal}
        class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium
               hover:bg-primary/90 inline-flex items-center gap-2"
      >
        <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4" />
        </svg>
        Create Website
      </button>
    </div>
  </div>

  <!-- ── Stats row ────────────────────────────────────────────────────────── -->
  <!-- Quick stats (clickable filters) -->
  {#if !isLoading}
    <div class="flex items-center gap-3 text-sm text-muted-foreground flex-wrap">
      <button
        on:click={() => filterStatus = ''}
        class="hover:text-foreground transition-colors {filterStatus === '' ? 'text-foreground font-semibold' : ''}"
      >
        {totalSites} total
      </button>
      <span class="opacity-30">·</span>
      <button
        on:click={() => filterStatus = 'active'}
        class="hover:text-green-400 transition-colors {filterStatus === 'active' ? 'text-green-400 font-semibold' : ''}"
      >
        {activeSites} active
      </button>
      <span class="opacity-30">·</span>
      <button
        on:click={() => filterStatus = 'suspended'}
        class="hover:text-amber-400 transition-colors {filterStatus === 'suspended' ? 'text-amber-400 font-semibold' : ''}"
      >
        {suspendedSites} suspended
      </button>
    </div>
  {/if}

  <!-- ── Filter / sort bar ────────────────────────────────────────────────── -->
  <div class="flex flex-col sm:flex-row gap-3 items-start sm:items-center justify-between">
    <!-- Status pills -->
    <div class="flex items-center gap-1.5 flex-wrap">
      {#each [
        { value: '',            label: 'All' },
        { value: 'active',      label: 'Active' },
        { value: 'suspended',   label: 'Suspended' },
        { value: 'provisioning', label: 'Installing' },
      ] as pill}
        <button
          on:click={() => filterStatus = pill.value}
          class="h-7 px-3 rounded-full text-xs font-medium transition-colors
                 {filterStatus === pill.value
                   ? 'bg-primary text-primary-foreground'
                   : 'bg-muted text-muted-foreground hover:bg-muted/80 hover:text-foreground'}"
        >
          {pill.label}
        </button>
      {/each}
    </div>

    <!-- Sort + mobile search -->
    <div class="flex items-center gap-2">
      <!-- Mobile search -->
      <div class="relative sm:hidden">
        <svg class="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground pointer-events-none"
             fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
            d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
        </svg>
        <input type="search"
          value={searchInput}
          on:input={(e) => { searchInput = (e.target as HTMLInputElement).value; onSearchInput(e); }}
          placeholder="Search…"
          class="h-9 pl-9 pr-3 rounded-lg border border-border bg-background text-sm text-foreground
                 placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50
                 focus:border-primary w-40" />
      </div>

      <span class="text-xs text-muted-foreground whitespace-nowrap hidden sm:block">Sort by</span>
      <select bind:value={sortBy}
        class="h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground
               focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary">
        <option value="domain">Domain</option>
        <option value="created">Created</option>
        <option value="php">PHP version</option>
      </select>
    </div>
  </div>

  <!-- ── Table / skeleton / empty ─────────────────────────────────────────── -->
  {#if isLoading}
    <!-- Skeleton -->
    <div class="rounded-xl border border-border overflow-hidden">
      <div class="bg-muted/50 px-4 py-3 flex gap-4">
        {#each [40, 20, 14, 16, 16] as w}
          <div class="h-3 rounded animate-pulse bg-muted" style="width: {w}%"></div>
        {/each}
      </div>
      {#each [1,2,3,4,5] as _}
        <div class="border-t border-border px-4 py-4 flex gap-4 items-center">
          <div class="h-4 w-5/12 rounded animate-pulse bg-muted"></div>
          <div class="h-5 w-1/12 rounded-full animate-pulse bg-muted"></div>
          <div class="h-4 w-1/12 rounded animate-pulse bg-muted"></div>
          <div class="h-5 w-1/12 rounded-full animate-pulse bg-muted"></div>
          <div class="h-4 w-2/12 rounded animate-pulse bg-muted ml-auto"></div>
        </div>
      {/each}
    </div>

  {:else if filteredSites.length === 0}
    <!-- Empty state -->
    <div class="py-20 text-center">
      <svg class="w-12 h-12 mx-auto text-muted-foreground mb-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
          d="M21 12a9 9 0 01-9 9m9-9a9 9 0 00-9-9m9 9H3m9 9a9 9 0 01-9-9m9 9c1.657 0 3-4.03
             3-9s-1.343-9-3-9m0 18c-1.657 0-3-4.03-3-9s1.343-9 3-9m-9 9a9 9 0 019-9" />
      </svg>
      {#if sites.length === 0}
        <h3 class="text-foreground font-medium">No websites yet</h3>
        <p class="text-muted-foreground text-sm mt-1">Create your first site to get started</p>
        <button on:click={openCreateModal}
          class="mt-4 h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                 hover:bg-primary/90 inline-flex items-center gap-2 mx-auto">
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4" />
          </svg>
          Create your first site
        </button>
      {:else}
        <h3 class="text-foreground font-medium">No results</h3>
        <p class="text-muted-foreground text-sm mt-1">No websites match your filters</p>
        <button on:click={() => { searchQuery = ''; searchInput = ''; filterStatus = ''; }}
          class="mt-4 h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground
                 hover:bg-muted hover:text-foreground inline-flex items-center gap-2 mx-auto">
          Clear filters
        </button>
      {/if}
    </div>

  {:else}

    <!-- ── Grid view ─────────────────────────────────────────────────────────── -->
    {#if viewMode === 'grid'}
      <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-4">
        {#each filteredSites as site, i (site.id)}
          <div
            class="bg-card border border-border rounded-xl p-5 transition-all duration-200
                   hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20 cursor-pointer fade-up"
            style="animation-delay: {i * 30}ms"
            on:click={() => goto(`/websites/${site.id}`)}
          >
            <div class="flex items-start justify-between mb-3">
              <div class="w-10 h-10 rounded-lg bg-muted/50 flex items-center justify-center">
                <svg class="w-5 h-5 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
                    d="M21 12a9 9 0 01-9 9m9-9a9 9 0 00-9-9m9 9H3m9 9a9 9 0 01-9-9m9 9c1.657 0
                       3-4.03 3-9s-1.343-9-3-9m0 18c-1.657 0-3-4.03-3-9s1.343-9 3-9m-9 9a9 9 0 019-9" />
                </svg>
              </div>
              {#if site.status === 'active'}
                <span class="bg-green-500/10 text-green-400 border border-green-500/20 px-2 py-0.5 rounded-full text-xs font-medium inline-flex items-center gap-1">
                  <span class="w-1.5 h-1.5 rounded-full bg-green-400"></span>Active
                </span>
              {:else if site.status === 'suspended'}
                <span class="bg-amber-500/10 text-amber-400 border border-amber-500/20 px-2 py-0.5 rounded-full text-xs font-medium inline-flex items-center gap-1">
                  <span class="w-1.5 h-1.5 rounded-full bg-amber-400"></span>Suspended
                </span>
              {:else if site.status === 'provisioning'}
                <span class="bg-blue-500/10 text-blue-400 border border-blue-500/20 px-2 py-0.5 rounded-full text-xs font-medium inline-flex items-center gap-1">
                  <span class="w-1.5 h-1.5 rounded-full bg-blue-400 animate-pulse"></span>Installing
                </span>
              {:else}
                <span class="bg-muted text-muted-foreground px-2 py-0.5 rounded-full text-xs font-medium">{site.status}</span>
              {/if}
            </div>
            <h3 class="font-semibold text-foreground truncate">{site.domain}</h3>
            <p class="text-xs text-muted-foreground mt-0.5">{site.web_server.toUpperCase()} · PHP {site.php_version}</p>
            <div class="flex items-center gap-2 mt-3 pt-3 border-t border-border">
              {#if site.ssl_status === 'active'}
                <span class="text-[10px] text-green-400 bg-green-500/10 border border-green-500/20 px-1.5 py-0.5 rounded-full">
                  🔒 HTTPS
                </span>
              {/if}
              <span class="text-[10px] text-muted-foreground">{site.server_label ?? 'Local'}</span>
              <div class="ml-auto flex gap-1" on:click|stopPropagation>
                <div class="relative" data-kebab>
                  <button
                    on:click={() => openMenuId = openMenuId === site.id ? null : site.id}
                    class="h-7 w-7 rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground
                           inline-flex items-center justify-center transition-colors"
                  >
                    <svg class="w-3.5 h-3.5" fill="currentColor" viewBox="0 0 24 24">
                      <circle cx="12" cy="5" r="1.5" /><circle cx="12" cy="12" r="1.5" /><circle cx="12" cy="19" r="1.5" />
                    </svg>
                  </button>
                  {#if openMenuId === site.id}
                    <div class="absolute right-0 bottom-full mb-1 w-52 bg-card border border-border
                                rounded-xl shadow-xl z-20 py-1 overflow-hidden">
                      <a href="https://{site.domain}" target="_blank" rel="noopener noreferrer"
                         class="flex items-center gap-2 px-3 py-2 text-sm text-foreground hover:bg-muted">
                        <svg class="w-4 h-4 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                            d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14" />
                        </svg>
                        Visit Site
                      </a>
                      <div class="border-t border-border my-1"></div>
                      <p class="px-3 py-1 text-[10px] text-muted-foreground uppercase tracking-wider font-medium">Switch Web Server</p>
                      {#each [{ v: 'nginx', l: 'Nginx' }, { v: 'apache', l: 'Apache' }, { v: 'ols', l: 'LiteSpeed' }] as ws}
                        <button
                          on:click={() => switchWebserver(site, ws.v)}
                          disabled={switchingWebserverId === site.id}
                          class="w-full flex items-center gap-2 px-3 py-2 text-sm hover:bg-muted text-left
                                 {site.web_server === ws.v ? 'text-primary font-medium' : 'text-muted-foreground'}"
                        >
                          {#if site.web_server === ws.v}
                            <svg class="w-3.5 h-3.5 shrink-0 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>
                            </svg>
                          {:else}
                            <span class="w-3.5 h-3.5 shrink-0"></span>
                          {/if}
                          {ws.l}
                        </button>
                      {/each}
                      <div class="border-t border-border my-1"></div>
                      {#if site.status === 'active'}
                        <button on:click={() => handleSuspend(site)}
                          class="w-full flex items-center gap-2 px-3 py-2 text-sm text-amber-400 hover:bg-muted text-left">
                          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 9v6m4-6v6m7-3a9 9 0 11-18 0 9 9 0 0118 0z" />
                          </svg>
                          Suspend
                        </button>
                      {:else if site.status === 'suspended'}
                        <button on:click={() => handleUnsuspend(site)}
                          class="w-full flex items-center gap-2 px-3 py-2 text-sm text-green-400 hover:bg-muted text-left">
                          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M14.752 11.168l-3.197-2.132A1 1 0 0010 9.87v4.263a1 1 0 001.555.832l3.197-2.132a1 1 0 000-1.664z" /><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
                          </svg>
                          Resume
                        </button>
                      {/if}
                      <div class="border-t border-border my-1"></div>
                      <button on:click={() => openDeleteModal(site)}
                        class="w-full flex items-center gap-2 px-3 py-2 text-sm text-red-400 hover:bg-muted text-left">
                        <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                            d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5
                               7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
                        </svg>
                        Delete
                      </button>
                    </div>
                  {/if}
                </div>
              </div>
            </div>
          </div>
        {/each}
      </div>

    {:else}
      <!-- List view -->

      <!-- Desktop table -->
    <div class="hidden md:block rounded-xl border border-border overflow-x-auto">
      <table class="w-full">
        <thead class="bg-muted/50">
          <tr>
            <th class="px-4 py-3 w-10"></th>
            <th class="px-4 py-3 text-xs text-muted-foreground uppercase tracking-wider font-medium text-left">Domain</th>
            <th class="px-4 py-3 text-xs text-muted-foreground uppercase tracking-wider font-medium text-left">Status</th>
            <th class="px-4 py-3 text-xs text-muted-foreground uppercase tracking-wider font-medium text-left">PHP</th>
            <th class="px-4 py-3 text-xs text-muted-foreground uppercase tracking-wider font-medium text-left">Web Server</th>
            <th class="px-4 py-3 text-xs text-muted-foreground uppercase tracking-wider font-medium text-left">Server</th>
            <th class="px-4 py-3 text-xs text-muted-foreground uppercase tracking-wider font-medium text-left">Created</th>
            <th class="px-4 py-3 text-xs text-muted-foreground uppercase tracking-wider font-medium text-right">Actions</th>
          </tr>
        </thead>
        <tbody>
          {#each filteredSites as site (site.id)}
            <tr
              class="border-b border-border last:border-0 transition-colors duration-150 cursor-pointer
                     {selectedIds.has(site.id) ? 'bg-primary/5' : 'hover:bg-muted/30'}"
              on:click={() => goto(`/websites/${site.id}`)}
            >
              <!-- Checkbox -->
              <td class="px-4 py-3" on:click|stopPropagation>
                <input type="checkbox" checked={selectedIds.has(site.id)}
                  on:change={() => toggleSelect(site.id)}
                  class="rounded border-border accent-primary cursor-pointer" />
              </td>

              <!-- Domain -->
              <td class="px-4 py-3 text-sm text-foreground">
                <div class="flex items-center gap-2">
                  <svg class="w-4 h-4 text-muted-foreground shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
                      d="M21 12a9 9 0 01-9 9m9-9a9 9 0 00-9-9m9 9H3m9 9a9 9 0 01-9-9m9 9c1.657
                         0 3-4.03 3-9s-1.343-9-3-9m0 18c-1.657 0-3-4.03-3-9s1.343-9 3-9m-9 9a9 9 0 019-9" />
                  </svg>
                  <span class="font-medium">{site.domain}</span>
                  <a
                    href="https://{site.domain}"
                    target="_blank"
                    rel="noopener noreferrer"
                    on:click|stopPropagation
                    class="text-muted-foreground hover:text-foreground"
                    title="Visit site"
                  >
                    <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                        d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14" />
                    </svg>
                  </a>
                </div>
              </td>

              <!-- Status badge -->
              <td class="px-4 py-3 text-sm">
                {#if site.status === 'active'}
                  <span class="bg-green-500/10 text-green-400 border border-green-500/20 px-2 py-0.5 rounded-full text-xs font-medium inline-flex items-center gap-1">
                    <span class="w-1.5 h-1.5 rounded-full bg-green-400"></span>Active
                  </span>
                {:else if site.status === 'suspended'}
                  <span class="bg-amber-500/10 text-amber-400 border border-amber-500/20 px-2 py-0.5 rounded-full text-xs font-medium inline-flex items-center gap-1">
                    <span class="w-1.5 h-1.5 rounded-full bg-amber-400"></span>Suspended
                  </span>
                {:else if site.status === 'provisioning'}
                  <span class="bg-blue-500/10 text-blue-400 border border-blue-500/20 px-2 py-0.5 rounded-full text-xs font-medium inline-flex items-center gap-1">
                    <span class="w-1.5 h-1.5 rounded-full bg-blue-400 animate-pulse"></span>Installing
                  </span>
                {:else if site.status === 'error'}
                  <span class="bg-red-500/10 text-red-400 border border-red-500/20 px-2 py-0.5 rounded-full text-xs font-medium inline-flex items-center gap-1">
                    <span class="w-1.5 h-1.5 rounded-full bg-red-400"></span>Error
                  </span>
                {:else}
                  <span class="bg-muted text-muted-foreground px-2 py-0.5 rounded-full text-xs font-medium">{site.status}</span>
                {/if}
              </td>

              <!-- PHP -->
              <td class="px-4 py-3 text-sm">
                <span class="bg-muted text-muted-foreground px-2 py-0.5 rounded-full text-xs font-medium">
                  PHP {site.php_version}
                </span>
              </td>

              <!-- Web server -->
              <td class="px-4 py-3 text-sm">
                <span class="bg-muted text-muted-foreground px-2 py-0.5 rounded-full text-xs font-medium uppercase">
                  {site.web_server}
                </span>
              </td>

              <!-- Server -->
              <td class="px-4 py-3 text-sm text-muted-foreground">{site.server_label ?? 'Local'}</td>

              <!-- Created -->
              <td class="px-4 py-3 text-sm text-muted-foreground">{formatDate(site.created_at)}</td>

              <!-- Actions -->
              <td class="px-4 py-3 text-sm" on:click|stopPropagation>
                <div class="flex items-center gap-1 justify-end">
                  <a
                    href="/websites/{site.id}"
                    class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground
                           hover:bg-muted hover:text-foreground inline-flex items-center gap-2"
                  >
                    Manage
                  </a>

                  <!-- Kebab menu -->
                  <div class="relative" data-kebab>
                    <button
                      on:click={() => openMenuId = openMenuId === site.id ? null : site.id}
                      class="h-9 w-9 rounded-lg border border-border text-muted-foreground
                             hover:bg-muted hover:text-foreground inline-flex items-center justify-center"
                    >
                      <svg class="w-4 h-4" fill="currentColor" viewBox="0 0 24 24">
                        <circle cx="12" cy="5"  r="1.5" /><circle cx="12" cy="12" r="1.5" /><circle cx="12" cy="19" r="1.5" />
                      </svg>
                    </button>
                    {#if openMenuId === site.id}
                      <div class="absolute right-0 top-full mt-1 w-52 bg-card border border-border
                                  rounded-xl shadow-xl z-20 py-1 overflow-hidden">
                        <a href="https://{site.domain}" target="_blank" rel="noopener noreferrer"
                           class="flex items-center gap-2 px-3 py-2 text-sm text-foreground hover:bg-muted">
                          <svg class="w-4 h-4 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                              d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14" />
                          </svg>
                          Visit Site
                        </a>
                        <div class="border-t border-border my-1"></div>
                        <p class="px-3 py-1 text-[10px] text-muted-foreground uppercase tracking-wider font-medium">Switch Web Server</p>
                        {#each [{ v: 'nginx', l: 'Nginx' }, { v: 'apache', l: 'Apache' }, { v: 'ols', l: 'LiteSpeed' }] as ws}
                          <button
                            on:click={() => switchWebserver(site, ws.v)}
                            disabled={switchingWebserverId === site.id}
                            class="w-full flex items-center gap-2 px-3 py-2 text-sm hover:bg-muted text-left
                                   {site.web_server === ws.v ? 'text-primary font-medium' : 'text-muted-foreground'}"
                          >
                            {#if site.web_server === ws.v}
                              <svg class="w-3.5 h-3.5 shrink-0 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>
                              </svg>
                            {:else}
                              <span class="w-3.5 h-3.5 shrink-0"></span>
                            {/if}
                            {ws.l}
                            {#if switchingWebserverId === site.id && site.web_server !== ws.v}
                              <svg class="w-3.5 h-3.5 ml-auto animate-spin text-muted-foreground" fill="none" viewBox="0 0 24 24">
                                <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                                <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"></path>
                              </svg>
                            {/if}
                          </button>
                        {/each}
                        <div class="border-t border-border my-1"></div>
                        {#if site.status === 'active'}
                          <button on:click={() => handleSuspend(site)}
                            class="w-full flex items-center gap-2 px-3 py-2 text-sm text-amber-400 hover:bg-muted text-left">
                            <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 9v6m4-6v6m7-3a9 9 0 11-18 0 9 9 0 0118 0z" />
                            </svg>
                            Suspend
                          </button>
                        {:else if site.status === 'suspended'}
                          <button on:click={() => handleUnsuspend(site)}
                            class="w-full flex items-center gap-2 px-3 py-2 text-sm text-green-400 hover:bg-muted text-left">
                            <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M14.752 11.168l-3.197-2.132A1 1 0 0010 9.87v4.263a1 1 0 001.555.832l3.197-2.132a1 1 0 000-1.664z" /><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
                            </svg>
                            Resume
                          </button>
                        {/if}
                        <div class="border-t border-border my-1"></div>
                        <button on:click={() => openDeleteModal(site)}
                          class="w-full flex items-center gap-2 px-3 py-2 text-sm text-red-400 hover:bg-muted text-left">
                          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                              d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5
                                 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
                          </svg>
                          Delete
                        </button>
                      </div>
                    {/if}
                  </div>
                </div>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>

    <!-- Mobile cards -->
    <div class="md:hidden space-y-3">
      {#each filteredSites as site (site.id)}
        <div
          class="bg-card border border-border rounded-xl overflow-hidden cursor-pointer hover:bg-muted/20"
          on:click={() => goto(`/websites/${site.id}`)}
        >
          <div class="flex items-center justify-between px-4 py-3 border-b border-border">
            <div class="flex items-center gap-2 min-w-0" on:click|stopPropagation>
              <input type="checkbox" checked={selectedIds.has(site.id)}
                on:change={() => toggleSelect(site.id)}
                class="rounded border-border accent-primary cursor-pointer shrink-0" />
              <svg class="w-4 h-4 text-muted-foreground shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
                  d="M21 12a9 9 0 01-9 9m9-9a9 9 0 00-9-9m9 9H3m9 9a9 9 0 01-9-9m9 9c1.657 0
                     3-4.03 3-9s-1.343-9-3-9m0 18c-1.657 0-3-4.03-3-9s1.343-9 3-9m-9 9a9 9 0 019-9" />
              </svg>
              <span class="font-medium text-foreground truncate">{site.domain}</span>
            </div>
            {#if site.status === 'active'}
              <span class="bg-green-500/10 text-green-400 border border-green-500/20 px-2 py-0.5 rounded-full text-xs font-medium inline-flex items-center gap-1 shrink-0">
                <span class="w-1.5 h-1.5 rounded-full bg-green-400"></span>Active
              </span>
            {:else if site.status === 'suspended'}
              <span class="bg-amber-500/10 text-amber-400 border border-amber-500/20 px-2 py-0.5 rounded-full text-xs font-medium inline-flex items-center gap-1 shrink-0">
                <span class="w-1.5 h-1.5 rounded-full bg-amber-400"></span>Suspended
              </span>
            {:else if site.status === 'provisioning'}
              <span class="bg-blue-500/10 text-blue-400 border border-blue-500/20 px-2 py-0.5 rounded-full text-xs font-medium inline-flex items-center gap-1 shrink-0">
                <span class="w-1.5 h-1.5 rounded-full bg-blue-400 animate-pulse"></span>Installing
              </span>
            {/if}
          </div>
          <div class="grid grid-cols-3 gap-3 px-4 py-3 text-xs text-muted-foreground">
            <div><span class="block text-muted-foreground/60 mb-0.5">PHP</span>{site.php_version}</div>
            <div><span class="block text-muted-foreground/60 mb-0.5">Server</span><span class="uppercase">{site.web_server}</span></div>
            <div><span class="block text-muted-foreground/60 mb-0.5">Host</span>{site.server_label ?? 'Local'}</div>
          </div>
          <div class="flex gap-2 px-4 py-3 border-t border-border" on:click|stopPropagation>
            <a href="/websites/{site.id}"
               class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground
                      hover:bg-muted hover:text-foreground inline-flex items-center gap-2">
              Manage
            </a>
            {#if site.status === 'active'}
              <button on:click={() => handleSuspend(site)}
                class="h-9 px-4 rounded-lg bg-amber-500/10 text-amber-400 border border-amber-500/20
                       text-sm font-medium hover:bg-amber-500/20 inline-flex items-center gap-2">
                Suspend
              </button>
            {:else if site.status === 'suspended'}
              <button on:click={() => handleUnsuspend(site)}
                class="h-9 px-4 rounded-lg bg-green-500/10 text-green-400 border border-green-500/20
                       text-sm font-medium hover:bg-green-500/20 inline-flex items-center gap-2">
                Resume
              </button>
            {/if}
            <button on:click={() => openDeleteModal(site)}
              class="h-9 px-4 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-sm
                     font-medium hover:bg-red-500/20 inline-flex items-center gap-2 ml-auto">
              Delete
            </button>
          </div>
        </div>
      {/each}
    </div>

    {/if}
  {/if}

</div>

<!-- ── Bulk action floating bar -->
{#if someSelected}
  <div class="fixed bottom-6 left-1/2 -translate-x-1/2 z-40 fade-up">
    <div class="bg-card border border-border rounded-2xl shadow-2xl shadow-black/30 px-4 py-3
                flex items-center gap-3">
      <span class="text-sm font-medium text-foreground">{selectedIds.size} selected</span>
      <div class="w-px h-4 bg-border"></div>
      <button
        on:click={bulkSuspend}
        disabled={bulkLoading}
        class="h-8 px-3 rounded-lg bg-amber-500/10 text-amber-400 border border-amber-500/20 text-xs
               font-medium hover:bg-amber-500/20 transition-colors disabled:opacity-50 inline-flex items-center gap-1.5"
      >
        <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
          <path stroke-linecap="round" stroke-linejoin="round" d="M10 9v6m4-6v6m7-3a9 9 0 11-18 0 9 9 0 0118 0z" />
        </svg>
        Suspend all
      </button>
      <button
        on:click={bulkResume}
        disabled={bulkLoading}
        class="h-8 px-3 rounded-lg bg-green-500/10 text-green-400 border border-green-500/20 text-xs
               font-medium hover:bg-green-500/20 transition-colors disabled:opacity-50 inline-flex items-center gap-1.5"
      >
        <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
          <path stroke-linecap="round" stroke-linejoin="round" d="M14.752 11.168l-3.197-2.132A1 1 0 0010 9.87v4.263a1 1 0 001.555.832l3.197-2.132a1 1 0 000-1.664z" /><path stroke-linecap="round" stroke-linejoin="round" d="M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
        </svg>
        Resume all
      </button>
      <button
        on:click={openBulkDeleteModal}
        disabled={bulkLoading}
        class="h-8 px-3 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-xs
               font-medium hover:bg-red-500/20 transition-colors disabled:opacity-50 inline-flex items-center gap-1.5"
      >
        <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
          <path stroke-linecap="round" stroke-linejoin="round"
            d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
        </svg>
        Delete all
      </button>
      <button
        on:click={() => { selectedIds = new Set(); }}
        class="h-8 w-8 rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground
               transition-colors inline-flex items-center justify-center"
        title="Clear selection"
      >
        <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
          <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12" />
        </svg>
      </button>
    </div>
  </div>
{/if}

<!-- ── Create Website modal ──────────────────────────────────────────────── -->
{#if showCreateModal}
  <div
    class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
    role="dialog"
    aria-modal="true"
    aria-labelledby="create-modal-title"
    on:click|self={() => { if (!createLoading) showCreateModal = false; }}
  >
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-lg shadow-2xl">
      <div class="flex items-center justify-between mb-5">
        <h2 id="create-modal-title" class="text-lg font-semibold text-foreground flex items-center gap-2"><svg class="w-4 h-4 text-primary shrink-0" viewBox="0 0 24 24" fill="none" aria-hidden="true"><circle cx="12" cy="12" r="2.8" fill="currentColor"/><ellipse cx="12" cy="12" rx="8" ry="3.2" stroke="currentColor" stroke-width="1.5" fill="none" transform="rotate(-30 12 12)"/><circle cx="18.9" cy="8.0" r="1.4" fill="currentColor"/></svg>Create Website</h2>
        <button
          on:click={() => showCreateModal = false}
          disabled={createLoading}
          class="w-8 h-8 rounded-lg flex items-center justify-center text-muted-foreground
                 hover:bg-muted hover:text-foreground"
        >
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      <!-- Quick Setup toggle -->
      <div class="flex items-center gap-3 mb-5 p-3 rounded-xl bg-muted/30 border border-border">
        <div class="flex-1">
          <p class="text-sm font-medium text-foreground">Quick Setup</p>
          <p class="text-xs text-muted-foreground mt-0.5">
            {quickSetup ? 'Domain + PHP only' : 'Full configuration options'}
          </p>
        </div>
        <button
          type="button"
          on:click={() => quickSetup = !quickSetup}
          class="relative inline-flex h-6 w-11 items-center rounded-full transition-colors
                 {quickSetup ? 'bg-primary' : 'bg-muted border border-border'}"
        >
          <span class="inline-block h-4 w-4 transform rounded-full bg-white shadow transition-transform
                       {quickSetup ? 'translate-x-6' : 'translate-x-1'}"></span>
        </button>
      </div>

      <div class="space-y-4">
        <!-- Domain -->
        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5" for="new-domain">Domain name</label>
          <input
            id="new-domain"
            type="text"
            bind:value={newSite.domain}
            on:blur={() => { newSite.domain = normalizeDomain(newSite.domain); }}
            placeholder="example.com"
            autocomplete="off"
            class="h-9 rounded-lg border {createError && createError.toLowerCase().includes('domain') ? 'border-red-500' : 'border-border'} bg-background px-3 text-sm text-foreground
                   placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50
                   focus:border-primary w-full"
          />
          {#if createError && createError.toLowerCase().includes('domain')}
            <p class="mt-1 text-xs text-red-400">{createError}</p>
          {/if}
        </div>

        <!-- PHP version -->
        <div>
          <label class="block text-sm font-medium text-foreground mb-2">PHP version</label>
          <div class="grid grid-cols-5 gap-2">
            {#each ['8.3', '8.2', '8.1', '8.0', '7.4'] as ver}
              <button
                type="button"
                on:click={() => newSite.php_version = ver}
                class="py-2 rounded-lg border text-sm font-medium transition-colors
                       {newSite.php_version === ver
                         ? 'border-primary bg-primary/10 text-primary'
                         : 'border-border text-muted-foreground hover:bg-muted hover:text-foreground'}"
              >
                {ver}
              </button>
            {/each}
          </div>
        </div>

        {#if !quickSetup}
          <!-- Web server -->
          <div>
            <label class="block text-sm font-medium text-foreground mb-2">Web server</label>
            <div class="grid grid-cols-3 gap-2">
              {#each [{ value: 'nginx', label: 'Nginx' }, { value: 'apache', label: 'Apache' }, { value: 'ols', label: 'LiteSpeed' }] as ws}
                <button
                  type="button"
                  on:click={() => newSite.web_server = ws.value}
                  class="py-2.5 rounded-lg border text-sm font-medium transition-colors
                         {newSite.web_server === ws.value
                           ? 'border-primary bg-primary/10 text-primary'
                           : 'border-border text-muted-foreground hover:bg-muted hover:text-foreground'}"
                >
                  {ws.label}
                </button>
              {/each}
            </div>
          </div>

          <!-- Site type -->
          <div>
            <label class="block text-sm font-medium text-foreground mb-2">Site type</label>
            <div class="grid grid-cols-3 gap-2">
              {#each [
                { value: 'static',    label: 'Static' },
                { value: 'wordpress', label: 'WordPress' },
                { value: 'php',       label: 'PHP App' },
              ] as st}
                <button
                  type="button"
                  on:click={() => newSite.plan = st.value}
                  class="py-2.5 rounded-lg border text-sm font-medium transition-colors
                         {newSite.plan === st.value
                           ? 'border-primary bg-primary/10 text-primary'
                           : 'border-border text-muted-foreground hover:bg-muted hover:text-foreground'}"
                >
                  {st.label}
                </button>
              {/each}
            </div>
          </div>

          <!-- Disk quota -->
          <div>
            <label class="block text-sm font-medium text-foreground mb-1.5" for="disk-quota">
              Disk quota <span class="text-muted-foreground font-normal">(optional, e.g. 5G)</span>
            </label>
            <input
              id="disk-quota"
              type="text"
              bind:value={diskQuota}
              placeholder="Unlimited"
              class="h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground
                     placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50
                     focus:border-primary w-full"
            />
          </div>

          <!-- PHP extensions preset -->
          <div>
            <label class="block text-sm font-medium text-foreground mb-2">PHP extensions preset</label>
            <select bind:value={phpPreset}
              class="h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground
                     focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary w-full">
              <option value="none">None (bare PHP)</option>
              <option value="wordpress">WordPress (gd, mbstring, xml, zip, curl)</option>
              <option value="laravel">Laravel (pdo, mbstring, openssl, tokenizer)</option>
              <option value="full">Full stack (all common extensions)</option>
            </select>
          </div>
        {/if}

        {#if createError}
          <p class="text-sm text-red-400 bg-red-500/10 border border-red-500/20 rounded-lg px-3 py-2">
            {createError}
          </p>
        {/if}

        <div class="flex gap-3 pt-1">
          <button
            type="button"
            on:click={() => showCreateModal = false}
            disabled={createLoading}
            class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground
                   hover:bg-muted hover:text-foreground inline-flex items-center gap-2 flex-1 justify-center"
          >
            Cancel
          </button>
          <button
            type="button"
            on:click={handleCreate}
            disabled={createLoading || !newSite.domain.trim()}
            class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                   hover:bg-primary/90 inline-flex items-center gap-2 flex-1 justify-center
                   disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {#if createLoading}
              <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
                <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"></path>
              </svg>
              Creating…
            {:else}
              Create Website
            {/if}
          </button>
        </div>
      </div>
    </div>
  </div>
{/if}

<!-- ── Delete confirmation modal ──────────────────────────────────────────── -->
{#if showDeleteModal && siteToDelete}
  <div
    class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
    role="dialog"
    aria-modal="true"
    aria-labelledby="delete-modal-title"
  >
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-lg shadow-2xl">
      <h2 id="delete-modal-title" class="text-lg font-semibold text-foreground mb-2 flex items-center gap-2"><svg class="w-4 h-4 text-primary shrink-0" viewBox="0 0 24 24" fill="none" aria-hidden="true"><circle cx="12" cy="12" r="2.8" fill="currentColor"/><ellipse cx="12" cy="12" rx="8" ry="3.2" stroke="currentColor" stroke-width="1.5" fill="none" transform="rotate(-30 12 12)"/><circle cx="18.9" cy="8.0" r="1.4" fill="currentColor"/></svg>Delete website</h2>
      <p class="text-sm text-muted-foreground mb-4">
        This will permanently delete <strong class="text-foreground">{siteToDelete.domain}</strong>
        and all its files, databases, and email accounts.
        <strong class="text-red-400">This cannot be undone.</strong>
      </p>
      <p class="text-sm text-foreground mb-2">
        Type <code class="font-mono bg-muted px-1.5 py-0.5 rounded text-xs">{siteToDelete.domain}</code> to confirm:
      </p>
      <input
        type="text"
        bind:value={confirmDeleteDomain}
        placeholder={siteToDelete.domain}
        class="h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground
               placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50
               focus:border-primary w-full mb-4"
      />
      <div class="flex gap-3">
        <button
          on:click={() => { showDeleteModal = false; siteToDelete = null; }}
          class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground
                 hover:bg-muted hover:text-foreground inline-flex items-center gap-2 flex-1 justify-center"
        >
          Cancel
        </button>
        <button
          on:click={handleDelete}
          disabled={confirmDeleteDomain !== siteToDelete.domain}
          class="h-9 px-4 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-sm
                 font-medium hover:bg-red-500/20 inline-flex items-center gap-2 flex-1 justify-center
                 disabled:opacity-40 disabled:cursor-not-allowed"
        >
          Delete website
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- ── Bulk delete confirmation modal ─────────────────────────────────────── -->
{#if showBulkDeleteModal}
  <div
    class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
    role="dialog"
    aria-modal="true"
  >
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-md shadow-2xl">
      <h2 class="text-lg font-semibold text-foreground mb-2">Delete {selectedIds.size} websites</h2>
      <p class="text-sm text-muted-foreground mb-4">
        This will permanently delete <strong class="text-foreground">{selectedIds.size} sites</strong>
        and all their files, databases, and email accounts.
        <strong class="text-red-400">This cannot be undone.</strong>
      </p>
      <p class="text-sm text-foreground mb-2">
        Type <code class="font-mono bg-muted px-1.5 py-0.5 rounded text-xs">DELETE</code> to confirm:
      </p>
      <input
        type="text"
        bind:value={bulkDeleteConfirm}
        placeholder="DELETE"
        class="h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground
               placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50
               focus:border-primary w-full mb-4"
      />
      <div class="flex gap-3">
        <button
          on:click={() => showBulkDeleteModal = false}
          class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground
                 hover:bg-muted hover:text-foreground inline-flex items-center gap-2 flex-1 justify-center"
        >
          Cancel
        </button>
        <button
          on:click={handleBulkDelete}
          disabled={bulkDeleteConfirm !== 'DELETE'}
          class="h-9 px-4 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-sm
                 font-medium hover:bg-red-500/20 inline-flex items-center gap-2 flex-1 justify-center
                 disabled:opacity-40 disabled:cursor-not-allowed"
        >
          Delete {selectedIds.size} sites
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- ── Toast ──────────────────────────────────────────────────────────────── -->
{#if toastMessage}
  <div
    class="fixed bottom-4 right-4 z-50 bg-card border border-border rounded-xl px-4 py-3 text-sm shadow-lg
           flex items-center gap-2 {toastType === 'success' ? 'text-green-400' : 'text-red-400'}"
    role="status"
    aria-live="polite"
  >
    {#if toastType === 'success'}
      <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7" />
      </svg>
    {:else}
      <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
      </svg>
    {/if}
    {toastMessage}
  </div>
{/if}
