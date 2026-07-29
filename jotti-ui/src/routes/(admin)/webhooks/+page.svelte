<script lang="ts">
  import { onMount } from 'svelte';
  import { get } from 'svelte/store';
  import { auth } from '$lib/stores/auth';
  import { t } from '$lib/i18n';

  // ── Types ──────────────────────────────────────────────────────────────────

  type Webhook = {
    id: string;
    url: string;
    events: string[];
    enabled: boolean;
    secret: string;
    created_at: string;
  };

  type TestResult = {
    success: boolean;
    status_code: number;
    response_ms: number;
  };

  // ── State ──────────────────────────────────────────────────────────────────

  let webhooks: Webhook[] = [];
  let loading = true;
  let loadError = '';

  // Modal
  let showModal = false;
  let modalSaving = false;
  let modalError = '';
  let form = {
    url: '',
    events: [] as string[],
    secret: '',
  };

  // Per-row test results
  let testResults: Record<string, { loading: boolean; result: TestResult | null; error: string }> = {};

  // Toggle loading per row
  let toggleLoading: Record<string, boolean> = {};

  // Delete loading per row
  let deleteLoading: Record<string, boolean> = {};
  let confirmDeleteWebhookId: string | null = null;

  // Toast
  let toastMessage = '';
  let toastType: 'success' | 'error' = 'success';
  let toastTimer: ReturnType<typeof setTimeout> | null = null;

  // ── Constants ──────────────────────────────────────────────────────────────

  const ALL_EVENTS = [
    'site.created',
    'site.deleted',
    'ssl.issued',
    'ssl.expired',
    'backup.completed',
    'backup.failed',
    'deploy.completed',
    'malware.detected',
    'user.created',
  ];

  // ── Auth ───────────────────────────────────────────────────────────────────

  function authH(): Record<string, string> {
    const t = get(auth).token;
    return t
      ? { Authorization: 'Bearer ' + t, 'Content-Type': 'application/json' }
      : { 'Content-Type': 'application/json' };
  }

  // ── Data loading ───────────────────────────────────────────────────────────

  async function loadWebhooks() {
    loading = true;
    loadError = '';
    try {
      const r = await fetch('/api/v1/webhooks', { headers: authH() });
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      webhooks = (await r.json()) as Webhook[];
    } catch (err: unknown) {
      const e = err as { message?: string };
      loadError = e.message ?? 'Failed to load webhooks';
    } finally {
      loading = false;
    }
  }

  onMount(loadWebhooks);

  // ── Create ─────────────────────────────────────────────────────────────────

  function openModal() {
    form = { url: '', events: [], secret: '' };
    modalError = '';
    showModal = true;
  }

  function closeModal() {
    showModal = false;
    modalError = '';
  }

  function toggleEvent(event: string) {
    if (form.events.includes(event)) {
      form.events = form.events.filter((e) => e !== event);
    } else {
      form.events = [...form.events, event];
    }
  }

  async function createWebhook() {
    if (!form.url.trim()) {
      modalError = 'URL is required';
      return;
    }
    if (form.events.length === 0) {
      modalError = 'Select at least one event';
      return;
    }
    modalSaving = true;
    modalError = '';
    try {
      const body: Record<string, unknown> = { url: form.url.trim(), events: form.events };
      if (form.secret.trim()) body.secret = form.secret.trim();
      const r = await fetch('/api/v1/webhooks', {
        method: 'POST',
        headers: authH(),
        body: JSON.stringify(body),
      });
      if (!r.ok) {
        const data = (await r.json().catch(() => ({}))) as { message?: string };
        throw new Error(data.message ?? `HTTP ${r.status}`);
      }
      const created = (await r.json()) as Webhook;
      webhooks = [created, ...webhooks];
      closeModal();
      showToast('Webhook added', 'success');
    } catch (err: unknown) {
      const e = err as { message?: string };
      modalError = e.message ?? 'Failed to create webhook';
    } finally {
      modalSaving = false;
    }
  }

  // ── Delete ─────────────────────────────────────────────────────────────────

  async function deleteWebhook(id: string) {
    confirmDeleteWebhookId = null;
    deleteLoading = { ...deleteLoading, [id]: true };
    try {
      const r = await fetch(`/api/v1/webhooks/${id}`, { method: 'DELETE', headers: authH() });
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      webhooks = webhooks.filter((w) => w.id !== id);
      showToast('Webhook deleted', 'success');
    } catch {
      showToast('Failed to delete webhook', 'error');
    } finally {
      deleteLoading = { ...deleteLoading, [id]: false };
    }
  }

  // ── Toggle enabled ─────────────────────────────────────────────────────────

  async function toggleEnabled(webhook: Webhook) {
    toggleLoading = { ...toggleLoading, [webhook.id]: true };
    const newEnabled = !webhook.enabled;
    try {
      const r = await fetch(`/api/v1/webhooks/${webhook.id}`, {
        method: 'PUT',
        headers: authH(),
        body: JSON.stringify({ enabled: newEnabled }),
      });
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      webhooks = webhooks.map((w) => (w.id === webhook.id ? { ...w, enabled: newEnabled } : w));
    } catch {
      showToast('Failed to update webhook', 'error');
    } finally {
      toggleLoading = { ...toggleLoading, [webhook.id]: false };
    }
  }

  // ── Test ───────────────────────────────────────────────────────────────────

  async function testWebhook(id: string) {
    testResults = { ...testResults, [id]: { loading: true, result: null, error: '' } };
    try {
      const r = await fetch(`/api/v1/webhooks/${id}/test`, { method: 'POST', headers: authH() });
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      const data = (await r.json()) as TestResult;
      testResults = { ...testResults, [id]: { loading: false, result: data, error: '' } };
    } catch (err: unknown) {
      const e = err as { message?: string };
      testResults = {
        ...testResults,
        [id]: { loading: false, result: null, error: e.message ?? 'Test failed' },
      };
    }
  }

  // ── Helpers ────────────────────────────────────────────────────────────────

  function showToast(msg: string, type: 'success' | 'error') {
    if (toastTimer) clearTimeout(toastTimer);
    toastMessage = msg;
    toastType = type;
    toastTimer = setTimeout(() => (toastMessage = ''), 4000);
  }

  function formatDate(str: string): string {
    return new Date(str).toLocaleDateString();
  }

  function visibleEvents(events: string[]): { visible: string[]; extra: number } {
    if (events.length <= 2) return { visible: events, extra: 0 };
    return { visible: events.slice(0, 2), extra: events.length - 2 };
  }

  function eventLabel(e: string): string {
    return e.replace('.', '​.');
  }
</script>

<svelte:head>
  <title>{$t('webhooks.title')} — JottiCP</title>
</svelte:head>

<div class="p-4 lg:p-6 space-y-6">

  <!-- Page header -->
  <div class="flex items-start justify-between gap-4">
    <div>
      <h1 class="text-2xl font-bold text-foreground">{$t('webhooks.title')}</h1>
      <p class="text-muted-foreground text-sm mt-1">{$t('webhooks.subtitle')}</p>
    </div>
    <button
      on:click={openModal}
      class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 transition-all duration-150 active:scale-95 shrink-0"
    >
      <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4" />
      </svg>{$t('webhooks.add_webhook')}</button>
  </div>

  <!-- Webhooks table card -->
  <div class="bg-card border border-border rounded-xl p-5">
    <div class="flex items-center gap-3 mb-4">
      <div class="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
        <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1" />
        </svg>
      </div>
      <h3 class="text-sm font-semibold text-foreground">{$t('webhooks.configured_endpoints')}</h3>
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

    {:else if webhooks.length === 0}
      <!-- Empty state -->
      <div class="flex flex-col items-center justify-center py-14 text-center">
        <div class="w-12 h-12 rounded-2xl bg-muted/50 flex items-center justify-center mb-3">
          <svg class="w-6 h-6 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" d="M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1" />
          </svg>
        </div>
        <p class="text-sm font-medium text-foreground mb-1">{$t('webhooks.no_webhooks')}</p>
        <p class="text-xs text-muted-foreground mb-4">{$t('webhooks.no_webhooks_desc')}</p>
        <button
          on:click={openModal}
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
        >
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4" />
          </svg>{$t('webhooks.add_webhook')}</button>
      </div>

    {:else}
      <div class="border border-border rounded-xl overflow-hidden">
        <table class="w-full text-sm">
          <thead class="bg-muted/30">
            <tr>
              <th class="px-4 py-2.5 text-left text-xs font-semibold text-muted-foreground uppercase tracking-wide">{$t('webhooks.col_url')}</th>
              <th class="px-4 py-2.5 text-left text-xs font-semibold text-muted-foreground uppercase tracking-wide">{$t('webhooks.col_events')}</th>
              <th class="px-4 py-2.5 text-left text-xs font-semibold text-muted-foreground uppercase tracking-wide">{$t('common.status')}</th>
              <th class="px-4 py-2.5 text-left text-xs font-semibold text-muted-foreground uppercase tracking-wide">{$t('common.created')}</th>
              <th class="px-4 py-2.5 text-right text-xs font-semibold text-muted-foreground uppercase tracking-wide">{$t('common.actions')}</th>
            </tr>
          </thead>
          <tbody class="divide-y divide-border">
            {#each webhooks as wh (wh.id)}
              {@const ev = visibleEvents(wh.events)}
              {@const tr = testResults[wh.id]}
              <tr class="hover:bg-muted/10 transition-colors duration-100">
                <!-- URL -->
                <td class="px-4 py-3 max-w-xs">
                  <span class="font-mono text-xs text-foreground truncate block max-w-[220px]" title={wh.url}>
                    {wh.url}
                  </span>
                </td>

                <!-- Events pills -->
                <td class="px-4 py-3">
                  <div class="flex flex-wrap gap-1 items-center">
                    {#each ev.visible as evt}
                      <span class="inline-block px-1.5 py-0.5 rounded bg-primary/10 text-primary text-[10px] font-medium whitespace-nowrap">
                        {eventLabel(evt)}
                      </span>
                    {/each}
                    {#if ev.extra > 0}
                      <span class="inline-block px-1.5 py-0.5 rounded bg-muted text-muted-foreground text-[10px] font-medium">
                        +{ev.extra} more
                      </span>
                    {/if}
                  </div>
                </td>

                <!-- Enabled toggle -->
                <td class="px-4 py-3">
                  <button
                    type="button"
                    disabled={toggleLoading[wh.id]}
                    on:click={() => toggleEnabled(wh)}
                    class="relative inline-flex h-5 w-9 items-center rounded-full transition-colors duration-200 focus:outline-none disabled:opacity-50 {wh.enabled ? 'bg-emerald-500' : 'bg-muted'}"
                    role="switch"
                    aria-checked={wh.enabled}
                    aria-label={wh.enabled ? 'Disable webhook' : 'Enable webhook'}
                  >
                    <span class="inline-block h-3.5 w-3.5 rounded-full bg-white shadow transition-transform duration-200 {wh.enabled ? 'translate-x-4' : 'translate-x-0.5'}"></span>
                  </button>
                </td>

                <!-- Created date -->
                <td class="px-4 py-3 text-xs text-muted-foreground whitespace-nowrap">
                  {formatDate(wh.created_at)}
                </td>

                <!-- Actions -->
                <td class="px-4 py-3">
                  <div class="flex items-center justify-end gap-1 flex-wrap">
                    <!-- Test result badge -->
                    {#if tr}
                      {#if tr.loading}
                        <span class="text-xs text-muted-foreground px-2 py-0.5 rounded bg-muted animate-pulse">{$t('webhooks.testing')}</span>
                      {:else if tr.result}
                        <span class="text-[10px] font-medium px-2 py-0.5 rounded whitespace-nowrap
                          {tr.result.success ? 'bg-emerald-500/15 text-emerald-400' : 'bg-red-500/15 text-red-400'}">
                          {tr.result.success ? 'OK' : 'Fail'} {tr.result.status_code} · {tr.result.response_ms}ms
                        </span>
                      {:else if tr.error}
                        <span class="text-[10px] font-medium px-2 py-0.5 rounded bg-red-500/15 text-red-400 whitespace-nowrap">
                          {tr.error}
                        </span>
                      {/if}
                    {/if}

                    <!-- Test button -->
                    <button
                      on:click={() => testWebhook(wh.id)}
                      disabled={tr?.loading}
                      class="h-7 px-2.5 rounded-lg border border-border text-xs text-muted-foreground hover:bg-muted hover:text-foreground transition-colors disabled:opacity-50 whitespace-nowrap"
                    >
                      Test
                    </button>

                    <!-- Delete button -->
                    {#if confirmDeleteWebhookId === wh.id}
                      <div class="flex items-center gap-1.5">
                        <span class="text-xs text-destructive">{$t('common.delete_confirm')}</span>
                        <button class="text-xs px-2 py-1 rounded bg-destructive text-white hover:bg-destructive/90" on:click={() => deleteWebhook(wh.id)}>{$t("common.yes")}</button>
                        <button class="text-xs px-2 py-1 rounded bg-muted" on:click={() => confirmDeleteWebhookId = null}>{$t("common.no")}</button>
                      </div>
                    {:else}
                      <button
                        on:click={() => confirmDeleteWebhookId = wh.id}
                        disabled={deleteLoading[wh.id]}
                        class="text-red-400 hover:bg-red-500/10 px-3 py-1.5 rounded-lg text-sm transition-colors disabled:opacity-50"
                        aria-label="Delete webhook"
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

  <!-- Security info card -->
  <div class="bg-card border border-border rounded-xl p-5">
    <div class="flex items-center gap-3 mb-4">
      <div class="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
        <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z" />
        </svg>
      </div>
      <h3 class="text-sm font-semibold text-foreground">{$t('webhooks.webhook_security')}</h3>
    </div>
    <div class="space-y-3 text-sm text-muted-foreground">
      <p>{$t('webhooks.every_request_jotticp_sends_to_your_endp')}<span class="text-foreground font-medium">{$t('webhooks.hmac_sha256')}</span>{$t('webhooks.the_signature_is_included_in_the')}<code class="font-mono text-xs bg-muted px-1 py-0.5 rounded text-foreground">{$t('webhooks.x_jotticp_signature')}</code>{$t('webhooks.header_as_a_hex_encoded_digest_of_the_ra')}</p>
      <div class="bg-muted/30 border border-border rounded-lg p-3 font-mono text-xs text-foreground space-y-1">
        <p class="text-muted-foreground">{$t('webhooks.verify_signature')}</p>
        <p>{$t('webhooks.computed_hmac_sha256secret_raw_body')}</p>
        <p>{$t('webhooks.assert_requestheadersx_jotticp_signature')}</p>
      </div>
      <ul class="list-disc list-inside space-y-1 text-xs">
        <li>{$t('webhooks.security_note1')}</li>
        <li>{$t('webhooks.security_note2')}</li>
        <li>{$t('webhooks.security_note3')}</li>
        <li>{$t('webhooks.each_request_includes_an')}<code class="font-mono bg-muted px-1 py-0.5 rounded text-foreground">{$t('webhooks.x_jotticp_event')}</code>{$t('webhooks.header_naming_the_event_type')}</li>
      </ul>
    </div>
  </div>
</div>

<!-- ── Add Webhook Modal ──────────────────────────────────────────────────── -->

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
          <h2 class="text-base font-semibold text-foreground">{$t('webhooks.add_webhook')}</h2>
          <p class="text-xs text-muted-foreground mt-0.5">{$t('webhooks.modal_desc')}</p>
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
        <!-- URL -->
        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5" for="wh-url">{$t('webhooks.endpoint_url')}<span class="text-red-400">*</span>
          </label>
          <input
            id="wh-url"
            type="url"
            bind:value={form.url}
            placeholder="https://example.com/webhook"
            class="h-10 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring w-full placeholder:text-muted-foreground"
          />
        </div>

        <!-- Events -->
        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5">{$t('webhooks.col_events')}<span class="text-red-400">*</span>
          </label>
          <div class="border border-border rounded-lg overflow-y-auto max-h-48 divide-y divide-border">
            {#each ALL_EVENTS as evt}
              <label class="flex items-center gap-3 px-3 py-2.5 cursor-pointer hover:bg-muted/20 transition-colors">
                <input
                  type="checkbox"
                  checked={form.events.includes(evt)}
                  on:change={() => toggleEvent(evt)}
                  class="w-4 h-4 rounded border-border accent-primary shrink-0"
                />
                <span class="text-sm text-foreground font-mono">{evt}</span>
              </label>
            {/each}
          </div>
          <p class="text-xs text-muted-foreground mt-1">{form.events.length} of {ALL_EVENTS.length} selected</p>
        </div>

        <!-- Secret -->
        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5" for="wh-secret">{$t('webhooks.signing_secret')}<span class="text-muted-foreground font-normal ml-1">{$t('cron.label_optional')}</span>
          </label>
          <input
            id="wh-secret"
            type="text"
            bind:value={form.secret}
            placeholder="Leave blank to auto-generate"
            class="h-10 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring w-full placeholder:text-muted-foreground font-mono"
          />
          <p class="text-xs text-muted-foreground mt-1">{$t('webhooks.used_to_verify_the')}<code class="font-mono bg-muted px-1 rounded">{$t('webhooks.x_jotticp_signature')}</code>{$t('webhooks.header')}</p>
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
          on:click={createWebhook}
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
      {$t('webhooks.add_webhook')}
          {/if}
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- ── Toast ──────────────────────────────────────────────────────────────── -->

{#if toastMessage}
  <div
    class="fixed bottom-4 right-4 z-50 flex items-center gap-2 rounded-xl px-4 py-3 text-sm font-medium shadow-xl border transition-all duration-300
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
