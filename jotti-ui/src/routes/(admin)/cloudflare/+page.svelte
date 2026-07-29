<script lang="ts">
  import { onMount } from 'svelte';
  import { get } from 'svelte/store';
  import { auth } from '$lib/stores/auth';
  import { t } from '$lib/i18n';

  // ── Types ──────────────────────────────────────────────────────────────────

  type CfToken = {
    id: string;
    label: string;
    zone_id: string;
    domain: string;
    proxy: boolean;
    created_at: string;
  };

  type SyncResult = {
    synced: number;
    errors: number;
    message: string;
  };

  // ── State ──────────────────────────────────────────────────────────────────

  let tokens: CfToken[] = [];
  let loading = true;
  let loadError = '';

  // Modal
  let showModal = false;
  let modalSaving = false;
  let modalError = '';
  let form = {
    label: '',
    api_token: '',
    zone_id: '',
    domain: '',
    proxy: false,
  };
  let showApiToken = false;

  // Per-row loading states
  let syncLoading: Record<string, boolean> = {};
  let proxyLoading: Record<string, boolean> = {};
  let deleteLoading: Record<string, boolean> = {};

  // Inline confirm state
  let confirmDeleteTokenId: string | null = null;

  // Toast
  let toastMessage = '';
  let toastType: 'success' | 'error' = 'success';
  let toastTimer: ReturnType<typeof setTimeout> | null = null;

  // ── Auth ───────────────────────────────────────────────────────────────────

  function authH(): Record<string, string> {
    const t = get(auth).token;
    return t
      ? { Authorization: 'Bearer ' + t, 'Content-Type': 'application/json' }
      : { 'Content-Type': 'application/json' };
  }

  // ── Data loading ───────────────────────────────────────────────────────────

  async function loadTokens() {
    loading = true;
    loadError = '';
    try {
      const r = await fetch('/api/v1/cloudflare/tokens', { headers: authH() });
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      tokens = (await r.json()) as CfToken[];
    } catch (err: unknown) {
      const e = err as { message?: string };
      loadError = e.message ?? 'Failed to load Cloudflare tokens';
    } finally {
      loading = false;
    }
  }

  onMount(loadTokens);

  // ── Create ─────────────────────────────────────────────────────────────────

  function openModal() {
    form = { label: '', api_token: '', zone_id: '', domain: '', proxy: false };
    modalError = '';
    showApiToken = false;
    showModal = true;
  }

  function closeModal() {
    showModal = false;
    modalError = '';
  }

  async function createToken() {
    if (!form.label.trim()) { modalError = 'Label is required'; return; }
    if (!form.api_token.trim()) { modalError = 'API Token is required'; return; }
    if (!form.zone_id.trim()) { modalError = 'Zone ID is required'; return; }
    if (!form.domain.trim()) { modalError = 'Domain is required'; return; }
    modalSaving = true;
    modalError = '';
    try {
      const body = {
        label: form.label.trim(),
        api_token: form.api_token.trim(),
        zone_id: form.zone_id.trim(),
        domain: form.domain.trim(),
        proxy: form.proxy,
      };
      const r = await fetch('/api/v1/cloudflare/tokens', {
        method: 'POST',
        headers: authH(),
        body: JSON.stringify(body),
      });
      if (!r.ok) {
        const data = (await r.json().catch(() => ({}))) as { message?: string };
        throw new Error(data.message ?? `HTTP ${r.status}`);
      }
      const created = (await r.json()) as CfToken;
      tokens = [created, ...tokens];
      closeModal();
      showToast('Cloudflare token added', 'success');
    } catch (err: unknown) {
      const e = err as { message?: string };
      modalError = e.message ?? 'Failed to add token';
    } finally {
      modalSaving = false;
    }
  }

  // ── Delete ─────────────────────────────────────────────────────────────────

  async function deleteToken(id: string) {
    confirmDeleteTokenId = null;
    deleteLoading = { ...deleteLoading, [id]: true };
    try {
      const r = await fetch(`/api/v1/cloudflare/tokens/${id}`, { method: 'DELETE', headers: authH() });
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      tokens = tokens.filter((t) => t.id !== id);
      showToast('Token removed', 'success');
    } catch {
      showToast('Failed to remove token', 'error');
    } finally {
      deleteLoading = { ...deleteLoading, [id]: false };
    }
  }

  // ── Sync ───────────────────────────────────────────────────────────────────

  async function syncToken(id: string) {
    syncLoading = { ...syncLoading, [id]: true };
    try {
      const r = await fetch(`/api/v1/cloudflare/tokens/${id}/sync`, { method: 'POST', headers: authH() });
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      const data = (await r.json()) as SyncResult;
      if (data.errors > 0) {
        showToast(`Synced ${data.synced} records · ${data.errors} error(s): ${data.message}`, 'error');
      } else {
        showToast(`Synced ${data.synced} DNS record${data.synced !== 1 ? 's' : ''} successfully`, 'success');
      }
    } catch {
      showToast('Sync failed — check your token permissions', 'error');
    } finally {
      syncLoading = { ...syncLoading, [id]: false };
    }
  }

  // ── Toggle proxy ───────────────────────────────────────────────────────────

  async function toggleProxy(token: CfToken) {
    proxyLoading = { ...proxyLoading, [token.id]: true };
    const newProxy = !token.proxy;
    try {
      const r = await fetch(`/api/v1/cloudflare/tokens/${token.id}/proxy`, {
        method: 'PUT',
        headers: authH(),
        body: JSON.stringify({ proxy: newProxy }),
      });
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      tokens = tokens.map((t) => (t.id === token.id ? { ...t, proxy: newProxy } : t));
      showToast(newProxy ? 'Proxy enabled (orange cloud)' : 'Proxy disabled (grey cloud)', 'success');
    } catch {
      showToast('Failed to update proxy setting', 'error');
    } finally {
      proxyLoading = { ...proxyLoading, [token.id]: false };
    }
  }

  // ── Helpers ────────────────────────────────────────────────────────────────

  function showToast(msg: string, type: 'success' | 'error') {
    if (toastTimer) clearTimeout(toastTimer);
    toastMessage = msg;
    toastType = type;
    toastTimer = setTimeout(() => (toastMessage = ''), 5000);
  }

  function formatDate(str: string): string {
    return new Date(str).toLocaleDateString();
  }

  function shortZoneId(id: string): string {
    return id.length > 8 ? id.slice(0, 8) + '…' : id;
  }
</script>

<svelte:head>
  <title>{$t('cloudflare.title')} — JottiCP</title>
</svelte:head>

<div class="p-4 lg:p-6 space-y-6">

  <!-- Page header -->
  <div class="flex items-start justify-between gap-4">
    <div>
      <h1 class="text-2xl font-bold text-foreground">{$t('cloudflare.title')}</h1>
      <p class="text-muted-foreground text-sm mt-1">{$t('cloudflare.subtitle')}</p>
    </div>
    <button
      on:click={openModal}
      class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 transition-all duration-150 active:scale-95 shrink-0"
    >
      <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4" />
      </svg>{$t('cloudflare.add_token')}</button>
  </div>

  <!-- Tokens table card -->
  <div class="bg-card border border-border rounded-xl p-5">
    <div class="flex items-center gap-3 mb-4">
      <div class="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
        <!-- Cloudflare-style cloud icon -->
        <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M3 15a4 4 0 004 4h9a5 5 0 10-.1-9.999 5.002 5.002 0 10-9.78 2.096A4.001 4.001 0 003 15z" />
        </svg>
      </div>
      <h3 class="text-sm font-semibold text-foreground">{$t('cloudflare.api_tokens')}</h3>
    </div>

    {#if loading}
      <div class="space-y-2">
        {#each [1, 2, 3] as _}
          <div class="h-12 bg-muted/40 rounded-lg animate-pulse"></div>
        {/each}
      </div>

    {:else if loadError}
      <div class="flex items-center gap-2 text-sm text-red-400 bg-red-500/10 border border-red-500/20 rounded-lg px-4 py-3">
        <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
        </svg>
        {loadError}
      </div>

    {:else if tokens.length === 0}
      <!-- Empty state -->
      <div class="flex flex-col items-center justify-center py-14 text-center">
        <div class="w-12 h-12 rounded-2xl bg-muted/50 flex items-center justify-center mb-3">
          <svg class="w-6 h-6 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" d="M3 15a4 4 0 004 4h9a5 5 0 10-.1-9.999 5.002 5.002 0 10-9.78 2.096A4.001 4.001 0 003 15z" />
          </svg>
        </div>
        <p class="text-sm font-medium text-foreground mb-1">{$t('cloudflare.no_tokens')}</p>
        <p class="text-xs text-muted-foreground mb-4">{$t('cloudflare.no_tokens_desc')}</p>
        <button
          on:click={openModal}
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
        >
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4" />
          </svg>{$t('cloudflare.add_token')}</button>
      </div>

    {:else}
      <div class="border border-border rounded-xl overflow-hidden">
        <table class="w-full text-sm">
          <thead class="bg-muted/30">
            <tr>
              <th class="px-4 py-2.5 text-left text-xs font-semibold text-muted-foreground uppercase tracking-wide">{$t('settings.label')}</th>
              <th class="px-4 py-2.5 text-left text-xs font-semibold text-muted-foreground uppercase tracking-wide">{$t('sites.domain')}</th>
              <th class="px-4 py-2.5 text-left text-xs font-semibold text-muted-foreground uppercase tracking-wide">{$t('cloudflare.col_zone_id')}</th>
              <th class="px-4 py-2.5 text-left text-xs font-semibold text-muted-foreground uppercase tracking-wide">{$t('cloudflare.col_proxy')}</th>
              <th class="px-4 py-2.5 text-left text-xs font-semibold text-muted-foreground uppercase tracking-wide">{$t('common.created')}</th>
              <th class="px-4 py-2.5 text-right text-xs font-semibold text-muted-foreground uppercase tracking-wide">{$t('common.actions')}</th>
            </tr>
          </thead>
          <tbody class="divide-y divide-border">
            {#each tokens as tok (tok.id)}
              <tr class="hover:bg-muted/10 transition-colors duration-100">
                <!-- Label -->
                <td class="px-4 py-3">
                  <span class="text-sm font-medium text-foreground">{tok.label}</span>
                </td>

                <!-- Domain -->
                <td class="px-4 py-3">
                  <span class="text-sm text-foreground font-mono">{tok.domain}</span>
                </td>

                <!-- Zone ID (truncated) -->
                <td class="px-4 py-3">
                  <span
                    class="font-mono text-xs text-muted-foreground cursor-default"
                    title={tok.zone_id}
                  >
                    {shortZoneId(tok.zone_id)}
                  </span>
                </td>

                <!-- Proxy badge / toggle -->
                <td class="px-4 py-3">
                  <button
                    on:click={() => toggleProxy(tok)}
                    disabled={proxyLoading[tok.id]}
                    class="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-[10px] font-semibold border transition-all duration-150 disabled:opacity-50
                      {tok.proxy
                        ? 'bg-orange-500/15 border-orange-500/30 text-orange-400 hover:bg-orange-500/25'
                        : 'bg-muted border-border text-muted-foreground hover:bg-muted/70'}"
                    title={tok.proxy ? 'Proxied through Cloudflare — click to disable' : 'DNS only — click to enable proxy'}
                  >
                    <!-- Cloud icon — orange when proxied, grey when not -->
                    <svg class="w-3 h-3" fill="currentColor" viewBox="0 0 20 20">
                      <path d="M5.5 16a3.5 3.5 0 01-.369-6.98 4 4 0 117.753-1.977A4.5 4.5 0 1113.5 16H5.5z" />
                    </svg>
                    {tok.proxy ? 'Proxied' : 'DNS only'}
                  </button>
                </td>

                <!-- Created date -->
                <td class="px-4 py-3 text-xs text-muted-foreground whitespace-nowrap">
                  {formatDate(tok.created_at)}
                </td>

                <!-- Actions -->
                <td class="px-4 py-3">
                  <div class="flex items-center justify-end gap-1">
                    <!-- Sync button -->
                    <button
                      on:click={() => syncToken(tok.id)}
                      disabled={syncLoading[tok.id]}
                      class="h-7 px-2.5 rounded-lg border border-border text-xs text-muted-foreground hover:bg-muted hover:text-foreground transition-colors disabled:opacity-50 inline-flex items-center gap-1.5 whitespace-nowrap"
                    >
                      {#if syncLoading[tok.id]}
                        <svg class="w-3 h-3 animate-spin" fill="none" viewBox="0 0 24 24">
                          <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                          <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
                        </svg>
                        Syncing…
                      {:else}
                        <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
                        </svg>
                        Sync
                      {/if}
                    </button>

                    <!-- Delete button -->
                    {#if confirmDeleteTokenId === tok.id}
                      <div class="flex items-center gap-1.5">
                        <span class="text-xs text-destructive">{$t('servers.remove')}</span>
                        <button class="text-xs px-2 py-1 rounded bg-destructive text-white hover:bg-destructive/90" on:click={() => deleteToken(tok.id)}>{$t("common.yes")}</button>
                        <button class="text-xs px-2 py-1 rounded bg-muted" on:click={() => confirmDeleteTokenId = null}>{$t("common.no")}</button>
                      </div>
                    {:else}
                      <button
                        on:click={() => confirmDeleteTokenId = tok.id}
                        disabled={deleteLoading[tok.id]}
                        class="text-red-400 hover:bg-red-500/10 px-3 py-1.5 rounded-lg text-sm transition-colors disabled:opacity-50"
                        aria-label="Delete token"
                      >
                        <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
                        </svg>
                      </button>
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

  <!-- Info card -->
  <div class="bg-card border border-border rounded-xl p-5">
    <div class="flex items-center gap-3 mb-4">
      <div class="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
        <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
        </svg>
      </div>
      <h3 class="text-sm font-semibold text-foreground">{$t('cloudflare.setup_guide')}</h3>
    </div>
    <div class="space-y-3 text-sm text-muted-foreground">
      <p>{$t('cloudflare.jotticp_uses_cloudflares_dns_edit_api_to')}</p>

      <div class="grid gap-3 sm:grid-cols-3">
        <div class="bg-muted/30 border border-border rounded-lg p-3">
          <p class="text-xs font-semibold text-foreground mb-1">{$t('cloudflare.guide_step1')}</p>
          <p class="text-xs">{$t('cloudflare.in_cloudflare_my_profile_api_tokens_crea')}<em>{$t('cloudflare.edit_zone_dns')}</em>{$t('cloudflare.template_and_restrict_it_to_the_specific')}</p>
        </div>
        <div class="bg-muted/30 border border-border rounded-lg p-3">
          <p class="text-xs font-semibold text-foreground mb-1">{$t('cloudflare.guide_step2')}</p>
          <p class="text-xs">{$t('cloudflare.guide_step2_desc')}</p>
        </div>
        <div class="bg-muted/30 border border-border rounded-lg p-3">
          <p class="text-xs font-semibold text-foreground mb-1">{$t('cloudflare.3_add_amp_sync')}</p>
          <p class="text-xs">{$t('cloudflare.guide_step3_desc')}</p>
        </div>
      </div>

      <p class="text-xs">
        <span class="font-medium text-foreground">{$t('cloudflare.required_perm')}</span>{$t('cloudflare.zone_dns_edit_restrict_the_token_to_a_si')}</p>
    </div>
  </div>
</div>

<!-- ── Add Token Modal ────────────────────────────────────────────────────── -->

{#if showModal}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
    on:click|self={closeModal}
  >
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-md mx-4 shadow-2xl">
      <!-- Modal header -->
      <div class="flex items-center justify-between mb-5">
        <div>
          <h2 class="text-base font-semibold text-foreground">{$t('cloudflare.modal_title')}</h2>
          <p class="text-xs text-muted-foreground mt-0.5">{$t('cloudflare.modal_desc')}</p>
        </div>
        <button
          on:click={closeModal}
          class="w-7 h-7 rounded-lg hover:bg-muted flex items-center justify-center text-muted-foreground hover:text-foreground transition-colors"
          aria-label="Close"
        >
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      <div class="space-y-4">
        <!-- Label -->
        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5" for="cf-label">{$t('settings.label')}<span class="text-red-400">*</span>
          </label>
          <input
            id="cf-label"
            type="text"
            bind:value={form.label}
            placeholder="Production zone"
            class="h-10 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring w-full placeholder:text-muted-foreground"
          />
        </div>

        <!-- API Token -->
        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5" for="cf-token">{$t('cloudflare.api_token')}<span class="text-red-400">*</span>
          </label>
          <div class="relative">
            <input
              id="cf-token"
              type={showApiToken ? 'text' : 'password'}
              bind:value={form.api_token}
              placeholder="Create at dash.cloudflare.com → My Profile → API Tokens"
              class="h-10 px-3 pr-10 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring w-full placeholder:text-muted-foreground font-mono"
              autocomplete="off"
            />
            <button
              type="button"
              on:click={() => (showApiToken = !showApiToken)}
              class="absolute right-3 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground transition-colors"
              aria-label={showApiToken ? 'Hide token' : 'Show token'}
            >
              {#if showApiToken}
                <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M9.88 9.88l-3.29-3.29m7.532 7.532l3.29 3.29M3 3l3.59 3.59m0 0A9.953 9.953 0 0112 5c4.478 0 8.268 2.943 9.543 7a10.025 10.025 0 01-4.132 5.411m0 0L21 21" />
                </svg>
              {:else}
                <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z" />
                </svg>
              {/if}
            </button>
          </div>
        </div>

        <!-- Zone ID -->
        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5" for="cf-zone">{$t('cloudflare.col_zone_id')}<span class="text-red-400">*</span>
          </label>
          <input
            id="cf-zone"
            type="text"
            bind:value={form.zone_id}
            placeholder="32-char hex string from Cloudflare zone overview"
            maxlength={32}
            class="h-10 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring w-full placeholder:text-muted-foreground font-mono"
          />
        </div>

        <!-- Domain -->
        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5" for="cf-domain">{$t('sites.domain')}<span class="text-red-400">*</span>
          </label>
          <input
            id="cf-domain"
            type="text"
            bind:value={form.domain}
            placeholder="example.com"
            class="h-10 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring w-full placeholder:text-muted-foreground font-mono"
          />
        </div>

        <!-- Proxy toggle -->
        <div class="flex items-center justify-between rounded-lg border border-border px-3 py-3">
          <div>
            <p class="text-sm font-medium text-foreground">{$t('cloudflare.proxy_label')}</p>
            <p class="text-xs text-muted-foreground mt-0.5">{$t('cloudflare.proxy_desc')}</p>
          </div>
          <button
            type="button"
            on:click={() => (form.proxy = !form.proxy)}
            class="relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors duration-200 focus:outline-none
              {form.proxy ? 'bg-orange-500' : 'bg-muted'}"
            role="switch"
            aria-checked={form.proxy}
          >
            <span class="inline-block h-3.5 w-3.5 rounded-full bg-white shadow transition-transform duration-200 {form.proxy ? 'translate-x-4' : 'translate-x-0.5'}"></span>
          </button>
        </div>

        <!-- Error -->
        {#if modalError}
          <div class="flex items-center gap-2 text-sm text-red-400 bg-red-500/10 border border-red-500/20 rounded-lg px-3 py-2.5">
            <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
            </svg>
            {modalError}
          </div>
        {/if}
      </div>

      <!-- Actions -->
      <div class="flex items-center justify-end gap-2 mt-6 pt-4 border-t border-border">
        <button
          on:click={closeModal}
          class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground transition-colors"
        >{$t('common.cancel')}</button>
        <button
          on:click={createToken}
          disabled={modalSaving}
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 transition-all duration-150 active:scale-95 disabled:opacity-50"
        >
          {#if modalSaving}
            <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
              <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
              <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
            </svg>
            Adding…
          {:else}
      {$t('cloudflare.add_token')}
          {/if}
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- ── Toast ──────────────────────────────────────────────────────────────── -->

{#if toastMessage}
  <div
    class="fixed bottom-4 right-4 z-50 flex items-center gap-2 rounded-xl px-4 py-3 text-sm font-medium shadow-xl border transition-all duration-300 max-w-sm
      {toastType === 'success'
        ? 'bg-card border-emerald-500/30 text-green-400'
        : 'bg-card border-red-500/30 text-red-400'}"
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
