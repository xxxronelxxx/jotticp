<script lang="ts">
  import { onMount } from 'svelte';
  import { get } from 'svelte/store';
  import { auth } from '$lib/stores/auth';
  import { api } from '$api/client';
  import type { Site } from '$api/client';

  export let siteId: string;
  export let site: Site;

  // ── Types ──────────────────────────────────────────────────────────────────
  interface AppInstall {
    id: string;
    app_slug: string;
    app_name: string;
    version: string;
    domain: string;
    path: string;
    status: string;
    installed_at: string;
    admin_url: string | null;
  }

  interface WpCore { version: string; update_available: boolean; update_version: string | null }
  interface WpPlugin { name: string; slug: string; version: string; status: 'active' | 'inactive'; update_available: boolean; update_version: string | null }
  interface WpTheme { name: string; slug: string; version: string; status: 'active' | 'inactive' }
  interface WpUser { id: number; login: string; email: string; display_name: string; role: string }
  interface UpdateAllResult { core: boolean; plugins: number; themes: number; errors: string[] }
  interface Toast { id: number; message: string; type: 'success' | 'error' }

  // ── Auth helper ────────────────────────────────────────────────────────────
  function authH(): Record<string, string> {
    const t = get(auth).token;
    return t ? { Authorization: 'Bearer ' + t, 'Content-Type': 'application/json' } : { 'Content-Type': 'application/json' };
  }

  // ── State ──────────────────────────────────────────────────────────────────
  let apps: AppInstall[] = [];
  let appsLoading = true;

  // WordPress state per app (keyed by app id)
  let expandedApp: string | null = null;

  // WordPress data (loaded when WordPress app is expanded)
  let wpCore: WpCore | null = null;
  let wpPlugins: WpPlugin[] = [];
  let wpThemes: WpTheme[] = [];
  let wpUsers: WpUser[] = [];
  let wpLoading = false;
  let coreUpdating = false;
  let pluginBusy: Record<string, boolean> = {};
  let themeBusy: Record<string, boolean> = {};
  let updatingAll = false;
  let updateAllResult: UpdateAllResult | null = null;

  // Confirm state
  let confirmDeletePlugin: { slug: string; name: string } | null = null;
  let confirmDeleteTheme: { slug: string; name: string } | null = null;
  let confirmUpdateCore = false;
  let confirmUpdateAll = false;

  // Install plugin modal
  let showInstallPlugin = false;
  let installSlug = '';
  let installing = false;
  let installError = '';

  // Login URL modal
  let showLoginUrl: { user: WpUser; url: string } | null = null;
  let loginUrlLoading: number | null = null;

  let toasts: Toast[] = [];
  let toastCounter = 0;

  // Active sub-section within the WordPress panel
  let wpSection: 'overview' | 'plugins' | 'themes' | 'users' = 'overview';

  // ── Helpers ────────────────────────────────────────────────────────────────
  function showToast(message: string, type: 'success' | 'error' = 'success') {
    const id = ++toastCounter;
    toasts = [...toasts, { id, message, type }];
    setTimeout(() => { toasts = toasts.filter(t => t.id !== id); }, 4500);
  }

  async function copyToClipboard(text: string) {
    try { await navigator.clipboard.writeText(text); showToast('Copied'); }
    catch { showToast('Copy failed', 'error'); }
  }

  function appIcon(slug: string): string {
    if (slug.includes('wordpress')) return '⟁';
    if (slug.includes('joomla')) return 'J';
    if (slug.includes('drupal')) return 'D';
    if (slug.includes('laravel')) return 'L';
    return '◈';
  }

  function appColor(slug: string): string {
    if (slug.includes('wordpress')) return 'bg-blue-500/10 text-blue-400 border-blue-500/20';
    if (slug.includes('joomla')) return 'bg-orange-500/10 text-orange-400 border-orange-500/20';
    if (slug.includes('drupal')) return 'bg-cyan-500/10 text-cyan-400 border-cyan-500/20';
    return 'bg-primary/10 text-primary border-primary/20';
  }

  // ── App loading ────────────────────────────────────────────────────────────
  onMount(async () => {
    appsLoading = true;
    try {
      apps = await api.apps.listInstalled(siteId) as AppInstall[];
    } catch {
      apps = [];
    } finally {
      appsLoading = false;
    }
  });

  // ── WordPress management ───────────────────────────────────────────────────
  async function expandApp(app: AppInstall) {
    if (expandedApp === app.id) { expandedApp = null; return; }
    expandedApp = app.id;
    if (!app.app_slug.includes('wordpress')) return;
    wpSection = 'overview';
    await loadWpAll(app);
  }

  async function loadWpAll(app: AppInstall) {
    wpLoading = true;
    wpCore = null; wpPlugins = []; wpThemes = []; wpUsers = [];
    try {
      const [coreRes, plugRes, themeRes, userRes] = await Promise.allSettled([
        fetch(`/api/v1/wordpress/${siteId}/core`, { headers: authH() }),
        fetch(`/api/v1/wordpress/${siteId}/plugins`, { headers: authH() }),
        fetch(`/api/v1/wordpress/${siteId}/themes`, { headers: authH() }),
        fetch(`/api/v1/wordpress/${siteId}/users`, { headers: authH() }),
      ]);
      if (coreRes.status === 'fulfilled' && coreRes.value.ok) wpCore = await coreRes.value.json() as WpCore;
      if (plugRes.status === 'fulfilled' && plugRes.value.ok) wpPlugins = await plugRes.value.json() as WpPlugin[];
      if (themeRes.status === 'fulfilled' && themeRes.value.ok) wpThemes = await themeRes.value.json() as WpTheme[];
      if (userRes.status === 'fulfilled' && userRes.value.ok) wpUsers = await userRes.value.json() as WpUser[];
    } catch { /* non-fatal */ }
    finally { wpLoading = false; }
  }

  async function updateCore() {
    confirmUpdateCore = false; coreUpdating = true;
    try {
      const res = await fetch(`/api/v1/wordpress/${siteId}/core/update`, { method: 'POST', headers: authH() });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      showToast('WordPress core updated');
      const r = await fetch(`/api/v1/wordpress/${siteId}/core`, { headers: authH() });
      if (r.ok) wpCore = await r.json() as WpCore;
    } catch { showToast('Update failed', 'error'); }
    finally { coreUpdating = false; }
  }

  async function activatePlugin(slug: string) {
    pluginBusy = { ...pluginBusy, [slug]: true };
    try {
      const res = await fetch(`/api/v1/wordpress/${siteId}/plugins/${encodeURIComponent(slug)}/activate`, { method: 'POST', headers: authH() });
      if (!res.ok) throw new Error();
      wpPlugins = wpPlugins.map(p => p.slug === slug ? { ...p, status: 'active' } : p);
      showToast('Plugin activated');
    } catch { showToast('Failed', 'error'); }
    finally { pluginBusy = { ...pluginBusy, [slug]: false }; }
  }

  async function deactivatePlugin(slug: string) {
    pluginBusy = { ...pluginBusy, [slug]: true };
    try {
      const res = await fetch(`/api/v1/wordpress/${siteId}/plugins/${encodeURIComponent(slug)}/deactivate`, { method: 'POST', headers: authH() });
      if (!res.ok) throw new Error();
      wpPlugins = wpPlugins.map(p => p.slug === slug ? { ...p, status: 'inactive' } : p);
      showToast('Plugin deactivated');
    } catch { showToast('Failed', 'error'); }
    finally { pluginBusy = { ...pluginBusy, [slug]: false }; }
  }

  async function deletePlugin(slug: string, name: string) {
    if (!confirmDeletePlugin || confirmDeletePlugin.slug !== slug) { confirmDeletePlugin = { slug, name }; return; }
    confirmDeletePlugin = null;
    pluginBusy = { ...pluginBusy, [slug]: true };
    try {
      const res = await fetch(`/api/v1/wordpress/${siteId}/plugins/${encodeURIComponent(slug)}`, { method: 'DELETE', headers: authH() });
      if (!res.ok) throw new Error();
      wpPlugins = wpPlugins.filter(p => p.slug !== slug);
      showToast('Plugin deleted');
    } catch { showToast('Failed', 'error'); }
    finally { pluginBusy = { ...pluginBusy, [slug]: false }; }
  }

  async function activateTheme(slug: string) {
    themeBusy = { ...themeBusy, [slug]: true };
    try {
      const res = await fetch(`/api/v1/wordpress/${siteId}/themes/${encodeURIComponent(slug)}/activate`, { method: 'POST', headers: authH() });
      if (!res.ok) throw new Error();
      wpThemes = wpThemes.map(t => ({ ...t, status: t.slug === slug ? 'active' : 'inactive' }));
      showToast('Theme activated');
    } catch { showToast('Failed', 'error'); }
    finally { themeBusy = { ...themeBusy, [slug]: false }; }
  }

  async function deleteTheme(slug: string, name: string) {
    if (!confirmDeleteTheme || confirmDeleteTheme.slug !== slug) { confirmDeleteTheme = { slug, name }; return; }
    confirmDeleteTheme = null;
    themeBusy = { ...themeBusy, [slug]: true };
    try {
      const res = await fetch(`/api/v1/wordpress/${siteId}/themes/${encodeURIComponent(slug)}`, { method: 'DELETE', headers: authH() });
      if (!res.ok) throw new Error();
      wpThemes = wpThemes.filter(t => t.slug !== slug);
      showToast('Theme deleted');
    } catch { showToast('Failed', 'error'); }
    finally { themeBusy = { ...themeBusy, [slug]: false }; }
  }

  async function getLoginUrl(user: WpUser) {
    loginUrlLoading = user.id;
    try {
      const res = await fetch(`/api/v1/wordpress/${siteId}/users/${user.id}/login-url`, { method: 'POST', headers: authH() });
      if (!res.ok) throw new Error();
      const data = await res.json() as { login_url: string };
      showLoginUrl = { user, url: data.login_url };
    } catch { showToast('Failed to get login URL', 'error'); }
    finally { loginUrlLoading = null; }
  }

  async function runUpdateAll() {
    if (!confirmUpdateAll) { confirmUpdateAll = true; return; }
    confirmUpdateAll = false; updatingAll = true; updateAllResult = null;
    try {
      const res = await fetch(`/api/v1/wordpress/${siteId}/update-all`, { method: 'POST', headers: authH() });
      if (!res.ok) throw new Error();
      updateAllResult = await res.json() as UpdateAllResult;
      showToast('Update All completed');
      const r = await fetch(`/api/v1/wordpress/${siteId}/core`, { headers: authH() });
      if (r.ok) wpCore = await r.json() as WpCore;
    } catch { showToast('Update All failed', 'error'); }
    finally { updatingAll = false; }
  }

  async function handleInstallPlugin() {
    installError = '';
    if (!installSlug.trim()) { installError = 'Plugin slug is required'; return; }
    installing = true;
    try {
      const res = await fetch(`/api/v1/wordpress/${siteId}/plugins`, { method: 'POST', headers: authH(), body: JSON.stringify({ slug: installSlug.trim() }) });
      if (!res.ok) { const e = await res.json() as { message?: string }; throw new Error(e.message ?? 'Failed'); }
      showInstallPlugin = false;
      showToast(`"${installSlug.trim()}" installed`);
      const r = await fetch(`/api/v1/wordpress/${siteId}/plugins`, { headers: authH() });
      if (r.ok) wpPlugins = await r.json() as WpPlugin[];
    } catch (e: unknown) { installError = e instanceof Error ? e.message : 'Installation failed'; }
    finally { installing = false; }
  }
</script>

<svelte:window on:keydown={(e) => { if (e.key === 'Escape') { showInstallPlugin = false; showLoginUrl = null; } }} />

<style>
  @keyframes fadeUp { from { opacity:0; transform:translateY(4px) } to { opacity:1; transform:none } }
  .tab-content { animation: fadeUp 0.2s ease-out both }
  @keyframes slideDown { from { opacity:0; max-height:0 } to { opacity:1; max-height:2000px } }
  .panel-open { animation: slideDown 0.25s ease-out both }
</style>

<div class="tab-content space-y-4">

  <!-- ── Header ────────────────────────────────────────────────────────────── -->
  <div class="flex items-center justify-between">
    <div>
      <h2 class="text-base font-semibold text-foreground">Installed Applications</h2>
      <p class="text-xs text-muted-foreground mt-0.5">Apps installed on <span class="font-mono">{site.domain}</span></p>
    </div>
    {#if !appsLoading}
      <span class="inline-flex items-center px-2.5 py-1 rounded-full text-xs font-medium bg-muted text-muted-foreground border border-border">
        {apps.length} app{apps.length !== 1 ? 's' : ''}
      </span>
    {/if}
  </div>

  <!-- ── Loading ───────────────────────────────────────────────────────────── -->
  {#if appsLoading}
    <div class="space-y-3">
      {#each [1, 2] as _}
        <div class="bg-card border border-border rounded-xl p-5 animate-pulse">
          <div class="flex items-center gap-4">
            <div class="w-12 h-12 rounded-xl bg-muted shrink-0"></div>
            <div class="flex-1 space-y-2">
              <div class="h-4 bg-muted rounded w-1/3"></div>
              <div class="h-3 bg-muted rounded w-1/2"></div>
            </div>
          </div>
        </div>
      {/each}
    </div>

  <!-- ── Empty state ───────────────────────────────────────────────────────── -->
  {:else if apps.length === 0}
    <div class="bg-card border border-border rounded-xl p-12 flex flex-col items-center text-center gap-4">
      <div class="w-16 h-16 rounded-2xl bg-muted flex items-center justify-center">
        <svg class="w-8 h-8 text-muted-foreground/50" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.25">
          <path stroke-linecap="round" stroke-linejoin="round" d="M4 6a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2H6a2 2 0 01-2-2V6zM14 6a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2h-2a2 2 0 01-2-2V6zM4 16a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2H6a2 2 0 01-2-2v-2zM14 16a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2h-2a2 2 0 01-2-2v-2z"/>
        </svg>
      </div>
      <div>
        <h3 class="text-sm font-semibold text-foreground">No applications installed</h3>
        <p class="text-xs text-muted-foreground mt-1 max-w-xs">Install WordPress, Joomla, or other apps from the Apps installer in the Overview tab.</p>
      </div>
    </div>

  <!-- ── App cards ─────────────────────────────────────────────────────────── -->
  {:else}
    <div class="space-y-3">
      {#each apps as app (app.id)}
        {@const isWp = app.app_slug.includes('wordpress')}
        {@const isOpen = expandedApp === app.id}

        <div class="bg-card border border-border rounded-xl overflow-hidden transition-all {isOpen ? 'ring-1 ring-primary/20' : ''}">

          <!-- App card header -->
          <button
            type="button"
            class="w-full flex items-center gap-4 p-5 text-left hover:bg-muted/20 transition-colors"
            on:click={() => expandApp(app)}
          >
            <!-- App icon -->
            <div class="w-12 h-12 rounded-xl border flex items-center justify-center shrink-0 text-xl font-bold {appColor(app.app_slug)}">
              {#if isWp}
                <svg class="w-7 h-7" viewBox="0 0 24 24" fill="currentColor">
                  <path d="M12 2C6.486 2 2 6.486 2 12s4.486 10 10 10 10-4.486 10-10S17.514 2 12 2zm0 1.5c1.8 0 3.47.54 4.86 1.46L5.96 16.86A8.504 8.504 0 013.5 12c0-4.687 3.813-8.5 8.5-8.5zm0 17c-1.8 0-3.47-.54-4.86-1.46l10.9-11.9A8.504 8.504 0 0120.5 12c0 4.687-3.813 8.5-8.5 8.5z"/>
                </svg>
              {:else}
                {appIcon(app.app_slug)}
              {/if}
            </div>

            <!-- App info -->
            <div class="flex-1 min-w-0">
              <div class="flex items-center gap-2 flex-wrap">
                <span class="font-semibold text-sm text-foreground">{app.app_name}</span>
                <span class="inline-flex items-center px-2 py-0.5 rounded text-xs font-mono font-medium bg-muted text-muted-foreground">v{app.version}</span>
                {#if app.status === 'active'}
                  <span class="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium bg-green-500/10 text-green-400 border border-green-500/20">
                    <span class="w-1.5 h-1.5 rounded-full bg-green-400"></span>Active
                  </span>
                {/if}
              </div>
              <div class="flex items-center gap-3 mt-1 text-xs text-muted-foreground flex-wrap">
                <span class="flex items-center gap-1">
                  <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M21 12a9 9 0 01-9 9m9-9a9 9 0 00-9-9m9 9H3"/></svg>
                  <span class="font-mono">{app.domain}{app.path !== '/' ? app.path : ''}</span>
                </span>
                {#if isWp && wpCore && isOpen}
                  {#if wpCore.update_available}
                    <span class="text-blue-400">→ v{wpCore.update_version} available</span>
                  {:else}
                    <span class="text-green-400">Up to date</span>
                  {/if}
                {/if}
              </div>
            </div>

            <!-- Right actions -->
            <div class="flex items-center gap-2 shrink-0" on:click|stopPropagation>
              {#if app.admin_url}
                <a
                  href={app.admin_url}
                  target="_blank"
                  rel="noopener noreferrer"
                  class="h-8 px-3 rounded-lg border border-border text-xs font-medium text-muted-foreground hover:bg-muted hover:text-foreground transition-colors inline-flex items-center gap-1.5"
                >
                  Admin
                  <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14"/></svg>
                </a>
              {/if}
              <!-- Expand chevron -->
              <svg class="w-4 h-4 text-muted-foreground transition-transform {isOpen ? 'rotate-180' : ''}" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                <path stroke-linecap="round" stroke-linejoin="round" d="M19 9l-7 7-7-7"/>
              </svg>
            </div>
          </button>

          <!-- ── WordPress Management Panel ──────────────────────────── -->
          {#if isOpen && isWp}
            <div class="panel-open border-t border-border">

              {#if wpLoading}
                <div class="p-6 flex items-center justify-center gap-3 text-muted-foreground">
                  <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"/></svg>
                  <span class="text-sm">Loading WordPress data…</span>
                </div>

              {:else}
                <!-- Sub-nav -->
                <div class="flex items-center gap-1 px-4 pt-4 pb-0">
                  {#each [['overview','Overview'], ['plugins','Plugins'], ['themes','Themes'], ['users','Users']] as [key, label]}
                    <button
                      class="h-8 px-3 rounded-lg text-xs font-medium transition-colors {wpSection === key ? 'bg-muted text-foreground' : 'text-muted-foreground hover:text-foreground hover:bg-muted/50'}"
                      on:click={() => { wpSection = key as typeof wpSection; }}
                    >{label}{key === 'plugins' && wpPlugins.length > 0 ? ` (${wpPlugins.length})` : ''}{key === 'themes' && wpThemes.length > 0 ? ` (${wpThemes.length})` : ''}</button>
                  {/each}

                  <div class="flex-1"></div>

                  <!-- Update All button -->
                  {#if confirmUpdateAll}
                    <div class="flex items-center gap-2 px-3 py-1.5 rounded-lg bg-amber-500/10 border border-amber-500/30">
                      <span class="text-xs text-amber-300">Update all now?</span>
                      <button class="h-6 px-2.5 rounded bg-primary text-primary-foreground text-[10px] font-medium" on:click={runUpdateAll}>Confirm</button>
                      <button class="h-6 px-2.5 rounded border border-border text-[10px] text-muted-foreground hover:bg-muted" on:click={() => confirmUpdateAll = false}>Cancel</button>
                    </div>
                  {:else}
                    <button
                      class="h-8 px-3 rounded-lg border border-border text-xs font-medium text-muted-foreground hover:bg-muted transition-colors inline-flex items-center gap-1.5 disabled:opacity-40"
                      disabled={updatingAll}
                      on:click={runUpdateAll}
                    >
                      {#if updatingAll}
                        <svg class="w-3 h-3 animate-spin" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"/></svg>
                      {/if}
                      Update All
                    </button>
                  {/if}
                </div>

                <div class="p-4 space-y-4">

                  <!-- OVERVIEW section -->
                  {#if wpSection === 'overview'}
                    <div class="grid grid-cols-2 sm:grid-cols-4 gap-3">
                      <!-- Core version card -->
                      <div class="bg-muted/30 border border-border rounded-xl p-4">
                        <p class="text-xs text-muted-foreground mb-1">Core</p>
                        <p class="font-mono text-sm font-bold text-foreground">{wpCore?.version ?? '—'}</p>
                        {#if wpCore?.update_available}
                          <p class="text-[10px] text-blue-400 mt-0.5">→ {wpCore.update_version}</p>
                        {:else}
                          <p class="text-[10px] text-green-400 mt-0.5">Up to date</p>
                        {/if}
                      </div>
                      <div class="bg-muted/30 border border-border rounded-xl p-4 cursor-pointer hover:border-primary/40 transition-colors" on:click={() => wpSection = 'plugins'} role="button" tabindex="0" on:keydown={(e) => e.key === 'Enter' && (wpSection = 'plugins')}>
                        <p class="text-xs text-muted-foreground mb-1">Plugins</p>
                        <p class="text-sm font-bold text-foreground">{wpPlugins.length}</p>
                        {#if wpPlugins.filter(p => p.update_available).length > 0}
                          <p class="text-[10px] text-blue-400 mt-0.5">{wpPlugins.filter(p => p.update_available).length} update{wpPlugins.filter(p => p.update_available).length > 1 ? 's' : ''}</p>
                        {:else}
                          <p class="text-[10px] text-muted-foreground mt-0.5">{wpPlugins.filter(p => p.status === 'active').length} active</p>
                        {/if}
                      </div>
                      <div class="bg-muted/30 border border-border rounded-xl p-4 cursor-pointer hover:border-primary/40 transition-colors" on:click={() => wpSection = 'themes'} role="button" tabindex="0" on:keydown={(e) => e.key === 'Enter' && (wpSection = 'themes')}>
                        <p class="text-xs text-muted-foreground mb-1">Themes</p>
                        <p class="text-sm font-bold text-foreground">{wpThemes.length}</p>
                        <p class="text-[10px] text-muted-foreground mt-0.5">{wpThemes.find(t => t.status === 'active')?.name ?? '—'} active</p>
                      </div>
                      <div class="bg-muted/30 border border-border rounded-xl p-4 cursor-pointer hover:border-primary/40 transition-colors" on:click={() => wpSection = 'users'} role="button" tabindex="0" on:keydown={(e) => e.key === 'Enter' && (wpSection = 'users')}>
                        <p class="text-xs text-muted-foreground mb-1">Users</p>
                        <p class="text-sm font-bold text-foreground">{wpUsers.length}</p>
                        <p class="text-[10px] text-muted-foreground mt-0.5">{wpUsers.filter(u => u.role === 'administrator').length} admin</p>
                      </div>
                    </div>

                    {#if wpCore?.update_available}
                      <div class="flex items-center justify-between p-4 rounded-xl bg-blue-500/10 border border-blue-500/20">
                        <div>
                          <p class="text-sm font-medium text-blue-300">WordPress {wpCore.update_version} available</p>
                          <p class="text-xs text-blue-400/70 mt-0.5">Currently on {wpCore.version}</p>
                        </div>
                        {#if confirmUpdateCore}
                          <div class="flex items-center gap-2">
                            <button class="h-8 px-3 rounded-lg bg-primary text-primary-foreground text-xs font-medium hover:bg-primary/90 disabled:opacity-50" disabled={coreUpdating} on:click={updateCore}>
                              {coreUpdating ? 'Updating…' : 'Confirm'}
                            </button>
                            <button class="h-8 px-3 rounded-lg border border-border text-xs text-muted-foreground hover:bg-muted" on:click={() => confirmUpdateCore = false}>Cancel</button>
                          </div>
                        {:else}
                          <button class="h-8 px-4 rounded-lg bg-blue-500 text-white text-xs font-medium hover:bg-blue-600 transition-colors" on:click={() => confirmUpdateCore = true}>
                            Update Core
                          </button>
                        {/if}
                      </div>
                    {/if}

                    {#if updateAllResult}
                      <div class="p-4 rounded-xl border border-border bg-muted/20">
                        <p class="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">Last Update Summary</p>
                        <div class="flex flex-wrap gap-4 text-sm">
                          <span class="flex items-center gap-1.5"><span class="w-2 h-2 rounded-full {updateAllResult.core ? 'bg-green-400' : 'bg-muted-foreground'}"></span>Core: {updateAllResult.core ? 'Updated' : 'Already current'}</span>
                          <span class="flex items-center gap-1.5"><span class="w-2 h-2 rounded-full bg-blue-400"></span>{updateAllResult.plugins} plugins</span>
                          <span class="flex items-center gap-1.5"><span class="w-2 h-2 rounded-full bg-purple-400"></span>{updateAllResult.themes} themes</span>
                        </div>
                        {#if updateAllResult.errors.length > 0}
                          <div class="mt-3 p-3 bg-red-500/10 border border-red-500/20 rounded-lg">
                            {#each updateAllResult.errors as err}<p class="text-xs text-red-300">{err}</p>{/each}
                          </div>
                        {/if}
                      </div>
                    {/if}

                  <!-- PLUGINS section -->
                  {:else if wpSection === 'plugins'}
                    <div class="flex items-center justify-between mb-2">
                      <p class="text-xs text-muted-foreground">{wpPlugins.filter(p => p.status === 'active').length} active · {wpPlugins.filter(p => p.update_available).length} with updates</p>
                      <button class="h-8 px-3 rounded-lg bg-primary text-primary-foreground text-xs font-medium hover:bg-primary/90 transition-colors" on:click={() => { installSlug = ''; installError = ''; showInstallPlugin = true; }}>
                        + Install Plugin
                      </button>
                    </div>
                    {#if wpPlugins.length === 0}
                      <p class="text-center py-8 text-sm text-muted-foreground">No plugins installed</p>
                    {:else}
                      <div class="border border-border rounded-xl overflow-hidden">
                        <table class="w-full text-sm">
                          <tbody class="divide-y divide-border">
                            {#each wpPlugins as plugin (plugin.slug)}
                              <tr class="hover:bg-muted/10 transition-colors">
                                <td class="px-4 py-3">
                                  <span class="font-medium text-foreground">{plugin.name}</span>
                                  {#if plugin.update_available && plugin.update_version}
                                    <span class="ml-2 text-xs text-blue-400">→ {plugin.update_version}</span>
                                  {/if}
                                  <span class="block text-xs text-muted-foreground font-mono">{plugin.slug} · v{plugin.version}</span>
                                </td>
                                <td class="px-4 py-3 w-28">
                                  <span class="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium {plugin.status === 'active' ? 'bg-green-500/10 text-green-400 border border-green-500/20' : 'bg-muted text-muted-foreground border border-border'}">
                                    <span class="w-1.5 h-1.5 rounded-full {plugin.status === 'active' ? 'bg-green-400' : 'bg-muted-foreground'}"></span>
                                    {plugin.status === 'active' ? 'Active' : 'Inactive'}
                                  </span>
                                </td>
                                <td class="px-4 py-3 w-40">
                                  <div class="flex items-center gap-1">
                                    {#if plugin.status === 'active'}
                                      <button class="h-7 px-2.5 rounded border border-border text-xs text-muted-foreground hover:bg-muted disabled:opacity-40" disabled={pluginBusy[plugin.slug]} on:click={() => deactivatePlugin(plugin.slug)}>{pluginBusy[plugin.slug] ? '…' : 'Deactivate'}</button>
                                    {:else}
                                      <button class="h-7 px-2.5 rounded border border-border text-xs text-muted-foreground hover:bg-muted disabled:opacity-40" disabled={pluginBusy[plugin.slug]} on:click={() => activatePlugin(plugin.slug)}>{pluginBusy[plugin.slug] ? '…' : 'Activate'}</button>
                                    {/if}
                                    <button title="Delete" class="h-7 w-7 flex items-center justify-center rounded text-red-400 hover:bg-red-500/10 disabled:opacity-40" disabled={pluginBusy[plugin.slug]} on:click={() => deletePlugin(plugin.slug, plugin.name)}>
                                      <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"/></svg>
                                    </button>
                                    {#if confirmDeletePlugin?.slug === plugin.slug}
                                      <div class="flex items-center gap-1 px-2 py-1 rounded-lg bg-red-500/10 border border-red-500/30">
                                        <span class="text-[10px] text-red-300">Delete?</span>
                                        <button class="h-5 px-1.5 rounded bg-red-500 text-white text-[10px]" disabled={pluginBusy[plugin.slug]} on:click={() => deletePlugin(plugin.slug, plugin.name)}>Yes</button>
                                        <button class="h-5 px-1.5 rounded border border-border text-[10px] text-muted-foreground" on:click={() => confirmDeletePlugin = null}>No</button>
                                      </div>
                                    {/if}
                                  </div>
                                </td>
                              </tr>
                            {/each}
                          </tbody>
                        </table>
                      </div>
                    {/if}

                  <!-- THEMES section -->
                  {:else if wpSection === 'themes'}
                    {#if wpThemes.length === 0}
                      <p class="text-center py-8 text-sm text-muted-foreground">No themes installed</p>
                    {:else}
                      <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3">
                        {#each wpThemes as theme (theme.slug)}
                          <div class="border border-border rounded-xl p-4 {theme.status === 'active' ? 'ring-1 ring-primary/30 border-primary/30' : ''}">
                            <div class="flex items-start justify-between mb-2">
                              <div>
                                <p class="font-medium text-sm text-foreground">{theme.name}</p>
                                <p class="text-xs text-muted-foreground font-mono">{theme.slug} · v{theme.version}</p>
                              </div>
                              {#if theme.status === 'active'}
                                <span class="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium bg-primary/10 text-primary border border-primary/20">Active</span>
                              {/if}
                            </div>
                            <div class="flex gap-2 mt-3">
                              {#if theme.status !== 'active'}
                                <button class="h-7 px-3 rounded border border-border text-xs text-muted-foreground hover:bg-muted disabled:opacity-40 flex-1" disabled={themeBusy[theme.slug]} on:click={() => activateTheme(theme.slug)}>{themeBusy[theme.slug] ? '…' : 'Activate'}</button>
                                <button title="Delete" class="h-7 w-7 flex items-center justify-center rounded text-red-400 hover:bg-red-500/10 disabled:opacity-40" disabled={themeBusy[theme.slug]} on:click={() => deleteTheme(theme.slug, theme.name)}>
                                  <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"/></svg>
                                </button>
                                {#if confirmDeleteTheme?.slug === theme.slug}
                                  <div class="flex items-center gap-1 px-2 py-1 rounded-lg bg-red-500/10 border border-red-500/30">
                                    <button class="h-5 px-1.5 rounded bg-red-500 text-white text-[10px]" on:click={() => deleteTheme(theme.slug, theme.name)}>Yes</button>
                                    <button class="h-5 px-1.5 rounded border border-border text-[10px] text-muted-foreground" on:click={() => confirmDeleteTheme = null}>No</button>
                                  </div>
                                {/if}
                              {:else}
                                <span class="text-xs text-muted-foreground italic">Active — cannot delete</span>
                              {/if}
                            </div>
                          </div>
                        {/each}
                      </div>
                    {/if}

                  <!-- USERS section -->
                  {:else if wpSection === 'users'}
                    {#if wpUsers.length === 0}
                      <p class="text-center py-8 text-sm text-muted-foreground">No users found</p>
                    {:else}
                      <div class="border border-border rounded-xl overflow-hidden">
                        <table class="w-full text-sm">
                          <tbody class="divide-y divide-border">
                            {#each wpUsers as user (user.id)}
                              <tr class="hover:bg-muted/10 transition-colors">
                                <td class="px-4 py-3">
                                  <span class="font-mono font-medium text-foreground text-sm">{user.login}</span>
                                  {#if user.display_name && user.display_name !== user.login}
                                    <span class="block text-xs text-muted-foreground">{user.display_name}</span>
                                  {/if}
                                </td>
                                <td class="px-4 py-3 text-xs text-muted-foreground">{user.email}</td>
                                <td class="px-4 py-3">
                                  <span class="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-primary/10 text-primary border border-primary/20 capitalize">{user.role}</span>
                                </td>
                                <td class="px-4 py-3 text-right">
                                  <button
                                    class="h-7 px-3 rounded-lg border border-border text-xs font-medium text-muted-foreground hover:bg-muted transition-colors inline-flex items-center gap-1.5 disabled:opacity-40"
                                    disabled={loginUrlLoading === user.id}
                                    on:click={() => getLoginUrl(user)}
                                  >
                                    {#if loginUrlLoading === user.id}
                                      <svg class="w-3 h-3 animate-spin" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"/></svg>
                                    {/if}
                                    Login URL
                                  </button>
                                </td>
                              </tr>
                            {/each}
                          </tbody>
                        </table>
                      </div>
                    {/if}
                  {/if}

                </div>
              {/if}
            </div>
          {/if}

          <!-- Non-WordPress expanded panel -->
          {#if isOpen && !isWp}
            <div class="panel-open border-t border-border p-4">
              <div class="grid grid-cols-2 sm:grid-cols-3 gap-3 text-sm">
                <div class="bg-muted/30 border border-border rounded-xl p-3">
                  <p class="text-xs text-muted-foreground">Domain</p>
                  <p class="font-mono text-foreground mt-0.5">{app.domain}</p>
                </div>
                <div class="bg-muted/30 border border-border rounded-xl p-3">
                  <p class="text-xs text-muted-foreground">Path</p>
                  <p class="font-mono text-foreground mt-0.5">{app.path}</p>
                </div>
                <div class="bg-muted/30 border border-border rounded-xl p-3">
                  <p class="text-xs text-muted-foreground">Version</p>
                  <p class="font-mono text-foreground mt-0.5">{app.version}</p>
                </div>
              </div>
            </div>
          {/if}

        </div>
      {/each}
    </div>
  {/if}

</div>

<!-- ── Install Plugin Modal ────────────────────────────────────────────────── -->
{#if showInstallPlugin}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm" on:click|self={() => showInstallPlugin = false}>
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-md mx-4 shadow-2xl">
      <div class="flex items-center justify-between mb-5">
        <h3 class="text-base font-semibold text-foreground">Install WordPress Plugin</h3>
        <button class="text-muted-foreground hover:text-foreground" on:click={() => showInstallPlugin = false}>
          <svg class="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/></svg>
        </button>
      </div>
      <label class="block text-xs font-medium text-muted-foreground mb-1">Plugin Slug</label>
      <input type="text" bind:value={installSlug} placeholder="e.g. woocommerce" class="w-full h-10 px-3 rounded-lg border border-border bg-background text-sm font-mono text-foreground focus:outline-none focus:ring-2 focus:ring-ring mb-1"/>
      <p class="text-xs text-muted-foreground mb-4">The slug from wordpress.org/plugins/&lt;slug&gt;</p>
      {#if installError}<p class="text-sm text-red-400 mb-3">{installError}</p>{/if}
      <div class="flex justify-end gap-2">
        <button class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted" on:click={() => showInstallPlugin = false}>Cancel</button>
        <button class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 disabled:opacity-50" disabled={installing || !installSlug.trim()} on:click={handleInstallPlugin}>{installing ? 'Installing…' : 'Install'}</button>
      </div>
    </div>
  </div>
{/if}

<!-- ── Login URL Modal ────────────────────────────────────────────────────── -->
{#if showLoginUrl}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm" on:click|self={() => showLoginUrl = null}>
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-md mx-4 shadow-2xl">
      <div class="flex items-center justify-between mb-4">
        <h3 class="text-base font-semibold text-foreground">Login URL — <span class="font-mono">{showLoginUrl.user.login}</span></h3>
        <button class="text-muted-foreground hover:text-foreground" on:click={() => showLoginUrl = null}>
          <svg class="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/></svg>
        </button>
      </div>
      <p class="text-xs text-muted-foreground mb-3">One-time admin login link. Keep it safe — it grants direct dashboard access.</p>
      <div class="bg-muted/30 border border-border rounded-lg px-3 py-2 mb-4">
        <p class="font-mono text-xs text-foreground break-all">{showLoginUrl.url}</p>
      </div>
      <div class="flex justify-end gap-2">
        <button class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted" on:click={() => showLoginUrl = null}>Close</button>
        <button class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-1.5" on:click={() => copyToClipboard(showLoginUrl!.url)}>
          <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z"/></svg>
          Copy
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- ── Toasts ─────────────────────────────────────────────────────────────── -->
<div class="fixed bottom-5 right-5 z-[60] flex flex-col gap-2 pointer-events-none">
  {#each toasts as toast (toast.id)}
    <div class="pointer-events-auto flex items-center gap-3 px-4 py-3 rounded-xl shadow-lg border text-sm font-medium {toast.type === 'success' ? 'bg-card border-green-500/30 text-foreground' : 'bg-card border-red-500/30 text-foreground'}">
      {#if toast.type === 'success'}
        <svg class="w-4 h-4 text-green-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/></svg>
      {:else}
        <svg class="w-4 h-4 text-red-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/></svg>
      {/if}
      <span>{toast.message}</span>
    </div>
  {/each}
</div>
