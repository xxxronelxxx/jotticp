<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$api/client';
  import type { DnsZone, DnsRecord, Site } from '$api/client';

  // ── Props ─────────────────────────────────────────────────────────────────────

  export let siteId: string;
  export let site: Site;

  // ── State ─────────────────────────────────────────────────────────────────────

  let zone: DnsZone | null = null;
  let records: DnsRecord[] = [];
  let loading = true;
  let zoneCreating = false;
  let showAddRecord = false;
  let editingRecord: DnsRecord | null = null;
  let editContent = '';
  let editTtl = 3600;
  let editPriority: number | null = null;
  let recordSaving = false;
  let searchQuery = '';
  let sortType = false;
  let newType: DnsRecord['type'] = 'A';
  let newName = '';
  let newContent = '';
  let newTtl = 3600;
  let newPriority: number | null = null;
  let recordCreating = false;
  let propagationStatus: Record<string, 'checking' | 'ok' | 'pending'> = {};

  // Inline confirm state
  let confirmDeleteZone = false;
  let confirmDeleteRecord: string | null = null;

  // Quick Setup state
  let showEmailWizard = false;
  let emailWizardStep = 1;
  let emailWizardApplying = false;
  let emailWizardDone = false;
  let subdomainInput = '';
  let subdomainCreating = false;
  let wwwCreating = false;

  // Toast
  interface Toast { message: string; type: 'success' | 'error' }
  let toast: Toast | null = null;
  let toastTimer: ReturnType<typeof setTimeout> | null = null;

  // ── Constants ─────────────────────────────────────────────────────────────────

  const recordTypes: DnsRecord['type'][] = ['A', 'AAAA', 'MX', 'TXT', 'CNAME', 'NS', 'SRV', 'CAA'];

  const typeColors: Record<string, string> = {
    A:     'bg-blue-500/10 text-blue-400 border border-blue-500/20',
    AAAA:  'bg-indigo-500/10 text-indigo-400 border border-indigo-500/20',
    MX:    'bg-purple-500/10 text-purple-400 border border-purple-500/20',
    TXT:   'bg-amber-500/10 text-amber-400 border border-amber-500/20',
    CNAME: 'bg-green-500/10 text-green-400 border border-green-500/20',
    NS:    'bg-muted text-muted-foreground border border-border',
    SRV:   'bg-orange-500/10 text-orange-400 border border-orange-500/20',
    CAA:   'bg-red-500/10 text-red-400 border border-red-500/20',
  };

  const contentPlaceholders: Record<string, string> = {
    A:     'IPv4 address (e.g. 1.2.3.4)',
    AAAA:  'IPv6 address',
    MX:    'mail.example.com',
    TXT:   'v=spf1 ...',
    CNAME: 'target.example.com',
    NS:    'ns1.example.com',
    SRV:   '_service._proto TTL class SRV priority weight port target',
    CAA:   '0 issue "letsencrypt.org"',
  };

  const contentHelp: Record<string, string> = {
    A:     'Enter the IPv4 address this name should resolve to.',
    AAAA:  'Enter the IPv6 address (e.g. 2001:db8::1).',
    MX:    'Enter the mail server hostname. Lower priority = higher preference.',
    TXT:   'Enter the raw text value, e.g. SPF or verification strings.',
    CNAME: 'Enter the canonical (target) hostname including trailing dot if FQDN.',
    NS:    'Enter the authoritative nameserver hostname.',
    SRV:   'Format: priority weight port target — e.g. 10 5 5060 sip.example.com',
    CAA:   'Format: flags tag value — e.g. 0 issue "letsencrypt.org"',
  };

  const ttlOptions = [
    { label: '1 min',  value: 60 },
    { label: '5 min',  value: 300 },
    { label: '30 min', value: 1800 },
    { label: '1 hr',   value: 3600 },
    { label: '1 day',  value: 86400 },
  ];

  // ── Reactive ──────────────────────────────────────────────────────────────────

  $: filteredRecords = records
    .filter(r => !searchQuery || r.name.toLowerCase().includes(searchQuery.toLowerCase()) || r.content.toLowerCase().includes(searchQuery.toLowerCase()))
    .sort((a, b) => sortType ? a.type.localeCompare(b.type) : 0);

  // ── Helpers ───────────────────────────────────────────────────────────────────

  function formatTtl(ttl: number): string {
    if (ttl === 60)    return '1 min';
    if (ttl === 300)   return '5 min';
    if (ttl === 1800)  return '30 min';
    if (ttl === 3600)  return '1 hr';
    if (ttl === 86400) return '1 day';
    return String(ttl);
  }

  function formatDate(iso: string): string {
    return new Date(iso).toLocaleDateString('en-US', { year: 'numeric', month: 'short', day: 'numeric' });
  }

  function showToast(message: string, type: 'success' | 'error') {
    if (toastTimer) clearTimeout(toastTimer);
    toast = { message, type };
    toastTimer = setTimeout(() => { toast = null; }, 4000);
  }

  function startEdit(record: DnsRecord) {
    editingRecord = record;
    editContent   = record.content;
    editTtl       = record.ttl;
    editPriority  = record.priority;
  }

  function cancelEdit() {
    editingRecord = null;
  }

  // ── Data loading ──────────────────────────────────────────────────────────────

  onMount(async () => {
    await loadZone();
  });

  async function loadZone() {
    loading = true;
    try {
      const zones = (await api.dns.zones.list()) ?? [];
      zone = zones.find(z => z.zone === site?.domain || z.site_id === siteId) ?? null;
      if (zone) {
        records = (await api.dns.records.list(zone.id)) ?? [];
      }
    } catch {
      showToast('Failed to load DNS zone', 'error');
    } finally {
      loading = false;
    }
  }

  // ── Zone actions ──────────────────────────────────────────────────────────────

  async function createZone() {
    zoneCreating = true;
    try {
      zone = await api.dns.zones.create({ domain: site.domain, site_id: siteId });
      records = [];
      showToast('DNS zone created', 'success');
    } catch {
      showToast('Failed to create DNS zone', 'error');
    } finally {
      zoneCreating = false;
    }
  }

  async function deleteZone() {
    if (!zone) return;
    try {
      await api.dns.zones.delete(zone.id);
      zone = null;
      records = [];
      confirmDeleteZone = false;
      showToast('DNS zone deleted', 'success');
    } catch {
      showToast('Failed to delete zone', 'error');
    }
  }

  // ── DNS record validation ─────────────────────────────────────────────────────

  function validateDnsValue(v: string, type: string): string {
    if (!v.trim()) return 'Value is required';
    // Reject shell-injection characters
    if (/[<>'"`;&|$\\]/.test(v)) return 'Value contains invalid characters';
    if (v.length > 4096) return 'Value too long (max 4096 characters)';
    return '';
  }

  // ── Record actions ────────────────────────────────────────────────────────────

  // Inline edit error
  let editContentError = '';

  async function saveEdit() {
    if (!zone || !editingRecord) return;
    editContentError = validateDnsValue(editContent, editingRecord.type);
    if (editContentError) { showToast(editContentError, 'error'); return; }
    recordSaving = true;
    try {
      const updated = await api.dns.records.update(zone.id, editingRecord.id, {
        content:  editContent,
        ttl:      editTtl,
        priority: editPriority,
      });
      records = records.map(r => r.id === updated.id ? updated : r);
      editingRecord = null;
      editContentError = '';
      showToast('Record updated', 'success');
    } catch {
      showToast('Failed to update record', 'error');
    } finally {
      recordSaving = false;
    }
  }

  async function deleteRecord(record: DnsRecord) {
    if (!zone) return;
    try {
      await api.dns.records.delete(zone.id, record.id);
      records = records.filter(r => r.id !== record.id);
      confirmDeleteRecord = null;
      showToast('Record deleted', 'success');
    } catch {
      showToast('Failed to delete record', 'error');
    }
  }

  // Add record error
  let newContentError = '';

  async function createRecord() {
    if (!zone) return;
    newContentError = validateDnsValue(newContent, newType);
    if (newContentError) return;
    recordCreating = true;
    try {
      const created = await api.dns.records.create(zone.id, {
        name:     newName || '@',
        type:     newType,
        content:  newContent,
        ttl:      newTtl,
        priority: newPriority,
        disabled: false,
      });
      records = [...records, created];
      showAddRecord = false;
      newName = ''; newContent = ''; newType = 'A'; newTtl = 3600; newPriority = null;
      newContentError = '';
      showToast('Record added', 'success');
    } catch {
      showToast('Failed to add record', 'error');
    } finally {
      recordCreating = false;
    }
  }

  async function checkPropagation(record: DnsRecord) {
    if (!zone) return;
    propagationStatus = { ...propagationStatus, [record.id]: 'checking' };
    try {
      const res = await fetch(`/api/v1/dns/propagation/${zone.zone}/${record.type}`);
      const data = await res.json() as { propagated?: boolean };
      propagationStatus = { ...propagationStatus, [record.id]: data.propagated ? 'ok' : 'pending' };
    } catch {
      propagationStatus = { ...propagationStatus, [record.id]: 'pending' };
    }
  }

  // ── Quick Setup ───────────────────────────────────────────────────────────────

  async function applyEmailAuth() {
    if (!zone) return;
    emailWizardApplying = true;
    try {
      await api.dns.records.create(zone.id, {
        name: '@', type: 'TXT',
        content: `v=spf1 mx a ~all`,
        ttl: 3600, priority: null, disabled: false,
      });
      await api.dns.records.create(zone.id, {
        name: `default._domainkey`, type: 'TXT',
        content: `v=DKIM1; k=rsa; p=YOUR_DKIM_PUBLIC_KEY`,
        ttl: 3600, priority: null, disabled: false,
      });
      await api.dns.records.create(zone.id, {
        name: `_dmarc`, type: 'TXT',
        content: `v=DMARC1; p=none; rua=mailto:dmarc@${site.domain}`,
        ttl: 3600, priority: null, disabled: false,
      });
      records = await api.dns.records.list(zone.id);
      emailWizardDone = true;
      showToast('Email authentication records added', 'success');
    } catch {
      showToast('Failed to add email auth records', 'error');
    } finally {
      emailWizardApplying = false;
    }
  }

  async function addWwwCname() {
    if (!zone) return;
    wwwCreating = true;
    try {
      const created = await api.dns.records.create(zone.id, {
        name: 'www', type: 'CNAME',
        content: site.domain,
        ttl: 3600, priority: null, disabled: false,
      });
      records = [...records, created];
      showToast('WWW CNAME added', 'success');
    } catch {
      showToast('Failed to add CNAME', 'error');
    } finally {
      wwwCreating = false;
    }
  }

  async function addSubdomainA() {
    if (!zone || !subdomainInput.trim()) return;
    subdomainCreating = true;
    try {
      const serverIp = site.server_label ?? '';
      const created = await api.dns.records.create(zone.id, {
        name: subdomainInput.trim(), type: 'A',
        content: serverIp,
        ttl: 3600, priority: null, disabled: false,
      });
      records = [...records, created];
      subdomainInput = '';
      showToast('Subdomain A record added', 'success');
    } catch {
      showToast('Failed to add A record', 'error');
    } finally {
      subdomainCreating = false;
    }
  }

  // ── Modal keyboard handler ────────────────────────────────────────────────────

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      showAddRecord = false;
      showEmailWizard = false;
      cancelEdit();
    }
  }
</script>

<svelte:window on:keydown={handleKeydown} />

<style>
  @keyframes fadeUp { from { opacity:0; transform:translateY(4px) } to { opacity:1; transform:none } }
  .tab-content { animation: fadeUp 0.2s ease-out both }
</style>

<div class="tab-content space-y-5">

  <!-- ── Loading skeleton ──────────────────────────────────────────────────────── -->
  {#if loading}
    <div class="bg-card border border-border rounded-xl p-5 space-y-3 animate-pulse">
      <div class="h-4 bg-muted rounded w-48"></div>
      <div class="h-3 bg-muted rounded w-32"></div>
      <div class="flex gap-2 mt-4">
        <div class="h-9 bg-muted rounded-lg w-28"></div>
        <div class="h-9 bg-muted rounded-lg w-28"></div>
      </div>
    </div>

  <!-- ── No zone — empty state ─────────────────────────────────────────────────── -->
  {:else if !zone}
    <div class="bg-card border border-border rounded-xl p-10 text-center">
      <div class="w-14 h-14 mx-auto mb-4 rounded-full bg-muted flex items-center justify-center">
        <svg class="w-7 h-7 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
          <path stroke-linecap="round" stroke-linejoin="round" d="M12 21a9.004 9.004 0 008.716-6.747M12 21a9.004 9.004 0 01-8.716-6.747M12 21c2.485 0 4.5-4.03 4.5-9S14.485 3 12 3m0 18c-2.485 0-4.5-4.03-4.5-9S9.515 3 12 3m0 0a8.997 8.997 0 017.843 4.582M12 3a8.997 8.997 0 00-7.843 4.582m15.686 0A11.953 11.953 0 0112 10.5c-2.998 0-5.74-1.1-7.843-2.918m15.686 0A8.959 8.959 0 0121 12c0 .778-.099 1.533-.284 2.253m0 0A17.919 17.919 0 0112 16.5c-3.162 0-6.133-.815-8.716-2.247m0 0A9.015 9.015 0 013 12c0-1.605.42-3.113 1.157-4.418" />
        </svg>
      </div>
      <h3 class="text-sm font-semibold text-foreground mb-1">No DNS Zone</h3>
      <p class="text-sm text-muted-foreground mb-5">Manage DNS records without leaving this page</p>
      <button
        on:click={createZone}
        disabled={zoneCreating}
        class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors disabled:opacity-50 disabled:cursor-not-allowed inline-flex items-center gap-2"
      >
        {#if zoneCreating}
          <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
            <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
            <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
          </svg>
          Creating...
        {:else}
          Create DNS Zone for {site.domain}
        {/if}
      </button>
    </div>

  <!-- ── Zone exists ───────────────────────────────────────────────────────────── -->
  {:else}

    <!-- Zone header card -->
    <div class="bg-card border border-border rounded-xl px-5 py-4 flex items-start justify-between gap-4">
      <div>
        <div class="flex items-center gap-2.5 mb-1">
          <h2 class="text-base font-bold text-foreground">{zone.zone}</h2>
          <span class="text-xs text-muted-foreground font-mono">serial: {zone.serial}</span>
        </div>
        <div class="flex items-center gap-3 text-xs text-muted-foreground">
          <span>{records.length} record{records.length !== 1 ? 's' : ''}</span>
          <span>·</span>
          <span>Created {formatDate(zone.created_at)}</span>
        </div>
      </div>
      <div class="flex items-center gap-2 shrink-0">
        <button
          on:click={() => { showAddRecord = !showAddRecord; }}
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors inline-flex items-center gap-1.5"
        >
          <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5">
            <path stroke-linecap="round" stroke-linejoin="round" d="M12 4v16m8-8H4" />
          </svg>
          Add Record
        </button>
        {#if confirmDeleteZone}
          <div class="flex items-center gap-1.5">
            <span class="text-xs text-destructive">All records deleted. Confirm?</span>
            <button class="h-8 px-3 rounded-lg bg-destructive text-white text-xs hover:bg-destructive/90" on:click={deleteZone}>Yes, delete</button>
            <button class="h-8 px-3 rounded-lg border border-border text-xs" on:click={() => confirmDeleteZone = false}>Cancel</button>
          </div>
        {:else}
          <button
            on:click={() => confirmDeleteZone = true}
            class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-red-400 hover:bg-red-500/10 hover:border-red-500/30 transition-colors"
          >
            Delete Zone
          </button>
        {/if}
      </div>
    </div>

    <!-- Add Record modal/panel -->
    {#if showAddRecord}
      <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
      <div
        class="fixed inset-0 z-40 bg-black/50 backdrop-blur-sm flex items-center justify-center p-4"
        on:click|self={() => { showAddRecord = false; }}
      >
        <div class="bg-card border border-border rounded-xl w-full max-w-lg shadow-2xl">
          <div class="px-5 py-4 border-b border-border flex items-center justify-between">
            <h3 class="text-sm font-semibold text-foreground">Add DNS Record</h3>
            <button on:click={() => { showAddRecord = false; }} class="text-muted-foreground hover:text-foreground transition-colors">
              <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
          </div>
          <div class="px-5 py-4 space-y-4">
            <!-- Type + Name row -->
            <div class="grid grid-cols-2 gap-3">
              <div>
                <label class="block text-xs font-medium text-muted-foreground mb-1.5">Type</label>
                <select
                  bind:value={newType}
                  class="w-full h-9 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
                >
                  {#each recordTypes as t}
                    <option value={t}>{t}</option>
                  {/each}
                </select>
              </div>
              <div>
                <label class="block text-xs font-medium text-muted-foreground mb-1.5">Name</label>
                <input
                  bind:value={newName}
                  placeholder="@ or subdomain"
                  class="w-full h-9 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
                />
              </div>
            </div>

            <!-- Content -->
            <div>
              <label class="block text-xs font-medium text-muted-foreground mb-1.5">Content</label>
              <input
                bind:value={newContent}
                on:input={() => { newContentError = ''; }}
                placeholder={contentPlaceholders[newType] ?? 'Value'}
                class="w-full h-9 px-3 rounded-lg border {newContentError ? 'border-red-500' : 'border-border'} bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
              />
              {#if newContentError}
                <p class="mt-1 text-xs text-red-400">{newContentError}</p>
              {:else}
                <p class="mt-1.5 text-xs text-muted-foreground">{contentHelp[newType] ?? ''}</p>
              {/if}
            </div>

            <!-- TTL + Priority row -->
            <div class="grid grid-cols-2 gap-3">
              <div>
                <label class="block text-xs font-medium text-muted-foreground mb-1.5">TTL</label>
                <select
                  bind:value={newTtl}
                  class="w-full h-9 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
                >
                  {#each ttlOptions as opt}
                    <option value={opt.value}>{opt.label}</option>
                  {/each}
                </select>
              </div>
              {#if newType === 'MX' || newType === 'SRV'}
                <div>
                  <label class="block text-xs font-medium text-muted-foreground mb-1.5">Priority</label>
                  <input
                    type="number"
                    bind:value={newPriority}
                    min="0"
                    placeholder="10"
                    class="w-full h-9 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
                  />
                </div>
              {/if}
            </div>
          </div>
          <div class="px-5 py-3.5 border-t border-border flex items-center justify-end gap-2">
            <button
              on:click={() => { showAddRecord = false; }}
              class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted transition-colors"
            >
              Cancel
            </button>
            <button
              on:click={createRecord}
              disabled={recordCreating || !newContent.trim()}
              class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors disabled:opacity-50 disabled:cursor-not-allowed inline-flex items-center gap-2"
            >
              {#if recordCreating}
                <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
                  <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                  <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
                </svg>
                Adding...
              {:else}
                Add Record
              {/if}
            </button>
          </div>
        </div>
      </div>
    {/if}

    <!-- Records table card -->
    <div class="bg-card border border-border rounded-xl overflow-hidden">

      <!-- Table toolbar -->
      <div class="px-4 py-3 border-b border-border flex items-center gap-3 flex-wrap">
        <div class="relative flex-1 min-w-48">
          <svg class="absolute left-3 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
          </svg>
          <input
            type="search"
            bind:value={searchQuery}
            placeholder="Search records..."
            class="w-full h-9 pl-9 pr-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
          />
        </div>
        <button
          on:click={() => { sortType = !sortType; }}
          class="h-9 px-3 rounded-lg border border-border text-xs font-medium text-muted-foreground hover:bg-muted transition-colors inline-flex items-center gap-1.5"
        >
          <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M3 4h13M3 8h9m-9 4h6m4 0l4-4m0 0l4 4m-4-4v12" />
          </svg>
          {sortType ? 'Unsort' : 'Sort by type'}
        </button>
      </div>

      <!-- Table -->
      <div class="overflow-x-auto">
        <table class="w-full text-sm min-w-[640px]">
          <thead>
            <tr class="border-b border-border text-left bg-muted/30">
              <th class="px-4 py-2.5 text-xs font-medium text-muted-foreground uppercase tracking-wide">Type</th>
              <th class="px-4 py-2.5 text-xs font-medium text-muted-foreground uppercase tracking-wide">Name</th>
              <th class="px-4 py-2.5 text-xs font-medium text-muted-foreground uppercase tracking-wide">Content</th>
              <th class="px-4 py-2.5 text-xs font-medium text-muted-foreground uppercase tracking-wide">TTL</th>
              <th class="px-4 py-2.5 text-xs font-medium text-muted-foreground uppercase tracking-wide">Priority</th>
              <th class="px-4 py-2.5 text-xs font-medium text-muted-foreground uppercase tracking-wide sr-only">Actions</th>
            </tr>
          </thead>
          <tbody class="divide-y divide-border">
            {#if filteredRecords.length === 0}
              <tr>
                <td colspan="6" class="px-4 py-10 text-center text-sm text-muted-foreground">
                  {searchQuery ? 'No records match your search.' : 'No DNS records in this zone.'}
                </td>
              </tr>
            {:else}
              {#each filteredRecords as record (record.id)}
                {@const isEditing = editingRecord?.id === record.id}
                {@const propState = propagationStatus[record.id]}
                <tr class="hover:bg-muted/20 transition-colors {record.disabled ? 'opacity-40' : ''}">
                  <!-- Type badge -->
                  <td class="px-4 py-3 whitespace-nowrap">
                    <span class="inline-flex items-center h-5 px-2 rounded text-xs font-medium {typeColors[record.type] ?? 'bg-muted text-muted-foreground'}">
                      {record.type}
                    </span>
                  </td>

                  <!-- Name -->
                  <td class="px-4 py-3 font-mono text-xs text-foreground whitespace-nowrap">{record.name}</td>

                  <!-- Content (editable) -->
                  <td class="px-4 py-3 max-w-[220px]">
                    {#if isEditing}
                      <input
                        bind:value={editContent}
                        class="w-full h-7 px-2 rounded-md border border-ring bg-background text-xs text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
                      />
                    {:else}
                      <span class="font-mono text-xs text-muted-foreground block truncate" title={record.content}>
                        {record.content}
                      </span>
                    {/if}
                  </td>

                  <!-- TTL (editable) -->
                  <td class="px-4 py-3 whitespace-nowrap">
                    {#if isEditing}
                      <input
                        type="number"
                        bind:value={editTtl}
                        min="60"
                        class="w-20 h-7 px-2 rounded-md border border-ring bg-background text-xs text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
                      />
                    {:else}
                      <span class="text-xs text-muted-foreground">{formatTtl(record.ttl)}</span>
                    {/if}
                  </td>

                  <!-- Priority (editable for MX/SRV) -->
                  <td class="px-4 py-3 whitespace-nowrap">
                    {#if isEditing && (record.type === 'MX' || record.type === 'SRV')}
                      <input
                        type="number"
                        bind:value={editPriority}
                        min="0"
                        class="w-16 h-7 px-2 rounded-md border border-ring bg-background text-xs text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
                      />
                    {:else}
                      <span class="text-xs text-muted-foreground">{record.priority ?? '—'}</span>
                    {/if}
                  </td>

                  <!-- Actions -->
                  <td class="px-4 py-3">
                    <div class="flex items-center gap-1.5 justify-end">
                      <!-- Propagation check -->
                      {#if !isEditing}
                        <button
                          on:click={() => checkPropagation(record)}
                          disabled={propState === 'checking'}
                          title="Check propagation"
                          class="h-7 px-2.5 rounded-md text-xs font-medium border border-border text-muted-foreground hover:bg-muted transition-colors disabled:opacity-50 inline-flex items-center gap-1"
                        >
                          {#if propState === 'checking'}
                            <svg class="w-3 h-3 animate-spin" fill="none" viewBox="0 0 24 24">
                              <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                              <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
                            </svg>
                          {:else if propState === 'ok'}
                            <span class="text-green-400">Propagated</span>
                          {:else if propState === 'pending'}
                            <span class="text-amber-400">Pending</span>
                          {:else}
                            Check
                          {/if}
                        </button>
                      {/if}

                      {#if isEditing}
                        <!-- Save -->
                        <button
                          on:click={saveEdit}
                          disabled={recordSaving}
                          title="Save"
                          class="h-7 w-7 rounded-md flex items-center justify-center text-green-400 hover:bg-green-500/10 transition-colors disabled:opacity-50"
                        >
                          {#if recordSaving}
                            <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
                              <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                              <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
                            </svg>
                          {:else}
                            <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5">
                              <path stroke-linecap="round" stroke-linejoin="round" d="M5 13l4 4L19 7" />
                            </svg>
                          {/if}
                        </button>
                        <!-- Cancel -->
                        <button
                          on:click={cancelEdit}
                          title="Cancel"
                          class="h-7 w-7 rounded-md flex items-center justify-center text-muted-foreground hover:bg-muted transition-colors"
                        >
                          <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5">
                            <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12" />
                          </svg>
                        </button>
                      {:else}
                        <!-- Edit -->
                        <button
                          on:click={() => startEdit(record)}
                          title="Edit"
                          class="h-7 w-7 rounded-md flex items-center justify-center text-muted-foreground hover:text-foreground hover:bg-muted transition-colors"
                        >
                          <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                            <path stroke-linecap="round" stroke-linejoin="round" d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z" />
                          </svg>
                        </button>
                        <!-- Delete -->
                        {#if confirmDeleteRecord === record.id}
                          <div class="flex items-center gap-1">
                            <button class="text-xs px-1.5 py-0.5 rounded bg-destructive text-white hover:bg-destructive/90" on:click={() => deleteRecord(record)}>Del</button>
                            <button class="text-xs px-1.5 py-0.5 rounded bg-muted" on:click={() => confirmDeleteRecord = null}>✕</button>
                          </div>
                        {:else}
                          <button
                            on:click={() => confirmDeleteRecord = record.id}
                            title="Delete"
                            class="h-7 w-7 rounded-md flex items-center justify-center text-muted-foreground hover:text-red-400 hover:bg-red-500/10 transition-colors"
                          >
                            <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                              <path stroke-linecap="round" stroke-linejoin="round" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
                            </svg>
                          </button>
                        {/if}
                      {/if}
                    </div>
                  </td>
                </tr>
              {/each}
            {/if}
          </tbody>
        </table>
      </div>
    </div>

    <!-- ── Quick Setup ──────────────────────────────────────────────────────────── -->
    <div class="bg-card border border-border rounded-xl overflow-hidden">
      <div class="px-5 py-3.5 border-b border-border">
        <h3 class="text-sm font-semibold text-foreground">Quick Setup</h3>
        <p class="text-xs text-muted-foreground mt-0.5">One-click wizards to configure common DNS patterns</p>
      </div>
      <div class="p-4 grid grid-cols-1 sm:grid-cols-3 gap-3">

        <!-- Email Authentication -->
        <div class="border border-border rounded-lg p-4 space-y-2">
          <div class="flex items-center gap-2">
            <div class="w-8 h-8 rounded-md bg-purple-500/10 border border-purple-500/20 flex items-center justify-center shrink-0">
              <svg class="w-4 h-4 text-purple-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                <path stroke-linecap="round" stroke-linejoin="round" d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z" />
              </svg>
            </div>
            <p class="text-sm font-medium text-foreground">Email Auth</p>
          </div>
          <p class="text-xs text-muted-foreground">Add SPF, DKIM, and DMARC records for email authentication.</p>
          <button
            on:click={() => { showEmailWizard = true; emailWizardStep = 1; emailWizardDone = false; }}
            class="h-7 px-2.5 rounded-md text-xs font-medium bg-primary text-primary-foreground hover:bg-primary/90 transition-colors w-full"
          >
            Configure
          </button>
        </div>

        <!-- WWW Redirect -->
        <div class="border border-border rounded-lg p-4 space-y-2">
          <div class="flex items-center gap-2">
            <div class="w-8 h-8 rounded-md bg-green-500/10 border border-green-500/20 flex items-center justify-center shrink-0">
              <svg class="w-4 h-4 text-green-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                <path stroke-linecap="round" stroke-linejoin="round" d="M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1" />
              </svg>
            </div>
            <p class="text-sm font-medium text-foreground">WWW Redirect</p>
          </div>
          <p class="text-xs text-muted-foreground">Add CNAME so www.{site.domain} points here.</p>
          <button
            on:click={addWwwCname}
            disabled={wwwCreating}
            class="h-7 px-2.5 rounded-md text-xs font-medium bg-primary text-primary-foreground hover:bg-primary/90 transition-colors disabled:opacity-50 w-full inline-flex items-center justify-center gap-1"
          >
            {#if wwwCreating}
              <svg class="w-3 h-3 animate-spin" fill="none" viewBox="0 0 24 24">
                <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
              </svg>
            {/if}
            Add www CNAME
          </button>
        </div>

        <!-- Parked / Subdomain -->
        <div class="border border-border rounded-lg p-4 space-y-2">
          <div class="flex items-center gap-2">
            <div class="w-8 h-8 rounded-md bg-blue-500/10 border border-blue-500/20 flex items-center justify-center shrink-0">
              <svg class="w-4 h-4 text-blue-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                <path stroke-linecap="round" stroke-linejoin="round" d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2m-2-4h.01M17 16h.01" />
              </svg>
            </div>
            <p class="text-sm font-medium text-foreground">Point Subdomain</p>
          </div>
          <p class="text-xs text-muted-foreground">Create an A record pointing a subdomain to this server.</p>
          <div class="flex gap-1.5">
            <input
              bind:value={subdomainInput}
              placeholder="sub"
              class="flex-1 h-7 px-2 rounded-md border border-border bg-background text-xs text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
            />
            <button
              on:click={addSubdomainA}
              disabled={subdomainCreating || !subdomainInput.trim()}
              class="h-7 px-2.5 rounded-md text-xs font-medium bg-primary text-primary-foreground hover:bg-primary/90 transition-colors disabled:opacity-50 inline-flex items-center gap-1 shrink-0"
            >
              {#if subdomainCreating}
                <svg class="w-3 h-3 animate-spin" fill="none" viewBox="0 0 24 24">
                  <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                  <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
                </svg>
              {/if}
              Add
            </button>
          </div>
        </div>

      </div>
    </div>

  {/if}

</div>

<!-- ── Email Auth Wizard Modal ────────────────────────────────────────────────── -->
{#if showEmailWizard}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div
    class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
    on:click|self={() => { showEmailWizard = false; }}
  >
    <div class="bg-card border border-border rounded-xl w-full max-w-md shadow-2xl">
      <div class="px-5 py-4 border-b border-border flex items-center justify-between">
        <h3 class="text-sm font-semibold text-foreground">Email Authentication Setup</h3>
        <button on:click={() => { showEmailWizard = false; }} class="text-muted-foreground hover:text-foreground transition-colors">
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      {#if emailWizardDone}
        <!-- Success state -->
        <div class="px-5 py-8 text-center space-y-3">
          <div class="w-12 h-12 mx-auto rounded-full bg-green-500/10 border border-green-500/20 flex items-center justify-center">
            <svg class="w-6 h-6 text-green-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M5 13l4 4L19 7" />
            </svg>
          </div>
          <p class="text-sm font-semibold text-foreground">Records added successfully</p>
          <ul class="text-xs text-left text-muted-foreground space-y-1.5 max-w-xs mx-auto">
            <li class="flex items-center gap-2"><span class="text-green-400">✓</span> SPF (TXT) — @</li>
            <li class="flex items-center gap-2"><span class="text-green-400">✓</span> DKIM (TXT) — default._domainkey</li>
            <li class="flex items-center gap-2"><span class="text-green-400">✓</span> DMARC (TXT) — _dmarc</li>
          </ul>
          <p class="text-xs text-muted-foreground">Update the DKIM record with your actual public key after configuring your mail server.</p>
        </div>
        <div class="px-5 py-3.5 border-t border-border flex justify-end">
          <button
            on:click={() => { showEmailWizard = false; }}
            class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors"
          >
            Done
          </button>
        </div>

      {:else if emailWizardStep === 1}
        <!-- Step 1: Preview -->
        <div class="px-5 py-4 space-y-3">
          <p class="text-xs text-muted-foreground">The following DNS records will be added to <strong class="text-foreground">{site.domain}</strong>:</p>
          <div class="space-y-2">
            {#each [
              { type: 'TXT', name: '@',              value: 'v=spf1 mx a ~all',                                   label: 'SPF' },
              { type: 'TXT', name: 'default._domainkey', value: 'v=DKIM1; k=rsa; p=YOUR_DKIM_PUBLIC_KEY',        label: 'DKIM' },
              { type: 'TXT', name: '_dmarc',          value: `v=DMARC1; p=none; rua=mailto:dmarc@${site.domain}`, label: 'DMARC' },
            ] as row}
              <div class="bg-muted/40 rounded-lg px-3 py-2.5">
                <div class="flex items-center gap-2 mb-0.5">
                  <span class="inline-flex items-center h-5 px-1.5 rounded text-xs font-medium {typeColors.TXT}">{row.type}</span>
                  <span class="text-xs font-medium text-foreground">{row.label}</span>
                  <span class="font-mono text-xs text-muted-foreground">— {row.name}</span>
                </div>
                <p class="font-mono text-xs text-muted-foreground truncate">{row.value}</p>
              </div>
            {/each}
          </div>
        </div>
        <div class="px-5 py-3.5 border-t border-border flex items-center justify-between">
          <button
            on:click={() => { showEmailWizard = false; }}
            class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted transition-colors"
          >
            Cancel
          </button>
          <button
            on:click={() => { emailWizardStep = 2; }}
            class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors"
          >
            Continue
          </button>
        </div>

      {:else}
        <!-- Step 2: Confirm & apply -->
        <div class="px-5 py-5 space-y-2">
          <p class="text-sm text-foreground font-medium">Apply these records?</p>
          <p class="text-xs text-muted-foreground">This will create 3 TXT records in the <strong class="text-foreground">{site.domain}</strong> zone. You can edit or delete them afterwards.</p>
        </div>
        <div class="px-5 py-3.5 border-t border-border flex items-center justify-between">
          <button
            on:click={() => { emailWizardStep = 1; }}
            class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted transition-colors"
          >
            Back
          </button>
          <button
            on:click={applyEmailAuth}
            disabled={emailWizardApplying}
            class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors disabled:opacity-50 disabled:cursor-not-allowed inline-flex items-center gap-2"
          >
            {#if emailWizardApplying}
              <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
                <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
              </svg>
              Applying...
            {:else}
              Apply Records
            {/if}
          </button>
        </div>
      {/if}
    </div>
  </div>
{/if}

<!-- ── Toast ──────────────────────────────────────────────────────────────────── -->
{#if toast}
  <div
    class="fixed bottom-4 right-4 z-[60] flex items-center gap-3 px-4 py-3 rounded-xl shadow-2xl border transition-all duration-300
      {toast.type === 'success'
        ? 'bg-green-950/90 border-green-800 text-green-300'
        : 'bg-red-950/90 border-red-800 text-red-300'}"
    role="alert"
    aria-live="polite"
  >
    {#if toast.type === 'success'}
      <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round" d="M5 13l4 4L19 7" />
      </svg>
    {:else}
      <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
      </svg>
    {/if}
    <span class="text-sm font-medium">{toast.message}</span>
    <button
      on:click={() => { toast = null; }}
      class="text-current opacity-60 hover:opacity-100 transition-opacity"
      aria-label="Dismiss"
    >
      <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12" />
      </svg>
    </button>
  </div>
{/if}
