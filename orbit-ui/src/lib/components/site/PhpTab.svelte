<script lang="ts">
  import { onMount, createEventDispatcher } from 'svelte';
  import { api } from '$api/client';
  import type { Site } from '$api/client';

  export let siteId: string;
  export let site: Site;

  const dispatch = createEventDispatcher<{ siteUpdated: void }>();

  // ── Types ──────────────────────────────────────────────────────────────────

  interface OpcacheStats {
    enabled: boolean; hit_rate: number;
    memory_used_mb: number; memory_free_mb: number;
    cached_scripts: number; jit_enabled: boolean; jit_buffer_size: number;
  }
  interface PhpExtension {
    name: string; enabled: boolean; description: string;
    dev_only: boolean; category: string; special_note?: string;
  }
  interface Toast { message: string; type: 'success' | 'error' }

  interface PhpInfoRow { key: string; value: string; localValue?: string }
  interface PhpInfoSection { id: string; title: string; rows: PhpInfoRow[] }
  interface PhpInfoFiltered extends PhpInfoSection { filteredRows: PhpInfoRow[] }
  interface ParsedPhpInfo { version: string; buildDate: string; sapi: string; sections: PhpInfoSection[] }

  type SettingType = 'bytes' | 'seconds' | 'integer' | 'toggle' | 'select';
  interface SettingDef {
    key: string; label: string; desc: string; type: SettingType;
    min?: number; max?: number; step?: number;
    options?: { v: string; l: string }[];
    default: string;
  }

  // ── Setting groups definition ──────────────────────────────────────────────

  const GROUPS: { id: string; label: string; settings: SettingDef[] }[] = [
    {
      id: 'performance', label: 'Performance',
      settings: [
        { key: 'memory_limit',       label: 'Memory Limit',       desc: 'Max memory a script may consume',                 type: 'bytes',   min: 32,  max: 2048, step: 32,  default: '128M' },
        { key: 'max_execution_time', label: 'Max Execution Time', desc: 'Max seconds a script may run (0 = no limit)',     type: 'seconds', min: 10,  max: 600,  step: 10,  default: '30' },
        { key: 'max_input_time',     label: 'Max Input Time',     desc: 'Max seconds to parse request data',               type: 'seconds', min: 10,  max: 600,  step: 10,  default: '60' },
      ]
    },
    {
      id: 'upload', label: 'File Upload',
      settings: [
        { key: 'upload_max_filesize', label: 'Upload File Size Limit', desc: 'Maximum size of a single uploaded file',     type: 'bytes',   min: 1,   max: 2048, step: 1,   default: '2M' },
        { key: 'post_max_size',       label: 'POST Max Size',          desc: 'Max POST data size (≥ upload limit)',         type: 'bytes',   min: 8,   max: 2048, step: 8,   default: '8M' },
        { key: 'max_file_uploads',    label: 'Max Files Per Request',  desc: 'Number of files allowed in one upload',      type: 'integer', min: 1,   max: 200,              default: '20' },
      ]
    },
    {
      id: 'vars', label: 'Variables',
      settings: [
        { key: 'max_input_vars', label: 'Max Input Variables', desc: 'Max number of POST/GET/cookie variables accepted',   type: 'integer', min: 500, max: 50000, step: 500, default: '1000' },
      ]
    },
    {
      id: 'errors', label: 'Error Handling',
      settings: [
        { key: 'display_errors',  label: 'Display Errors',  desc: 'Show errors on page (disable in production)',          type: 'toggle', default: 'Off' },
        { key: 'log_errors',      label: 'Log Errors',      desc: 'Write errors to the server error log',                 type: 'toggle', default: 'On' },
        { key: 'error_reporting', label: 'Error Level',     desc: 'Which classes of errors are reported',                 type: 'select',
          options: [
            { v: 'production',  l: 'Production — E_ALL & ~E_DEPRECATED & ~E_STRICT' },
            { v: 'development', l: 'Development — E_ALL (all errors shown)' },
            { v: 'strict',      l: 'Strict — E_ALL | E_STRICT' },
          ], default: 'production' },
      ]
    },
    {
      id: 'session', label: 'Session',
      settings: [
        { key: 'session.gc_maxlifetime', label: 'Session Lifetime',    desc: 'Seconds before session data is garbage-collected', type: 'seconds', min: 300, max: 86400, step: 300, default: '1440' },
        { key: 'session.save_handler',   label: 'Session Storage',     desc: 'Where session data is persisted',                  type: 'select',
          options: [{ v: 'files', l: 'Files (default)' }, { v: 'redis', l: 'Redis (recommended)' }], default: 'files' },
        { key: 'session.cookie_httponly', label: 'HTTP-Only Cookies',  desc: 'Block JavaScript from reading session cookie',      type: 'toggle', default: '1' },
        { key: 'session.use_strict_mode', label: 'Strict Session Mode',desc: 'Reject unrecognised session IDs (anti-fixation)',   type: 'toggle', default: '1' },
      ]
    },
    {
      id: 'opcache_conf', label: 'OPcache',
      settings: [
        { key: 'opcache.enable',              label: 'Enable OPcache',        desc: 'Cache compiled PHP bytecode for better performance', type: 'toggle',  default: '1' },
        { key: 'opcache.memory_consumption',  label: 'OPcache Memory (MB)',   desc: 'MB of memory allocated to the OPcache',              type: 'integer', min: 32, max: 1024, step: 32, default: '128' },
        { key: 'opcache.jit',                 label: 'JIT Compiler',          desc: 'Just-In-Time compilation engine (PHP 8+)',            type: 'select',
          options: [{ v: 'off', l: 'Off' }, { v: 'tracing', l: 'Tracing (best for most apps)' }, { v: 'function', l: 'Function (safer)' }], default: 'off' },
        { key: 'opcache.validate_timestamps', label: 'Validate Timestamps',   desc: 'Check files for changes (disable for max speed)',     type: 'toggle',  default: '1' },
        { key: 'opcache.revalidate_freq',     label: 'Revalidate Every (sec)',desc: 'How often to check for changed files',                type: 'integer', min: 0, max: 120, step: 2, default: '2' },
      ]
    },
    {
      id: 'timezone', label: 'Timezone',
      settings: [
        { key: 'date.timezone', label: 'Default Timezone', desc: 'Timezone used by date/time functions',                   type: 'select',
          options: [
            { v: 'UTC',                  l: 'UTC' },
            { v: 'America/New_York',     l: 'America/New_York (EST/EDT)' },
            { v: 'America/Chicago',      l: 'America/Chicago (CST/CDT)' },
            { v: 'America/Denver',       l: 'America/Denver (MST/MDT)' },
            { v: 'America/Los_Angeles',  l: 'America/Los_Angeles (PST/PDT)' },
            { v: 'America/Sao_Paulo',    l: 'America/Sao_Paulo' },
            { v: 'Europe/London',        l: 'Europe/London (GMT/BST)' },
            { v: 'Europe/Paris',         l: 'Europe/Paris (CET/CEST)' },
            { v: 'Europe/Berlin',        l: 'Europe/Berlin' },
            { v: 'Europe/Moscow',        l: 'Europe/Moscow (MSK)' },
            { v: 'Africa/Nairobi',       l: 'Africa/Nairobi (EAT)' },
            { v: 'Asia/Dubai',           l: 'Asia/Dubai (GST)' },
            { v: 'Asia/Kolkata',         l: 'Asia/Kolkata (IST)' },
            { v: 'Asia/Dhaka',           l: 'Asia/Dhaka (BST)' },
            { v: 'Asia/Bangkok',         l: 'Asia/Bangkok (ICT)' },
            { v: 'Asia/Singapore',       l: 'Asia/Singapore (SGT)' },
            { v: 'Asia/Tokyo',           l: 'Asia/Tokyo (JST)' },
            { v: 'Asia/Shanghai',        l: 'Asia/Shanghai (CST)' },
            { v: 'Australia/Sydney',     l: 'Australia/Sydney (AEST/AEDT)' },
            { v: 'Pacific/Auckland',     l: 'Pacific/Auckland (NZST/NZDT)' },
          ], default: 'UTC' },
      ]
    },
  ];

  // ── Constants ──────────────────────────────────────────────────────────────

  const PHP_VERSIONS = ['8.3', '8.2', '8.1', '8.0', '7.4'] as const;
  type PhpVersion = typeof PHP_VERSIONS[number];

  const EXT_CATEGORIES = ['all', 'core', 'database', 'networking', 'image', 'math', 'compression', 'i18n', 'caching', 'system', 'auth', 'mail', 'debug', 'special'] as const;
  type ExtCat = typeof EXT_CATEGORIES[number];

  // ── Inner tab state ────────────────────────────────────────────────────────

  type InnerTab = 'config' | 'opcache' | 'extensions' | 'info';
  let innerTab: InnerTab = 'config';

  // ── Version state ──────────────────────────────────────────────────────────

  let selectedVersion: PhpVersion = (site.php_version as PhpVersion) ?? '8.2';
  let pendingVersion: PhpVersion | null = null;
  let versionLoading = false;
  let showVersionConfirm = false;

  // ── Settings state ─────────────────────────────────────────────────────────

  let settings: Record<string, string> = {};
  let settingsDirty: Record<string, string> = {};  // user-changed values
  let settingsLoading = true;
  let settingsSaving = false;
  let settingsError = false;
  let activeGroup = 'performance';

  // ── OPcache state ──────────────────────────────────────────────────────────

  let opcache: OpcacheStats | null = null;
  let opcacheLoading = true;
  let opcacheError = false;
  let opcacheFlushLoading = false;
  let jitToggling = false;

  // ── Extensions state ───────────────────────────────────────────────────────

  let extensions: PhpExtension[] = [];
  let extensionsLoading = true;
  let extensionsError = false;
  let togglingExtension: string | null = null;
  let extSearch = '';
  let extCat: ExtCat = 'all';

  // ── PHP Info state ─────────────────────────────────────────────────────────

  let phpInfoHtml = '';
  let phpInfoLoading = false;
  let phpInfoLoaded = false;
  let phpInfoError = false;
  let infoSearch = '';
  let copiedKey = '';
  let collapsedSections = new Set<string>();

  // ── Toast ──────────────────────────────────────────────────────────────────

  let toast: Toast | null = null;
  let toastTimer: ReturnType<typeof setTimeout> | null = null;

  function showToast(message: string, type: 'success' | 'error' = 'success') {
    if (toastTimer) clearTimeout(toastTimer);
    toast = { message, type };
    toastTimer = setTimeout(() => { toast = null; }, 4000);
  }

  // ── Lifecycle ──────────────────────────────────────────────────────────────

  onMount(async () => {
    await Promise.all([loadSettings(), loadOpcache(), loadExtensions()]);
  });

  // ── Settings helpers ───────────────────────────────────────────────────────

  function parseMB(val: string): number {
    const m = val.match(/^(\d+(?:\.\d+)?)\s*([KMGTP]?)/i);
    if (!m) return parseInt(val) || 0;
    const n = parseFloat(m[1]);
    switch ((m[2] || '').toUpperCase()) {
      case 'K': return Math.round(n / 1024);
      case 'G': return Math.round(n * 1024);
      case 'T': return Math.round(n * 1024 * 1024);
      default:  return Math.round(n);
    }
  }

  function formatMB(mb: number): string {
    if (mb >= 1024 && mb % 1024 === 0) return `${mb / 1024}G`;
    return `${mb}M`;
  }

  function toBool(val: string): boolean {
    const v = val.toLowerCase();
    return v === 'on' || v === '1' || v === 'true' || v === 'yes';
  }

  function fromBool(b: boolean, def: string): string {
    const d = def.toLowerCase();
    if (d === 'on' || d === 'off') return b ? 'On' : 'Off';
    return b ? '1' : '0';
  }

  function getVal(key: string, def: string): string {
    return settingsDirty[key] ?? settings[key] ?? def;
  }

  function setVal(key: string, value: string) {
    settingsDirty = { ...settingsDirty, [key]: value };
  }

  function setRange(node: HTMLInputElement, val: number) {
    node.value = String(val);
    return { update: (v: number) => { node.value = String(v); } };
  }

  $: dirtyCount = Object.keys(settingsDirty).length;

  // ── Data loading ───────────────────────────────────────────────────────────

  async function loadSettings() {
    settingsLoading = true;
    settingsError = false;
    try {
      settings = await api.php.getSettings(siteId);
    } catch {
      settingsError = true;
    } finally {
      settingsLoading = false;
    }
  }

  async function loadOpcache() {
    opcacheLoading = true;
    opcacheError = false;
    try {
      opcache = await api.php.opcacheStats(siteId);
    } catch {
      opcacheError = true;
    } finally {
      opcacheLoading = false;
    }
  }

  async function loadExtensions() {
    extensionsLoading = true;
    extensionsError = false;
    try {
      extensions = (await api.php.extensions(siteId)) ?? [];
    } catch {
      extensionsError = true;
    } finally {
      extensionsLoading = false;
    }
  }

  async function loadPhpInfo() {
    phpInfoLoading = true;
    phpInfoError = false;
    try {
      const res = await api.php.phpInfo(siteId);
      phpInfoHtml = res.html;
      phpInfoLoaded = true;
    } catch {
      phpInfoError = true;
    } finally {
      phpInfoLoading = false;
    }
  }

  // ── PHP Info parser ────────────────────────────────────────────────────────

  function escHtml(s: string): string {
    return s.replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');
  }

  function highlightMatch(text: string, query: string): string {
    const safe = escHtml(text);
    if (!query) return safe;
    const re = new RegExp(escHtml(query).replace(/[.*+?^${}()|[\]\\]/g,'\\$&'), 'gi');
    return safe.replace(re, m => `<mark class="bg-yellow-400/25 text-yellow-200 rounded-sm">${m}</mark>`);
  }

  function parsePhpInfo(html: string): ParsedPhpInfo {
    if (typeof DOMParser === 'undefined') return { version:'', buildDate:'', sapi:'', sections:[] };
    const doc = new DOMParser().parseFromString(html, 'text/html');
    const sections: PhpInfoSection[] = [];
    let version = '';
    let buildDate = '';
    let sapi = '';

    const h1 = doc.querySelector('h1');
    if (h1) version = (h1.textContent || '').replace(/PHP Version/i,'').trim();

    let title = 'PHP Information';
    let idx = 0;
    for (const el of Array.from(doc.body?.querySelectorAll('h2, table') ?? [])) {
      if (el.tagName === 'H2') {
        title = el.textContent?.trim() || 'Section';
      } else if (el.tagName === 'TABLE') {
        const rows: PhpInfoRow[] = [];
        for (const tr of Array.from(el.querySelectorAll('tr'))) {
          if (tr.classList.contains('h')) continue;
          const tds = tr.querySelectorAll('td');
          if (tds.length >= 2) {
            const key = tds[0].textContent?.trim() || '';
            const val = tds[1].textContent?.trim() || '';
            const local = tds[2]?.textContent?.trim();
            if (key) rows.push({ key, value: val, localValue: local && local !== val ? local : undefined });
          }
        }
        if (rows.length > 0) {
          if (idx === 0) {
            buildDate = rows.find(r => r.key === 'Build Date')?.value || '';
            sapi = rows.find(r => r.key === 'Server API')?.value || '';
          }
          sections.push({ id: `s${idx++}`, title, rows });
          title = 'Section';
        }
      }
    }
    return { version, buildDate, sapi, sections };
  }

  function getValueStyle(val: string): { badge: boolean; cls: string } {
    const v = val.toLowerCase().trim();
    if (['enabled','on','1','yes','active'].includes(v))
      return { badge: true, cls: 'bg-green-500/15 text-green-400 border border-green-500/25' };
    if (['disabled','off','0','no'].includes(v))
      return { badge: true, cls: 'bg-muted text-muted-foreground border border-border' };
    if (/^\d+\.\d+/.test(val)) return { badge: false, cls: 'text-indigo-400 font-semibold' };
    return { badge: false, cls: 'text-foreground' };
  }

  async function copyValue(val: string, key: string) {
    try { await navigator.clipboard.writeText(val); } catch {}
    copiedKey = key;
    setTimeout(() => { copiedKey = ''; }, 2000);
  }

  function toggleSection(id: string) {
    if (collapsedSections.has(id)) collapsedSections.delete(id);
    else collapsedSections.add(id);
    collapsedSections = collapsedSections;
  }

  // ── Actions ────────────────────────────────────────────────────────────────

  async function saveSettings() {
    if (!dirtyCount) return;
    settingsSaving = true;
    try {
      await api.php.saveSettings(siteId, settingsDirty);
      settings = { ...settings, ...settingsDirty };
      settingsDirty = {};
      showToast('PHP settings saved — PHP-FPM reloading', 'success');
    } catch (e: any) {
      showToast(e.message || 'Failed to save settings', 'error');
    } finally {
      settingsSaving = false;
    }
  }

  function resetSettings() {
    settingsDirty = {};
  }

  function requestVersionChange(version: PhpVersion) {
    if (version === selectedVersion) return;
    pendingVersion = version;
    showVersionConfirm = true;
  }

  async function confirmVersionChange() {
    if (!pendingVersion) return;
    const version = pendingVersion;
    versionLoading = true;
    showVersionConfirm = false;
    try {
      await api.sites.changePhpVersion(siteId, version);
      selectedVersion = version;
      showToast(`Switched to PHP ${version} — PHP-FPM restarting`, 'success');
      dispatch('siteUpdated');
    } catch {
      showToast(`Failed to switch to PHP ${version}`, 'error');
    } finally {
      versionLoading = false;
      pendingVersion = null;
    }
  }

  async function flushOpcache() {
    opcacheFlushLoading = true;
    try {
      await api.php.flushOpcache(siteId);
      showToast('OPcache flush queued', 'success');
      setTimeout(() => loadOpcache(), 1500);
    } catch {
      showToast('Failed to flush OPcache', 'error');
    } finally {
      opcacheFlushLoading = false;
    }
  }

  async function toggleJit() {
    if (!opcache) return;
    jitToggling = true;
    const newState = !opcache.jit_enabled;
    try {
      await api.php.toggleJit(siteId, newState);
      opcache = { ...opcache, jit_enabled: newState };
      showToast(`JIT ${newState ? 'enabled' : 'disabled'} — PHP-FPM reloading`, 'success');
    } catch {
      showToast('Failed to toggle JIT', 'error');
    } finally {
      jitToggling = false;
    }
  }

  async function toggleExtension(ext: PhpExtension) {
    togglingExtension = ext.name;
    const newEnabled = !ext.enabled;
    try {
      await api.php.toggleExtension(siteId, ext.name, newEnabled);
      extensions = extensions.map(e => e.name === ext.name ? { ...e, enabled: newEnabled } : e);
      showToast(`${ext.name} ${newEnabled ? 'enabled' : 'disabled'}`, 'success');
    } catch (e: any) {
      showToast(e.message || `Failed to toggle ${ext.name}`, 'error');
    } finally {
      togglingExtension = null;
    }
  }

  // ── Derived ────────────────────────────────────────────────────────────────

  $: opcacheMemTotal = opcache ? opcache.memory_used_mb + opcache.memory_free_mb : 0;
  $: opcacheMemPct   = opcacheMemTotal > 0 ? Math.round((opcache!.memory_used_mb / opcacheMemTotal) * 100) : 0;

  $: filteredExtensions = extensions.filter(e => {
    const inCat = extCat === 'all' || e.category === extCat;
    const inSearch = !extSearch || e.name.toLowerCase().includes(extSearch.toLowerCase()) || e.description.toLowerCase().includes(extSearch.toLowerCase());
    return inCat && inSearch;
  });

  $: extCounts = EXT_CATEGORIES.reduce((acc, cat) => {
    acc[cat] = cat === 'all'
      ? extensions.length
      : extensions.filter(e => e.category === cat).length;
    return acc;
  }, {} as Record<string, number>);

  $: activeGroupDef = GROUPS.find(g => g.id === activeGroup);

  $: parsedInfo = (typeof DOMParser !== 'undefined' && phpInfoHtml) ? parsePhpInfo(phpInfoHtml) : null;

  $: filteredSections = (parsedInfo?.sections ?? []).map((s): PhpInfoFiltered => ({
    ...s,
    filteredRows: infoSearch
      ? s.rows.filter(r =>
          r.key.toLowerCase().includes(infoSearch.toLowerCase()) ||
          r.value.toLowerCase().includes(infoSearch.toLowerCase()))
      : s.rows,
  })).filter(s => !infoSearch || s.filteredRows.length > 0);

  $: totalMatches = infoSearch ? filteredSections.reduce((n,s) => n + s.filteredRows.length, 0) : 0;
</script>

<style>
  @keyframes fadeUp { from { opacity:0; transform:translateY(6px) } to { opacity:1; transform:none } }
  .tab-content { animation: fadeUp 0.2s ease-out both }
  .setting-slider::-webkit-slider-thumb { -webkit-appearance:none; width:16px; height:16px; border-radius:50%; background:var(--accent,#6366f1); cursor:pointer; }
  .setting-slider::-moz-range-thumb { width:16px; height:16px; border-radius:50%; background:var(--accent,#6366f1); cursor:pointer; border:none; }
  .setting-slider { -webkit-appearance:none; appearance:none; width:100%; height:4px; border-radius:2px; background:#e2e8f0; outline:none; }
  :global(.dark) .setting-slider { background:#2d3148; }
</style>

<!-- ── Toast ─────────────────────────────────────────────────────────────────── -->
{#if toast}
  <div class="fixed bottom-4 right-4 z-50 flex items-center gap-3 px-4 py-3 rounded-xl shadow-2xl border
    {toast.type === 'success' ? 'bg-green-950/90 border-green-800 text-green-300' : 'bg-red-950/90 border-red-800 text-red-300'}"
    role="alert">
    {#if toast.type === 'success'}
      <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M5 13l4 4L19 7"/></svg>
    {:else}
      <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M12 9v4m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"/></svg>
    {/if}
    <span class="text-sm font-medium">{toast.message}</span>
    <button on:click={() => toast = null} class="ml-2 opacity-60 hover:opacity-100 transition-opacity">
      <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12"/></svg>
    </button>
  </div>
{/if}

<div class="tab-content space-y-4">

  <!-- ── PHP Version + inner tabs header ──────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl overflow-hidden">
    <!-- Version row -->
    <div class="px-5 py-4 border-b border-border flex items-center justify-between gap-4 flex-wrap">
      <div class="flex items-center gap-3">
        <div class="w-8 h-8 rounded-lg bg-primary/10 flex items-center justify-center text-base">🐘</div>
        <div>
          <h3 class="text-sm font-semibold text-foreground">PHP Version</h3>
          <p class="text-xs text-muted-foreground">Active: <span class="font-mono font-medium text-foreground">PHP {selectedVersion}</span></p>
        </div>
      </div>
      <div class="flex items-center gap-1.5 flex-wrap">
        {#each PHP_VERSIONS as version}
          <button
            on:click={() => requestVersionChange(version)}
            disabled={versionLoading}
            class="h-8 px-4 rounded-lg text-xs font-medium transition-all disabled:cursor-not-allowed
              {version === selectedVersion
                ? 'bg-primary text-primary-foreground shadow shadow-primary/30'
                : 'border border-border text-muted-foreground hover:bg-muted hover:text-foreground'}"
          >
            {version === selectedVersion && versionLoading ? '⟳' : ''} PHP {version}
          </button>
        {/each}
      </div>
    </div>

    <!-- Inner tab bar -->
    <div class="flex overflow-x-auto border-b border-border bg-muted/20">
      {#each [
        { id: 'config',     label: 'Configuration', icon: '⚙️' },
        { id: 'opcache',    label: 'OPcache',        icon: '⚡' },
        { id: 'extensions', label: 'Extensions',     icon: '🧩' },
        { id: 'info',       label: 'PHP Info',       icon: '📋' },
      ] as tab}
        <button
          on:click={() => { innerTab = tab.id as InnerTab; if (tab.id === 'info' && !phpInfoLoaded && !phpInfoLoading) loadPhpInfo(); }}
          class="flex items-center gap-2 px-5 py-3 text-sm font-medium whitespace-nowrap transition-colors border-b-2
            {innerTab === tab.id
              ? 'border-primary text-primary bg-primary/5'
              : 'border-transparent text-muted-foreground hover:text-foreground hover:bg-muted/30'}"
        >
          <span class="text-base leading-none">{tab.icon}</span>
          {tab.label}
        </button>
      {/each}
    </div>
  </div>

  <!-- ══ Configuration tab ══════════════════════════════════════════════════ -->
  {#if innerTab === 'config'}
    <div class="bg-card border border-border rounded-xl overflow-hidden">
      {#if settingsLoading}
        <div class="p-10 flex flex-col items-center gap-3 text-muted-foreground">
          <svg class="w-6 h-6 animate-spin" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"/></svg>
          <span class="text-sm">Loading PHP configuration…</span>
        </div>
      {:else if settingsError}
        <div class="p-10 text-center space-y-3">
          <p class="text-sm text-muted-foreground">Failed to load PHP settings</p>
          <button on:click={loadSettings} class="h-8 px-4 rounded-lg border border-border text-xs font-medium hover:bg-muted transition-colors">Retry</button>
        </div>
      {:else}
        <div class="flex h-full min-h-0" style="min-height:480px">
          <!-- Category sidebar -->
          <nav class="w-48 shrink-0 border-r border-border bg-muted/20 flex flex-col py-2">
            {#each GROUPS as g}
              <button
                on:click={() => { activeGroup = g.id; }}
                class="flex items-center gap-2.5 px-4 py-2.5 text-sm text-left transition-colors
                  {activeGroup === g.id
                    ? 'bg-primary/10 text-primary font-medium border-r-2 border-primary'
                    : 'text-muted-foreground hover:bg-muted/40 hover:text-foreground'}"
              >
                <span class="text-base leading-none">
                  {g.id === 'performance' ? '⚡' : g.id === 'upload' ? '📤' : g.id === 'vars' ? '🔢' : g.id === 'errors' ? '🐛' : g.id === 'session' ? '🔐' : g.id === 'opcache_conf' ? '💾' : '🌍'}
                </span>
                {g.label}
              </button>
            {/each}
          </nav>

          <!-- Settings panel -->
          <div class="flex-1 flex flex-col min-w-0">
            <div class="flex-1 overflow-y-auto p-6 space-y-6">
              {#if activeGroupDef}
                {#each activeGroupDef.settings as def}
                  {@const currentVal = settingsDirty[def.key] ?? settings[def.key] ?? def.default}
                  {@const isDirty = def.key in settingsDirty}
                  <div class="space-y-2 pb-5 border-b border-border last:border-0 last:pb-0">
                    <div class="flex items-center justify-between">
                      <div>
                        <div class="flex items-center gap-2">
                          <span class="text-sm font-semibold text-foreground">{def.label}</span>
                          {#if isDirty}
                            <span class="inline-flex items-center px-1.5 py-0.5 rounded text-xs font-medium bg-amber-500/15 text-amber-400 border border-amber-500/25">unsaved</span>
                          {/if}
                        </div>
                        <p class="text-xs text-muted-foreground mt-0.5">{def.desc}</p>
                      </div>
                      <!-- Current value display -->
                      <div class="text-right shrink-0 ml-4">
                        {#if def.type === 'bytes'}
                          <span class="text-sm font-mono font-semibold text-primary">{formatMB(parseMB(currentVal))}</span>
                        {:else if def.type === 'seconds'}
                          {@const secs = parseInt(currentVal) || 0}
                          <span class="text-sm font-mono font-semibold text-primary">
                            {secs >= 60 ? `${(secs/60).toFixed(0)}m ${secs%60}s` : `${secs}s`}
                          </span>
                        {:else if def.type === 'integer'}
                          <span class="text-sm font-mono font-semibold text-primary">{currentVal}</span>
                        {/if}
                      </div>
                    </div>

                    <!-- Controls -->
                    {#if def.type === 'bytes'}
                      {@const mb = parseMB(currentVal)}
                      <div class="space-y-1.5">
                        <input
                          type="range" min={def.min ?? 1} max={def.max ?? 2048} step={def.step ?? 1}
                          use:setRange={mb}
                          class="setting-slider"
                          on:input={e => setVal(def.key, formatMB(parseInt((e.target as HTMLInputElement).value)))}
                        />
                        <div class="flex justify-between text-xs text-muted-foreground">
                          <span>{formatMB(def.min ?? 1)}</span>
                          <span>{formatMB(def.max ?? 2048)}</span>
                        </div>
                      </div>
                    {:else if def.type === 'seconds'}
                      {@const secs = parseInt(currentVal) || 0}
                      <div class="space-y-1.5">
                        <input
                          type="range" min={def.min ?? 1} max={def.max ?? 600} step={def.step ?? 1}
                          use:setRange={secs}
                          class="setting-slider"
                          on:input={e => setVal(def.key, (e.target as HTMLInputElement).value)}
                        />
                        <div class="flex justify-between text-xs text-muted-foreground">
                          <span>{def.min ?? 1}s</span>
                          <span>{def.max ?? 600}s</span>
                        </div>
                      </div>
                    {:else if def.type === 'integer'}
                      <div class="flex items-center gap-3">
                        <input
                          type="range" min={def.min ?? 0} max={def.max ?? 100} step={def.step ?? 1}
                          use:setRange={parseInt(currentVal) || 0}
                          class="setting-slider flex-1"
                          on:input={e => setVal(def.key, (e.target as HTMLInputElement).value)}
                        />
                        <input
                          type="number" min={def.min} max={def.max} step={def.step ?? 1}
                          use:setRange={parseInt(currentVal) || 0}
                          class="w-24 h-8 px-2 rounded-lg border border-border bg-background text-sm text-foreground text-center focus:outline-none focus:ring-1 focus:ring-ring font-mono"
                          on:change={e => setVal(def.key, (e.target as HTMLInputElement).value)}
                        />
                      </div>
                    {:else if def.type === 'toggle'}
                      {@const on = toBool(currentVal)}
                      <button
                        on:click={() => setVal(def.key, fromBool(!on, def.default))}
                        role="switch" aria-checked={on}
                        class="relative inline-flex h-6 w-11 items-center rounded-full transition-colors
                          {on ? 'bg-primary' : 'bg-muted border border-border'}"
                      >
                        <span class="inline-block h-4 w-4 rounded-full bg-white shadow transition-transform
                          {on ? 'translate-x-6' : 'translate-x-1'}"></span>
                      </button>
                    {:else if def.type === 'select'}
                      <select
                        value={currentVal}
                        on:change={e => setVal(def.key, (e.target as HTMLSelectElement).value)}
                        class="h-9 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring max-w-sm w-full"
                      >
                        {#each def.options ?? [] as opt}
                          <option value={opt.v}>{opt.l}</option>
                        {/each}
                      </select>
                    {/if}
                  </div>
                {/each}
              {/if}
            </div>

            <!-- Sticky save bar -->
            <div class="flex items-center justify-between px-6 py-4 border-t border-border bg-card/80 backdrop-blur-sm shrink-0">
              <span class="text-xs text-muted-foreground">
                {#if dirtyCount > 0}
                  <span class="text-amber-400 font-medium">{dirtyCount} unsaved change{dirtyCount !== 1 ? 's' : ''}</span>
                {:else}
                  All settings saved
                {/if}
              </span>
              <div class="flex items-center gap-2">
                {#if dirtyCount > 0}
                  <button on:click={resetSettings} class="h-8 px-3 rounded-lg border border-border text-xs font-medium text-muted-foreground hover:bg-muted transition-colors">
                    Discard
                  </button>
                {/if}
                <button
                  on:click={saveSettings}
                  disabled={settingsSaving || dirtyCount === 0}
                  class="h-8 px-4 rounded-lg bg-primary text-primary-foreground text-xs font-semibold transition-all
                    disabled:opacity-50 disabled:cursor-not-allowed hover:bg-primary/90 inline-flex items-center gap-1.5"
                >
                  {#if settingsSaving}
                    <svg class="w-3 h-3 animate-spin" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"/></svg>
                    Saving…
                  {:else}
                    ✓ Apply Changes
                  {/if}
                </button>
              </div>
            </div>
          </div>
        </div>
      {/if}
    </div>
  {/if}

  <!-- ══ OPcache tab ════════════════════════════════════════════════════════ -->
  {#if innerTab === 'opcache'}
    <div class="space-y-4">
      <!-- Stats cards -->
      <div class="bg-card border border-border rounded-xl overflow-hidden">
        <div class="flex items-center justify-between px-5 py-4 border-b border-border">
          <div class="flex items-center gap-2.5">
            <div class="w-7 h-7 rounded-lg bg-yellow-500/10 flex items-center justify-center">
              <svg class="w-3.5 h-3.5 text-yellow-500" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75"><path stroke-linecap="round" stroke-linejoin="round" d="M3.75 13.5l10.5-11.25L12 10.5h8.25L9.75 21.75 12 13.5H3.75z"/></svg>
            </div>
            <h3 class="text-sm font-semibold text-foreground">OPcache Statistics</h3>
          </div>
          {#if !opcacheLoading && !opcacheError}
            <div class="flex items-center gap-2">
              <!-- JIT toggle -->
              {#if opcache}
                <button
                  on:click={toggleJit}
                  disabled={jitToggling}
                  class="h-8 px-3 rounded-lg text-xs font-medium transition-colors inline-flex items-center gap-1.5
                    {opcache.jit_enabled ? 'bg-purple-500/10 text-purple-400 hover:bg-purple-500/20' : 'border border-border text-muted-foreground hover:bg-muted'}"
                >
                  {#if jitToggling}
                    <svg class="w-3 h-3 animate-spin" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"/></svg>
                  {/if}
                  JIT {opcache.jit_enabled ? 'ON' : 'OFF'}
                </button>
              {/if}
              <button
                on:click={flushOpcache}
                disabled={opcacheFlushLoading}
                class="h-8 px-3 rounded-lg text-xs font-medium text-orange-400 hover:bg-orange-500/10 transition-colors disabled:opacity-50 inline-flex items-center gap-1.5"
              >
                {#if opcacheFlushLoading}
                  <svg class="w-3 h-3 animate-spin" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"/></svg>
                  Flushing…
                {:else}
                  <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/></svg>
                  Flush Cache
                {/if}
              </button>
              <button on:click={loadOpcache} class="h-8 w-8 rounded-lg border border-border text-muted-foreground hover:bg-muted transition-colors flex items-center justify-center">
                <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/></svg>
              </button>
            </div>
          {/if}
        </div>

        {#if opcacheLoading}
          <div class="p-6 grid grid-cols-2 lg:grid-cols-4 gap-5 animate-pulse">
            {#each [0,1,2,3] as _}
              <div class="space-y-2"><div class="h-3 bg-muted rounded w-16"></div><div class="h-7 bg-muted rounded w-20"></div><div class="h-2 bg-muted rounded w-full"></div></div>
            {/each}
          </div>
        {:else if opcacheError}
          <div class="p-10 text-center space-y-3">
            <p class="text-sm text-muted-foreground">Failed to load OPcache stats</p>
            <button on:click={loadOpcache} class="h-8 px-4 rounded-lg border border-border text-xs font-medium hover:bg-muted transition-colors">Retry</button>
          </div>
        {:else if opcache}
          {#if !opcache.enabled}
            <div class="px-5 py-3 border-b border-border bg-amber-500/5 flex items-center gap-2 text-amber-400 text-xs">
              <svg class="w-3.5 h-3.5 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M12 9v4m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"/></svg>
              OPcache is disabled for this site — enable it in the Configuration tab
            </div>
          {/if}
          <div class="p-6 grid grid-cols-2 lg:grid-cols-4 gap-6">
            <!-- Hit Rate -->
            <div>
              <p class="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">Hit Rate</p>
              <p class="text-3xl font-bold mb-2 {opcache.hit_rate >= 80 ? 'text-green-400' : opcache.hit_rate >= 50 ? 'text-yellow-400' : 'text-red-400'}">
                {opcache.hit_rate.toFixed(1)}%
              </p>
              <div class="h-2 bg-muted rounded-full overflow-hidden">
                <div class="h-full rounded-full transition-all {opcache.hit_rate >= 80 ? 'bg-green-500' : opcache.hit_rate >= 50 ? 'bg-yellow-500' : 'bg-red-500'}" style="width:{Math.min(opcache.hit_rate,100)}%"></div>
              </div>
              <p class="text-xs text-muted-foreground mt-1">{opcache.hit_rate >= 80 ? '✓ Excellent' : opcache.hit_rate >= 50 ? '⚠ Good' : '✗ Poor'}</p>
            </div>
            <!-- Memory -->
            <div>
              <p class="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">Memory Used</p>
              <p class="text-xl font-bold text-foreground mb-1">{opcache.memory_used_mb.toFixed(1)} <span class="text-sm font-normal text-muted-foreground">/ {opcacheMemTotal.toFixed(0)} MB</span></p>
              <div class="h-2 bg-muted rounded-full overflow-hidden">
                <div class="h-full rounded-full bg-blue-500 transition-all" style="width:{Math.min(opcacheMemPct,100)}%"></div>
              </div>
              <p class="text-xs text-muted-foreground mt-1">{opcacheMemPct}% used</p>
            </div>
            <!-- Cached Scripts -->
            <div>
              <p class="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">Cached Scripts</p>
              <p class="text-3xl font-bold text-foreground">{opcache.cached_scripts.toLocaleString()}</p>
              <p class="text-xs text-muted-foreground mt-2">PHP files in opcode cache</p>
            </div>
            <!-- JIT -->
            <div>
              <p class="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">JIT Compiler</p>
              {#if opcache.jit_enabled}
                <div class="inline-flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg bg-purple-500/10 text-purple-400 border border-purple-500/20 text-sm font-medium">
                  <span class="w-1.5 h-1.5 rounded-full bg-purple-400 animate-pulse"></span>
                  Active
                </div>
                {#if opcache.jit_buffer_size > 0}
                  <p class="text-xs text-muted-foreground mt-1.5">{(opcache.jit_buffer_size / 1048576).toFixed(0)} MB buffer</p>
                {/if}
              {:else}
                <div class="inline-flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg bg-muted text-muted-foreground border border-border text-sm font-medium">
                  <span class="w-1.5 h-1.5 rounded-full bg-muted-foreground"></span>
                  Disabled
                </div>
                <p class="text-xs text-muted-foreground mt-1.5">Enable for CPU-intensive PHP</p>
              {/if}
            </div>
          </div>
        {/if}
      </div>

      <!-- OPcache config hint -->
      <div class="bg-blue-500/5 border border-blue-500/20 rounded-xl px-5 py-4 flex items-start gap-3">
        <svg class="w-4 h-4 text-blue-400 shrink-0 mt-0.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"/><path stroke-linecap="round" d="M12 16v-4m0-4h.01"/></svg>
        <p class="text-xs text-blue-300">
          OPcache memory and JIT settings can also be configured in the <button on:click={() => { innerTab = 'config'; activeGroup = 'opcache_conf'; }} class="underline hover:text-blue-200 transition-colors">Configuration → OPcache</button> tab.
          Stats are read directly from the PHP-FPM pool for this site.
        </p>
      </div>
    </div>
  {/if}

  <!-- ══ Extensions tab ════════════════════════════════════════════════════ -->
  {#if innerTab === 'extensions'}
    <div class="bg-card border border-border rounded-xl overflow-hidden">
      <div class="px-5 py-4 border-b border-border space-y-3">
        <div class="flex items-center justify-between">
          <div class="flex items-center gap-2.5">
            <div class="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
              <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75"><path stroke-linecap="round" stroke-linejoin="round" d="M14.25 9.75L16.5 12l-2.25 2.25m-4.5 0L7.5 12l2.25-2.25M6 20.25h12A2.25 2.25 0 0020.25 18V6A2.25 2.25 0 0018 3.75H6A2.25 2.25 0 003.75 6v12A2.25 2.25 0 006 20.25z"/></svg>
            </div>
            <div>
              <h3 class="text-sm font-semibold text-foreground">PHP Extensions</h3>
              {#if !extensionsLoading && !extensionsError}
                <p class="text-xs text-muted-foreground">{extensions.length} available · {extensions.filter(e=>e.enabled).length} enabled</p>
              {/if}
            </div>
          </div>
          {#if !extensionsLoading && !extensionsError}
            <div class="relative">
              <svg class="absolute left-3 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground pointer-events-none" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"/></svg>
              <input bind:value={extSearch} type="search" placeholder="Search…"
                class="h-8 pl-9 pr-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-1 focus:ring-ring w-44" />
            </div>
          {/if}
        </div>

        <!-- Category tabs -->
        {#if !extensionsLoading && !extensionsError}
          <div class="flex gap-1 flex-wrap">
            {#each EXT_CATEGORIES.filter(c => extCounts[c] > 0) as cat}
              <button
                on:click={() => extCat = cat}
                class="h-7 px-3 rounded-full text-xs font-medium transition-colors capitalize
                  {extCat === cat
                    ? 'bg-primary text-primary-foreground'
                    : 'bg-muted text-muted-foreground hover:text-foreground'}"
              >
                {cat} {extCounts[cat] > 0 && cat !== 'all' ? `(${extCounts[cat]})` : ''}
              </button>
            {/each}
          </div>
        {/if}
      </div>

      {#if extensionsLoading}
        <div class="p-5 grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-3">
          {#each Array(12) as _, i (i)}
            <div class="animate-pulse rounded-lg border border-border p-3 space-y-2">
              <div class="flex justify-between"><div class="h-4 bg-muted rounded w-20"></div><div class="h-5 w-9 bg-muted rounded-full"></div></div>
              <div class="h-3 bg-muted rounded w-full"></div>
            </div>
          {/each}
        </div>
      {:else if extensionsError}
        <div class="p-10 text-center space-y-3">
          <p class="text-sm text-muted-foreground">Failed to load extensions</p>
          <button on:click={loadExtensions} class="h-8 px-4 rounded-lg border border-border text-xs font-medium hover:bg-muted transition-colors">Retry</button>
        </div>
      {:else if filteredExtensions.length === 0}
        <div class="p-10 text-center">
          <p class="text-sm text-muted-foreground">No extensions match your filter</p>
        </div>
      {:else}
        <div class="p-5 grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-3">
          {#each filteredExtensions as ext (ext.name)}
            {@const isToggling = togglingExtension === ext.name}
            <div class="rounded-xl border p-3.5 flex flex-col gap-2 transition-all
              {ext.enabled ? 'bg-card border-border hover:border-primary/40 hover:shadow-sm hover:shadow-primary/5' : 'bg-muted/20 border-border/60 opacity-75 hover:opacity-90'}">
              <div class="flex items-start justify-between gap-2">
                <div class="min-w-0">
                  <div class="flex items-center gap-1.5 flex-wrap">
                    <span class="text-sm font-semibold text-foreground font-mono">{ext.name}</span>
                    {#if ext.dev_only}
                      <span class="inline-flex items-center px-1.5 py-0.5 rounded text-xs font-medium bg-amber-500/10 text-amber-400">dev</span>
                    {/if}
                  </div>
                </div>
                <!-- Toggle switch -->
                <button
                  on:click={() => toggleExtension(ext)}
                  disabled={isToggling}
                  role="switch" aria-checked={ext.enabled}
                  aria-label="Toggle {ext.name}"
                  class="relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors disabled:opacity-50 disabled:cursor-not-allowed
                    {ext.enabled ? 'bg-primary' : 'bg-muted border border-border'}"
                >
                  {#if isToggling}
                    <span class="absolute inset-0 flex items-center justify-center">
                      <svg class="w-3 h-3 animate-spin text-white" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"/></svg>
                    </span>
                  {:else}
                    <span class="inline-block h-3.5 w-3.5 rounded-full bg-white shadow transition-transform {ext.enabled ? 'translate-x-[18px]' : 'translate-x-0.5'}"></span>
                  {/if}
                </button>
              </div>
              <p class="text-xs text-muted-foreground leading-snug line-clamp-2">{ext.description || 'No description available'}</p>
              {#if ext.special_note}
                <p class="text-xs text-amber-400 leading-snug">{ext.special_note}</p>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    </div>
  {/if}

  <!-- ══ PHP Info tab ═══════════════════════════════════════════════════════ -->
  {#if innerTab === 'info'}
    <div class="space-y-3">

      <!-- ── Header + hero card ───────────────────────────────────────────── -->
      <div class="bg-card border border-border rounded-xl overflow-hidden">

        <!-- Title bar -->
        <div class="flex items-center justify-between px-5 py-4 border-b border-border">
          <div class="flex items-center gap-2.5">
            <div class="w-8 h-8 rounded-lg bg-indigo-500/10 flex items-center justify-center shrink-0">
              <svg class="w-4 h-4 text-indigo-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
                <circle cx="12" cy="12" r="10"/>
                <path stroke-linecap="round" d="M12 16v-5m0-2.01V8"/>
              </svg>
            </div>
            <div>
              <h3 class="text-sm font-semibold text-foreground">PHP Runtime Info</h3>
              <p class="text-xs text-muted-foreground">Parsed phpinfo() for PHP {selectedVersion}</p>
            </div>
          </div>
          <div class="flex items-center gap-2">
            {#if parsedInfo}
              <button
                on:click={() => {
                  if (collapsedSections.size > 0) collapsedSections = new Set();
                  else collapsedSections = new Set(parsedInfo!.sections.map(s => s.id));
                }}
                class="h-8 px-3 rounded-lg border border-border text-xs font-medium text-muted-foreground hover:bg-muted transition-colors"
              >
                {collapsedSections.size > 0 ? 'Expand All' : 'Collapse All'}
              </button>
            {/if}
            <button
              on:click={loadPhpInfo}
              disabled={phpInfoLoading}
              class="h-8 px-3 rounded-lg border border-border text-xs font-medium text-muted-foreground hover:bg-muted transition-colors disabled:opacity-50 inline-flex items-center gap-1.5"
            >
              <svg class="w-3 h-3 {phpInfoLoading ? 'animate-spin' : ''}" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                <path stroke-linecap="round" stroke-linejoin="round" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/>
              </svg>
              {phpInfoLoading ? 'Loading…' : 'Refresh'}
            </button>
          </div>
        </div>

        {#if parsedInfo}
          <!-- Version hero strip -->
          <div class="px-6 py-5 flex flex-wrap items-start gap-8 border-b border-border bg-gradient-to-br from-indigo-500/5 via-transparent to-purple-500/5">
            <div>
              <p class="text-[10px] font-semibold text-muted-foreground uppercase tracking-widest mb-2">PHP Version</p>
              <div class="inline-flex items-center gap-2.5 px-3.5 py-2 rounded-xl bg-indigo-500/10 border border-indigo-500/20">
                <span class="text-lg leading-none">🐘</span>
                <span class="text-xl font-bold text-indigo-300 font-mono">{parsedInfo.version || selectedVersion}</span>
              </div>
            </div>
            {#if parsedInfo.sapi}
              <div>
                <p class="text-[10px] font-semibold text-muted-foreground uppercase tracking-widest mb-2">Server API</p>
                <span class="inline-flex items-center gap-2 px-3 py-2 rounded-xl bg-card border border-border text-sm font-mono text-foreground">
                  <span class="w-1.5 h-1.5 rounded-full bg-green-400 shrink-0"></span>
                  {parsedInfo.sapi}
                </span>
              </div>
            {/if}
            {#if parsedInfo.buildDate}
              <div>
                <p class="text-[10px] font-semibold text-muted-foreground uppercase tracking-widest mb-2">Build Date</p>
                <p class="text-sm font-mono text-foreground mt-1.5">{parsedInfo.buildDate}</p>
              </div>
            {/if}
            <div class="ml-auto text-right self-center">
              <p class="text-[10px] font-semibold text-muted-foreground uppercase tracking-widest mb-1">Sections</p>
              <p class="text-3xl font-bold text-foreground">{parsedInfo.sections.length}</p>
            </div>
          </div>

          <!-- Search + security badge row -->
          <div class="px-5 py-3 flex items-center gap-3">
            <div class="relative flex-1">
              <svg class="absolute left-3 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground pointer-events-none" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                <path stroke-linecap="round" stroke-linejoin="round" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"/>
              </svg>
              <input
                bind:value={infoSearch}
                type="search"
                placeholder="Search directives, values, extensions…"
                class="h-9 w-full pl-9 pr-9 rounded-lg border border-border bg-background text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-ring"
              />
              {#if infoSearch}
                <button on:click={() => infoSearch = ''} class="absolute right-2.5 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground">
                  <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12"/></svg>
                </button>
              {/if}
            </div>
            {#if infoSearch}
              <span class="text-xs text-muted-foreground whitespace-nowrap shrink-0">
                {totalMatches} result{totalMatches !== 1 ? 's' : ''} · {filteredSections.length} section{filteredSections.length !== 1 ? 's' : ''}
              </span>
            {/if}
            <div class="flex items-center gap-1.5 text-xs text-amber-400 shrink-0">
              <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                <path stroke-linecap="round" stroke-linejoin="round" d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z"/>
              </svg>
              <span class="hidden sm:inline text-[11px]">Paths redacted</span>
            </div>
          </div>
        {/if}
      </div>

      <!-- ── Content area ─────────────────────────────────────────────────── -->
      {#if phpInfoLoading}
        <!-- Skeleton -->
        <div class="space-y-3">
          {#each [7,11,5,9] as rowCount}
            <div class="bg-card border border-border rounded-xl overflow-hidden animate-pulse">
              <div class="px-5 py-4 flex items-center gap-3 border-b border-border">
                <div class="h-4 bg-muted rounded w-36"></div>
                <div class="h-5 bg-muted rounded-full w-7"></div>
              </div>
              <div class="divide-y divide-border/40">
                {#each Array(Math.min(rowCount, 5)) as _, i}
                  <div class="flex items-center px-5 py-2.5 gap-6">
                    <div class="h-3 bg-muted rounded shrink-0 w-44"></div>
                    <div class="h-3 bg-muted rounded {i % 3 === 0 ? 'w-10' : i % 3 === 1 ? 'w-36' : 'w-24'}"></div>
                  </div>
                {/each}
              </div>
            </div>
          {/each}
        </div>

      {:else if phpInfoError}
        <!-- Error state -->
        <div class="bg-card border border-border rounded-xl p-14 text-center space-y-4">
          <div class="w-12 h-12 mx-auto rounded-2xl bg-red-500/10 flex items-center justify-center">
            <svg class="w-6 h-6 text-red-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M12 9v4m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"/>
            </svg>
          </div>
          <div>
            <p class="text-sm font-semibold text-foreground">Failed to load PHP info</p>
            <p class="text-xs text-muted-foreground mt-1">The phpinfo() script could not be executed for this site</p>
          </div>
          <button on:click={loadPhpInfo} class="h-9 px-5 rounded-lg border border-border text-sm font-medium hover:bg-muted transition-colors">
            Try again
          </button>
        </div>

      {:else if parsedInfo}
        <!-- Sections -->
        {#each filteredSections as section (section.id)}
          {@const isCollapsed = collapsedSections.has(section.id)}
          <div class="bg-card border border-border rounded-xl overflow-hidden">
            <!-- Section header (click to collapse) -->
            <button
              class="w-full flex items-center justify-between px-5 py-3.5 hover:bg-muted/20 transition-colors text-left"
              on:click={() => toggleSection(section.id)}
            >
              <div class="flex items-center gap-2.5 min-w-0">
                <span class="text-sm font-semibold text-foreground truncate">{section.title}</span>
                <span class="inline-flex items-center h-5 px-1.5 rounded bg-muted text-[11px] text-muted-foreground font-mono shrink-0">{section.filteredRows.length}</span>
              </div>
              <svg class="w-4 h-4 text-muted-foreground shrink-0 transition-transform {isCollapsed ? '-rotate-90' : ''}" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                <path stroke-linecap="round" stroke-linejoin="round" d="M19 9l-7 7-7-7"/>
              </svg>
            </button>

            {#if !isCollapsed}
              <div class="border-t border-border">
                {#each section.filteredRows as row, i}
                  {@const vs = getValueStyle(row.value)}
                  {@const ck = section.id + row.key}
                  <div class="flex items-start px-5 py-2 group hover:bg-muted/10 transition-colors {i % 2 === 0 ? '' : 'bg-muted/[.03]'}">
                    <!-- Directive name -->
                    <span class="w-56 xl:w-72 shrink-0 text-xs text-muted-foreground pr-4 py-0.5 leading-relaxed truncate" title={row.key}>
                      {@html highlightMatch(row.key, infoSearch)}
                    </span>
                    <!-- Value area -->
                    <div class="flex-1 flex items-start gap-2 min-w-0">
                      {#if vs.badge}
                        <span class="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium {vs.cls}">
                          {@html highlightMatch(row.value, infoSearch)}
                        </span>
                      {:else}
                        <span class="text-xs font-mono break-all leading-relaxed {vs.cls}">
                          {@html highlightMatch(row.value, infoSearch)}
                        </span>
                      {/if}
                      {#if row.localValue}
                        <span class="text-[11px] text-muted-foreground shrink-0 ml-1">(local: <span class="font-mono">{row.localValue}</span>)</span>
                      {/if}
                      <!-- Copy button -->
                      <button
                        class="ml-auto shrink-0 opacity-0 group-hover:opacity-100 transition-opacity h-5 w-5 flex items-center justify-center rounded hover:bg-muted text-muted-foreground hover:text-foreground"
                        on:click={() => copyValue(row.value, ck)}
                        title="Copy value"
                      >
                        {#if copiedKey === ck}
                          <svg class="w-3 h-3 text-green-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5">
                            <path stroke-linecap="round" stroke-linejoin="round" d="M5 13l4 4L19 7"/>
                          </svg>
                        {:else}
                          <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                            <path stroke-linecap="round" stroke-linejoin="round" d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z"/>
                          </svg>
                        {/if}
                      </button>
                    </div>
                  </div>
                {/each}
              </div>
            {/if}
          </div>
        {/each}

        {#if infoSearch && filteredSections.length === 0}
          <div class="bg-card border border-border rounded-xl py-12 text-center">
            <p class="text-sm text-muted-foreground">No directives match <span class="font-mono text-foreground">"{infoSearch}"</span></p>
            <button on:click={() => infoSearch = ''} class="mt-3 text-xs text-primary hover:underline">Clear search</button>
          </div>
        {/if}

      {:else}
        <!-- Not loaded yet -->
        <div class="bg-card border border-border rounded-xl py-16 text-center space-y-5">
          <div class="w-16 h-16 mx-auto rounded-2xl bg-indigo-500/10 flex items-center justify-center text-3xl">🐘</div>
          <div class="space-y-1.5">
            <h3 class="text-base font-semibold text-foreground">PHP Runtime Information</h3>
            <p class="text-sm text-muted-foreground">Loads phpinfo() and parses it into {String.fromCodePoint(0x1F50D)} searchable structured sections</p>
          </div>
          <div class="flex items-center justify-center gap-4 text-xs text-muted-foreground">
            <span class="flex items-center gap-1.5"><span class="w-1.5 h-1.5 rounded-full bg-green-400"></span>Value badges</span>
            <span class="flex items-center gap-1.5"><span class="w-1.5 h-1.5 rounded-full bg-indigo-400"></span>Collapsible sections</span>
            <span class="flex items-center gap-1.5"><span class="w-1.5 h-1.5 rounded-full bg-amber-400"></span>Paths redacted</span>
          </div>
          <button
            on:click={loadPhpInfo}
            disabled={phpInfoLoading}
            class="h-9 px-6 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors disabled:opacity-50 inline-flex items-center gap-2"
          >
            {#if phpInfoLoading}
              <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"/></svg>
            {/if}
            Load PHP Info
          </button>
        </div>
      {/if}

    </div>
  {/if}

</div>

<!-- ── PHP version confirm dialog ────────────────────────────────────────────── -->
{#if showVersionConfirm && pendingVersion}
  <div class="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-sm">
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-sm shadow-2xl">
      <div class="flex items-center gap-3 mb-3">
        <div class="w-10 h-10 rounded-full bg-primary/10 flex items-center justify-center shrink-0 text-xl">🐘</div>
        <h3 class="text-base font-semibold text-foreground">Switch PHP Version?</h3>
      </div>
      <p class="text-sm text-muted-foreground mb-5">
        Switch <strong class="text-foreground">{site.domain}</strong> to
        <strong class="font-mono text-foreground">PHP {pendingVersion}</strong>?
        This restarts PHP-FPM and may cause a brief interruption.
      </p>
      <div class="flex items-center gap-2 justify-end">
        <button
          on:click={() => { showVersionConfirm = false; pendingVersion = null; }}
          class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted transition-colors"
        >Cancel</button>
        <button
          on:click={confirmVersionChange}
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors"
        >
          Switch to PHP {pendingVersion}
        </button>
      </div>
    </div>
  </div>
{/if}
