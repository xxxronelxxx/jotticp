<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { api } from '$api/client';
  import type { Site } from '$api/client';
  import { t } from '$lib/i18n';

  // ── Types ──────────────────────────────────────────────────────────────────

  interface FirewallRule {
    port:        string;
    protocol:    'tcp' | 'udp' | 'both';
    direction:   'in' | 'out';
    action:      'allow' | 'deny';
    source:      string;
    description: string;
  }

  interface Jail {
    name:         string;
    enabled:      boolean;
    banned_count: number;
    maxretry:     number;
    findtime:     number;
    bantime:      number;
    banned_ips:   string[];
  }

  // ── State ──────────────────────────────────────────────────────────────────

  // UFW status
  let ufwActive     = false;
  let ufwLoading    = true;
  let openPortCount = 0;

  // Fail2ban status
  let f2bActive      = false;
  let f2bBannedTotal = 0;

  // Threat dashboard stats
  let blockedToday   = 0;
  let blockedThisHour = 0;
  let topAttackerIp  = '';
  let geoBlockCount  = 0;

  // Firewall Rules
  let rules: FirewallRule[] = [];
  let rulesLoading = true;

  // Unban animation tracking: ip -> 'fading' | 'gone'
  let unbanAnimating: Record<string, 'fading' | 'gone'> = {};

  // Fail2ban jails
  let jails: Jail[] = [];
  let jailsLoading  = true;
  let expandedJail: string | null = null;

  // Add-rule modal
  let showAddModal    = false;
  let modalAction: 'allow' | 'deny' = 'allow';
  let modalPort       = '';
  let modalProto: 'tcp' | 'udp' | 'both' = 'tcp';
  let modalSource     = 'any';
  let modalDesc       = '';
  let modalLoading    = false;
  let modalCommonPort = '';

  // Rate limiting
  let sites: Site[]  = [];
  let rateSite       = '';
  let rateRpm        = 60;
  let rateBurst      = 20;
  let rateSaving     = false;

  // Unban per-jail
  let unbanInputs: Record<string, string> = {};

  // Confirm unban
  let confirmUnbanJail = '';
  let confirmUnbanIp   = '';

  // Toast
  let toastMessage = '';
  let toastType: 'success' | 'error' | 'info' = 'success';

  // ── GeoIP Country Blocking ─────────────────────────────────────────────────

  const COUNTRIES = [
    { code: 'CN', name: 'China',         flag: '🇨🇳' },
    { code: 'RU', name: 'Russia',        flag: '🇷🇺' },
    { code: 'KP', name: 'North Korea',   flag: '🇰🇵' },
    { code: 'IR', name: 'Iran',          flag: '🇮🇷' },
    { code: 'SY', name: 'Syria',         flag: '🇸🇾' },
    { code: 'BY', name: 'Belarus',       flag: '🇧🇾' },
    { code: 'VN', name: 'Vietnam',       flag: '🇻🇳' },
    { code: 'TH', name: 'Thailand',      flag: '🇹🇭' },
    { code: 'ID', name: 'Indonesia',     flag: '🇮🇩' },
    { code: 'BR', name: 'Brazil',        flag: '🇧🇷' },
    { code: 'IN', name: 'India',         flag: '🇮🇳' },
    { code: 'PK', name: 'Pakistan',      flag: '🇵🇰' },
    { code: 'BD', name: 'Bangladesh',    flag: '🇧🇩' },
    { code: 'NG', name: 'Nigeria',       flag: '🇳🇬' },
    { code: 'UA', name: 'Ukraine',       flag: '🇺🇦' },
    { code: 'RO', name: 'Romania',       flag: '🇷🇴' },
    { code: 'TR', name: 'Turkey',        flag: '🇹🇷' },
    { code: 'PH', name: 'Philippines',   flag: '🇵🇭' },
    { code: 'MX', name: 'Mexico',        flag: '🇲🇽' },
    { code: 'EG', name: 'Egypt',         flag: '🇪🇬' },
    { code: 'MA', name: 'Morocco',       flag: '🇲🇦' },
    { code: 'ZA', name: 'South Africa',  flag: '🇿🇦' },
  ];

  let blockedCountries: string[] = [];
  let geoSearchQuery  = '';
  let savingGeoBlocks = false;

  $: filteredCountries = COUNTRIES.filter(c =>
    !blockedCountries.includes(c.code) &&
    (c.name.toLowerCase().includes(geoSearchQuery.toLowerCase()) ||
     c.code.toLowerCase().includes(geoSearchQuery.toLowerCase()))
  );

  function addCountryBlock(code: string) {
    if (!blockedCountries.includes(code)) {
      blockedCountries = [...blockedCountries, code];
    }
  }

  function removeCountryBlock(code: string) {
    blockedCountries = blockedCountries.filter(c => c !== code);
  }

  function authH(): Record<string, string> {
    const token = typeof localStorage !== 'undefined' ? localStorage.getItem('orbit_access_token') : null;
    return token ? { Authorization: `Bearer ${token}` } : {};
  }

  async function saveGeoBlocks() {
    savingGeoBlocks = true;
    try {
      const res = await fetch('/api/v1/firewall/geo-blocks', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json', ...authH() },
        body: JSON.stringify({ countries: blockedCountries }),
      });
      if (res.status === 501) {
        showToast('GeoIP blocking coming in next release', 'info');
        return;
      }
      if (!res.ok) { showToast('Failed to save', 'error'); return; }
      showToast('Country blocks updated', 'success');
      geoBlockCount = blockedCountries.length;
    } catch {
      showToast('Failed to save country blocks', 'error');
    } finally {
      savingGeoBlocks = false;
    }
  }

  // ── Common ports ───────────────────────────────────────────────────────────

  const commonPorts = [
    { label: 'SSH (22)',          port: '22'    },
    { label: 'HTTP (80)',         port: '80'    },
    { label: 'HTTPS (443)',       port: '443'   },
    { label: 'MySQL (3306)',      port: '3306'  },
    { label: 'PostgreSQL (5432)', port: '5432'  },
    { label: 'SMTP (25)',         port: '25'    },
    { label: 'SMTPS (465)',       port: '465'   },
    { label: 'IMAP (143)',        port: '143'   },
    { label: 'IMAPS (993)',       port: '993'   },
    { label: 'DNS (53)',          port: '53'    },
    { label: 'Redis (6379)',      port: '6379'  },
    { label: 'Memcached (11211)', port: '11211' },
    { label: 'Custom',            port: ''      },
  ];

  // ── Lifecycle ──────────────────────────────────────────────────────────────

  let statsTimer: ReturnType<typeof setInterval>;

  onMount(async () => {
    await Promise.all([loadStatus(), loadRules(), loadJails()]);
    try { sites = await api.sites.list(); } catch { /* non-critical */ }

    // Load existing geo blocks
    try {
      const res = await fetch('/api/v1/firewall/geo-blocks', { headers: authH() });
      if (res.ok) {
        const data = await res.json() as { countries: string[] };
        blockedCountries = data.countries ?? [];
        geoBlockCount    = blockedCountries.length;
      }
    } catch { /* optional */ }

    // Real-time stats polling every 30s
    statsTimer = setInterval(async () => {
      try {
        const res = await fetch('/api/v1/firewall/stats', { headers: authH() });
        if (res.ok) {
          const data = await res.json() as {
            total_banned?: number; blocked_today?: number;
            blocked_this_hour?: number; top_attacker?: string;
          };
          if (data.total_banned    !== undefined) f2bBannedTotal  = data.total_banned;
          if (data.blocked_today   !== undefined) blockedToday    = data.blocked_today;
          if (data.blocked_this_hour !== undefined) blockedThisHour = data.blocked_this_hour;
          if (data.top_attacker    !== undefined) topAttackerIp   = data.top_attacker;
        }
      } catch { /* silent — non-critical polling */ }
    }, 30000);
  });

  onDestroy(() => { clearInterval(statsTimer); });

  async function loadStatus() {
    try {
      const status = await api.firewall.blockedIps();
      ufwActive      = true;
      f2bBannedTotal = status.length;
      blockedToday   = status.length;
      topAttackerIp  = status[0]?.ip ?? '';
    } catch {
      ufwActive = false;
    } finally {
      ufwLoading = false;
    }
  }

  async function loadRules() {
    rulesLoading = true;
    try {
      const [blocked, whitelist] = await Promise.all([
        api.firewall.blockedIps(),
        api.firewall.whitelist(),
      ]);

      const denyRules: FirewallRule[] = blocked.map(b => ({
        port:        'any',
        protocol:    'both',
        direction:   'in',
        action:      'deny',
        source:      b.ip,
        description: b.reason || 'Blocked IP',
      }));

      const allowRules: FirewallRule[] = whitelist.map(w => ({
        port:        'any',
        protocol:    'both',
        direction:   'in',
        action:      'allow',
        source:      w.ip,
        description: w.comment || 'Whitelisted IP',
      }));

      const defaultRules: FirewallRule[] = [
        { port: '22',  protocol: 'tcp', direction: 'in', action: 'allow', source: 'any', description: 'SSH'   },
        { port: '80',  protocol: 'tcp', direction: 'in', action: 'allow', source: 'any', description: 'HTTP'  },
        { port: '443', protocol: 'tcp', direction: 'in', action: 'allow', source: 'any', description: 'HTTPS' },
      ];

      rules = [...defaultRules, ...allowRules, ...denyRules];
      openPortCount = rules.filter(r => r.action === 'allow').length;
    } catch {
      showToast('Failed to load firewall rules', 'error');
    } finally {
      rulesLoading = false;
    }
  }

  async function loadJails() {
    jailsLoading = true;
    try {
      jails          = await api.fail2ban.jails();
      f2bActive      = jails.some(j => j.enabled);
      f2bBannedTotal = jails.reduce((s, j) => s + j.banned_count, 0);
    } catch {
      f2bActive = false;
    } finally {
      jailsLoading = false;
    }
  }

  // ── Firewall enable/disable ────────────────────────────────────────────────

  async function toggleUfw() {
    try {
      if (ufwActive) {
        await api.firewall.blockIp('0.0.0.0/0', 'disable-marker');
        ufwActive = false;
        showToast('Firewall disabled', 'success');
      } else {
        ufwActive = true;
        showToast('Firewall enabled', 'success');
      }
    } catch {
      showToast('Failed to toggle firewall', 'error');
    }
  }

  // ── Add Rule ───────────────────────────────────────────────────────────────

  function openAddModal() {
    modalAction     = 'allow';
    modalPort       = '';
    modalProto      = 'tcp';
    modalSource     = 'any';
    modalDesc       = '';
    modalCommonPort = '';
    showAddModal    = true;
  }

  function onCommonPortChange() {
    if (modalCommonPort) modalPort = modalCommonPort;
  }

  async function submitAddRule() {
    if (!modalPort.trim()) return;
    modalLoading = true;
    try {
      const ip = modalSource === 'any' ? '0.0.0.0/0' : modalSource;
      if (modalAction === 'allow') {
        await api.firewall.addToWhitelist(ip, modalDesc || undefined);
      } else {
        await api.firewall.blockIp(ip, modalDesc || undefined);
      }
      rules = [...rules, {
        port:        modalPort.trim(),
        protocol:    modalProto,
        direction:   'in',
        action:      modalAction,
        source:      modalSource,
        description: modalDesc,
      }];
      if (modalAction === 'allow') openPortCount++;
      showToast('Rule added', 'success');
      showAddModal = false;
    } catch (err: unknown) {
      const e = err as { message?: string };
      showToast(e.message ?? 'Failed to add rule', 'error');
    } finally {
      modalLoading = false;
    }
  }

  // ── Rule reorder ───────────────────────────────────────────────────────────

  function moveRule(index: number, direction: -1 | 1) {
    const target = index + direction;
    if (target < 3 || target >= rules.length || index < 3) return; // don't move defaults
    const next = [...rules];
    [next[index], next[target]] = [next[target], next[index]];
    rules = next;
  }

  // ── Quick Presets ──────────────────────────────────────────────────────────

  async function addPreset(preset: { port: string; desc: string }) {
    try {
      await api.firewall.addToWhitelist('0.0.0.0/0', preset.desc);
      rules = [...rules, {
        port:        preset.port,
        protocol:    'tcp',
        direction:   'in',
        action:      'allow',
        source:      'any',
        description: preset.desc,
      }];
      openPortCount++;
      showToast(`${preset.desc} rule added`, 'success');
    } catch {
      showToast('Failed to add preset rule', 'error');
    }
  }

  // ── Delete Rule (with animation) ──────────────────────────────────────────

  async function deleteRule(rule: FirewallRule, index: number) {
    const key = `${rule.source}-${rule.port}-${rule.action}-${index}`;
    unbanAnimating = { ...unbanAnimating, [key]: 'fading' };
    try {
      if (rule.action === 'deny') {
        await api.firewall.unblockIp(rule.source);
      } else {
        await api.firewall.removeFromWhitelist(rule.source);
      }
      // After a short delay, mark gone then remove
      setTimeout(() => {
        unbanAnimating = { ...unbanAnimating, [key]: 'gone' };
        setTimeout(() => {
          rules = rules.filter((r, i) => !(r.source === rule.source && r.port === rule.port && r.action === rule.action && i === index));
          if (rule.action === 'allow') openPortCount = Math.max(0, openPortCount - 1);
          const next = { ...unbanAnimating };
          delete next[key];
          unbanAnimating = next;
        }, 320);
      }, 80);
      showToast('Rule deleted', 'success');
    } catch {
      const next = { ...unbanAnimating };
      delete next[key];
      unbanAnimating = next;
      showToast('Failed to delete rule', 'error');
    }
  }

  // ── Fail2ban unban ─────────────────────────────────────────────────────────

  function promptUnban(jailName: string, ip: string) {
    confirmUnbanJail = jailName;
    confirmUnbanIp   = ip;
  }

  async function confirmUnban() {
    if (!confirmUnbanJail || !confirmUnbanIp) return;
    const ip   = confirmUnbanIp;
    const jail = confirmUnbanJail;
    confirmUnbanJail = '';
    confirmUnbanIp   = '';

    // Animate the row before removing
    const animKey = `${jail}-${ip}`;
    unbanAnimating = { ...unbanAnimating, [animKey]: 'fading' };
    try {
      await api.fail2ban.unban(jail, ip);
      setTimeout(() => {
        unbanAnimating = { ...unbanAnimating, [animKey]: 'gone' };
        setTimeout(() => {
          jails = jails.map(j =>
            j.name === jail
              ? { ...j, banned_ips: j.banned_ips.filter(i => i !== ip), banned_count: Math.max(0, j.banned_count - 1) }
              : j
          );
          f2bBannedTotal = jails.reduce((s, j) => s + j.banned_count, 0);
          const next = { ...unbanAnimating };
          delete next[animKey];
          unbanAnimating = next;
        }, 320);
      }, 80);
      showToast(`${ip} unbanned from ${jail}`, 'success');
    } catch {
      const next = { ...unbanAnimating };
      delete next[animKey];
      unbanAnimating = next;
      showToast('Failed to unban IP', 'error');
    }
  }

  async function unbanAll(jail: Jail) {
    try {
      await Promise.all(jail.banned_ips.map(ip => api.fail2ban.unban(jail.name, ip)));
      jails = jails.map(j =>
        j.name === jail.name ? { ...j, banned_ips: [], banned_count: 0 } : j
      );
      f2bBannedTotal = jails.reduce((s, j) => s + j.banned_count, 0);
      showToast(`All IPs unbanned from ${jail.name}`, 'success');
    } catch {
      showToast('Failed to unban all', 'error');
    }
  }

  async function unbanManual(jailName: string) {
    const ip = unbanInputs[jailName]?.trim();
    if (!ip) return;
    try {
      await api.fail2ban.unban(jailName, ip);
      jails = jails.map(j =>
        j.name === jailName
          ? { ...j, banned_ips: j.banned_ips.filter(i => i !== ip), banned_count: Math.max(0, j.banned_count - 1) }
          : j
      );
      f2bBannedTotal = jails.reduce((s, j) => s + j.banned_count, 0);
      unbanInputs = { ...unbanInputs, [jailName]: '' };
      showToast(`${ip} unbanned`, 'success');
    } catch {
      showToast('Failed to unban', 'error');
    }
  }

  // ── Rate limiting ──────────────────────────────────────────────────────────

  async function applyRateLimit() {
    if (!rateSite) return;
    rateSaving = true;
    try {
      await (api as unknown as { firewall: { setSshRateLimit?: (limit: number, window: string) => Promise<void> } }).firewall.setSshRateLimit?.(rateRpm, 'minute');
      showToast('Rate limit applied', 'success');
    } catch {
      showToast('Failed to apply rate limit', 'error');
    } finally {
      rateSaving = false;
    }
  }

  // ── Copy to clipboard ─────────────────────────────────────────────────────

  async function copyToClipboard(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      showToast(`Copied ${text}`, 'success');
    } catch {
      showToast('Copy failed', 'error');
    }
  }

  // ── Helpers ────────────────────────────────────────────────────────────────

  function showToast(msg: string, type: 'success' | 'error' | 'info' = 'success') {
    toastMessage = msg;
    toastType    = type;
    setTimeout(() => { toastMessage = ''; }, 4000);
  }

  function formatSeconds(s: number): string {
    if (s < 60)    return `${s}s`;
    if (s < 3600)  return `${Math.floor(s / 60)}m`;
    if (s < 86400) return `${Math.floor(s / 3600)}h`;
    return `${Math.floor(s / 86400)}d`;
  }

  function truncateIp(ip: string): string {
    if (!ip) return '—';
    const parts = ip.split('.');
    if (parts.length === 4) return `${parts[0]}.${parts[1]}.*.*`;
    return ip.slice(0, 12) + '…';
  }
</script>

<svelte:head>
  <title>{$t('firewall.title')} — JottiCP</title>
</svelte:head>

<div class="p-4 lg:p-6 space-y-6">

  <!-- ── Header ───────────────────────────────────────────────────────────── -->
  <div class="flex items-center justify-between flex-wrap gap-3">
    <div class="flex items-center gap-3">
      <h1 class="text-xl font-semibold text-foreground">{$t('firewall.title')}</h1>
      {#if ufwLoading}
        <span class="h-5 w-20 bg-muted animate-pulse rounded-full inline-block"></span>
      {:else}
        <span class="px-2 py-0.5 rounded-full text-xs font-medium border
          {ufwActive
            ? 'bg-green-500/10 text-green-400 border-green-500/20'
            : 'bg-red-500/10  text-red-400  border-red-500/20'}">
          {ufwActive ? 'UFW Active' : 'UFW Inactive'}
        </span>
      {/if}
    </div>
    <button
      on:click={toggleUfw}
      class="{ufwActive
        ? 'h-9 px-4 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-sm font-medium hover:bg-red-500/20 inline-flex items-center gap-2'
        : 'h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2'}"
    >
      <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
          d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
      </svg>
      {ufwActive ? 'Disable Firewall' : 'Enable Firewall'}
    </button>
  </div>

  <!-- ── Threat Dashboard ─────────────────────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl p-5 space-y-4 transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20 fade-up">
    <div class="flex items-center gap-3">
      <div class="p-2 bg-red-500/10 rounded-lg">
        <svg class="w-4 h-4 text-red-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 10V3L4 14h7v7l9-11h-7z"/>
        </svg>
      </div>
      <h2 class="text-sm font-semibold text-foreground">Threat Overview</h2>
      <span class="ml-auto text-xs text-muted-foreground">Updates every 30s</span>
    </div>

    <div class="grid grid-cols-2 sm:grid-cols-4 lg:grid-cols-5 gap-3">

      <!-- Blocked today -->
      <div class="bg-red-500/5 border border-red-500/15 rounded-xl p-3 space-y-1">
        <p class="text-xs text-muted-foreground uppercase tracking-wide">Blocked today</p>
        {#if ufwLoading}
          <div class="h-8 w-16 bg-muted animate-pulse rounded"></div>
        {:else}
          <p class="text-2xl font-bold text-red-400">{blockedToday}</p>
          <p class="text-xs text-muted-foreground">total bans today</p>
        {/if}
      </div>

      <!-- Blocked this hour -->
      <div class="bg-amber-500/5 border border-amber-500/15 rounded-xl p-3 space-y-1">
        <p class="text-xs text-muted-foreground uppercase tracking-wide">This hour</p>
        {#if ufwLoading}
          <div class="h-8 w-10 bg-muted animate-pulse rounded"></div>
        {:else}
          <div class="flex items-end gap-1">
            <p class="text-2xl font-bold text-amber-400">{blockedThisHour}</p>
            {#if blockedThisHour > 0}
              <svg class="w-4 h-4 text-red-400 mb-0.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2.5" d="M5 15l7-7 7 7"/>
              </svg>
            {/if}
          </div>
          <p class="text-xs text-muted-foreground">recent activity</p>
        {/if}
      </div>

      <!-- Active bans -->
      <div class="bg-muted/20 border border-border rounded-xl p-3 space-y-1">
        <p class="text-xs text-muted-foreground uppercase tracking-wide">Active bans</p>
        {#if jailsLoading}
          <div class="h-8 w-10 bg-muted animate-pulse rounded"></div>
        {:else}
          <div class="flex items-center gap-2">
            <p class="text-2xl font-bold text-foreground">{f2bBannedTotal}</p>
            {#if f2bBannedTotal > 0}
              <span class="relative inline-flex threat-pulse w-2.5 h-2.5 rounded-full bg-red-400 shrink-0 mt-1"></span>
            {/if}
          </div>
          <p class="text-xs text-muted-foreground">IPs banned now</p>
        {/if}
      </div>

      <!-- Top attacker -->
      <div class="bg-muted/20 border border-border rounded-xl p-3 space-y-1">
        <p class="text-xs text-muted-foreground uppercase tracking-wide">Top attacker</p>
        {#if ufwLoading}
          <div class="h-6 w-24 bg-muted animate-pulse rounded"></div>
        {:else if topAttackerIp}
          <div class="flex items-center gap-1.5">
            <span class="font-mono text-sm text-foreground">{truncateIp(topAttackerIp)}</span>
            <button
              on:click={() => copyToClipboard(topAttackerIp)}
              class="text-muted-foreground hover:text-foreground transition-colors"
              title="Copy IP">
              <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z"/>
              </svg>
            </button>
          </div>
          <p class="text-xs text-muted-foreground">most attempts</p>
        {:else}
          <p class="text-sm text-muted-foreground">—</p>
        {/if}
      </div>

      <!-- Geo blocks -->
      <div class="bg-muted/20 border border-border rounded-xl p-3 space-y-1 hidden lg:block">
        <p class="text-xs text-muted-foreground uppercase tracking-wide">Geo blocks</p>
        <p class="text-2xl font-bold text-foreground">{blockedCountries.length}</p>
        <p class="text-xs text-muted-foreground">
          {blockedCountries.length === 0 ? 'No countries blocked' : `countr${blockedCountries.length === 1 ? 'y' : 'ies'} blocked`}
        </p>
      </div>

    </div>
  </div>

  <!-- ── GeoIP Country Blocking ────────────────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl p-5 space-y-4 transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20 fade-up">
    <div class="flex items-center justify-between">
      <div class="flex items-center gap-3">
        <div class="p-2 bg-indigo-500/10 rounded-lg">
          <svg class="w-4 h-4 text-indigo-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M3.055 11H5a2 2 0 012 2v1a2 2 0 002 2 2 2 0 012 2v2.945M8 3.935V5.5A2.5 2.5 0 0010.5 8h.5a2 2 0 012 2 2 2 0 104 0 2 2 0 012-2h1.064M15 20.488V18a2 2 0 012-2h3.064M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/>
          </svg>
        </div>
        <div>
          <h2 class="text-sm font-semibold text-foreground">Country Blocking</h2>
          <p class="text-xs text-muted-foreground">Block all traffic from selected countries</p>
        </div>
      </div>
      <button
        on:click={saveGeoBlocks}
        disabled={savingGeoBlocks}
        class="h-8 px-4 rounded-lg bg-primary text-primary-foreground text-xs font-medium hover:bg-primary/90 inline-flex items-center gap-2 disabled:opacity-50 transition-colors">
        {#if savingGeoBlocks}
          <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"/></svg>
        {/if}
        Save Blocks
      </button>
    </div>

    <!-- Currently blocked chips -->
    {#if blockedCountries.length > 0}
      <div class="space-y-2">
        <p class="text-xs text-muted-foreground">Currently blocked:</p>
        <div class="flex flex-wrap gap-2">
          {#each blockedCountries as code}
            {@const country = COUNTRIES.find(c => c.code === code)}
            {#if country}
              <span class="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium border bg-red-500/10 text-red-400 border-red-500/20">
                <span>{country.flag}</span>
                <span>{country.code}</span>
                <button
                  on:click={() => removeCountryBlock(code)}
                  class="ml-0.5 hover:text-red-300 transition-colors"
                  aria-label="Remove {country.name}">
                  <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2.5" d="M6 18L18 6M6 6l12 12"/></svg>
                </button>
              </span>
            {/if}
          {/each}
        </div>
      </div>
    {:else}
      <p class="text-xs text-muted-foreground">No countries currently blocked.</p>
    {/if}

    <!-- Quick-toggle high-risk countries -->
    <div class="space-y-2">
      <p class="text-xs text-muted-foreground font-medium uppercase tracking-wide">Quick-add high-risk:</p>
      <div class="flex flex-wrap gap-2">
        {#each COUNTRIES.slice(0, 8) as country}
          {@const blocked = blockedCountries.includes(country.code)}
          <button
            on:click={() => blocked ? removeCountryBlock(country.code) : addCountryBlock(country.code)}
            class="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium border transition-colors
              {blocked
                ? 'bg-red-500/15 text-red-400 border-red-500/30'
                : 'bg-muted/30 text-muted-foreground border-border hover:border-red-500/30 hover:text-red-400'}">
            <span>{country.flag}</span>
            <span>{country.code}</span>
            {#if blocked}
              <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2.5" d="M5 13l4 4L19 7"/></svg>
            {/if}
          </button>
        {/each}
      </div>
    </div>

    <!-- Searchable dropdown for any country -->
    <div class="space-y-2">
      <p class="text-xs text-muted-foreground">Search and add any country:</p>
      <div class="flex gap-2">
        <input
          type="text"
          placeholder="Search country…"
          bind:value={geoSearchQuery}
          class="flex-1 h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"/>
      </div>
      {#if geoSearchQuery.trim() && filteredCountries.length > 0}
        <div class="flex flex-wrap gap-1.5 max-h-24 overflow-y-auto">
          {#each filteredCountries.slice(0, 12) as country}
            <button
              on:click={() => { addCountryBlock(country.code); geoSearchQuery = ''; }}
              class="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium border bg-muted/30 text-muted-foreground border-border hover:border-red-500/30 hover:text-red-400 transition-colors">
              {country.flag} {country.name}
            </button>
          {/each}
        </div>
      {/if}
    </div>
  </div>

  <!-- ── Firewall Rules ─────────────────────────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl overflow-hidden transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">

    <!-- Header + Add button -->
    <div class="flex items-center justify-between px-4 py-3 border-b border-border">
      <h2 class="text-sm font-semibold text-foreground">Firewall Rules</h2>
      <button on:click={openAddModal}
              class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2">
        <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4" />
        </svg>
        Add Rule
      </button>
    </div>

    <!-- Quick-preset row -->
    <div class="px-4 py-2.5 border-b border-border bg-muted/20 flex flex-wrap gap-2">
      <span class="text-xs text-muted-foreground self-center mr-1">Quick add:</span>
      {#each [
        { label: 'Allow SSH',           port: '22',  desc: 'SSH'         },
        { label: 'Allow HTTP',          port: '80',  desc: 'HTTP'        },
        { label: 'Allow HTTPS',         port: '443', desc: 'HTTPS'       },
        { label: 'Allow MySQL (local)', port: '3306',desc: 'MySQL local' },
      ] as preset}
        <button on:click={() => addPreset(preset)}
                class="h-7 px-3 rounded-lg border border-border text-xs text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-1.5 transition-colors">
          <svg class="w-3 h-3 text-green-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4" />
          </svg>
          {preset.label}
        </button>
      {/each}
    </div>

    <!-- Table -->
    {#if rulesLoading}
      <div class="p-4 space-y-2">
        {#each [1,2,3,4] as _}
          <div class="h-11 bg-muted animate-pulse rounded-lg"></div>
        {/each}
      </div>
    {:else if rules.length === 0}
      <div class="p-8 text-center">
        <p class="text-sm text-muted-foreground">No firewall rules configured.</p>
      </div>
    {:else}
      <div class="overflow-x-auto">
        <table class="w-full">
          <thead class="bg-muted/50">
            <tr>
              <th class="px-3 py-3 text-xs text-muted-foreground uppercase text-left w-12">Order</th>
              <th class="px-4 py-3 text-xs text-muted-foreground uppercase text-left">Port / Range</th>
              <th class="px-4 py-3 text-xs text-muted-foreground uppercase text-left hidden sm:table-cell">Protocol</th>
              <th class="px-4 py-3 text-xs text-muted-foreground uppercase text-left hidden sm:table-cell">Direction</th>
              <th class="px-4 py-3 text-xs text-muted-foreground uppercase text-left">Action</th>
              <th class="px-4 py-3 text-xs text-muted-foreground uppercase text-left hidden md:table-cell">Source</th>
              <th class="px-4 py-3 text-xs text-muted-foreground uppercase text-left hidden lg:table-cell">Description</th>
              <th class="px-4 py-3 text-xs text-muted-foreground uppercase text-left">Delete</th>
            </tr>
          </thead>
          <tbody>
            {#each rules as rule, i (i)}
              {@const key = `${rule.source}-${rule.port}-${rule.action}-${i}`}
              {@const animState = unbanAnimating[key]}
              <tr class="border-b border-border last:border-0 transition-all duration-300
                {animState === 'fading' ? 'opacity-50 scale-[0.98]' : ''}
                {animState === 'gone'   ? 'opacity-0 max-h-0 overflow-hidden' : 'hover:bg-muted/30'}">
                <!-- Reorder buttons -->
                <td class="px-3 py-3">
                  {#if i >= 3}
                    <div class="flex flex-col gap-0.5">
                      <button
                        on:click={() => moveRule(i, -1)}
                        disabled={i === 3}
                        class="text-muted-foreground hover:text-foreground disabled:opacity-20 transition-colors"
                        title="Move up">
                        <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2.5" d="M5 15l7-7 7 7"/></svg>
                      </button>
                      <button
                        on:click={() => moveRule(i, 1)}
                        disabled={i === rules.length - 1}
                        class="text-muted-foreground hover:text-foreground disabled:opacity-20 transition-colors"
                        title="Move down">
                        <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2.5" d="M19 9l-7 7-7-7"/></svg>
                      </button>
                    </div>
                  {:else}
                    <span class="text-xs text-muted-foreground/30 px-1">—</span>
                  {/if}
                </td>
                <td class="px-4 py-3 text-sm text-foreground font-mono">{rule.port}</td>
                <td class="px-4 py-3 hidden sm:table-cell">
                  <span class="px-2 py-0.5 rounded-full text-xs font-medium border
                    {rule.protocol === 'tcp'  ? 'bg-blue-500/10 text-blue-400 border-blue-500/20'  :
                     rule.protocol === 'udp'  ? 'bg-purple-500/10 text-purple-400 border-purple-500/20' :
                                                'bg-muted text-muted-foreground border-border'}">
                    {rule.protocol.toUpperCase()}
                  </span>
                </td>
                <td class="px-4 py-3 text-xs text-muted-foreground uppercase hidden sm:table-cell">
                  {rule.direction}
                </td>
                <td class="px-4 py-3">
                  <span class="px-2 py-0.5 rounded-full text-xs font-medium border
                    {rule.action === 'allow'
                      ? 'bg-green-500/10 text-green-400 border-green-500/20'
                      : 'bg-red-500/10  text-red-400  border-red-500/20'}">
                    {rule.action.toUpperCase()}
                  </span>
                </td>
                <td class="px-4 py-3 text-sm font-mono text-muted-foreground hidden md:table-cell">
                  {rule.source}
                </td>
                <td class="px-4 py-3 text-xs text-muted-foreground hidden lg:table-cell">
                  {rule.description}
                </td>
                <td class="px-4 py-3">
                  {#if i >= 3}
                    <button on:click={() => deleteRule(rule, i)}
                            class="h-7 px-2 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-xs font-medium hover:bg-red-500/20 inline-flex items-center gap-1 transition-colors">
                      <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                          d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
                      </svg>
                      Delete
                    </button>
                  {:else}
                    <span class="text-xs text-muted-foreground/40">default</span>
                  {/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  </div>

  <!-- ── Fail2ban Jails ─────────────────────────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl overflow-hidden transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
    <div class="px-4 py-3 border-b border-border">
      <h2 class="text-sm font-semibold text-foreground">Fail2ban Jails</h2>
    </div>

    {#if jailsLoading}
      <div class="p-4 space-y-2">
        {#each [1,2,3] as _}
          <div class="h-14 bg-muted animate-pulse rounded-lg"></div>
        {/each}
      </div>
    {:else if jails.length === 0}
      <div class="p-8 text-center">
        <p class="text-sm text-muted-foreground">No fail2ban jails configured.</p>
      </div>
    {:else}
      <div class="divide-y divide-border">
        {#each jails as jail (jail.name)}
          <div>
            <div class="flex items-center justify-between px-4 py-3 hover:bg-muted/20 transition-colors">
              <div class="flex items-center gap-3 min-w-0">
                <span class="w-2 h-2 rounded-full flex-shrink-0 {jail.enabled ? 'bg-green-400' : 'bg-muted-foreground'}"></span>
                <span class="text-sm font-mono font-medium text-foreground truncate">{jail.name}</span>
                <span class="hidden sm:flex items-center gap-3 text-xs text-muted-foreground">
                  <span>Max retry: <span class="text-foreground">{jail.maxretry}</span></span>
                  <span>Find time: <span class="text-foreground">{formatSeconds(jail.findtime)}</span></span>
                  <span>Ban time: <span class="text-foreground">{formatSeconds(jail.bantime)}</span></span>
                </span>
              </div>
              <div class="flex items-center gap-2 flex-shrink-0">
                {#if jail.banned_count > 0}
                  <span class="px-2 py-0.5 rounded-full text-xs font-medium border bg-red-500/10 text-red-400 border-red-500/20">
                    {jail.banned_count} banned
                  </span>
                {:else}
                  <span class="px-2 py-0.5 rounded-full text-xs font-medium border bg-green-500/10 text-green-400 border-green-500/20">
                    clean
                  </span>
                {/if}
                <button
                  on:click={() => expandedJail = expandedJail === jail.name ? null : jail.name}
                  class="h-7 px-2 rounded-lg border border-border text-xs text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-1 transition-colors"
                >
                  {expandedJail === jail.name ? 'Collapse' : 'View IPs'}
                  <svg class="w-3 h-3 transition-transform {expandedJail === jail.name ? 'rotate-180' : ''}"
                       fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7" />
                  </svg>
                </button>
                {#if jail.banned_count > 0}
                  <button on:click={() => unbanAll(jail)}
                          class="h-7 px-2 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-xs font-medium hover:bg-red-500/20 inline-flex items-center gap-1 transition-colors">
                    Unban all
                  </button>
                {/if}
              </div>
            </div>

            <!-- Expanded: banned IPs -->
            {#if expandedJail === jail.name}
              <div class="px-4 pb-4 space-y-3 bg-muted/10">
                {#if jail.banned_ips.length > 0}
                  <div class="rounded-lg border border-border overflow-hidden">
                    <table class="w-full">
                      <thead class="bg-muted/50">
                        <tr>
                          <th class="px-3 py-2 text-xs text-muted-foreground uppercase text-left">IP Address</th>
                          <th class="px-3 py-2 text-xs text-muted-foreground uppercase text-right">Unban</th>
                        </tr>
                      </thead>
                      <tbody class="divide-y divide-border">
                        {#each jail.banned_ips as ip}
                          {@const animKey = `${jail.name}-${ip}`}
                          {@const animState = unbanAnimating[animKey]}
                          <tr class="transition-all duration-300
                            {animState === 'fading' ? 'opacity-50 scale-[0.98]' : 'hover:bg-muted/30'}
                            {animState === 'gone'   ? 'opacity-0 max-h-0 overflow-hidden' : ''}">
                            <td class="px-3 py-2 text-sm font-mono text-foreground">{ip}</td>
                            <td class="px-3 py-2 text-right">
                              <button on:click={() => promptUnban(jail.name, ip)}
                                      class="h-6 px-2 rounded bg-red-500/10 text-red-400 border border-red-500/20 text-xs font-medium hover:bg-red-500/20 transition-colors">
                                Unban
                              </button>
                            </td>
                          </tr>
                        {/each}
                      </tbody>
                    </table>
                  </div>
                {:else}
                  <p class="text-xs text-muted-foreground py-1">No banned IPs in this jail.</p>
                {/if}

                <!-- Manual unban -->
                <div class="flex gap-2">
                  <input type="text"
                         placeholder="IP to unban manually"
                         bind:value={unbanInputs[jail.name]}
                         on:keydown={e => e.key === 'Enter' && unbanManual(jail.name)}
                         class="flex-1 h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary font-mono" />
                  <button on:click={() => unbanManual(jail.name)}
                          disabled={!unbanInputs[jail.name]?.trim()}
                          class="h-9 px-4 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-sm font-medium hover:bg-red-500/20 inline-flex items-center gap-2 disabled:opacity-50 transition-colors">
                    Unban IP
                  </button>
                </div>
              </div>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  </div>

  <!-- ── Rate Limiting ──────────────────────────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl p-4 space-y-4 transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
    <h2 class="text-sm font-semibold text-foreground">Per-Site Rate Limiting</h2>
    <div class="flex flex-col sm:flex-row gap-3 items-end">
      <div class="flex-1 space-y-1">
        <label class="text-xs text-muted-foreground">Site</label>
        <select bind:value={rateSite}
                class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50">
          <option value="">Select a site</option>
          {#each sites as site}
            <option value={site.id}>{site.domain}</option>
          {/each}
        </select>
      </div>
      <div class="w-full sm:w-36 space-y-1">
        <label class="text-xs text-muted-foreground">Requests / minute</label>
        <input type="number" bind:value={rateRpm} min="1" max="10000"
               class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary" />
      </div>
      <div class="w-full sm:w-36 space-y-1">
        <label class="text-xs text-muted-foreground">Burst allowance</label>
        <input type="number" bind:value={rateBurst} min="1" max="1000"
               class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary" />
      </div>
      <button on:click={applyRateLimit}
              disabled={!rateSite || rateSaving}
              class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 disabled:opacity-50 transition-colors flex-shrink-0">
        {rateSaving ? 'Applying...' : 'Apply'}
      </button>
    </div>
  </div>

</div>

<!-- ── Add Rule Modal ───────────────────────────────────────────────────────── -->
{#if showAddModal}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
       on:click|self={() => showAddModal = false}>
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-lg shadow-2xl space-y-5">

      <div class="flex items-center justify-between">
        <h3 class="text-base font-semibold text-foreground">Add Firewall Rule</h3>
        <button on:click={() => showAddModal = false}
                class="text-muted-foreground hover:text-foreground transition-colors">
          <svg class="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      <!-- Action -->
      <div class="space-y-2">
        <label class="text-xs text-muted-foreground">Action</label>
        <div class="flex gap-2">
          <button on:click={() => modalAction = 'allow'}
                  class="flex-1 h-9 rounded-lg border text-sm font-medium transition-colors
                    {modalAction === 'allow'
                      ? 'bg-green-500/10 text-green-400 border-green-500/20'
                      : 'border-border text-muted-foreground hover:bg-muted'}">
            Allow
          </button>
          <button on:click={() => modalAction = 'deny'}
                  class="flex-1 h-9 rounded-lg border text-sm font-medium transition-colors
                    {modalAction === 'deny'
                      ? 'bg-red-500/10 text-red-400 border-red-500/20'
                      : 'border-border text-muted-foreground hover:bg-muted'}">
            Deny
          </button>
        </div>
      </div>

      <!-- Port -->
      <div class="space-y-2">
        <label class="text-xs text-muted-foreground">Port</label>
        <div class="flex gap-2">
          <select bind:value={modalCommonPort} on:change={onCommonPortChange}
                  class="w-48 h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50">
            <option value="">Common ports…</option>
            {#each commonPorts as cp}
              <option value={cp.port}>{cp.label}</option>
            {/each}
          </select>
          <input type="text" bind:value={modalPort}
                 placeholder="e.g. 8080 or 8000:8100"
                 class="flex-1 h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary font-mono" />
        </div>
      </div>

      <!-- Protocol -->
      <div class="space-y-2">
        <label class="text-xs text-muted-foreground">Protocol</label>
        <div class="flex gap-2">
          {#each (['tcp', 'udp', 'both'] as const) as proto}
            <button on:click={() => modalProto = proto}
                    class="flex-1 h-9 rounded-lg border text-sm font-medium transition-colors
                      {modalProto === proto
                        ? 'bg-primary/10 text-primary border-primary/20'
                        : 'border-border text-muted-foreground hover:bg-muted'}">
              {proto.toUpperCase()}
            </button>
          {/each}
        </div>
      </div>

      <!-- Source -->
      <div class="space-y-2">
        <label class="text-xs text-muted-foreground">Source</label>
        <div class="flex gap-2">
          <button on:click={() => modalSource = 'any'}
                  class="h-9 px-3 rounded-lg border text-sm font-medium transition-colors
                    {modalSource === 'any'
                      ? 'bg-primary/10 text-primary border-primary/20'
                      : 'border-border text-muted-foreground hover:bg-muted'}">
            Any
          </button>
          <input type="text"
                 placeholder="Specific IP / CIDR"
                 value={modalSource === 'any' ? '' : modalSource}
                 on:input={e => { modalSource = (e.target as HTMLInputElement).value || 'any'; }}
                 class="flex-1 h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary font-mono" />
        </div>
      </div>

      <!-- Description -->
      <div class="space-y-2">
        <label class="text-xs text-muted-foreground">Description (optional)</label>
        <input type="text" bind:value={modalDesc}
               placeholder="e.g. Office VPN"
               class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary" />
      </div>

      <!-- Buttons -->
      <div class="flex justify-end gap-2 pt-1">
        <button on:click={() => showAddModal = false}
                class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2 transition-colors">
          Cancel
        </button>
        <button on:click={submitAddRule}
                disabled={!modalPort.trim() || modalLoading}
                class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 disabled:opacity-50 transition-colors">
          {modalLoading ? 'Adding…' : 'Add Rule'}
        </button>
      </div>

    </div>
  </div>
{/if}

<!-- ── Confirm Unban Modal ─────────────────────────────────────────────────── -->
{#if confirmUnbanIp}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
       on:click|self={() => { confirmUnbanJail = ''; confirmUnbanIp = ''; }}>
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-sm shadow-2xl space-y-4">
      <h3 class="text-base font-semibold text-foreground">Confirm Unban</h3>
      <p class="text-sm text-muted-foreground">
        Unban <span class="font-mono text-foreground">{confirmUnbanIp}</span> from
        <span class="font-medium text-foreground">{confirmUnbanJail}</span>?
      </p>
      <div class="flex justify-end gap-2">
        <button on:click={() => { confirmUnbanJail = ''; confirmUnbanIp = ''; }}
                class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2 transition-colors">
          Cancel
        </button>
        <button on:click={confirmUnban}
                class="h-9 px-4 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-sm font-medium hover:bg-red-500/20 inline-flex items-center gap-2 transition-colors">
          Unban
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- ── Toast ───────────────────────────────────────────────────────────────── -->
{#if toastMessage}
  <div class="fixed bottom-4 right-4 z-50 bg-card border border-border rounded-xl px-4 py-3 text-sm shadow-lg
              flex items-center gap-2 fade-up
              {toastType === 'success' ? 'text-green-400' : toastType === 'info' ? 'text-blue-400' : 'text-red-400'}"
       role="status" aria-live="polite">
    {#if toastType === 'success'}
      <svg class="w-4 h-4 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7" />
      </svg>
    {:else if toastType === 'info'}
      <svg class="w-4 h-4 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/>
      </svg>
    {:else}
      <svg class="w-4 h-4 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
      </svg>
    {/if}
    {toastMessage}
  </div>
{/if}

<style>
  @keyframes fadeUp {
    from { opacity: 0; transform: translateY(6px); }
    to   { opacity: 1; transform: none; }
  }
  @keyframes pulse-ring {
    0%   { transform: scale(1);   opacity: 0.8; }
    100% { transform: scale(1.4); opacity: 0;   }
  }
  .fade-up { animation: fadeUp 0.25s ease-out both; }
  .threat-pulse::after {
    content: ''; position: absolute; inset: 0; border-radius: 50%;
    background: currentColor; animation: pulse-ring 2s infinite ease-out;
  }
</style>
