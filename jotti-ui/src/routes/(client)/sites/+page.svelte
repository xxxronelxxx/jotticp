<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$api/client';
  import type { Site } from '$api/client';
  import StatusBadge from '$components/ui/StatusBadge.svelte';
  import PageHeader from '$components/ui/PageHeader.svelte';

  // ── State ──────────────────────────────────────────────────────────────────
  let sites: Site[] = [];
  let loading = true;
  let search = '';
  let confirmDeleteId: string | null = null;
  let deleteLoading = false;

  let toastMessage = '';
  let toastType: 'success' | 'error' = 'success';

  let showAddModal = false;
  let addForm = { domain: '', php_version: '8.3', web_server: 'ols' };
  let addLoading = false;
  let addError = '';

  onMount(async () => {
    await loadSites();
  });

  async function loadSites() {
    loading = true;
    try {
      sites = await api.client.sites();
    } catch {
      showToast('Failed to load sites', 'error');
    } finally {
      loading = false;
    }
  }

  async function deleteSite(id: string) {
    deleteLoading = true;
    try {
      await api.sites.delete(id);
      sites = sites.filter(s => s.id !== id);
      confirmDeleteId = null;
      showToast('Site deleted', 'success');
    } catch {
      showToast('Failed to delete site', 'error');
    } finally {
      deleteLoading = false;
    }
  }

  async function addSite(e: SubmitEvent) {
    e.preventDefault();
    if (!addForm.domain.trim()) return;
    addLoading = true;
    addError = '';
    try {
      const newSite = await api.sites.create({
        domain:      addForm.domain.trim(),
        php_version: addForm.php_version,
        web_server:  addForm.web_server,
      });
      sites = [newSite, ...sites];
      showAddModal = false;
      addForm = { domain: '', php_version: '8.3', web_server: 'ols' };
      showToast(`Site ${newSite.domain} created`, 'success');
    } catch (err: unknown) {
      const e = err as { message?: string };
      addError = e.message ?? 'Failed to create site';
    } finally {
      addLoading = false;
    }
  }

  function showToast(msg: string, type: 'success' | 'error') {
    toastMessage = msg;
    toastType = type;
    setTimeout(() => { toastMessage = ''; }, 4000);
  }

  function formatDisk(mb: number): string {
    if (mb >= 1024) return `${(mb / 1024).toFixed(1)} GB`;
    return `${mb} MB`;
  }

  function sslColor(status: string): string {
    return status === 'active'  ? 'text-green-600'
         : status === 'expired' ? 'text-red-500'
         :                        'text-amber-500';
  }

  $: filtered = sites.filter(s =>
    !search || s.domain.toLowerCase().includes(search.toLowerCase())
  );
</script>

<svelte:head>
  <title>My Sites — JottiCP</title>
</svelte:head>

<div class="p-4 lg:p-6 space-y-4">

  <PageHeader title="My Sites" subtitle="Manage your hosted websites">
    <button
      type="button"
      on:click={() => showAddModal = true}
      class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium
             hover:bg-primary/90 transition-colors"
    >
      + Add Site
    </button>
  </PageHeader>

  <!-- Search -->
  <div class="flex gap-2">
    <input
      type="search"
      bind:value={search}
      placeholder="Search domains..."
      class="flex-1 h-9 px-3 rounded-lg border border-border bg-background text-sm
             text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
    />
  </div>

  <!-- Loading skeleton -->
  {#if loading}
    <div class="space-y-2">
      {#each [1,2,3] as _}
        <div class="h-16 bg-muted rounded-xl animate-pulse"></div>
      {/each}
    </div>

  {:else if filtered.length === 0}
    <div class="bg-card border border-border rounded-xl p-10 text-center">
      <svg class="w-10 h-10 mx-auto text-muted-foreground mb-3" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
          d="M12 2a10 10 0 100 20A10 10 0 0012 2zm0 0c-2.5 2.5-4 6.2-4 10s1.5 7.5 4 10m0-20c2.5 2.5 4 6.2 4 10s-1.5 7.5-4 10M2 12h20" />
      </svg>
      <p class="text-sm font-medium text-foreground mb-1">No sites found</p>
      <p class="text-xs text-muted-foreground mb-4">
        {search ? 'No sites match your search.' : "You don't have any sites yet."}
      </p>
      {#if !search}
        <button
          type="button"
          on:click={() => showAddModal = true}
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                 hover:bg-primary/90 transition-colors"
        >
          Add Your First Site
        </button>
      {/if}
    </div>

  {:else}
    <div class="bg-card border border-border rounded-xl overflow-hidden">
      <table class="w-full text-sm">
        <thead>
          <tr class="border-b border-border text-left">
            <th class="px-4 py-3 text-xs font-medium text-muted-foreground uppercase">Domain</th>
            <th class="px-4 py-3 text-xs font-medium text-muted-foreground uppercase hidden sm:table-cell">Status</th>
            <th class="px-4 py-3 text-xs font-medium text-muted-foreground uppercase hidden md:table-cell">PHP</th>
            <th class="px-4 py-3 text-xs font-medium text-muted-foreground uppercase hidden md:table-cell">Disk</th>
            <th class="px-4 py-3 text-xs font-medium text-muted-foreground uppercase hidden sm:table-cell">SSL</th>
            <th class="px-4 py-3 text-xs font-medium text-muted-foreground uppercase">Actions</th>
          </tr>
        </thead>
        <tbody class="divide-y divide-border">
          {#each filtered as site (site.id)}
            <tr class="hover:bg-muted/30">
              <td class="px-4 py-3">
                <div class="font-medium text-foreground">{site.domain}</div>
                <div class="text-xs text-muted-foreground mt-0.5 sm:hidden">
                  <StatusBadge status={site.status} />
                </div>
              </td>
              <td class="px-4 py-3 hidden sm:table-cell">
                <StatusBadge status={site.status} />
              </td>
              <td class="px-4 py-3 text-muted-foreground hidden md:table-cell text-xs">PHP {site.php_version}</td>
              <td class="px-4 py-3 text-muted-foreground hidden md:table-cell text-xs">{formatDisk(site.disk_mb)}</td>
              <td class="px-4 py-3 hidden sm:table-cell">
                <span class="text-xs font-medium {sslColor(site.ssl_status)}">
                  {site.ssl_status === 'active' ? 'Active' : site.ssl_status === 'expired' ? 'Expired' : 'None'}
                </span>
              </td>
              <td class="px-4 py-3">
                <div class="flex items-center gap-2">
                  <a href="/websites/{site.id}"
                     class="text-xs text-primary hover:underline font-medium">View</a>
                  <button
                    type="button"
                    on:click={() => confirmDeleteId = site.id}
                    class="text-xs text-destructive hover:underline font-medium"
                  >
                    Delete
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

<!-- Add Site Modal -->
{#if showAddModal}
  <div class="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/50"
       role="dialog" aria-modal="true" aria-label="Add Site">
    <div class="bg-card border border-border rounded-xl shadow-xl w-full max-w-md p-6 space-y-4">
      <h2 class="text-lg font-semibold text-foreground">Add New Site</h2>

      <form on:submit={addSite} class="space-y-3">
        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5">Domain</label>
          <input type="text" bind:value={addForm.domain} required placeholder="example.com"
                 class="w-full h-10 px-3 rounded-lg border border-border bg-background text-sm
                        text-foreground font-mono focus:outline-none focus:ring-2 focus:ring-ring" />
        </div>

        <div class="grid grid-cols-2 gap-3">
          <div>
            <label class="block text-sm font-medium text-foreground mb-1.5">PHP Version</label>
            <select bind:value={addForm.php_version}
                    class="w-full h-10 px-3 rounded-lg border border-border bg-background text-sm
                           text-foreground focus:outline-none focus:ring-2 focus:ring-ring">
              {#each ['8.3', '8.2', '8.1', '8.0', '7.4'] as ver}
                <option value={ver}>PHP {ver}</option>
              {/each}
            </select>
          </div>
          <div>
            <label class="block text-sm font-medium text-foreground mb-1.5">Web Server</label>
            <select bind:value={addForm.web_server}
                    class="w-full h-10 px-3 rounded-lg border border-border bg-background text-sm
                           text-foreground focus:outline-none focus:ring-2 focus:ring-ring">
              <option value="ols">OpenLiteSpeed</option>
              <option value="nginx">Nginx</option>
              <option value="apache">Apache</option>
            </select>
          </div>
        </div>

        {#if addError}
          <p class="text-sm text-destructive">{addError}</p>
        {/if}

        <div class="flex gap-2 pt-1">
          <button type="button" on:click={() => { showAddModal = false; addError = ''; }}
                  class="flex-1 h-10 rounded-lg border border-border text-sm font-medium text-muted-foreground
                         hover:bg-muted transition-colors">
            Cancel
          </button>
          <button type="submit" disabled={addLoading || !addForm.domain.trim()}
                  class="flex-1 h-10 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                         hover:bg-primary/90 transition-colors disabled:opacity-50">
            {addLoading ? 'Creating...' : 'Create Site'}
          </button>
        </div>
      </form>
    </div>
  </div>
{/if}

<!-- Confirm Delete Modal -->
{#if confirmDeleteId}
  {@const site = sites.find(s => s.id === confirmDeleteId)}
  <div class="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/50"
       role="dialog" aria-modal="true">
    <div class="bg-card border border-border rounded-xl shadow-xl w-full max-w-sm p-6 space-y-4">
      <h2 class="text-lg font-semibold text-foreground">Delete Site?</h2>
      <p class="text-sm text-muted-foreground">
        This will permanently delete <strong class="text-foreground">{site?.domain}</strong>
        and all its files, databases, and email accounts. This cannot be undone.
      </p>
      <div class="flex gap-2">
        <button type="button" on:click={() => confirmDeleteId = null}
                class="flex-1 h-10 rounded-lg border border-border text-sm font-medium text-muted-foreground
                       hover:bg-muted transition-colors">
          Cancel
        </button>
        <button type="button" on:click={() => confirmDeleteId && deleteSite(confirmDeleteId)}
                disabled={deleteLoading}
                class="flex-1 h-10 rounded-lg bg-destructive text-destructive-foreground text-sm font-medium
                       hover:bg-destructive/90 transition-colors disabled:opacity-50">
          {deleteLoading ? 'Deleting...' : 'Delete'}
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- Toast -->
{#if toastMessage}
  <div class="fixed bottom-6 right-4 z-50 flex items-center gap-2 px-4 py-3 rounded-lg shadow-lg text-sm font-medium
              {toastType === 'success' ? 'bg-green-600 text-white' : 'bg-destructive text-destructive-foreground'}"
       role="status" aria-live="polite">
    {toastMessage}
  </div>
{/if}
