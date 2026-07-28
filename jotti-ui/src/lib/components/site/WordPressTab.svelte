<script lang="ts">
  import { onMount } from 'svelte';
  import { get } from 'svelte/store';
  import { auth } from '$lib/stores/auth';
  import type { Site } from '$api/client';

  // ── Props ──────────────────────────────────────────────────────────────────
  export let siteId: string;
  export let site: Site;

  // ── Local types ────────────────────────────────────────────────────────────
  interface WpCore {
    version: string;
    update_available: boolean;
    update_version: string | null;
  }

  interface WpPlugin {
    name: string;
    slug: string;
    version: string;
    status: 'active' | 'inactive';
    update_available: boolean;
    update_version: string | null;
  }

  interface WpTheme {
    name: string;
    slug: string;
    version: string;
    status: 'active' | 'inactive';
  }

  interface WpUser {
    id: number;
    login: string;
    email: string;
    display_name: string;
    role: string;
  }

  interface UpdateAllResult {
    core: boolean;
    plugins: number;
    themes: number;
    errors: string[];
  }

  interface Toast {
    id: number;
    message: string;
    type: 'success' | 'error';
  }

  // ── Auth helper ────────────────────────────────────────────────────────────
  function authH(): Record<string, string> {
    const t = get(auth).token;
    return t
      ? { 'Authorization': 'Bearer ' + t, 'Content-Type': 'application/json' }
      : { 'Content-Type': 'application/json' };
  }

  // ── State ──────────────────────────────────────────────────────────────────
  let wpDetected = true;
  let coreLoading = true;
  let pluginsLoading = true;
  let themesLoading = true;
  let usersLoading = true;

  let core: WpCore | null = null;
  let plugins: WpPlugin[] = [];
  let themes: WpTheme[] = [];
  let users: WpUser[] = [];

  let coreUpdating = false;

  // Plugin actions in-flight
  let pluginBusy: Record<string, boolean> = {};

  // Theme actions in-flight
  let themeBusy: Record<string, boolean> = {};

  // Update All
  let updatingAll = false;
  let updateAllResult: UpdateAllResult | null = null;

  // Inline confirm state (replaces confirm() dialogs)
  let confirmDeletePlugin: { slug: string; name: string } | null = null;
  let confirmDeleteTheme: { slug: string; name: string } | null = null;
  let confirmUpdateCore = false;
  let confirmUpdateAll = false;

  // Install Plugin modal
  let showInstallPlugin = false;
  let installSlug = '';
  let installing = false;
  let installError = '';

  // Login URL modal
  let showLoginUrl: { user: WpUser; url: string } | null = null;
  let loginUrlLoading: number | null = null;

  // Toast
  let toasts: Toast[] = [];
  let toastCounter = 0;

  // ── Helpers ────────────────────────────────────────────────────────────────
  function showToast(message: string, type: 'success' | 'error' = 'success') {
    const id = ++toastCounter;
    toasts = [...toasts, { id, message, type }];
    setTimeout(() => { toasts = toasts.filter(t => t.id !== id); }, 4500);
  }

  async function copyToClipboard(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      showToast('Copied to clipboard');
    } catch {
      showToast('Copy failed', 'error');
    }
  }

  function closeOnEsc(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      showInstallPlugin = false;
      showLoginUrl = null;
    }
  }

  // ── API loaders ────────────────────────────────────────────────────────────
  async function loadCore() {
    coreLoading = true;
    try {
      const res = await fetch(`/api/v1/wordpress/${siteId}/core`, { headers: authH() });
      if (res.status === 404 || res.status === 422) {
        wpDetected = false;
        return;
      }
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      core = await res.json() as WpCore;
      wpDetected = true;
    } catch {
      wpDetected = false;
    } finally {
      coreLoading = false;
    }
  }

  async function loadPlugins() {
    pluginsLoading = true;
    try {
      const res = await fetch(`/api/v1/wordpress/${siteId}/plugins`, { headers: authH() });
      if (res.ok) plugins = await res.json() as WpPlugin[];
    } catch { /* non-fatal */ }
    finally { pluginsLoading = false; }
  }

  async function loadThemes() {
    themesLoading = true;
    try {
      const res = await fetch(`/api/v1/wordpress/${siteId}/themes`, { headers: authH() });
      if (res.ok) themes = await res.json() as WpTheme[];
    } catch { /* non-fatal */ }
    finally { themesLoading = false; }
  }

  async function loadUsers() {
    usersLoading = true;
    try {
      const res = await fetch(`/api/v1/wordpress/${siteId}/users`, { headers: authH() });
      if (res.ok) users = await res.json() as WpUser[];
    } catch { /* non-fatal */ }
    finally { usersLoading = false; }
  }

  // ── Core actions ───────────────────────────────────────────────────────────
  async function updateCore() {
    confirmUpdateCore = false;
    coreUpdating = true;
    try {
      const res = await fetch(`/api/v1/wordpress/${siteId}/core/update`, {
        method: 'POST',
        headers: authH(),
      });
      if (!res.ok) {
        const err = await res.json() as { message?: string };
        throw new Error(err.message ?? `HTTP ${res.status}`);
      }
      showToast('WordPress core updated successfully');
      await loadCore();
    } catch (err: unknown) {
      showToast((err as { message?: string }).message ?? 'Update failed', 'error');
    } finally {
      coreUpdating = false;
    }
  }

  // ── Plugin actions ─────────────────────────────────────────────────────────
  function openInstallPlugin() {
    installSlug = '';
    installError = '';
    showInstallPlugin = true;
  }

  async function handleInstallPlugin() {
    installError = '';
    if (!installSlug.trim()) {
      installError = 'Plugin slug is required';
      return;
    }
    installing = true;
    try {
      const res = await fetch(`/api/v1/wordpress/${siteId}/plugins`, {
        method: 'POST',
        headers: authH(),
        body: JSON.stringify({ slug: installSlug.trim() }),
      });
      if (!res.ok) {
        const err = await res.json() as { message?: string };
        throw new Error(err.message ?? `HTTP ${res.status}`);
      }
      showInstallPlugin = false;
      showToast(`Plugin "${installSlug.trim()}" installed`);
      await loadPlugins();
    } catch (err: unknown) {
      installError = (err as { message?: string }).message ?? 'Installation failed';
    } finally {
      installing = false;
    }
  }

  async function activatePlugin(slug: string) {
    pluginBusy = { ...pluginBusy, [slug]: true };
    try {
      const res = await fetch(`/api/v1/wordpress/${siteId}/plugins/${encodeURIComponent(slug)}/activate`, {
        method: 'POST',
        headers: authH(),
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      plugins = plugins.map(p => p.slug === slug ? { ...p, status: 'active' } : p);
      showToast('Plugin activated');
    } catch {
      showToast('Failed to activate plugin', 'error');
    } finally {
      pluginBusy = { ...pluginBusy, [slug]: false };
    }
  }

  async function deactivatePlugin(slug: string) {
    pluginBusy = { ...pluginBusy, [slug]: true };
    try {
      const res = await fetch(`/api/v1/wordpress/${siteId}/plugins/${encodeURIComponent(slug)}/deactivate`, {
        method: 'POST',
        headers: authH(),
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      plugins = plugins.map(p => p.slug === slug ? { ...p, status: 'inactive' } : p);
      showToast('Plugin deactivated');
    } catch {
      showToast('Failed to deactivate plugin', 'error');
    } finally {
      pluginBusy = { ...pluginBusy, [slug]: false };
    }
  }

  async function deletePlugin(slug: string, name: string) {
    if (!confirmDeletePlugin || confirmDeletePlugin.slug !== slug) {
      confirmDeletePlugin = { slug, name };
      return;
    }
    confirmDeletePlugin = null;
    pluginBusy = { ...pluginBusy, [slug]: true };
    try {
      const res = await fetch(`/api/v1/wordpress/${siteId}/plugins/${encodeURIComponent(slug)}`, {
        method: 'DELETE',
        headers: authH(),
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      plugins = plugins.filter(p => p.slug !== slug);
      showToast('Plugin deleted');
    } catch {
      showToast('Failed to delete plugin', 'error');
    } finally {
      pluginBusy = { ...pluginBusy, [slug]: false };
    }
  }

  // ── Theme actions ──────────────────────────────────────────────────────────
  async function activateTheme(slug: string) {
    themeBusy = { ...themeBusy, [slug]: true };
    try {
      const res = await fetch(`/api/v1/wordpress/${siteId}/themes/${encodeURIComponent(slug)}/activate`, {
        method: 'POST',
        headers: authH(),
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      themes = themes.map(t => ({ ...t, status: t.slug === slug ? 'active' : 'inactive' }));
      showToast('Theme activated');
    } catch {
      showToast('Failed to activate theme', 'error');
    } finally {
      themeBusy = { ...themeBusy, [slug]: false };
    }
  }

  async function deleteTheme(slug: string, name: string) {
    if (!confirmDeleteTheme || confirmDeleteTheme.slug !== slug) {
      confirmDeleteTheme = { slug, name };
      return;
    }
    confirmDeleteTheme = null;
    themeBusy = { ...themeBusy, [slug]: true };
    try {
      const res = await fetch(`/api/v1/wordpress/${siteId}/themes/${encodeURIComponent(slug)}`, {
        method: 'DELETE',
        headers: authH(),
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      themes = themes.filter(t => t.slug !== slug);
      showToast('Theme deleted');
    } catch {
      showToast('Failed to delete theme', 'error');
    } finally {
      themeBusy = { ...themeBusy, [slug]: false };
    }
  }

  // ── User actions ───────────────────────────────────────────────────────────
  async function getLoginUrl(user: WpUser) {
    loginUrlLoading = user.id;
    try {
      const res = await fetch(`/api/v1/wordpress/${siteId}/users/${user.id}/login-url`, {
        method: 'POST',
        headers: authH(),
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data = await res.json() as { login_url: string };
      showLoginUrl = { user, url: data.login_url };
    } catch {
      showToast('Failed to get login URL', 'error');
    } finally {
      loginUrlLoading = null;
    }
  }

  // ── Update All ─────────────────────────────────────────────────────────────
  async function runUpdateAll() {
    if (!confirmUpdateAll) { confirmUpdateAll = true; return; }
    confirmUpdateAll = false;
    updatingAll = true;
    updateAllResult = null;
    try {
      const res = await fetch(`/api/v1/wordpress/${siteId}/update-all`, {
        method: 'POST',
        headers: authH(),
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      updateAllResult = await res.json() as UpdateAllResult;
      showToast('Update All completed');
      // Reload data to reflect changes
      await Promise.all([loadCore(), loadPlugins(), loadThemes()]);
    } catch {
      showToast('Update All failed', 'error');
    } finally {
      updatingAll = false;
    }
  }

  // ── Lifecycle ──────────────────────────────────────────────────────────────
  onMount(async () => {
    await loadCore();
    if (wpDetected) {
      await Promise.all([loadPlugins(), loadThemes(), loadUsers()]);
    }
  });
</script>

<svelte:window on:keydown={closeOnEsc} />

<div class="tab-content space-y-6">

  <!-- ── WordPress Not Detected Banner ──────────────────────────────────────── -->
  {#if !coreLoading && !wpDetected}
    <div class="bg-amber-500/10 border border-amber-500/20 rounded-xl p-5 flex items-start gap-3">
      <svg class="w-5 h-5 text-amber-400 flex-shrink-0 mt-0.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round" d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126zM12 15.75h.007v.008H12v-.008z" />
      </svg>
      <div>
        <p class="text-sm font-semibold text-amber-300">WordPress not detected on this site.</p>
        <p class="text-xs text-amber-400 mt-1">Install WordPress via the Apps tab first, then return here to manage it.</p>
      </div>
    </div>

  {:else}

    <!-- ── Section 1: WordPress Core ────────────────────────────────────────── -->
    <div class="bg-card border border-border rounded-xl p-5">
      <div class="flex items-center justify-between mb-4">
        <div class="flex items-center gap-2.5">
          <div class="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
            <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
              <path stroke-linecap="round" stroke-linejoin="round" d="M12 21a9 9 0 100-18 9 9 0 000 18zm0 0v-9m0 0l-3-3m3 3l3-3" />
            </svg>
          </div>
          <h2 class="text-sm font-semibold text-foreground">WordPress Core</h2>
        </div>
      </div>

      {#if coreLoading}
        <div class="animate-pulse bg-muted rounded h-10"></div>
      {:else if core}
        <div class="flex items-center justify-between flex-wrap gap-3">
          <div class="flex items-center gap-3 flex-wrap">
            <div class="flex items-center gap-2 px-3 py-2 rounded-lg bg-muted/30 border border-border">
              <span class="text-xs text-muted-foreground">Version</span>
              <span class="font-mono text-sm font-semibold text-foreground">{core.version}</span>
            </div>
            {#if core.update_available && core.update_version}
              <span class="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium bg-blue-500/10 text-blue-400 border border-blue-500/20">
                <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5">
                  <path stroke-linecap="round" stroke-linejoin="round" d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-8l-4-4m0 0L8 8m4-4v12" />
                </svg>
                Update to {core.update_version} available
              </span>
            {:else}
              <span class="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium bg-green-500/10 text-green-400 border border-green-500/20">
                <span class="w-1.5 h-1.5 rounded-full bg-green-400"></span>
                Up to date
              </span>
            {/if}
          </div>
          {#if core.update_available}
            {#if confirmUpdateCore}
              <div class="flex items-center gap-2 px-3 py-2 rounded-lg bg-amber-500/10 border border-amber-500/30">
                <span class="text-xs text-amber-300">Update core now?</span>
                <button class="h-7 px-2.5 rounded bg-primary text-primary-foreground text-xs font-medium hover:bg-primary/90" on:click={updateCore}>Yes, update</button>
                <button class="h-7 px-2.5 rounded border border-border text-xs text-muted-foreground hover:bg-muted" on:click={() => confirmUpdateCore = false}>Cancel</button>
              </div>
            {:else}
            <button
              class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-all active:scale-95 inline-flex items-center gap-1.5 disabled:opacity-50"
              disabled={coreUpdating}
              on:click={() => confirmUpdateCore = true}
            >
              {#if coreUpdating}
                <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
                  <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                  <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
                </svg>
                Updating…
              {:else}
                <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                  <path stroke-linecap="round" stroke-linejoin="round" d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-8l-4-4m0 0L8 8m4-4v12" />
                </svg>
                Update Core
              {/if}
            </button>
            {/if}
          {/if}
        </div>
      {/if}
    </div>


    <!-- ── Section 2: Plugins ────────────────────────────────────────────────── -->
    <div class="bg-card border border-border rounded-xl p-5">
      <div class="flex items-center justify-between mb-4">
        <div class="flex items-center gap-2.5">
          <div class="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
            <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
              <path stroke-linecap="round" stroke-linejoin="round" d="M14.25 6.087c0-.355.186-.676.401-.959.221-.29.349-.634.349-1.003 0-1.036-1.007-1.875-2.25-1.875s-2.25.84-2.25 1.875c0 .369.128.713.349 1.003.215.283.401.604.401.959v0a.64.64 0 01-.657.643 48.39 48.39 0 01-4.163-.3c.186 1.613.293 3.25.315 4.907a.656.656 0 01-.658.663v0c-.355 0-.676-.186-.959-.401a1.647 1.647 0 00-1.003-.349c-1.036 0-1.875 1.007-1.875 2.25s.84 2.25 1.875 2.25c.369 0 .713-.128 1.003-.349.283-.215.604-.401.959-.401v0c.31 0 .555.26.532.57a48.039 48.039 0 01-.642 5.056c1.518.19 3.058.309 4.616.354a.64.64 0 00.657-.643v0c0-.355-.186-.676-.401-.959a1.647 1.647 0 01-.349-1.003c0-1.035 1.008-1.875 2.25-1.875 1.243 0 2.25.84 2.25 1.875 0 .369-.128.713-.349 1.003-.215.283-.4.604-.4.959v0c0 .333.277.599.61.58a48.1 48.1 0 005.427-.63 48.05 48.05 0 00.582-4.717.532.532 0 00-.533-.57v0c-.355 0-.676.186-.959.401-.29.221-.634.349-1.003.349-1.035 0-1.875-1.007-1.875-2.25s.84-2.25 1.875-2.25c.37 0 .713.128 1.003.349.283.215.604.401.959.401v0a.656.656 0 00.658-.663 48.422 48.422 0 00-.37-5.36c-1.886.342-3.81.574-5.766.689a.578.578 0 01-.61-.58v0z" />
            </svg>
          </div>
          <h2 class="text-sm font-semibold text-foreground">Plugins</h2>
          {#if !pluginsLoading}
            <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium bg-muted text-muted-foreground">{plugins.length}</span>
          {/if}
        </div>
        <button
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-all active:scale-95 inline-flex items-center gap-1.5"
          on:click={openInstallPlugin}
        >
          <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5">
            <path stroke-linecap="round" stroke-linejoin="round" d="M12 4.5v15m7.5-7.5h-15"/>
          </svg>
          Install Plugin
        </button>
      </div>

      {#if pluginsLoading}
        <div class="space-y-2">
          {#each [1, 2, 3] as _}
            <div class="animate-pulse bg-muted rounded h-10"></div>
          {/each}
        </div>
      {:else if plugins.length === 0}
        <div class="text-center py-10">
          <svg class="mx-auto mb-3 w-10 h-10 text-muted-foreground/40" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" d="M20.25 7.5l-.625 10.632a2.25 2.25 0 01-2.247 2.118H6.622a2.25 2.25 0 01-2.247-2.118L3.75 7.5M10 11.25h4M3.375 7.5h17.25c.621 0 1.125-.504 1.125-1.125v-1.5c0-.621-.504-1.125-1.125-1.125H3.375c-.621 0-1.125.504-1.125 1.125v1.5c0 .621.504 1.125 1.125 1.125z" />
          </svg>
          <p class="text-muted-foreground font-medium mb-1">No plugins installed</p>
          <p class="text-muted-foreground/60 text-sm mb-4">Install plugins to extend your WordPress site.</p>
          <button
            class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors"
            on:click={openInstallPlugin}
          >
            Install your first plugin
          </button>
        </div>
      {:else}
        <div class="border border-border rounded-xl overflow-hidden">
          <table class="w-full text-sm">
            <thead>
              <tr class="bg-muted/30">
                <th class="text-left px-4 py-3 text-xs font-medium text-muted-foreground uppercase tracking-wider">Name</th>
                <th class="text-left px-4 py-3 text-xs font-medium text-muted-foreground uppercase tracking-wider">Version</th>
                <th class="text-left px-4 py-3 text-xs font-medium text-muted-foreground uppercase tracking-wider">Status</th>
                <th class="text-right px-4 py-3 text-xs font-medium text-muted-foreground uppercase tracking-wider">Actions</th>
              </tr>
            </thead>
            <tbody class="divide-y divide-border">
              {#each plugins as plugin (plugin.slug)}
                <tr class="hover:bg-muted/10 transition-colors">
                  <td class="px-4 py-3">
                    <div>
                      <span class="font-medium text-foreground text-sm">{plugin.name}</span>
                      <span class="block text-xs text-muted-foreground font-mono">{plugin.slug}</span>
                    </div>
                  </td>
                  <td class="px-4 py-3 text-sm text-muted-foreground">
                    <span>{plugin.version}</span>
                    {#if plugin.update_available && plugin.update_version}
                      <span class="ml-2 inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-xs font-medium bg-blue-500/10 text-blue-400 border border-blue-500/20">
                        → {plugin.update_version}
                      </span>
                    {/if}
                  </td>
                  <td class="px-4 py-3">
                    <span class="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-xs font-medium
                      {plugin.status === 'active'
                        ? 'bg-green-500/10 text-green-400 border border-green-500/20'
                        : 'bg-muted text-muted-foreground border border-border'}">
                      <span class="w-1.5 h-1.5 rounded-full {plugin.status === 'active' ? 'bg-green-400' : 'bg-muted-foreground'}"></span>
                      {plugin.status === 'active' ? 'Active' : 'Inactive'}
                    </span>
                  </td>
                  <td class="px-4 py-3">
                    <div class="flex items-center justify-end gap-1">
                      {#if plugin.status === 'active'}
                        <button
                          class="h-8 px-3 rounded-lg border border-border text-xs font-medium text-muted-foreground hover:bg-muted transition-colors disabled:opacity-40"
                          disabled={pluginBusy[plugin.slug]}
                          on:click={() => deactivatePlugin(plugin.slug)}
                        >
                          {pluginBusy[plugin.slug] ? '…' : 'Deactivate'}
                        </button>
                      {:else}
                        <button
                          class="h-8 px-3 rounded-lg border border-border text-xs font-medium text-muted-foreground hover:bg-muted transition-colors disabled:opacity-40"
                          disabled={pluginBusy[plugin.slug]}
                          on:click={() => activatePlugin(plugin.slug)}
                        >
                          {pluginBusy[plugin.slug] ? '…' : 'Activate'}
                        </button>
                      {/if}
                      <button
                        title="Delete plugin"
                        class="h-8 w-8 flex items-center justify-center rounded-lg text-red-400 hover:bg-red-500/10 transition-colors disabled:opacity-40"
                        disabled={pluginBusy[plugin.slug]}
                        on:click={() => deletePlugin(plugin.slug, plugin.name)}
                      >
                        <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                            d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
                        </svg>
                      </button>
                      {#if confirmDeletePlugin?.slug === plugin.slug}
                        <div class="flex items-center gap-1 ml-1 px-2 py-1 rounded-lg bg-red-500/10 border border-red-500/30">
                          <span class="text-[10px] text-red-300">Delete?</span>
                          <button class="h-6 px-2 rounded bg-red-500 text-white text-[10px] font-medium hover:bg-red-600 disabled:opacity-40" disabled={pluginBusy[plugin.slug]} on:click={() => deletePlugin(plugin.slug, plugin.name)}>Yes</button>
                          <button class="h-6 px-2 rounded border border-border text-[10px] text-muted-foreground hover:bg-muted" on:click={() => confirmDeletePlugin = null}>No</button>
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
    </div>


    <!-- ── Section 3: Themes ─────────────────────────────────────────────────── -->
    <div class="bg-card border border-border rounded-xl p-5">
      <div class="flex items-center gap-2.5 mb-4">
        <div class="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
          <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
            <path stroke-linecap="round" stroke-linejoin="round" d="M4.098 19.902a3.75 3.75 0 005.304 0l6.401-6.402M6.75 21A3.75 3.75 0 013 17.25V4.125C3 3.504 3.504 3 4.125 3h5.25c.621 0 1.125.504 1.125 1.125v4.072M6.75 21a3.75 3.75 0 003.75-3.75V8.197M6.75 21h13.125c.621 0 1.125-.504 1.125-1.125v-5.25c0-.621-.504-1.125-1.125-1.125h-4.072M10.5 8.197l2.88-2.88c.438-.439 1.15-.439 1.59 0l3.712 3.713c.44.44.44 1.152 0 1.59l-2.879 2.88M6.75 17.25h.008v.008H6.75v-.008z" />
          </svg>
        </div>
        <h2 class="text-sm font-semibold text-foreground">Themes</h2>
        {#if !themesLoading}
          <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium bg-muted text-muted-foreground">{themes.length}</span>
        {/if}
      </div>

      {#if themesLoading}
        <div class="space-y-2">
          {#each [1, 2] as _}
            <div class="animate-pulse bg-muted rounded h-10"></div>
          {/each}
        </div>
      {:else if themes.length === 0}
        <div class="text-center py-10">
          <svg class="mx-auto mb-3 w-10 h-10 text-muted-foreground/40" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" d="M9.53 16.122a3 3 0 00-5.78 1.128 2.25 2.25 0 01-2.4 2.245 4.5 4.5 0 008.4-2.245c0-.399-.078-.78-.22-1.128zm0 0a15.998 15.998 0 003.388-1.62m-5.043-.025a15.994 15.994 0 011.622-3.395m3.42 3.42a15.995 15.995 0 004.764-4.648l3.876-5.814a1.151 1.151 0 00-1.597-1.597L14.146 6.32a15.996 15.996 0 00-4.649 4.763m3.42 3.42a6.776 6.776 0 00-3.42-3.42" />
          </svg>
          <p class="text-muted-foreground font-medium mb-1">No themes installed</p>
          <p class="text-muted-foreground/60 text-sm">Themes can be installed via the WordPress admin dashboard.</p>
        </div>
      {:else}
        <div class="border border-border rounded-xl overflow-hidden">
          <table class="w-full text-sm">
            <thead>
              <tr class="bg-muted/30">
                <th class="text-left px-4 py-3 text-xs font-medium text-muted-foreground uppercase tracking-wider">Name</th>
                <th class="text-left px-4 py-3 text-xs font-medium text-muted-foreground uppercase tracking-wider">Version</th>
                <th class="text-left px-4 py-3 text-xs font-medium text-muted-foreground uppercase tracking-wider">Status</th>
                <th class="text-right px-4 py-3 text-xs font-medium text-muted-foreground uppercase tracking-wider">Actions</th>
              </tr>
            </thead>
            <tbody class="divide-y divide-border">
              {#each themes as theme (theme.slug)}
                <tr class="hover:bg-muted/10 transition-colors">
                  <td class="px-4 py-3">
                    <div>
                      <span class="font-medium text-foreground text-sm">{theme.name}</span>
                      <span class="block text-xs text-muted-foreground font-mono">{theme.slug}</span>
                    </div>
                  </td>
                  <td class="px-4 py-3 text-sm text-muted-foreground">{theme.version}</td>
                  <td class="px-4 py-3">
                    <span class="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-xs font-medium
                      {theme.status === 'active'
                        ? 'bg-green-500/10 text-green-400 border border-green-500/20'
                        : 'bg-muted text-muted-foreground border border-border'}">
                      <span class="w-1.5 h-1.5 rounded-full {theme.status === 'active' ? 'bg-green-400' : 'bg-muted-foreground'}"></span>
                      {theme.status === 'active' ? 'Active' : 'Inactive'}
                    </span>
                  </td>
                  <td class="px-4 py-3">
                    <div class="flex items-center justify-end gap-1">
                      {#if theme.status !== 'active'}
                        <button
                          class="h-8 px-3 rounded-lg border border-border text-xs font-medium text-muted-foreground hover:bg-muted transition-colors disabled:opacity-40"
                          disabled={themeBusy[theme.slug]}
                          on:click={() => activateTheme(theme.slug)}
                        >
                          {themeBusy[theme.slug] ? '…' : 'Activate'}
                        </button>
                        <button
                          title="Delete theme"
                          class="h-8 w-8 flex items-center justify-center rounded-lg text-red-400 hover:bg-red-500/10 transition-colors disabled:opacity-40"
                          disabled={themeBusy[theme.slug]}
                          on:click={() => deleteTheme(theme.slug, theme.name)}
                        >
                          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                              d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
                          </svg>
                        </button>
                        {#if confirmDeleteTheme?.slug === theme.slug}
                          <div class="flex items-center gap-1 ml-1 px-2 py-1 rounded-lg bg-red-500/10 border border-red-500/30">
                            <span class="text-[10px] text-red-300">Delete?</span>
                            <button class="h-6 px-2 rounded bg-red-500 text-white text-[10px] font-medium hover:bg-red-600 disabled:opacity-40" disabled={themeBusy[theme.slug]} on:click={() => deleteTheme(theme.slug, theme.name)}>Yes</button>
                            <button class="h-6 px-2 rounded border border-border text-[10px] text-muted-foreground hover:bg-muted" on:click={() => confirmDeleteTheme = null}>No</button>
                          </div>
                        {/if}
                      {:else}
                        <span class="text-xs text-muted-foreground px-3">Active theme</span>
                      {/if}
                    </div>
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </div>


    <!-- ── Section 4: WordPress Users ───────────────────────────────────────── -->
    <div class="bg-card border border-border rounded-xl p-5">
      <div class="flex items-center gap-2.5 mb-4">
        <div class="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
          <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
            <path stroke-linecap="round" stroke-linejoin="round" d="M15.75 6a3.75 3.75 0 11-7.5 0 3.75 3.75 0 017.5 0zM4.501 20.118a7.5 7.5 0 0114.998 0A17.933 17.933 0 0112 21.75c-2.676 0-5.216-.584-7.499-1.632z"/>
          </svg>
        </div>
        <h2 class="text-sm font-semibold text-foreground">WordPress Users</h2>
        {#if !usersLoading}
          <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium bg-muted text-muted-foreground">{users.length}</span>
        {/if}
      </div>

      {#if usersLoading}
        <div class="space-y-2">
          {#each [1, 2] as _}
            <div class="animate-pulse bg-muted rounded h-10"></div>
          {/each}
        </div>
      {:else if users.length === 0}
        <div class="text-center py-10">
          <svg class="mx-auto mb-3 w-10 h-10 text-muted-foreground/40" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" d="M18 18.72a9.094 9.094 0 003.741-.479 3 3 0 00-4.682-2.72m.94 3.198l.001.031c0 .225-.012.447-.037.666A11.944 11.944 0 0112 21c-2.17 0-4.207-.576-5.963-1.584A6.062 6.062 0 016 18.719m12 0a5.971 5.971 0 00-.941-3.197m0 0A5.995 5.995 0 0012 12.75a5.995 5.995 0 00-5.058 2.772m0 0a3 3 0 00-4.681 2.72 8.986 8.986 0 003.74.477m.94-3.197a5.971 5.971 0 00-.94 3.197M15 6.75a3 3 0 11-6 0 3 3 0 016 0zm6 3a2.25 2.25 0 11-4.5 0 2.25 2.25 0 014.5 0zm-13.5 0a2.25 2.25 0 11-4.5 0 2.25 2.25 0 014.5 0z" />
          </svg>
          <p class="text-muted-foreground font-medium mb-1">No WordPress users found</p>
          <p class="text-muted-foreground/60 text-sm">Users can be created in the WordPress admin dashboard.</p>
        </div>
      {:else}
        <div class="border border-border rounded-xl overflow-hidden">
          <table class="w-full text-sm">
            <thead>
              <tr class="bg-muted/30">
                <th class="text-left px-4 py-3 text-xs font-medium text-muted-foreground uppercase tracking-wider">Login</th>
                <th class="text-left px-4 py-3 text-xs font-medium text-muted-foreground uppercase tracking-wider">Email</th>
                <th class="text-left px-4 py-3 text-xs font-medium text-muted-foreground uppercase tracking-wider">Role</th>
                <th class="text-right px-4 py-3 text-xs font-medium text-muted-foreground uppercase tracking-wider">Actions</th>
              </tr>
            </thead>
            <tbody class="divide-y divide-border">
              {#each users as user (user.id)}
                <tr class="hover:bg-muted/10 transition-colors">
                  <td class="px-4 py-3">
                    <div>
                      <span class="font-medium text-foreground font-mono text-sm">{user.login}</span>
                      {#if user.display_name && user.display_name !== user.login}
                        <span class="block text-xs text-muted-foreground">{user.display_name}</span>
                      {/if}
                    </div>
                  </td>
                  <td class="px-4 py-3 text-xs text-muted-foreground">{user.email}</td>
                  <td class="px-4 py-3">
                    <span class="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-primary/10 text-primary border border-primary/20 capitalize">
                      {user.role}
                    </span>
                  </td>
                  <td class="px-4 py-3">
                    <div class="flex justify-end">
                      <button
                        class="h-8 px-3 rounded-lg border border-border text-xs font-medium text-muted-foreground hover:bg-muted transition-colors inline-flex items-center gap-1.5 disabled:opacity-40"
                        disabled={loginUrlLoading === user.id}
                        on:click={() => getLoginUrl(user)}
                      >
                        {#if loginUrlLoading === user.id}
                          <svg class="w-3 h-3 animate-spin" fill="none" viewBox="0 0 24 24">
                            <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                            <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
                          </svg>
                          Getting URL…
                        {:else}
                          <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                            <path stroke-linecap="round" stroke-linejoin="round" d="M13.19 8.688a4.5 4.5 0 011.242 7.244l-4.5 4.5a4.5 4.5 0 01-6.364-6.364l1.757-1.757m13.35-.622l1.757-1.757a4.5 4.5 0 00-6.364-6.364l-4.5 4.5a4.5 4.5 0 001.242 7.244" />
                          </svg>
                          Get Login URL
                        {/if}
                      </button>
                    </div>
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </div>


    <!-- ── Section 5: Update All ─────────────────────────────────────────────── -->
    <div class="bg-card border border-border rounded-xl p-5">
      <div class="flex items-center justify-between flex-wrap gap-4">
        <div class="flex items-center gap-2.5">
          <div class="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
            <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
              <path stroke-linecap="round" stroke-linejoin="round" d="M16.023 9.348h4.992v-.001M2.985 19.644v-4.992m0 0h4.992m-4.993 0l3.181 3.183a8.25 8.25 0 0013.803-3.7M4.031 9.865a8.25 8.25 0 0113.803-3.7l3.181 3.182m0-4.991v4.99" />
            </svg>
          </div>
          <div>
            <h2 class="text-sm font-semibold text-foreground">Update All</h2>
            <p class="text-xs text-muted-foreground">Update WordPress core, all plugins, and all themes at once.</p>
          </div>
        </div>
        <button
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-all active:scale-95 inline-flex items-center gap-1.5 disabled:opacity-50"
          disabled={updatingAll}
          on:click={runUpdateAll}
        >
          {#if updatingAll}
            <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
              <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
              <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
            </svg>
            Updating…
          {:else}
            <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M16.023 9.348h4.992v-.001M2.985 19.644v-4.992m0 0h4.992m-4.993 0l3.181 3.183a8.25 8.25 0 0013.803-3.7M4.031 9.865a8.25 8.25 0 0113.803-3.7l3.181 3.182m0-4.991v4.99" />
            </svg>
            Update All
          {/if}
        </button>
        {#if confirmUpdateAll}
          <div class="flex items-center gap-2 px-3 py-2 rounded-lg bg-amber-500/10 border border-amber-500/30">
            <span class="text-xs text-amber-300">Update all now? This may take a few minutes.</span>
            <button class="h-7 px-2.5 rounded bg-primary text-primary-foreground text-xs font-medium" on:click={runUpdateAll}>Confirm</button>
            <button class="h-7 px-2.5 rounded border border-border text-xs text-muted-foreground hover:bg-muted" on:click={() => confirmUpdateAll = false}>Cancel</button>
          </div>
        {/if}
      </div>

      {#if updateAllResult}
        <div class="mt-4 p-4 rounded-xl border border-border bg-muted/20 space-y-2">
          <p class="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">Update Summary</p>
          <div class="flex flex-wrap gap-3">
            <div class="flex items-center gap-1.5 text-sm">
              <span class="w-2 h-2 rounded-full {updateAllResult.core ? 'bg-green-400' : 'bg-muted-foreground'}"></span>
              <span class="text-foreground">Core: <span class="font-medium">{updateAllResult.core ? 'Updated' : 'Already up to date'}</span></span>
            </div>
            <div class="flex items-center gap-1.5 text-sm">
              <span class="w-2 h-2 rounded-full bg-blue-400"></span>
              <span class="text-foreground">Plugins updated: <span class="font-medium">{updateAllResult.plugins}</span></span>
            </div>
            <div class="flex items-center gap-1.5 text-sm">
              <span class="w-2 h-2 rounded-full bg-purple-400"></span>
              <span class="text-foreground">Themes updated: <span class="font-medium">{updateAllResult.themes}</span></span>
            </div>
          </div>
          {#if updateAllResult.errors && updateAllResult.errors.length > 0}
            <div class="mt-2 p-3 bg-red-500/10 border border-red-500/20 rounded-lg">
              <p class="text-xs font-medium text-red-400 mb-1">Errors ({updateAllResult.errors.length})</p>
              <ul class="space-y-0.5">
                {#each updateAllResult.errors as err}
                  <li class="text-xs text-red-300">{err}</li>
                {/each}
              </ul>
            </div>
          {/if}
        </div>
      {/if}
    </div>

  {/if}
</div>


<style>
  @keyframes fadeUp { from { opacity:0; transform:translateY(4px) } to { opacity:1; transform:none } }
  .tab-content { animation: fadeUp 0.2s ease-out both }
</style>


<!-- ── Install Plugin Modal ─────────────────────────────────────────────────── -->
{#if showInstallPlugin}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
    on:click|self={() => { showInstallPlugin = false; }}
  >
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-md mx-4 shadow-2xl">
      <div class="flex items-center justify-between mb-5">
        <h3 class="text-base font-semibold text-foreground">Install Plugin</h3>
        <button class="text-muted-foreground hover:text-foreground transition-colors" on:click={() => { showInstallPlugin = false; }}>
          <svg class="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      <div class="mb-5">
        <label class="block text-xs font-medium text-muted-foreground mb-1">Plugin Slug</label>
        <input
          type="text"
          bind:value={installSlug}
          placeholder="e.g. woocommerce"
          class="w-full h-10 px-3 rounded-lg border border-border bg-background text-sm text-foreground font-mono focus:outline-none focus:ring-2 focus:ring-ring"
        />
        <p class="text-xs text-muted-foreground mt-1">Enter the WordPress.org plugin slug (the part after <code class="font-mono">/plugins/</code> in the URL).</p>
      </div>

      {#if installError}
        <p class="text-sm text-red-400 mb-3">{installError}</p>
      {/if}

      <div class="flex justify-end gap-2">
        <button
          class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted transition-colors"
          on:click={() => { showInstallPlugin = false; }}
        >
          Cancel
        </button>
        <button
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors disabled:opacity-50"
          disabled={installing || !installSlug.trim()}
          on:click={handleInstallPlugin}
        >
          {installing ? 'Installing…' : 'Install Plugin'}
        </button>
      </div>
    </div>
  </div>
{/if}


<!-- ── Login URL Modal ─────────────────────────────────────────────────────── -->
{#if showLoginUrl}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
    on:click|self={() => { showLoginUrl = null; }}
  >
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-md mx-4 shadow-2xl">
      <div class="flex items-center justify-between mb-5">
        <h3 class="text-base font-semibold text-foreground">Login URL — <span class="font-mono">{showLoginUrl.user.login}</span></h3>
        <button class="text-muted-foreground hover:text-foreground transition-colors" on:click={() => { showLoginUrl = null; }}>
          <svg class="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      <p class="text-xs text-muted-foreground mb-3">This one-time login link grants direct access to the WordPress admin dashboard. Keep it safe — it expires after first use.</p>

      <div class="bg-muted/30 border border-border rounded-lg px-3 py-2 mb-4">
        <p class="font-mono text-xs text-foreground break-all">{showLoginUrl.url}</p>
      </div>

      <div class="flex justify-end gap-2">
        <button
          class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted transition-colors"
          on:click={() => { showLoginUrl = null; }}
        >
          Close
        </button>
        <button
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors inline-flex items-center gap-1.5"
          on:click={() => copyToClipboard(showLoginUrl!.url)}
        >
          <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z"/>
          </svg>
          Copy URL
        </button>
      </div>
    </div>
  </div>
{/if}


<!-- ── Toast notifications ────────────────────────────────────────────────── -->
<div class="fixed bottom-5 right-5 z-[60] flex flex-col gap-2 pointer-events-none">
  {#each toasts as toast (toast.id)}
    <div
      class="pointer-events-auto flex items-center gap-3 px-4 py-3 rounded-xl shadow-lg border text-sm font-medium transition-all
        {toast.type === 'success'
          ? 'bg-card border-green-500/30 text-foreground'
          : 'bg-card border-red-500/30 text-foreground'}"
    >
      {#if toast.type === 'success'}
        <svg class="w-4 h-4 text-green-400 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7" />
        </svg>
      {:else}
        <svg class="w-4 h-4 text-red-400 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
        </svg>
      {/if}
      <span>{toast.message}</span>
    </div>
  {/each}
</div>
