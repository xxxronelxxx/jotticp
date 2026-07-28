<script lang="ts">
  import { onMount } from 'svelte';
  import { page } from '$app/stores';
  import { api } from '$api/client';
  import FileManager from '$lib/components/ui/FileManager.svelte';

  interface SiteOption {
    id: string;
    domain: string;
    unix_user: string;
    php_version?: string;
    ssl_status?: string;
    status?: string;
  }

  let sites: SiteOption[] = [];
  let selectedSiteId = '';
  let loading = true;
  let dropdownOpen = false;

  // Site picker search + recently accessed
  let pickerSearch = '';
  let pickerFocusIdx = -1;
  let recentSiteIds: string[] = [];
  const RECENT_KEY = 'orbit-fm-recent-sites';

  // Keyboard shortcuts help overlay
  let showShortcutsHelp = false;

  onMount(async () => {
    try {
      const stored = localStorage.getItem(RECENT_KEY);
      if (stored) recentSiteIds = JSON.parse(stored);
    } catch { /* ignore */ }

    try {
      const resp = await api.sites.list();
      sites = resp.map((s: any) => ({
        id: s.id,
        domain: s.domain,
        unix_user: s.unix_user,
        php_version: s.php_version,
        ssl_status: s.ssl_status,
        status: s.status,
      }));
      const urlSite = $page.url.searchParams.get('site');
      if (urlSite && sites.some(s => s.id === urlSite)) {
        selectedSiteId = urlSite;
      } else if (sites.length === 1) {
        selectedSiteId = sites[0].id;
      }
    } catch {
      // silent
    } finally {
      loading = false;
    }
  });

  $: selectedSite = sites.find(s => s.id === selectedSiteId);

  $: filteredPickerSites = pickerSearch.trim()
    ? sites.filter(s => s.domain.toLowerCase().includes(pickerSearch.toLowerCase()))
    : sortedPickerSites;

  $: sortedPickerSites = [
    ...sites.filter(s => recentSiteIds.includes(s.id)).sort((a, b) =>
      recentSiteIds.indexOf(a.id) - recentSiteIds.indexOf(b.id)),
    ...sites.filter(s => !recentSiteIds.includes(s.id)),
  ];

  function selectSite(id: string) {
    selectedSiteId = id;
    dropdownOpen = false;
    // Track recently accessed
    recentSiteIds = [id, ...recentSiteIds.filter(r => r !== id)].slice(0, 5);
    try { localStorage.setItem(RECENT_KEY, JSON.stringify(recentSiteIds)); } catch { /* ignore */ }
  }

  function handlePickerKeydown(e: KeyboardEvent) {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      pickerFocusIdx = Math.min(pickerFocusIdx + 1, filteredPickerSites.length - 1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      pickerFocusIdx = Math.max(pickerFocusIdx - 1, 0);
    } else if (e.key === 'Enter' && pickerFocusIdx >= 0) {
      const site = filteredPickerSites[pickerFocusIdx];
      if (site) selectSite(site.id);
    } else if (e.key === 'Escape') {
      pickerSearch = '';
      pickerFocusIdx = -1;
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') dropdownOpen = false;
    if (e.key === '?' && !e.ctrlKey && !e.metaKey) {
      const tag = (e.target as HTMLElement)?.tagName;
      if (tag !== 'INPUT' && tag !== 'TEXTAREA') {
        showShortcutsHelp = !showShortcutsHelp;
      }
    }
  }

  $: isRecentSite = (id: string) => recentSiteIds.includes(id);
</script>

<svelte:head><title>File Manager — JottiCP</title></svelte:head>

<svelte:window on:keydown={handleKeydown} />

<div class="flex flex-col h-[calc(100vh-4rem)] overflow-hidden">

  <!-- ── Page header ───────────────────────────────────────────────────────── -->
  <div class="flex items-center justify-between px-4 py-3 border-b border-border bg-background shrink-0">
    <!-- Left: icon + title + breadcrumb -->
    <div class="flex items-center gap-3 min-w-0">
      <div class="w-8 h-8 rounded-lg flex items-center justify-center shrink-0"
           style="background:rgba(245,158,11,.12)">
        <svg class="w-4 h-4" fill="#f59e0b" viewBox="0 0 20 20">
          <path d="M2 6a2 2 0 012-2h4l2 2h6a2 2 0 012 2v6a2 2 0 01-2 2H4a2 2 0 01-2-2V6z"/>
        </svg>
      </div>
      <div class="min-w-0">
        <div class="flex items-center gap-1.5 text-sm">
          <span class="font-semibold text-foreground">File Manager</span>
          {#if selectedSite}
            <svg class="w-3.5 h-3.5 text-muted-foreground shrink-0" fill="none" viewBox="0 0 16 16" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M6 12l4-4-4-4"/>
            </svg>
            <span class="text-muted-foreground font-mono truncate">{selectedSite.domain}</span>
          {/if}
        </div>
        {#if !selectedSite}
          <p class="text-xs text-muted-foreground">{sites.length} site{sites.length !== 1 ? 's' : ''} available</p>
        {:else}
          <p class="text-xs text-muted-foreground font-mono">/home/{selectedSite.unix_user}</p>
        {/if}
      </div>
    </div>

    <!-- Right: keyboard shortcuts help + site switcher -->
    <div class="flex items-center gap-2 shrink-0">
      <!-- Keyboard shortcuts help button -->
      {#if selectedSite}
        <button
          type="button"
          on:click={() => showShortcutsHelp = !showShortcutsHelp}
          class="h-8 w-8 rounded-lg border border-border text-muted-foreground hover:bg-muted hover:text-foreground
                 inline-flex items-center justify-center transition-colors text-xs font-bold"
          title="Keyboard shortcuts (?)"
        >
          ?
        </button>
      {/if}

    <!-- Site switcher pill (only when a site is selected and multiple sites exist) -->
    {#if selectedSite && sites.length > 1}
      <div class="relative shrink-0">
        <button
          type="button"
          on:click={() => dropdownOpen = !dropdownOpen}
          class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground
                 hover:bg-muted hover:text-foreground inline-flex items-center gap-2 transition-colors"
        >
          <svg class="w-4 h-4 text-amber-500 shrink-0" fill="currentColor" viewBox="0 0 20 20">
            <path d="M2 6a2 2 0 012-2h4l2 2h6a2 2 0 012 2v6a2 2 0 01-2 2H4a2 2 0 01-2-2V6z"/>
          </svg>
          <span class="font-mono truncate max-w-[160px]">{selectedSite.domain}</span>
          <svg class="w-3.5 h-3.5 transition-transform {dropdownOpen ? 'rotate-180' : ''}" fill="none" viewBox="0 0 16 16" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M4 6l4 4 4-4"/>
          </svg>
        </button>

        {#if dropdownOpen}
          <!-- svelte-ignore a11y-click-events-have-key-events -->
          <!-- svelte-ignore a11y-no-static-element-interactions -->
          <div
            class="absolute right-0 top-11 z-50 bg-card border border-border rounded-xl shadow-2xl
                   min-w-[220px] max-w-[300px] py-1 overflow-hidden"
            on:click|stopPropagation
          >
            <p class="px-3 py-2 text-[10px] font-semibold text-muted-foreground uppercase tracking-wider">
              Switch site
            </p>
            {#each sites as s}
              <button
                type="button"
                on:click={() => selectSite(s.id)}
                class="w-full flex items-center gap-2.5 px-3 py-2.5 text-left text-sm transition-colors
                       hover:bg-muted/60
                       {selectedSiteId === s.id ? 'text-primary font-medium' : 'text-foreground'}"
              >
                <div class="w-6 h-6 rounded-md flex items-center justify-center shrink-0"
                     style="background:rgba(245,158,11,.1)">
                  <svg class="w-3 h-3" fill="#f59e0b" viewBox="0 0 20 20">
                    <path d="M2 6a2 2 0 012-2h4l2 2h6a2 2 0 012 2v6a2 2 0 01-2 2H4a2 2 0 01-2-2V6z"/>
                  </svg>
                </div>
                <span class="font-mono truncate">{s.domain}</span>
                {#if selectedSiteId === s.id}
                  <svg class="w-3.5 h-3.5 ml-auto text-primary shrink-0" fill="none" viewBox="0 0 16 16" stroke="currentColor" stroke-width="2.5">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M3 8l4 4 6-7"/>
                  </svg>
                {/if}
              </button>
            {/each}
          </div>
          <!-- click-away -->
          <!-- svelte-ignore a11y-click-events-have-key-events -->
          <!-- svelte-ignore a11y-no-static-element-interactions -->
          <div class="fixed inset-0 z-40" on:click={() => dropdownOpen = false}></div>
        {/if}
      </div>
    {:else if sites.length === 1 && selectedSite}
      <span class="text-xs text-muted-foreground font-mono">{selectedSite.domain}</span>
    {/if}
    </div><!-- end shortcuts+switcher wrapper -->
  </div>

  <!-- ── Content area ──────────────────────────────────────────────────────── -->
  <div class="flex-1 overflow-hidden min-h-0">

    {#if loading}
      <!-- Loading state -->
      <div class="flex items-center justify-center h-full">
        <div class="flex flex-col items-center gap-3">
          <div class="w-8 h-8 rounded-full border-2 border-border border-t-primary animate-spin"></div>
          <p class="text-sm text-muted-foreground">Loading sites…</p>
        </div>
      </div>

    {:else if sites.length === 0}
      <!-- No sites at all -->
      <div class="flex items-center justify-center h-full p-8">
        <div class="text-center max-w-sm">
          <div class="w-16 h-16 rounded-2xl bg-muted flex items-center justify-center mx-auto mb-4">
            <svg class="w-8 h-8 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
              <path stroke-linecap="round" stroke-linejoin="round" d="M2.25 12.75V12A2.25 2.25 0 014.5 9.75h15A2.25 2.25 0 0121.75 12v.75m-8.69-6.44l-2.12-2.12a1.5 1.5 0 00-1.061-.44H4.5A2.25 2.25 0 002.25 6v12a2.25 2.25 0 002.25 2.25h15A2.25 2.25 0 0021.75 18V9a2.25 2.25 0 00-2.25-2.25h-5.379a1.5 1.5 0 01-1.06-.44z"/>
            </svg>
          </div>
          <h3 class="text-base font-semibold text-foreground mb-1">No sites yet</h3>
          <p class="text-sm text-muted-foreground">Create a site first to start browsing files.</p>
          <a href="/sites/new"
             class="mt-4 h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                    hover:bg-primary/90 inline-flex items-center gap-2 transition-colors">
            <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M12 4v16m8-8H4"/>
            </svg>
            Create site
          </a>
        </div>
      </div>

    {:else if !selectedSiteId}
      <!-- Site picker — card grid -->
      <div class="p-6 h-full overflow-y-auto">
        <div class="max-w-2xl mx-auto">
          <div class="flex items-center gap-3 mb-6">
            <div class="w-10 h-10 rounded-xl flex items-center justify-center shrink-0"
                 style="background:rgba(245,158,11,.12)">
              <svg class="w-5 h-5" fill="#f59e0b" viewBox="0 0 20 20">
                <path d="M2 6a2 2 0 012-2h4l2 2h6a2 2 0 012 2v6a2 2 0 01-2 2H4a2 2 0 01-2-2V6z"/>
              </svg>
            </div>
            <div>
              <h2 class="text-base font-semibold text-foreground">Select a site to browse files</h2>
              <p class="text-xs text-muted-foreground">{sites.length} site{sites.length !== 1 ? 's' : ''} available</p>
            </div>
          </div>

          <div class="grid grid-cols-1 sm:grid-cols-2 gap-3">
            {#each sites as s}
              <button
                type="button"
                on:click={() => selectSite(s.id)}
                class="bg-card border border-border rounded-xl p-4 text-left hover:border-primary/40
                       hover:bg-muted/30 transition-all group"
              >
                <div class="flex items-center gap-3">
                  <div class="w-10 h-10 rounded-xl flex items-center justify-center shrink-0 transition-colors
                              group-hover:bg-amber-500/20"
                       style="background:rgba(245,158,11,.1)">
                    <svg class="w-5 h-5" fill="#f59e0b" viewBox="0 0 20 20">
                      <path d="M2 6a2 2 0 012-2h4l2 2h6a2 2 0 012 2v6a2 2 0 01-2 2H4a2 2 0 01-2-2V6z"/>
                    </svg>
                  </div>
                  <div class="min-w-0">
                    <p class="text-sm font-medium text-foreground font-mono truncate">{s.domain}</p>
                    <p class="text-xs text-muted-foreground font-mono">/home/{s.unix_user}</p>
                  </div>
                  <svg class="w-4 h-4 text-muted-foreground ml-auto shrink-0 opacity-0 group-hover:opacity-100 transition-opacity"
                       fill="none" viewBox="0 0 16 16" stroke="currentColor" stroke-width="2">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M6 12l4-4-4-4"/>
                  </svg>
                </div>
              </button>
            {/each}
          </div>
        </div>
      </div>

    {:else}
      <!-- File manager full-screen embed -->
      {#key selectedSiteId}
        <FileManager
          siteId={selectedSiteId}
          unixUser={selectedSite?.unix_user ?? ''}
          initialPath="/"
          embedded={false}
        />
      {/key}
    {/if}

  </div>
</div>

<!-- ── Keyboard shortcuts overlay ──────────────────────────────────────── -->
{#if showShortcutsHelp}
  <!-- svelte-ignore a11y-click-events-have-key-events -->
  <!-- svelte-ignore a11y-no-static-element-interactions -->
  <div class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
       on:click|self={() => showShortcutsHelp = false}>
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-sm shadow-2xl">
      <div class="flex items-center justify-between mb-4">
        <h3 class="text-base font-semibold text-foreground">Keyboard Shortcuts</h3>
        <button
          type="button"
          on:click={() => showShortcutsHelp = false}
          class="w-7 h-7 rounded-lg flex items-center justify-center text-muted-foreground
                 hover:bg-muted hover:text-foreground transition-colors"
        >
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>
      <div class="space-y-2">
        {#each [
          ['Ctrl+A', 'Select all'],
          ['Del', 'Delete selected'],
          ['F2', 'Rename'],
          ['Ctrl+C', 'Copy'],
          ['Ctrl+V', 'Paste'],
          ['Ctrl+X', 'Cut'],
          ['Ctrl+Z', 'Undo'],
          ['?', 'Toggle this help'],
        ] as [key, desc]}
          <div class="flex items-center justify-between py-1.5 border-b border-border/50 last:border-0">
            <span class="text-sm text-muted-foreground">{desc}</span>
            <kbd class="text-xs font-mono bg-muted border border-border px-2 py-0.5 rounded text-foreground">{key}</kbd>
          </div>
        {/each}
      </div>
    </div>
  </div>
{/if}
