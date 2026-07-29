<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/i18n';
  import { api } from '$api/client';
  import type { DnsZone, DnsRecord } from '$api/client';

  // ── State ──────────────────────────────────────────────────────────────────
  let zones: DnsZone[] = [];
  let loading = true;
  let loadError = '';

  // Selected zone (left panel)
  let selectedZoneId: string | null = null;

  // Records per zone cache: zoneId → DnsRecord[]
  let recordsCache: Record<string, DnsRecord[]> = {};
  let recordsLoading: Record<string, boolean> = {};

  // Right-panel record filter
  let recordSearch = '';
  let recordTypeFilter = '';

  // Inline add-record form
  let showAddRecordForm = false;
  let newRecName = '';
  let newRecType = 'A';
  let newRecContent = '';
  let newRecTtl = 3600;
  let newRecPriority = '';
  let addRecLoading = false;
  let addRecError = '';

  // Inline edit: recordId → draft copy
  let editingRecordId: string | null = null;
  let editName = '';
  let editType = '';
  let editContent = '';
  let editTtl = 3600;
  let editPriority = '';
  let editLoading = false;
  let editError = '';

  // Delete record confirmation
  let deleteRecordTarget: DnsRecord | null = null;
  let deleteRecordLoading = false;

  // Add Zone modal
  let showAddZoneModal = false;
  let newZoneDomain = '';
  let newZoneIp = '';
  let createDefaultRecords = true;
  let addZoneLoading = false;
  let addZoneError = '';

  // Delete zone confirmation
  let deleteZoneTarget: DnsZone | null = null;
  let deleteZoneLoading = false;

  // Propagation check per zone: zoneId → { checking, results }
  let propCheckLoading: Record<string, boolean> = {};
  let propCheckResults: Record<string, { google: boolean; cloudflare: boolean; quad9: boolean }> = {};

  // Quick Setup template modal
  let showTemplateModal = false;
  let templateType: 'wordpress' | 'email' | 'redirect' | 'subdomain' | null = null;
  let templateDomain = '';
  let templateIp = '';
  let templateSubdomain = '';
  let templateLoading = false;
  let templateError = '';

  // Bulk Import modal
  let showImportModal = false;
  let importZoneFile = '';
  let importTargetZone = '';
  let importParsed: Array<{ name: string; type: string; content: string; ttl: number }> = [];
  let importParseError = '';
  let importLoading = false;

  // Toast
  let toastMessage = '';
  let toastType: 'success' | 'error' = 'success';

  // Zone search (left panel)
  let zoneSearch = '';

  // ── Lifecycle ──────────────────────────────────────────────────────────────
  onMount(async () => {
    await loadZones();
  });

  async function loadZones() {
    loading = true;
    loadError = '';
    try {
      zones = await api.dns.zones.list();
      // Auto-select first zone
      if (zones.length > 0) {
        selectedZoneId = zones[0].id;
        await loadRecords(selectedZoneId);
      }
      // Preload rest in background for health indicators
      Promise.all(zones.slice(1).map(z => loadRecords(z.id)));
    } catch (err: unknown) {
      loadError = (err as { message?: string })?.message ?? 'Failed to load DNS zones';
    } finally {
      loading = false;
    }
  }

  async function loadRecords(zoneId: string) {
    if (recordsCache[zoneId]) return;
    recordsLoading = { ...recordsLoading, [zoneId]: true };
    try {
      const recs = await api.dns.records.list(zoneId);
      recordsCache = { ...recordsCache, [zoneId]: recs };
    } catch {
      recordsCache = { ...recordsCache, [zoneId]: [] };
    } finally {
      recordsLoading = { ...recordsLoading, [zoneId]: false };
    }
  }

  async function selectZone(zoneId: string) {
    if (selectedZoneId === zoneId) return;
    selectedZoneId = zoneId;
    recordSearch = '';
    recordTypeFilter = '';
    showAddRecordForm = false;
    editingRecordId = null;
    await loadRecords(zoneId);
  }

  // ── Derived ────────────────────────────────────────────────────────────────
  $: filteredZones = zoneSearch
    ? zones.filter(z => z.zone.toLowerCase().includes(zoneSearch.toLowerCase()))
    : zones;

  $: selectedZone = zones.find(z => z.id === selectedZoneId) ?? null;

  $: selectedRecords = selectedZoneId ? (recordsCache[selectedZoneId] ?? []) : [];

  $: filteredRecords = selectedRecords.filter(r => {
    const matchesType   = !recordTypeFilter || r.type === recordTypeFilter;
    const matchesSearch = !recordSearch ||
      r.name.toLowerCase().includes(recordSearch.toLowerCase()) ||
      r.content.toLowerCase().includes(recordSearch.toLowerCase());
    return matchesType && matchesSearch;
  });

  $: uniqueTypes = [...new Set(selectedRecords.map(r => r.type))].sort();

  $: totalRecords = Object.values(recordsCache).reduce((sum, recs) => sum + recs.length, 0);

  $: zonesWithIssues = zones.filter(z => {
    const recs = recordsCache[z.id] ?? [];
    return !hasMx(recs) || !hasSpf(recs) || !hasDmarc(recs);
  });

  // ── Health helpers ─────────────────────────────────────────────────────────
  function hasMx(recs: DnsRecord[]): boolean {
    return recs.some(r => r.type === 'MX');
  }
  function hasSpf(recs: DnsRecord[]): boolean {
    return recs.some(r => r.type === 'TXT' && (r.content ?? '').includes('v=spf1'));
  }
  function hasDmarc(recs: DnsRecord[]): boolean {
    return recs.some(r => r.type === 'TXT' && (r.content ?? '').includes('v=DMARC1'));
  }
  function hasDkim(recs: DnsRecord[]): boolean {
    return recs.some(r => r.type === 'TXT' && (r.name ?? '').includes('_domainkey'));
  }
  function hasA(recs: DnsRecord[]): boolean {
    return recs.some(r => r.type === 'A');
  }

  function healthScore(zoneId: string): number {
    const recs = recordsCache[zoneId] ?? [];
    let score = 100;
    if (!hasA(recs))     score -= 20;
    if (!hasMx(recs))    score -= 20;
    if (!hasSpf(recs))   score -= 20;
    if (!hasDmarc(recs)) score -= 20;
    if (!hasDkim(recs))  score -= 20;
    return Math.max(0, score);
  }

  function healthDotClass(zoneId: string): string {
    const score = healthScore(zoneId);
    if (score >= 80) return 'bg-green-500';
    if (score >= 50) return 'bg-amber-500';
    return 'bg-red-500';
  }

  // ── Type badge colors ──────────────────────────────────────────────────────
  const typeColors: Record<string, string> = {
    A:     'bg-blue-500/10 text-blue-400 border-blue-500/20',
    AAAA:  'bg-indigo-500/10 text-indigo-400 border-indigo-500/20',
    CNAME: 'bg-purple-500/10 text-purple-400 border-purple-500/20',
    MX:    'bg-orange-500/10 text-orange-400 border-orange-500/20',
    TXT:   'bg-green-500/10 text-green-400 border-green-500/20',
    NS:    'bg-gray-500/10 text-gray-400 border-gray-500/20',
    CAA:   'bg-red-500/10 text-red-400 border-red-500/20',
    SRV:   'bg-cyan-500/10 text-cyan-400 border-cyan-500/20',
  };

  function typeColor(type: string): string {
    return typeColors[type] ?? 'bg-muted text-muted-foreground border-border';
  }

  // ── Toast ──────────────────────────────────────────────────────────────────
  function showToast(msg: string, type: 'success' | 'error') {
    toastMessage = msg;
    toastType = type;
    setTimeout(() => { toastMessage = ''; }, 4000);
  }

  // ── Validation ─────────────────────────────────────────────────────────────
  function validateZoneDomain(d: string): string {
    d = d.trim().toLowerCase().replace(/^https?:\/\//, '').replace(/\/$/, '');
    if (!d) return 'Domain is required';
    if (d.length > 253) return 'Domain too long (max 253 characters)';
    if (!/^[a-z0-9]([a-z0-9-]*[a-z0-9])?(\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)+$/.test(d))
      return 'Invalid domain format (e.g. example.com)';
    for (const label of d.split('.')) {
      if (label.length > 63) return 'Domain label too long (max 63 chars per segment)';
    }
    return '';
  }

  function validateIpv4(ip: string): string {
    if (!ip) return '';
    if (!/^(\d{1,3}\.){3}\d{1,3}$/.test(ip.trim())) return 'Invalid IPv4 address format';
    const parts = ip.trim().split('.').map(Number);
    if (parts.some(p => p > 255)) return 'IP address octets must be 0–255';
    return '';
  }

  // ── Add Zone ───────────────────────────────────────────────────────────────
  async function handleAddZone(e: SubmitEvent) {
    e.preventDefault();
    addZoneError = '';
    newZoneDomain = newZoneDomain.trim().toLowerCase().replace(/^https?:\/\//, '').replace(/\/$/, '');
    const domainErr = validateZoneDomain(newZoneDomain);
    if (domainErr) { addZoneError = domainErr; return; }
    if (newZoneIp) {
      const ipErr = validateIpv4(newZoneIp);
      if (ipErr) { addZoneError = ipErr; return; }
    }
    addZoneLoading = true;
    try {
      const token = typeof localStorage !== 'undefined' ? localStorage.getItem('orbit_access_token') : null;
      const createRes = await fetch('/api/v1/dns/zones', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          ...(token ? { 'Authorization': `Bearer ${token}` } : {}),
        },
        body: JSON.stringify({ domain: newZoneDomain.toLowerCase().trim() }),
      });
      if (!createRes.ok) {
        const body = await createRes.json().catch(() => ({})) as { message?: string };
        throw new Error(body.message ?? `HTTP ${createRes.status}`);
      }
      const zone = await createRes.json() as DnsZone;
      zones = [...zones, zone];
      recordsCache = { ...recordsCache, [zone.id]: [] };

      if (createDefaultRecords) {
        try {
          await fetch(`/api/v1/dns/zones/${zone.id}/email-auth`, {
            method: 'POST',
            headers: {
              'Authorization': `Bearer ${localStorage.getItem('orbit_access_token')}`,
              'Content-Type': 'application/json',
            },
          });
        } catch { /* non-fatal */ }
        delete recordsCache[zone.id];
        await loadRecords(zone.id);
      }

      showAddZoneModal = false;
      newZoneDomain = '';
      newZoneIp = '';
      // Select the newly created zone
      await selectZone(zone.id);
      showToast(`Zone ${zone.zone} created`, 'success');
    } catch (err: unknown) {
      const e2 = err as { message?: string };
      addZoneError = e2.message ?? 'Failed to create zone';
    } finally {
      addZoneLoading = false;
    }
  }

  // ── Delete Zone ────────────────────────────────────────────────────────────
  async function handleDeleteZone() {
    if (!deleteZoneTarget) return;
    deleteZoneLoading = true;
    try {
      await api.dns.zones.delete(deleteZoneTarget.id);
      const id = deleteZoneTarget.id;
      zones = zones.filter(z => z.id !== id);
      const { [id]: _, ...rest } = recordsCache;
      recordsCache = rest;
      deleteZoneTarget = null;
      // Re-select first available
      if (selectedZoneId === id) {
        selectedZoneId = zones.length > 0 ? zones[0].id : null;
        if (selectedZoneId) await loadRecords(selectedZoneId);
      }
      showToast('Zone deleted', 'success');
    } catch {
      showToast('Failed to delete zone', 'error');
    } finally {
      deleteZoneLoading = false;
    }
  }

  // ── Add Record ─────────────────────────────────────────────────────────────
  async function handleAddRecord() {
    if (!selectedZoneId) return;
    addRecError = '';
    if (!newRecName.trim()) { addRecError = 'Name is required'; return; }
    if (!newRecContent.trim()) { addRecError = 'Content is required'; return; }
    addRecLoading = true;
    try {
      const payload: { name: string; type: string; content: string; ttl: number; priority?: number } = {
        name: newRecName.trim(),
        type: newRecType,
        content: newRecContent.trim(),
        ttl: newRecTtl,
      };
      if (newRecPriority !== '') payload.priority = parseInt(newRecPriority, 10);
      await api.dns.records.create(selectedZoneId, payload);
      // Reload records
      delete recordsCache[selectedZoneId];
      await loadRecords(selectedZoneId);
      // Reset form
      showAddRecordForm = false;
      newRecName = '';
      newRecType = 'A';
      newRecContent = '';
      newRecTtl = 3600;
      newRecPriority = '';
      showToast('Record created', 'success');
    } catch (err: unknown) {
      addRecError = (err as { message?: string })?.message ?? 'Failed to create record';
    } finally {
      addRecLoading = false;
    }
  }

  // ── Edit Record ────────────────────────────────────────────────────────────
  function startEdit(rec: DnsRecord) {
    editingRecordId = rec.id;
    editName    = rec.name;
    editType    = rec.type;
    editContent = rec.content;
    editTtl     = rec.ttl;
    editPriority = rec.priority != null ? String(rec.priority) : '';
    editError   = '';
    showAddRecordForm = false;
  }

  function cancelEdit() {
    editingRecordId = null;
    editError = '';
  }

  async function handleSaveEdit(rec: DnsRecord) {
    if (!selectedZoneId) return;
    editError = '';
    if (!editName.trim()) { editError = 'Name is required'; return; }
    if (!editContent.trim()) { editError = 'Content is required'; return; }
    editLoading = true;
    try {
      const payload: { name: string; type: string; content: string; ttl: number; priority?: number } = {
        name: editName.trim(),
        type: editType,
        content: editContent.trim(),
        ttl: editTtl,
      };
      if (editPriority !== '') payload.priority = parseInt(editPriority, 10);
      await api.dns.records.update(selectedZoneId, rec.id, payload);
      delete recordsCache[selectedZoneId];
      await loadRecords(selectedZoneId);
      editingRecordId = null;
      showToast('Record updated', 'success');
    } catch (err: unknown) {
      editError = (err as { message?: string })?.message ?? 'Failed to update record';
    } finally {
      editLoading = false;
    }
  }

  // ── Delete Record ──────────────────────────────────────────────────────────
  async function handleDeleteRecord() {
    if (!deleteRecordTarget || !selectedZoneId) return;
    deleteRecordLoading = true;
    try {
      await api.dns.records.delete(selectedZoneId, deleteRecordTarget.id);
      delete recordsCache[selectedZoneId];
      await loadRecords(selectedZoneId);
      deleteRecordTarget = null;
      showToast('Record deleted', 'success');
    } catch {
      showToast('Failed to delete record', 'error');
    } finally {
      deleteRecordLoading = false;
    }
  }

  // ── Propagation check ──────────────────────────────────────────────────────
  async function checkPropagation(zone: DnsZone) {
    propCheckLoading = { ...propCheckLoading, [zone.id]: true };
    try {
      const res = await fetch(`/api/v1/dns/zones/${zone.id}/check-propagation`, {
        method: 'POST',
        headers: { 'Authorization': `Bearer ${localStorage.getItem('orbit_access_token')}` },
      });
      if (res.ok) {
        const results = await res.json() as Array<{ nameserver: string; resolved: boolean }>;
        const google     = results.find(r => r.nameserver === '8.8.8.8')?.resolved   ?? false;
        const cloudflare = results.find(r => r.nameserver === '1.1.1.1')?.resolved   ?? false;
        const quad9      = results.find(r => r.nameserver === '9.9.9.9')?.resolved   ?? false;
        propCheckResults = { ...propCheckResults, [zone.id]: { google, cloudflare, quad9 } };
        showToast('Propagation check complete', 'success');
      } else {
        showToast('Propagation check failed', 'error');
      }
    } catch {
      showToast('Propagation check failed', 'error');
    } finally {
      propCheckLoading = { ...propCheckLoading, [zone.id]: false };
    }
  }

  // ── Template modal ─────────────────────────────────────────────────────────
  const templates = [
    { id: 'wordpress' as const, label: 'WordPress Site',       icon: 'M12 2C6.477 2 2 6.477 2 12s4.477 10 10 10 10-4.477 10-10S17.523 2 12 2zm0 18c-4.411 0-8-3.589-8-8s3.589-8 8-8 8 3.589 8 8-3.589 8-8 8z', desc: 'A, CNAME(www), MX, SPF, DMARC' },
    { id: 'email' as const,     label: 'Email Only',           icon: 'M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z', desc: 'MX, SPF, DKIM, DMARC' },
    { id: 'redirect' as const,  label: 'Redirect to WWW',      icon: 'M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1', desc: 'A, CNAME(www→@)' },
    { id: 'subdomain' as const, label: 'Subdomain Delegation', icon: 'M8 9l3 3-3 3m5 0h3M5 20h14a2 2 0 002-2V6a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z', desc: 'NS records for subdomain' },
  ];

  function openTemplate(type: typeof templateType) {
    templateType = type;
    templateDomain = selectedZone?.zone ?? '';
    templateIp = '';
    templateSubdomain = '';
    templateError = '';
    showTemplateModal = true;
  }

  async function applyTemplate() {
    if (!templateDomain.trim()) { templateError = 'Domain is required'; return; }
    if ((templateType === 'wordpress' || templateType === 'redirect') && !templateIp.trim()) {
      templateError = 'Server IP is required for this template'; return;
    }
    if (templateIp.trim()) {
      const ipErr = validateIpv4(templateIp.trim());
      if (ipErr) { templateError = ipErr; return; }
    }
    if (templateType === 'subdomain' && templateSubdomain.trim()) {
      if (!/^[a-z0-9]([a-z0-9-]*[a-z0-9])?$/.test(templateSubdomain.trim())) {
        templateError = 'Subdomain may only contain lowercase letters, numbers, and hyphens';
        return;
      }
    }
    templateLoading = true;
    templateError = '';
    try {
      const zone = zones.find(z => z.zone === templateDomain.trim().toLowerCase());
      if (!zone) { templateError = `No zone found for "${templateDomain}". Add the zone first.`; return; }
      const token = typeof localStorage !== 'undefined' ? localStorage.getItem('orbit_access_token') : null;
      const headers = { 'Content-Type': 'application/json', ...(token ? { 'Authorization': `Bearer ${token}` } : {}) };
      const base = `/api/v1/dns/zones/${zone.id}/records`;

      if (templateType === 'wordpress') {
        await Promise.all([
          fetch(base, { method: 'POST', headers, body: JSON.stringify({ name: '@', type: 'A', content: templateIp, ttl: 3600 }) }),
          fetch(base, { method: 'POST', headers, body: JSON.stringify({ name: 'www', type: 'CNAME', content: '@', ttl: 3600 }) }),
          fetch(base, { method: 'POST', headers, body: JSON.stringify({ name: '@', type: 'MX', content: `mail.${templateDomain}`, priority: 10, ttl: 3600 }) }),
          fetch(base, { method: 'POST', headers, body: JSON.stringify({ name: '@', type: 'TXT', content: `v=spf1 ip4:${templateIp} ~all`, ttl: 3600 }) }),
          fetch(base, { method: 'POST', headers, body: JSON.stringify({ name: '_dmarc', type: 'TXT', content: 'v=DMARC1; p=none; rua=mailto:postmaster@' + templateDomain, ttl: 3600 }) }),
        ]);
      } else if (templateType === 'email') {
        await Promise.all([
          fetch(base, { method: 'POST', headers, body: JSON.stringify({ name: '@', type: 'MX', content: `mail.${templateDomain}`, priority: 10, ttl: 3600 }) }),
          fetch(base, { method: 'POST', headers, body: JSON.stringify({ name: '@', type: 'TXT', content: 'v=spf1 mx ~all', ttl: 3600 }) }),
          fetch(base, { method: 'POST', headers, body: JSON.stringify({ name: 'mail._domainkey', type: 'TXT', content: 'v=DKIM1; k=rsa; p=REPLACE_WITH_YOUR_DKIM_KEY', ttl: 3600 }) }),
          fetch(base, { method: 'POST', headers, body: JSON.stringify({ name: '_dmarc', type: 'TXT', content: 'v=DMARC1; p=quarantine; rua=mailto:postmaster@' + templateDomain, ttl: 3600 }) }),
        ]);
      } else if (templateType === 'redirect') {
        await Promise.all([
          fetch(base, { method: 'POST', headers, body: JSON.stringify({ name: '@', type: 'A', content: templateIp, ttl: 3600 }) }),
          fetch(base, { method: 'POST', headers, body: JSON.stringify({ name: 'www', type: 'CNAME', content: '@', ttl: 3600 }) }),
        ]);
      } else if (templateType === 'subdomain') {
        const sub = templateSubdomain.trim() || 'sub';
        await fetch(base, { method: 'POST', headers, body: JSON.stringify({ name: sub, type: 'NS', content: `ns1.${templateDomain}`, ttl: 3600 }) });
      }
      delete recordsCache[zone.id];
      await loadRecords(zone.id);
      showTemplateModal = false;
      showToast('Template applied successfully', 'success');
    } catch {
      templateError = 'Failed to apply template';
    } finally {
      templateLoading = false;
    }
  }

  // ── Bulk Import ────────────────────────────────────────────────────────────
  function parseBINDZoneFile(text: string): Array<{ name: string; type: string; content: string; ttl: number }> {
    const results: Array<{ name: string; type: string; content: string; ttl: number }> = [];
    const lines = text.split('\n');
    for (const raw of lines) {
      const line = raw.trim();
      if (!line || line.startsWith(';') || line.startsWith('$')) continue;
      const parts = line.split(/\s+/);
      if (parts.length < 4) continue;
      let idx = 0;
      const name = parts[idx++];
      let ttl = 3600;
      if (/^\d+$/.test(parts[idx])) { ttl = parseInt(parts[idx++], 10); }
      if (parts[idx] === 'IN') idx++;
      const type = parts[idx++];
      const content = parts.slice(idx).join(' ').replace(/"/g, '').replace(/\s*;.*$/, '');
      const knownTypes = ['A', 'AAAA', 'CNAME', 'MX', 'TXT', 'NS', 'SRV', 'CAA', 'PTR'];
      if (!knownTypes.includes(type)) continue;
      results.push({ name, type, content: content.trim(), ttl });
    }
    return results;
  }

  function handleParseImport() {
    importParseError = '';
    if (!importZoneFile.trim()) { importParseError = 'Paste a BIND zone file first'; return; }
    try {
      importParsed = parseBINDZoneFile(importZoneFile);
      if (importParsed.length === 0) importParseError = 'No recognizable records found';
    } catch {
      importParseError = 'Failed to parse zone file';
    }
  }

  async function handleDoImport() {
    if (!importTargetZone) { importParseError = 'Select a target zone'; return; }
    importLoading = true;
    importParseError = '';
    const zone = zones.find(z => z.id === importTargetZone);
    if (!zone) { importLoading = false; return; }
    const token = typeof localStorage !== 'undefined' ? localStorage.getItem('orbit_access_token') : null;
    const headers = { 'Content-Type': 'application/json', ...(token ? { 'Authorization': `Bearer ${token}` } : {}) };
    try {
      await Promise.all(importParsed.map(rec =>
        fetch(`/api/v1/dns/zones/${zone.id}/records`, { method: 'POST', headers, body: JSON.stringify(rec) })
      ));
      const count = importParsed.length;
      delete recordsCache[zone.id];
      await loadRecords(zone.id);
      showImportModal = false;
      importZoneFile = '';
      importParsed = [];
      showToast(`Imported ${count} records`, 'success');
    } catch {
      importParseError = 'Some records failed to import';
    } finally {
      importLoading = false;
    }
  }
</script>

<svelte:head>
  <title>{$t('dns.title')} — JottiCP</title>
</svelte:head>

<!-- ── Page wrapper ─────────────────────────────────────────────────────────── -->
<div class="flex flex-col h-full min-h-0" style="height: calc(100vh - 4rem);">

  <!-- ── Top header bar ──────────────────────────────────────────────────────── -->
  <div class="flex items-center justify-between px-4 lg:px-6 py-3 border-b border-border bg-background shrink-0">
    <div>
      <h1 class="text-lg font-semibold text-foreground">{$t('dns.title')}</h1>
      {#if !loading}
        <p class="text-xs text-muted-foreground">
          {zones.length} {zones.length === 1 ? $t('dns.zone_single') : $t('dns.zones')} · {$t('dns.total_records')}: {totalRecords}
          {#if zonesWithIssues.length > 0}
            · <span class="text-amber-400">{$t('dns.with_issues', { values: { count: zonesWithIssues.length } })}</span>
          {:else}
            · <span class="text-green-400">{$t('dns.all_healthy')}</span>
          {/if}
        </p>
      {/if}
    </div>
    <div class="flex items-center gap-2">
      <button
        on:click={() => { showTemplateModal = true; templateType = 'wordpress'; templateDomain = selectedZone?.zone ?? ''; templateIp = ''; templateSubdomain = ''; templateError = ''; }}
        class="h-8 px-3 rounded-lg border border-border text-xs text-muted-foreground
               hover:bg-muted hover:text-foreground inline-flex items-center gap-1.5 transition-all duration-150 active:scale-95"
      >
        <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 10V3L4 14h7v7l9-11h-7z"/>
        </svg>
        {$t('dns.templates')}
      </button>
      <button
        on:click={() => { showImportModal = true; importZoneFile = ''; importParsed = []; importParseError = ''; importTargetZone = selectedZoneId ?? ''; }}
        class="h-8 px-3 rounded-lg border border-border text-xs text-muted-foreground
               hover:bg-muted hover:text-foreground inline-flex items-center gap-1.5 transition-all duration-150 active:scale-95"
      >
        <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
            d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-8l-4-4m0 0L8 8m4-4v12"/>
        </svg>
        {$t('dns.import')}
      </button>
      <button
        on:click={() => { showAddZoneModal = true; addZoneError = ''; newZoneDomain = ''; newZoneIp = ''; }}
        class="h-8 px-4 rounded-lg bg-primary text-primary-foreground text-xs font-medium
               hover:bg-primary/90 inline-flex items-center gap-1.5 transition-all duration-150 active:scale-95"
      >
        <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4"/>
        </svg>
        {$t('dns.add_zone')}
      </button>
    </div>
  </div>

  <!-- ── Load error banner ───────────────────────────────────────────────────── -->
  {#if loadError}
    <div class="flex items-center gap-3 mx-4 mt-3 px-4 py-3 rounded-xl
                bg-red-500/10 border border-red-500/20 text-sm text-red-400">
      <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round"
          d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"/>
      </svg>
      <span>{loadError}</span>
      <button on:click={() => void loadZones()}
              class="ml-auto text-xs font-semibold underline underline-offset-2 hover:no-underline">
        Retry
      </button>
    </div>
  {/if}

  <!-- ── Split-view body ─────────────────────────────────────────────────────── -->
  {#if loading}
    <div class="flex-1 flex items-center justify-center">
      <div class="flex items-center gap-3 text-sm text-muted-foreground">
        <svg class="w-5 h-5 animate-spin" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
            d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/>
        </svg>{$t('dns.loading_zones')}</div>
    </div>
  {:else}
    <!-- On mobile: flex-col (zones on top, records below). On md+: flex-row. -->
    <div class="flex-1 flex flex-col md:flex-row min-h-0 overflow-hidden">

      <!-- ══ LEFT PANEL — Zone List ══════════════════════════════════════════════ -->
      <div class="w-full md:w-[280px] shrink-0 flex flex-col border-b md:border-b-0 md:border-r border-border bg-card overflow-hidden">
        <!-- Zone search -->
        <div class="p-3 border-b border-border">
          <div class="relative">
            <svg class="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground"
                 fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"/>
            </svg>
            <input type="search" bind:value={zoneSearch} placeholder={$t('dns.search_zones')}
                   class="w-full h-8 pl-8 pr-3 rounded-md border border-border bg-background text-xs
                          text-foreground placeholder:text-muted-foreground
                          focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"/>
          </div>
        </div>

        <!-- Zone list -->
        <div class="flex-1 overflow-y-auto">
          {#if filteredZones.length === 0}
            <div class="p-6 text-center">
              <p class="text-xs text-muted-foreground">
                {zoneSearch ? 'No zones match your search' : 'No DNS zones configured'}
              </p>
              {#if !zoneSearch}
                <button
                  on:click={() => { showAddZoneModal = true; addZoneError = ''; }}
                  class="mt-3 h-8 px-3 rounded-lg bg-primary text-primary-foreground text-xs font-medium
                         hover:bg-primary/90 inline-flex items-center gap-1.5 transition-all duration-150 active:scale-95"
                >
                  <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4"/>
                  </svg>{$t('dns.add_first_zone')}</button>
              {/if}
            </div>
          {:else}
            <ul class="py-1">
              {#each filteredZones as zone}
                {@const isSelected = selectedZoneId === zone.id}
                {@const recs       = recordsCache[zone.id] ?? []}
                {@const prop       = propCheckResults[zone.id]}
                <li>
                  <button
                    on:click={() => selectZone(zone.id)}
                    class="w-full text-left px-3 py-2.5 flex items-center gap-2.5 transition-colors duration-100
                           {isSelected
                             ? 'bg-primary/10 border-l-2 border-primary'
                             : 'hover:bg-muted border-l-2 border-transparent'}"
                  >
                    <!-- Health dot -->
                    <span class="w-2 h-2 rounded-full shrink-0 {healthDotClass(zone.id)}"></span>

                    <!-- Domain + count -->
                    <div class="flex-1 min-w-0">
                      <p class="text-xs font-medium truncate {isSelected ? 'text-primary' : 'text-foreground'}">
                        {zone.zone}
                      </p>
                      <p class="text-[10px] text-muted-foreground mt-0.5">
                        {recordsCache[zone.id] ? recs.length + ' record' + (recs.length !== 1 ? 's' : '') : 'Loading…'}
                      </p>
                    </div>

                    <!-- Propagation dots if checked -->
                    {#if prop}
                      <span class="flex items-center gap-0.5 shrink-0" title="Google / CF / Quad9">
                        <span class="w-1.5 h-1.5 rounded-full {prop.google ? 'bg-green-500' : 'bg-red-500'}"></span>
                        <span class="w-1.5 h-1.5 rounded-full {prop.cloudflare ? 'bg-green-500' : 'bg-red-500'}"></span>
                        <span class="w-1.5 h-1.5 rounded-full {prop.quad9 ? 'bg-green-500' : 'bg-red-500'}"></span>
                      </span>
                    {/if}
                  </button>
                </li>
              {/each}
            </ul>
          {/if}
        </div>

        <!-- Health summary footer in left panel -->
        {#if zones.length > 0}
          <div class="border-t border-border px-3 py-2 text-[10px] text-muted-foreground">
            {#if zonesWithIssues.length > 0}
              <span class="text-amber-400 font-medium">{zonesWithIssues.length} zone{zonesWithIssues.length !== 1 ? 's' : ''} need attention</span>
              — missing MX / SPF / DMARC
            {:else}
              <span class="text-green-400 font-medium">{$t('dns.all_zones_healthy')}</span>
            {/if}
          </div>
        {/if}
      </div>

      <!-- ══ RIGHT PANEL — Records ════════════════════════════════════════════════ -->
      <div class="flex-1 flex flex-col min-h-0 min-w-0 bg-background overflow-hidden">
        {#if !selectedZone}
          <div class="flex-1 flex items-center justify-center text-sm text-muted-foreground">{$t('dns.select_a_zone_to_view_its_records')}</div>
        {:else}
          <!-- Records panel header -->
          <div class="flex flex-col sm:flex-row sm:items-center gap-2 px-4 py-3 border-b border-border bg-card shrink-0">
            <!-- Left: zone title + actions -->
            <div class="flex items-center gap-3 flex-1 min-w-0">
              <div class="min-w-0">
                <p class="text-sm font-semibold text-foreground truncate">{$t('dns.records_for')}<span class="text-primary">{selectedZone.zone}</span>
                </p>
                {#if propCheckResults[selectedZone.id]}
                  {@const p = propCheckResults[selectedZone.id]}
                  <p class="text-[10px] text-muted-foreground mt-0.5 flex items-center gap-2">{$t('dns.dns_propagation')}<span class="{p.google     ? 'text-green-400' : 'text-red-400'}">
                      {p.google ? '✓' : '✗'} Google
                    </span>
                    <span class="{p.cloudflare ? 'text-green-400' : 'text-red-400'}">
                      {p.cloudflare ? '✓' : '✗'} CF
                    </span>
                    <span class="{p.quad9      ? 'text-green-400' : 'text-red-400'}">
                      {p.quad9 ? '✓' : '✗'} Quad9
                    </span>
                  </p>
                {/if}
              </div>
            </div>

            <!-- Right: toolbar -->
            <div class="flex items-center gap-2 shrink-0 flex-wrap">
              <!-- Type filter -->
              <select
                bind:value={recordTypeFilter}
                class="h-8 rounded-md border border-border bg-background px-2 text-xs text-foreground
                       focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
              >
                <option value="">{$t('dns.all_types')}</option>
                {#each uniqueTypes as t}
                  <option value={t}>{t}</option>
                {/each}
              </select>

              <!-- Record search -->
              <div class="relative">
                <svg class="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground"
                     fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                    d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"/>
                </svg>
                <input type="search" bind:value={recordSearch} placeholder="Search records…"
                       class="h-8 w-40 pl-8 pr-3 rounded-md border border-border bg-background text-xs
                              text-foreground placeholder:text-muted-foreground
                              focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"/>
              </div>

              <!-- Propagation check -->
              <button
                on:click={() => checkPropagation(selectedZone)}
                disabled={propCheckLoading[selectedZone.id]}
                title="Check propagation"
                class="h-8 px-2.5 rounded-md border border-border text-xs text-muted-foreground
                       hover:bg-muted hover:text-foreground inline-flex items-center gap-1.5
                       transition-all duration-150 active:scale-95 disabled:opacity-50"
              >
                <svg class="w-3.5 h-3.5 {propCheckLoading[selectedZone.id] ? 'animate-spin' : ''}"
                     fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                    d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/>
                </svg>
                <span class="hidden sm:inline">{$t('dns.check_prop')}</span>
              </button>

              <!-- Delete zone -->
              <button
                on:click={() => deleteZoneTarget = selectedZone}
                title="Delete this zone"
                class="h-8 px-2.5 rounded-md bg-red-500/10 text-red-400 border border-red-500/20
                       inline-flex items-center gap-1.5 text-xs hover:bg-red-500/20
                       transition-all duration-150 active:scale-95"
              >
                <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                    d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"/>
                </svg>
                <span class="hidden sm:inline">{$t('dns.delete_zone')}</span>
              </button>

              <!-- Add record -->
              <button
                on:click={() => { showAddRecordForm = !showAddRecordForm; editingRecordId = null; addRecError = ''; }}
                class="h-8 px-3 rounded-md bg-primary text-primary-foreground text-xs font-medium
                       hover:bg-primary/90 inline-flex items-center gap-1.5
                       transition-all duration-150 active:scale-95"
              >
                <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4"/>
                </svg>{$t('dns.add_record')}</button>
            </div>
          </div>

          <!-- Scrollable table area -->
          <div class="flex-1 overflow-auto">

            {#if recordsLoading[selectedZone.id]}
              <div class="p-6 space-y-2">
                {#each [1,2,3,4] as _}
                  <div class="h-9 bg-muted rounded animate-pulse"></div>
                {/each}
              </div>
            {:else}
              <!-- ── Inline Add Record form (slide-down) ──────────────────────── -->
              {#if showAddRecordForm}
                <div class="border-b border-border bg-primary/5 px-4 py-3 fade-up">
                  <p class="text-xs font-semibold text-foreground mb-2">{$t('dns.new_record')}</p>
                  <div class="flex flex-wrap items-end gap-2">
                    <!-- Type -->
                    <div class="flex flex-col gap-1">
                      <label class="text-[10px] text-muted-foreground uppercase tracking-wide">{$t('common.type')}</label>
                      <select bind:value={newRecType}
                              class="h-8 rounded-md border border-border bg-background px-2 text-xs text-foreground
                                     focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary">
                        {#each ['A','AAAA','CNAME','MX','TXT','NS','CAA','SRV'] as t}
                          <option value={t}>{t}</option>
                        {/each}
                      </select>
                    </div>
                    <!-- Name -->
                    <div class="flex flex-col gap-1 flex-1 min-w-[80px]">
                      <label class="text-[10px] text-muted-foreground uppercase tracking-wide">{$t('common.name')}</label>
                      <input bind:value={newRecName} placeholder="@ or subdomain"
                             class="h-8 rounded-md border border-border bg-background px-2 text-xs font-mono text-foreground
                                    placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"/>
                    </div>
                    <!-- Content -->
                    <div class="flex flex-col gap-1 flex-[3] min-w-[140px]">
                      <label class="text-[10px] text-muted-foreground uppercase tracking-wide">{$t('dns.content')}</label>
                      <input bind:value={newRecContent} placeholder="IP, hostname, or value"
                             class="h-8 rounded-md border border-border bg-background px-2 text-xs font-mono text-foreground
                                    placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"/>
                    </div>
                    <!-- TTL -->
                    <div class="flex flex-col gap-1 w-20">
                      <label class="text-[10px] text-muted-foreground uppercase tracking-wide">{$t('dns.ttl')}</label>
                      <input type="number" bind:value={newRecTtl} min="60"
                             class="h-8 rounded-md border border-border bg-background px-2 text-xs text-foreground
                                    focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"/>
                    </div>
                    <!-- Priority (MX/SRV) -->
                    {#if newRecType === 'MX' || newRecType === 'SRV'}
                      <div class="flex flex-col gap-1 w-16">
                        <label class="text-[10px] text-muted-foreground uppercase tracking-wide">{$t('dns.prio')}</label>
                        <input type="number" bind:value={newRecPriority} placeholder="10" min="0"
                               class="h-8 rounded-md border border-border bg-background px-2 text-xs text-foreground
                                      focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"/>
                      </div>
                    {/if}
                    <!-- Actions -->
                    <div class="flex items-end gap-1.5 pb-0">
                      <button
                        on:click={handleAddRecord}
                        disabled={addRecLoading}
                        class="h-8 px-3 rounded-md bg-primary text-primary-foreground text-xs font-medium
                               hover:bg-primary/90 disabled:opacity-50 transition-all duration-150 active:scale-95"
                      >
                        {addRecLoading ? 'Saving…' : 'Save'}
                      </button>
                      <button
                        on:click={() => { showAddRecordForm = false; addRecError = ''; }}
                        class="h-8 px-3 rounded-md border border-border text-xs text-muted-foreground
                               hover:bg-muted hover:text-foreground transition-colors"
                      >
                        Cancel
                      </button>
                    </div>
                  </div>
                  {#if addRecError}
                    <p class="mt-2 text-xs text-red-400">{addRecError}</p>
                  {/if}
                </div>
              {/if}

              <!-- ── Records table ─────────────────────────────────────────── -->
              {#if filteredRecords.length === 0}
                <div class="flex flex-col items-center justify-center py-16 text-sm text-muted-foreground">
                  <svg class="w-8 h-8 mb-3 opacity-40" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                      d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/>
                  </svg>
                  {#if recordSearch || recordTypeFilter}
                    <p>{$t('dns.no_records_filter')}</p>
                    <button
                      on:click={() => { recordSearch = ''; recordTypeFilter = ''; }}
                      class="mt-2 text-xs text-primary hover:underline"
                    >{$t("dns.clear_filters")}</button>
                  {:else}
                    <p>{$t('dns.no_records_zone')}</p>
                    <button
                      on:click={() => { showAddRecordForm = true; }}
                      class="mt-3 h-8 px-3 rounded-md bg-primary text-primary-foreground text-xs font-medium
                             hover:bg-primary/90 inline-flex items-center gap-1.5 transition-all duration-150 active:scale-95"
                    >
                      <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4"/>
                      </svg>{$t('dns.add_first_record')}</button>
                  {/if}
                </div>
              {:else}
                <div class="overflow-x-auto">
                <table class="w-full text-sm border-collapse">
                  <thead class="sticky top-0 z-10 bg-muted/80 backdrop-blur-sm">
                    <tr>
                      <th class="px-4 py-2.5 text-left text-[10px] font-semibold text-muted-foreground uppercase tracking-wider w-20">{$t('common.type')}</th>
                      <th class="px-4 py-2.5 text-left text-[10px] font-semibold text-muted-foreground uppercase tracking-wider w-32">{$t('common.name')}</th>
                      <th class="px-4 py-2.5 text-left text-[10px] font-semibold text-muted-foreground uppercase tracking-wider">{$t('dns.content')}</th>
                      <th class="px-4 py-2.5 text-left text-[10px] font-semibold text-muted-foreground uppercase tracking-wider w-20 hidden sm:table-cell">{$t('dns.ttl')}</th>
                      <th class="px-4 py-2.5 text-right text-[10px] font-semibold text-muted-foreground uppercase tracking-wider w-24">{$t('common.actions')}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {#each filteredRecords as rec (rec.id)}
                      {#if editingRecordId === rec.id}
                        <!-- ── Inline edit row ──────────────────────────────── -->
                        <tr class="border-t border-border bg-primary/5">
                          <!-- Type -->
                          <td class="px-3 py-2">
                            <select bind:value={editType}
                                    class="h-8 w-full rounded-md border border-border bg-background px-1.5 text-xs text-foreground
                                           focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary">
                              {#each ['A','AAAA','CNAME','MX','TXT','NS','CAA','SRV'] as t}
                                <option value={t}>{t}</option>
                              {/each}
                            </select>
                          </td>
                          <!-- Name -->
                          <td class="px-3 py-2">
                            <input bind:value={editName}
                                   class="h-8 w-full rounded-md border border-border bg-background px-2 text-xs font-mono text-foreground
                                          focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"/>
                          </td>
                          <!-- Content -->
                          <td class="px-3 py-2">
                            <div class="flex items-center gap-2">
                              <input bind:value={editContent}
                                     class="h-8 flex-1 rounded-md border border-border bg-background px-2 text-xs font-mono text-foreground
                                            focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"/>
                              {#if editType === 'MX' || editType === 'SRV'}
                                <input type="number" bind:value={editPriority} placeholder="Prio" min="0"
                                       class="h-8 w-14 rounded-md border border-border bg-background px-2 text-xs text-foreground
                                              focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"/>
                              {/if}
                            </div>
                          </td>
                          <!-- TTL -->
                          <td class="px-3 py-2 hidden sm:table-cell">
                            <input type="number" bind:value={editTtl} min="60"
                                   class="h-8 w-full rounded-md border border-border bg-background px-2 text-xs text-foreground
                                          focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"/>
                          </td>
                          <!-- Actions -->
                          <td class="px-3 py-2 text-right">
                            <div class="flex items-center justify-end gap-1.5">
                              <button
                                on:click={() => handleSaveEdit(rec)}
                                disabled={editLoading}
                                class="h-7 px-2.5 rounded-md bg-primary text-primary-foreground text-xs font-medium
                                       hover:bg-primary/90 disabled:opacity-50 transition-all duration-150 active:scale-95"
                              >{editLoading ? '…' : 'Save'}</button>
                              <button
                                on:click={cancelEdit}
                                class="h-7 px-2 rounded-md border border-border text-xs text-muted-foreground
                                       hover:bg-muted hover:text-foreground transition-colors"
                              >✕</button>
                            </div>
                            {#if editError}
                              <p class="mt-1 text-[10px] text-red-400 text-right">{editError}</p>
                            {/if}
                          </td>
                        </tr>
                      {:else}
                        <!-- ── Normal record row ────────────────────────────── -->
                        <tr class="border-t border-border hover:bg-muted/30 transition-colors duration-100 group">
                          <td class="px-4 py-2.5">
                            <span class="inline-flex items-center px-2 py-0.5 rounded text-[10px] font-semibold
                                         border {typeColor(rec.type)}">{rec.type}</span>
                          </td>
                          <td class="px-4 py-2.5 font-mono text-xs text-foreground max-w-[120px]">
                            <span class="truncate block" title={rec.name}>{rec.name}</span>
                          </td>
                          <td class="px-4 py-2.5 font-mono text-xs text-muted-foreground">
                            <span class="truncate block max-w-xs" title={rec.content}>
                              {rec.content}{rec.priority != null ? ` (pri: ${rec.priority})` : ''}
                            </span>
                          </td>
                          <td class="px-4 py-2.5 text-xs text-muted-foreground hidden sm:table-cell whitespace-nowrap">
                            {rec.ttl}s
                          </td>
                          <td class="px-4 py-2.5 text-right">
                            <div class="flex items-center justify-end gap-1 opacity-0 group-hover:opacity-100 transition-opacity duration-100">
                              <button
                                on:click={() => startEdit(rec)}
                                title="Edit record"
                                class="h-7 w-7 rounded-md border border-border inline-flex items-center justify-center
                                       text-muted-foreground hover:bg-muted hover:text-foreground transition-colors"
                              >
                                <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                                    d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z"/>
                                </svg>
                              </button>
                              <button
                                on:click={() => deleteRecordTarget = rec}
                                title="Delete record"
                                class="h-7 w-7 rounded-md bg-red-500/10 text-red-400 border border-red-500/20
                                       inline-flex items-center justify-center hover:bg-red-500/20 transition-colors"
                              >
                                <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                                    d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"/>
                                </svg>
                              </button>
                            </div>
                          </td>
                        </tr>
                      {/if}
                    {/each}
                  </tbody>
                </table>
                </div>
              {/if}

              <!-- ── Zone health issues for selected zone ───────────────────── -->
              {#if selectedZoneId && zonesWithIssues.some(z => z.id === selectedZoneId)}
                {@const recs = recordsCache[selectedZoneId] ?? []}
                <div class="mx-4 my-3 rounded-lg border border-amber-500/20 bg-amber-500/5 px-4 py-3">
                  <p class="text-xs font-semibold text-amber-400 mb-2">{$t('dns.health_issues')}</p>
                  <div class="space-y-1 text-xs text-amber-400/80">
                    {#if !hasMx(recs)}<p>{$t('dns.missing_mx')}</p>{/if}
                    {#if !hasSpf(recs)}<p>{$t('dns.missing_spf')}</p>{/if}
                    {#if !hasDmarc(recs)}<p>{$t('dns.missing_dmarc')}</p>{/if}
                  </div>
                  <button
                    on:click={async () => {
                      try {
                        await fetch(`/api/v1/dns/zones/${selectedZoneId}/email-auth`, {
                          method: 'POST',
                          headers: {
                            'Authorization': `Bearer ${localStorage.getItem('orbit_access_token')}`,
                            'Content-Type': 'application/json',
                          },
                        });
                        delete recordsCache[selectedZoneId];
                        await loadRecords(selectedZoneId);
                        showToast('Email auth records created', 'success');
                      } catch {
                        showToast('Failed to create records', 'error');
                      }
                    }}
                    class="mt-2 h-7 px-3 rounded-md bg-primary text-primary-foreground text-xs font-medium
                           hover:bg-primary/90 inline-flex items-center gap-1.5 transition-all duration-150 active:scale-95"
                  >
                    <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 10V3L4 14h7v7l9-11h-7z"/>
                    </svg>{$t('dns.autofix')}</button>
                </div>
              {/if}
            {/if}
          </div>
          <!-- /scrollable table area -->
        {/if}
      </div>
      <!-- /RIGHT PANEL -->

    </div>
    <!-- /split-view body -->
  {/if}

</div>

<!-- ── Delete Record Confirmation ─────────────────────────────────────────────── -->
{#if deleteRecordTarget}
  <div
    class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
    role="presentation"
    aria-hidden="true"
    on:click|self={() => { if (!deleteRecordLoading) deleteRecordTarget = null; }}
  >
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-sm shadow-2xl fade-up"
         role="dialog" aria-modal="true">
      <div class="flex items-start gap-3 mb-5">
        <div class="w-9 h-9 rounded-full bg-red-500/10 flex items-center justify-center shrink-0">
          <svg class="w-4 h-4 text-red-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"/>
          </svg>
        </div>
        <div>
          <h2 class="text-sm font-semibold text-foreground">{$t('dns.delete_record_title')}</h2>
          <p class="text-xs text-muted-foreground mt-1">{$t('common.delete')}<span class="font-mono text-foreground">{deleteRecordTarget.type} {deleteRecordTarget.name}</span>{$t('dns.this_action_cannot_be_undone')}</p>
        </div>
      </div>
      <div class="flex gap-2">
        <button
          on:click={handleDeleteRecord}
          disabled={deleteRecordLoading}
          class="flex-1 h-9 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-sm font-medium
                 hover:bg-red-500/20 inline-flex items-center justify-center gap-2
                 transition-all duration-150 active:scale-95 disabled:opacity-50"
        >
          {deleteRecordLoading ? 'Deleting…' : 'Delete Record'}
        </button>
        <button
          on:click={() => deleteRecordTarget = null}
          disabled={deleteRecordLoading}
          class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground
                 hover:bg-muted hover:text-foreground transition-colors disabled:opacity-50"
        >{$t("common.cancel")}</button>
      </div>
    </div>
  </div>
{/if}

<!-- ── Quick Setup Template Modal ──────────────────────────────────────────────── -->
{#if showTemplateModal}
  <div
    class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
    role="presentation"
    aria-hidden="true"
    on:click|self={() => { if (!templateLoading) showTemplateModal = false; }}
  >
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-lg shadow-2xl fade-up"
         role="dialog" aria-modal="true" aria-label="Quick Setup Template">
      <div class="flex items-center justify-between mb-5">
        <div>
          <h2 class="text-base font-semibold text-foreground">{$t('dns.template_title')}</h2>
          <p class="text-xs text-muted-foreground mt-0.5">{$t('dns.template_desc')}</p>
        </div>
        <button on:click={() => showTemplateModal = false} disabled={templateLoading}
                class="w-8 h-8 rounded-lg flex items-center justify-center text-muted-foreground hover:bg-muted hover:text-foreground transition-colors">
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
          </svg>
        </button>
      </div>

      <div class="space-y-4">
        <!-- Template type selector -->
        <div class="grid grid-cols-2 gap-2">
          {#each templates as tpl}
            <button
              on:click={() => { templateType = tpl.id; templateError = ''; }}
              class="flex flex-col items-start gap-1 rounded-lg border px-3 py-2.5 text-left transition-all duration-150 active:scale-95
                     {templateType === tpl.id ? 'border-primary bg-primary/10' : 'border-border hover:bg-muted hover:border-primary/30'}"
            >
              <span class="text-xs font-medium {templateType === tpl.id ? 'text-primary' : 'text-foreground'}">{tpl.label}</span>
              <span class="text-[10px] text-muted-foreground leading-snug">{tpl.desc}</span>
            </button>
          {/each}
        </div>

        <!-- Domain -->
        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5" for="tpl-domain">{$t('dns.target_domain')}</label>
          <select id="tpl-domain" bind:value={templateDomain}
                  class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground
                         focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary">
            <option value="">{$t('dns.select_zone')}</option>
            {#each zones as z}
              <option value={z.zone}>{z.zone}</option>
            {/each}
          </select>
        </div>

        {#if templateType === 'wordpress' || templateType === 'redirect'}
          <div>
            <label class="block text-sm font-medium text-foreground mb-1.5" for="tpl-ip">{$t('dns.server_ip')}</label>
            <input id="tpl-ip" type="text" bind:value={templateIp} placeholder="203.0.113.1"
                   class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground font-mono
                          focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"/>
          </div>
        {/if}
        {#if templateType === 'subdomain'}
          <div>
            <label class="block text-sm font-medium text-foreground mb-1.5" for="tpl-sub">{$t('dns.subdomain')}</label>
            <input id="tpl-sub" type="text" bind:value={templateSubdomain} placeholder="app"
                   class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground font-mono
                          focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"/>
          </div>
        {/if}

        {#if templateError}
          <div role="alert" class="text-sm text-red-400 bg-red-500/10 border border-red-500/20 rounded-lg px-3 py-2.5">
            {templateError}
          </div>
        {/if}

        <div class="flex gap-2 pt-1">
          <button on:click={applyTemplate} disabled={templateLoading}
                  class="flex-1 h-9 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                         hover:bg-primary/90 transition-all duration-150 active:scale-95 disabled:opacity-50
                         inline-flex items-center justify-center gap-2">
            {#if templateLoading}
              <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                  d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/>
              </svg>
              Applying…
            {:else}
              Apply Template
            {/if}
          </button>
          <button on:click={() => showTemplateModal = false} disabled={templateLoading}
                  class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground
                         hover:bg-muted hover:text-foreground transition-colors disabled:opacity-50">
            Cancel
          </button>
        </div>
      </div>
    </div>
  </div>
{/if}

<!-- ── Bulk Import Modal ────────────────────────────────────────────────────────── -->
{#if showImportModal}
  <div
    class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
    role="presentation"
    aria-hidden="true"
    on:click|self={() => { if (!importLoading) showImportModal = false; }}
  >
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-2xl shadow-2xl fade-up"
         role="dialog" aria-modal="true" aria-label="Import Zone File">
      <div class="flex items-center justify-between mb-5">
        <div>
          <h2 class="text-base font-semibold text-foreground">{$t('dns.import_title')}</h2>
          <p class="text-xs text-muted-foreground mt-0.5">{$t('dns.import_desc')}</p>
        </div>
        <button on:click={() => showImportModal = false} disabled={importLoading}
                class="w-8 h-8 rounded-lg flex items-center justify-center text-muted-foreground hover:bg-muted hover:text-foreground transition-colors">
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
          </svg>
        </button>
      </div>

      <div class="space-y-4">
        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5" for="imp-zone">{$t('dns.target_zone')}</label>
          <select id="imp-zone" bind:value={importTargetZone}
                  class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground
                         focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary">
            <option value="">{$t('dns.select_zone')}</option>
            {#each zones as z}
              <option value={z.id}>{z.zone}</option>
            {/each}
          </select>
        </div>

        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5" for="imp-file">{$t('dns.zone_file_content')}</label>
          <textarea id="imp-file" bind:value={importZoneFile} rows="8"
                    placeholder="; BIND zone file&#10;example.com.  3600  IN  A  203.0.113.1&#10;www           3600  IN  CNAME  @"
                    class="w-full rounded-lg border border-border bg-background px-3 py-2 text-sm text-foreground font-mono
                           placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50
                           focus:border-primary resize-none"></textarea>
        </div>

        <button on:click={handleParseImport}
                class="h-8 px-3 rounded-lg border border-border text-xs text-muted-foreground
                       hover:bg-muted hover:text-foreground transition-all duration-150 active:scale-95
                       inline-flex items-center gap-1.5">
          <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M15 12a3 3 0 11-6 0 3 3 0 016 0z M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z"/>
          </svg>{$t('dns.preview_records')}</button>

        {#if importParseError}
          <div role="alert" class="text-sm text-red-400 bg-red-500/10 border border-red-500/20 rounded-lg px-3 py-2.5">
            {importParseError}
          </div>
        {/if}

        {#if importParsed.length > 0}
          <div class="rounded-lg border border-border overflow-x-auto">
            <div class="bg-muted/50 px-3 py-2 text-xs text-muted-foreground font-medium">
              {importParsed.length} record{importParsed.length === 1 ? '' : 's'} parsed — ready to import
            </div>
            <div class="max-h-48 overflow-y-auto">
              <table class="w-full text-xs">
                <thead class="bg-muted/30 sticky top-0">
                  <tr>
                    <th class="px-3 py-1.5 text-left text-muted-foreground">{$t('common.name')}</th>
                    <th class="px-3 py-1.5 text-left text-muted-foreground">{$t('common.type')}</th>
                    <th class="px-3 py-1.5 text-left text-muted-foreground">{$t('dns.content')}</th>
                    <th class="px-3 py-1.5 text-left text-muted-foreground">{$t('dns.ttl')}</th>
                  </tr>
                </thead>
                <tbody>
                  {#each importParsed as rec}
                    <tr class="border-t border-border hover:bg-muted/20 transition-colors duration-150">
                      <td class="px-3 py-1.5 font-mono text-foreground">{rec.name}</td>
                      <td class="px-3 py-1.5">
                        <span class="px-1.5 py-0.5 rounded text-[10px] font-medium border {typeColor(rec.type)}">{rec.type}</span>
                      </td>
                      <td class="px-3 py-1.5 font-mono text-muted-foreground truncate max-w-[180px]">{rec.content}</td>
                      <td class="px-3 py-1.5 text-muted-foreground">{rec.ttl}s</td>
                    </tr>
                  {/each}
                </tbody>
              </table>
            </div>
          </div>
        {/if}

        <div class="flex gap-2 pt-1">
          <button on:click={handleDoImport}
                  disabled={importLoading || importParsed.length === 0 || !importTargetZone}
                  class="flex-1 h-9 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                         hover:bg-primary/90 transition-all duration-150 active:scale-95 disabled:opacity-50
                         inline-flex items-center justify-center gap-2">
            {#if importLoading}
              <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                  d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/>
              </svg>
              Importing…
            {:else}
              Import {importParsed.length > 0 ? `${importParsed.length} Records` : 'Records'}
            {/if}
          </button>
          <button on:click={() => showImportModal = false} disabled={importLoading}
                  class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground
                         hover:bg-muted hover:text-foreground transition-colors disabled:opacity-50">
            Cancel
          </button>
        </div>
      </div>
    </div>
  </div>
{/if}

<!-- ── Add Zone Modal ──────────────────────────────────────────────────────────── -->
{#if showAddZoneModal}
  <div
    class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
    role="presentation"
    aria-hidden="true"
    on:click|self={() => showAddZoneModal = false}
  >
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-lg shadow-2xl fade-up"
         role="dialog" aria-modal="true" aria-label="Add DNS Zone">
      <div class="flex items-center justify-between mb-5">
        <h2 class="text-base font-semibold text-foreground">{$t('dns.add_dns_zone')}</h2>
        <button on:click={() => showAddZoneModal = false}
                class="w-8 h-8 rounded-lg flex items-center justify-center text-muted-foreground hover:bg-muted hover:text-foreground transition-colors">
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
          </svg>
        </button>
      </div>

      <form on:submit={handleAddZone} class="space-y-4">
        <div>
          <label for="zone-domain" class="block text-sm font-medium text-foreground mb-1.5">{$t('dns.domain_name')}</label>
          <input id="zone-domain" type="text" bind:value={newZoneDomain} placeholder="example.com"
                 required autocomplete="off"
                 class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm
                        text-foreground placeholder:text-muted-foreground
                        focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"/>
        </div>

        <div>
          <label for="zone-ip" class="block text-sm font-medium text-foreground mb-1.5">{$t('dns.server_ip')}<span class="text-muted-foreground font-normal">{$t('dns.optional_used_for_default_a_record')}</span>
          </label>
          <input id="zone-ip" type="text" bind:value={newZoneIp} placeholder="192.0.2.1" autocomplete="off"
                 class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm
                        text-foreground placeholder:text-muted-foreground
                        focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"/>
        </div>

        <label class="flex items-start gap-2.5 cursor-pointer rounded-lg border border-border p-3 hover:bg-muted/30 transition-colors">
          <input type="checkbox" bind:checked={createDefaultRecords} class="mt-0.5 accent-primary"/>
          <div>
            <p class="text-sm font-medium text-foreground">{$t('dns.create_default_records')}</p>
            <p class="text-xs text-muted-foreground mt-0.5">{$t('dns.create_default_records_desc')}</p>
          </div>
        </label>

        {#if addZoneError}
          <div role="alert" class="text-sm text-red-400 bg-red-500/10 rounded-lg px-3 py-2.5 border border-red-500/20">
            {addZoneError}
          </div>
        {/if}

        <div class="flex gap-2 pt-1">
          <button type="submit" disabled={addZoneLoading}
                  class="flex-1 h-9 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                         hover:bg-primary/90 transition-all duration-150 active:scale-95 disabled:opacity-50">
            {addZoneLoading ? 'Creating…' : 'Create Zone'}
          </button>
          <button type="button" on:click={() => showAddZoneModal = false}
                  class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground
                         hover:bg-muted hover:text-foreground transition-colors">
            Cancel
          </button>
        </div>
      </form>
    </div>
  </div>
{/if}

<!-- ── Delete Zone Modal ────────────────────────────────────────────────────────── -->
{#if deleteZoneTarget}
  <div
    class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
    role="presentation"
    aria-hidden="true"
    on:click|self={() => { if (!deleteZoneLoading) deleteZoneTarget = null; }}
  >
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-md shadow-2xl fade-up"
         role="dialog" aria-modal="true">
      <div class="flex items-start gap-3 mb-5">
        <div class="w-10 h-10 rounded-full bg-red-500/10 flex items-center justify-center shrink-0">
          <svg class="w-5 h-5 text-red-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"/>
          </svg>
        </div>
        <div>
          <h2 class="text-base font-semibold text-foreground">{$t('dns.delete_zone_confirm_title')}</h2>
          <p class="text-sm text-muted-foreground mt-0.5">{$t('common.delete')}<span class="font-mono text-foreground">{deleteZoneTarget.zone}</span>{$t('dns.and_all_its_records_this_action_cannot_b')}</p>
        </div>
      </div>
      <div class="flex gap-2">
        <button on:click={handleDeleteZone} disabled={deleteZoneLoading}
                class="flex-1 h-9 px-4 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20
                       text-sm font-medium hover:bg-red-500/20 inline-flex items-center justify-center gap-2
                       transition-all duration-150 active:scale-95 disabled:opacity-50">
          {deleteZoneLoading ? 'Deleting…' : 'Delete Zone'}
        </button>
        <button on:click={() => deleteZoneTarget = null} disabled={deleteZoneLoading}
                class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground
                       hover:bg-muted hover:text-foreground transition-colors disabled:opacity-50">
          Cancel
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- ── Toast ───────────────────────────────────────────────────────────────────── -->
{#if toastMessage}
  <div class="fixed bottom-4 right-4 z-50 bg-card border border-border rounded-xl px-4 py-3 text-sm shadow-lg
              flex items-center gap-2 fade-up"
       role="status" aria-live="polite">
    {#if toastType === 'success'}
      <svg class="w-4 h-4 text-green-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>
      </svg>
    {:else}
      <svg class="w-4 h-4 text-red-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
      </svg>
    {/if}
    <span class="text-foreground">{toastMessage}</span>
  </div>
{/if}

<style>
  @keyframes fadeUp {
    from { opacity: 0; transform: translateY(6px) }
    to   { opacity: 1; transform: none }
  }
  .fade-up { animation: fadeUp 0.2s ease-out both }
</style>
