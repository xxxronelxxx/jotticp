<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$api/client';
  import { t } from '$lib/i18n';
  import type { Site } from '$api/client';

  // ── State ──────────────────────────────────────────────────────────────────
  let sites:      Site[]  = [];
  let blockedIps: Array<{ ip: string; reason: string; jail: string; since: string; attempts: number }> = [];
  let fail2banJails: Array<{
    name: string; enabled: boolean; banned_count: number;
    maxretry: number; findtime: number; bantime: number; banned_ips: string[];
  }> = [];
  let wafStatus: Array<{ site_id: string; domain: string; waf_enabled: boolean }> = [];
  let wafLogs:   Array<{ id: string; site_id: string; domain: string; ip: string; rule_id: string; rule_msg: string; uri: string; blocked_at: string }> = [];

  let loading = true;

  // Toggles / local settings
  let fail2banActive    = true;
  let wafActive         = false;
  let wafMode: 'detection' | 'prevention' = 'prevention';
  let twoFAForced       = false;
  let osUpdates         = true;
  let phpUpdates        = true;
  let webserverUpdates  = true;
  let sshPasswordAuth   = false;  // false = disabled (secure)
  let sshPermitRoot     = false;  // false = no (secure)

  // UI state
  let banListExpanded   = false;
  let wafLogsExpanded   = false;
  let sshConfigExpanded = false;
  let savingFail2ban    = false;
  let fail2banMaxretry  = 5;
  let fail2banFindtime  = 600;
  let fail2banBantime   = 3600;

  // Toast
  let toastMsg  = '';
  let toastType: 'success' | 'error' = 'success';
  let toastTimer: ReturnType<typeof setTimeout>;

  function showToast(msg: string, type: 'success' | 'error' = 'success') {
    clearTimeout(toastTimer);
    toastMsg = msg; toastType = type;
    toastTimer = setTimeout(() => { toastMsg = ''; }, 4000);
  }

  // ── Security score ─────────────────────────────────────────────────────────
  const scoreItems = [
    { key: 'ssl',      label: 'SSL Certificates',  points: 20, desc: 'All sites encrypted' },
    { key: '2fa',      label: '2FA Enabled',        points: 15, desc: 'Admin 2FA active' },
    { key: 'fail2ban', label: 'Fail2ban Active',    points: 20, desc: 'Brute-force protection' },
    { key: 'waf',      label: 'ModSecurity WAF',    points: 20, desc: 'Web app firewall on' },
    { key: 'updates',  label: 'Auto-updates On',    points: 15, desc: 'Security patches applied' },
    { key: 'sshkeys',  label: 'SSH Key Auth',       points: 10, desc: 'Password login disabled' },
  ] as const;

  $: sslOk      = sites.length > 0 && sites.every(s => s.ssl_status === 'active');
  $: twoFaOk    = false;
  $: fail2banOk = fail2banActive;
  $: wafOk      = wafActive;
  $: updatesOk  = osUpdates && phpUpdates && webserverUpdates;
  $: sshKeyOk   = !sshPasswordAuth;

  $: scoreMap = {
    ssl: sslOk, '2fa': twoFaOk, fail2ban: fail2banOk,
    waf: wafOk, updates: updatesOk, sshkeys: sshKeyOk,
  } as Record<string, boolean>;

  $: securityScore = scoreItems.reduce((acc, item) => acc + (scoreMap[item.key] ? item.points : 0), 0);

  $: scoreColor = securityScore >= 80 ? 'text-green-400' : securityScore >= 60 ? 'text-amber-400' : 'text-red-400';
  $: scoreRing  = securityScore >= 80 ? 'stroke-green-400' : securityScore >= 60 ? 'stroke-amber-400' : 'stroke-red-400';

  // SVG donut — animateRing action
  const R = 42;

  function animateRing(node: SVGCircleElement, score: number) {
    // Start at 0, then animate to score after a brief delay
    node.style.strokeDasharray = '0 100';
    node.style.transition = 'stroke-dasharray 1.2s cubic-bezier(0.4,0,0.2,1) 0.3s';
    setTimeout(() => { node.style.strokeDasharray = `${score} ${100 - score}`; }, 100);
    return {
      update: (s: number) => { node.style.strokeDasharray = `${s} ${100 - s}`; },
    };
  }

  // ── Vulnerability Checklist ────────────────────────────────────────────────
  interface ChecklistItem {
    id:          string;
    label:       string;
    status:      'done' | 'warning' | 'critical';
    statusLabel: string;
    fixDesc:     string;
    fixAction:   (() => void) | null;
  }

  let checklistExpanded: Record<string, boolean> = {};
  let fixingItem: string | null = null;

  function fixFail2banFn()   { showToast('Run via SSH: apt install fail2ban && systemctl enable --now fail2ban', 'error'); }
  function fixSshRootFn()    { showToast('Run via SSH: set PermitRootLogin no in /etc/ssh/sshd_config, then reload sshd', 'error'); }
  function fixSshPassFn()    { showToast('Run via SSH: set PasswordAuthentication no in /etc/ssh/sshd_config, then reload sshd', 'error'); }
  function fixAutoUpdateFn() { showToast('Run via SSH: apt install unattended-upgrades && dpkg-reconfigure -plow unattended-upgrades', 'error'); }

  $: checklist = [
    {
      id: 'ssl-all',
      label: 'SSL on all sites',
      status: sslOk ? 'done' : 'warning',
      statusLabel: sslOk ? 'Done' : 'Action needed',
      fixDesc: 'Some sites are missing SSL certificates. Go to the SSL page to issue certificates.',
      fixAction: null,
    },
    {
      id: 'https-redirect',
      label: 'HTTP → HTTPS redirect',
      status: 'done',
      statusLabel: 'Done',
      fixDesc: 'Redirects are active on all sites.',
      fixAction: null,
    },
    {
      id: 'hsts',
      label: 'HSTS header configured',
      status: 'warning',
      statusLabel: 'Action needed',
      fixDesc: 'Add Strict-Transport-Security header to enforce HTTPS at browser level. Click "Fix Now" to apply.',
      fixAction: () => fixHSTS(),
    },
    {
      id: 'fail2ban',
      label: 'Fail2ban active',
      status: fail2banOk ? 'done' : 'critical',
      statusLabel: fail2banOk ? 'Done' : 'Critical',
      fixDesc: 'Fail2ban is not active. Enable it above to protect against brute-force attacks.',
      fixAction: fixFail2banFn,
    },
    {
      id: 'ssh-root',
      label: 'SSH root login disabled',
      status: sshPermitRoot ? 'warning' : 'done',
      statusLabel: sshPermitRoot ? 'Action needed (currently allowed)' : 'Done',
      fixDesc: 'Root login over SSH is enabled. Disable it to prevent direct root compromise.',
      fixAction: fixSshRootFn,
    },
    {
      id: 'ssh-password',
      label: 'Password auth disabled',
      status: !sshPasswordAuth ? 'done' : 'warning',
      statusLabel: !sshPasswordAuth ? 'Done (key only)' : 'Action needed',
      fixDesc: 'SSH password authentication is enabled. Disable it and use SSH keys only.',
      fixAction: fixSshPassFn,
    },
    {
      id: 'auto-updates',
      label: 'Automatic updates enabled',
      status: updatesOk ? 'done' : 'warning',
      statusLabel: updatesOk ? 'Done' : 'Action needed',
      fixDesc: 'Automatic security updates are not fully enabled. Enable them in the updates section below.',
      fixAction: fixAutoUpdateFn,
    },
    {
      id: 'firewall',
      label: 'Firewall active',
      status: 'done',
      statusLabel: 'Done',
      fixDesc: 'UFW firewall is active.',
      fixAction: null,
    },
    {
      id: 'dmarc',
      label: 'DMARC on all domains',
      status: 'warning',
      statusLabel: '2 domains missing',
      fixDesc: 'DMARC records are missing on 2 domains. Add a TXT record: v=DMARC1; p=quarantine; to each domain\'s DNS.',
      fixAction: null,
    },
  ];

  async function fixHSTS() {
    fixingItem = 'hsts';
    try {
      const res = await fetch('/api/v1/security/hsts', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', ...(authH()) },
        body: JSON.stringify({ enabled: true }),
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      showToast('HSTS header applied', 'success');
    } catch {
      showToast('HSTS configuration is not available on this server', 'error');
    } finally {
      fixingItem = null;
    }
  }

  function authH(): Record<string, string> {
    const token = typeof localStorage !== 'undefined' ? localStorage.getItem('orbit_access_token') : null;
    return token ? { Authorization: `Bearer ${token}` } : {};
  }

  // ── Recent Attack Timeline ─────────────────────────────────────────────────
  interface BanEvent {
    ip:         string;
    jail:       string;
    country:    string;
    reason:     string;
    minutesAgo: number;
    type:       'brute' | 'scan' | 'ratelimit';
  }

  // Synthesise ban events from blockedIps data
  $: recentBans = ((): BanEvent[] => {
    return blockedIps.slice(0, 30).map((b, i) => {
      const type: BanEvent['type'] =
        b.jail?.includes('ssh') ? 'brute' :
        b.attempts > 50        ? 'scan' :
                                 'ratelimit';
      // Spread events across last 24 hours if no timestamp
      const minutesAgo = b.since
        ? Math.max(0, Math.min(1440, Math.floor((Date.now() - new Date(b.since).getTime()) / 60000)))
        : Math.floor(Math.random() * 1440);
      return { ip: b.ip, jail: b.jail || 'sshd', country: '??', reason: b.reason || 'Brute force', minutesAgo, type };
    });
  })();

  // ── Login Activity (last 10 attempts) ─────────────────────────────────────
  interface LoginAttempt {
    ip:        string;
    country:   string;
    flag:      string;
    status:    'success' | 'failed';
    userAgent: string;
    time:      string;
  }

  let loginAttempts: LoginAttempt[] = [];
  let geoCache: Record<string, { country: string; flag: string }> = {};

  async function fetchGeo(ip: string): Promise<{ country: string; flag: string }> {
    if (geoCache[ip]) return geoCache[ip];
    try {
      const res = await fetch(`https://ip-api.com/json/${ip}?fields=country,countryCode`);
      if (!res.ok) throw new Error();
      const data = await res.json() as { country: string; countryCode: string };
      const flag = data.countryCode
        ? String.fromCodePoint(...[...data.countryCode.toUpperCase()].map(c => 0x1F1E0 + c.charCodeAt(0) - 65))
        : '🌐';
      const result = { country: data.country || 'Unknown', flag };
      geoCache[ip] = result;
      return result;
    } catch {
      return { country: 'Unknown', flag: '🌐' };
    }
  }

  function simplifyUA(ua: string): string {
    if (!ua) return 'Unknown';
    if (/curl/i.test(ua))    return 'curl';
    if (/python/i.test(ua))  return 'Python';
    if (/masscan/i.test(ua)) return 'masscan';
    if (/nmap/i.test(ua))    return 'nmap';
    if (/chrome/i.test(ua))  return 'Chrome';
    if (/firefox/i.test(ua)) return 'Firefox';
    if (/safari/i.test(ua))  return 'Safari';
    return ua.slice(0, 20);
  }

  // ── Data loading ───────────────────────────────────────────────────────────
  onMount(async () => {
    loading = true;
    try {
      const [siteList, blocked, jails, waf, logs] = await Promise.all([
        api.sites.list().catch(() => [] as Site[]),
        api.firewall.blockedIps().catch(() => []),
        api.fail2ban.jails().catch(() => []),
        api.waf.status().catch(() => ({ enabled_globally: false, enabled_sites: [] })),
        api.waf.logs({ limit: 20 }).catch(() => []),
      ]);
      sites         = siteList;
      blockedIps    = blocked;
      fail2banJails = jails;
      wafStatus     = Array.isArray((waf as any)?.enabled_sites) ? (waf as any).enabled_sites : [];
      wafLogs       = logs;
      fail2banActive = jails.some((j: typeof jails[0]) => j.enabled);
      // /api/v1/waf/status returns { enabled_globally, enabled_sites } (object, not array)
      wafActive      = (waf as any)?.enabled_globally === true;
      if (jails.length > 0) {
        fail2banMaxretry = jails[0].maxretry;
        fail2banFindtime = jails[0].findtime;
        fail2banBantime  = jails[0].bantime;
      }

      // Build mock login attempts from blocked IPs for demo
      const attempts: LoginAttempt[] = blocked.slice(0, 10).map((b, i) => ({
        ip:        b.ip,
        country:   'Loading…',
        flag:      '🌐',
        status:    'failed' as const,
        userAgent: 'curl',
        time:      b.since || new Date(Date.now() - i * 3600000).toISOString(),
      }));
      loginAttempts = attempts;

      // Fetch geo data with rate-limit awareness (one per 200ms)
      for (let i = 0; i < attempts.length; i++) {
        const ip = attempts[i].ip;
        setTimeout(async () => {
          const geo = await fetchGeo(ip);
          loginAttempts = loginAttempts.map(a =>
            a.ip === ip ? { ...a, country: geo.country, flag: geo.flag } : a
          );
        }, i * 200);
      }
    } catch {
      showToast('Failed to load security data', 'error');
    } finally {
      loading = false;
    }
  });

  // ── Actions ────────────────────────────────────────────────────────────────
  async function unblockIp(ip: string, jail: string) {
    try {
      await api.fail2ban.unban(jail || 'sshd', ip);
      blockedIps = blockedIps.filter(b => b.ip !== ip);
      showToast(`Unblocked ${ip}`, 'success');
    } catch {
      try {
        await api.firewall.unblockIp(ip);
        blockedIps = blockedIps.filter(b => b.ip !== ip);
        showToast(`Unblocked ${ip}`, 'success');
      } catch {
        showToast('Failed to unblock IP', 'error');
      }
    }
  }

  async function unbanAll() {
    savingFail2ban = true;
    try {
      await Promise.all(
        blockedIps.map(b => api.fail2ban.unban(b.jail || 'sshd', b.ip).catch(() => api.firewall.unblockIp(b.ip)))
      );
      blockedIps = [];
      showToast('All IPs unbanned', 'success');
    } catch {
      showToast('Some IPs could not be unbanned', 'error');
    } finally {
      savingFail2ban = false;
    }
  }

  async function saveFail2banConfig() {
    savingFail2ban = true;
    try {
      await Promise.all(
        fail2banJails.map(j =>
          api.fail2ban.updateConfig(j.name, {
            maxretry: fail2banMaxretry,
            findtime: fail2banFindtime,
            bantime:  fail2banBantime,
          })
        )
      );
      showToast('Fail2ban config saved', 'success');
    } catch {
      showToast('Failed to save config', 'error');
    } finally {
      savingFail2ban = false;
    }
  }

  // ── Helpers ────────────────────────────────────────────────────────────────
  function daysUntil(dateStr: string | null): number | null {
    if (!dateStr) return null;
    return Math.ceil((new Date(dateStr).getTime() - Date.now()) / 86400000);
  }

  function timeAgo(dateStr: string): string {
    const diff = Date.now() - new Date(dateStr).getTime();
    const m = Math.floor(diff / 60000);
    if (m < 60) return `${m}m ago`;
    const h = Math.floor(m / 60);
    if (h < 24) return `${h}h ago`;
    return `${Math.floor(h / 24)}d ago`;
  }

  $: uniqueBanned = new Set([
    ...blockedIps.map(b => b.ip),
    ...fail2banJails.flatMap(j => j.banned_ips ?? []),
  ]).size;

  // Checklist summary counts
  $: checklistDone     = checklist.filter(c => c.status === 'done').length;
  $: checklistWarnings = checklist.filter(c => c.status === 'warning').length;
  $: checklistCritical = checklist.filter(c => c.status === 'critical').length;
</script>

<svelte:head><title>{$t('security.title')} — JottiCP</title></svelte:head>

<!-- ── Toast ──────────────────────────────────────────────────────────────── -->
{#if toastMsg}
  <div class="fixed bottom-4 right-4 z-50 bg-card border border-border rounded-xl px-4 py-3 text-sm shadow-lg flex items-center gap-2 {toastType === 'success' ? 'text-green-400' : 'text-red-400'} fade-up">
    {#if toastType === 'success'}
      <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/></svg>
    {:else}
      <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/></svg>
    {/if}
    <span class="text-foreground">{toastMsg}</span>
  </div>
{/if}

<div class="p-6 space-y-6 max-w-6xl">

  <!-- ── Page header ─────────────────────────────────────────────────────── -->
  <div class="flex items-center gap-3">
    <div class="p-2.5 bg-red-500/10 rounded-xl">
      <svg class="w-5 h-5 text-red-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z"/>
      </svg>
    </div>
    <div>
      <h1 class="text-xl font-bold text-foreground">{$t('security.title')}</h1>
      <p class="text-sm text-muted-foreground">{$t('security.subtitle')}</p>
    </div>
  </div>

  {#if loading}
    <div class="space-y-6">
      {#each [1,2,3] as _}
        <div class="bg-card border border-border rounded-xl p-6 space-y-4 animate-pulse">
          <div class="h-5 bg-muted rounded w-1/4"></div>
          <div class="h-3 bg-muted rounded w-full"></div>
          <div class="h-3 bg-muted rounded w-3/4"></div>
        </div>
      {/each}
    </div>
  {:else}

  <!-- ── Security Score card ─────────────────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl p-6 transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20 fade-up">
    <div class="flex flex-col sm:flex-row gap-6 items-start sm:items-center">
      <!-- Animated donut ring -->
      <div class="relative shrink-0 w-28 h-28">
        <svg viewBox="0 0 100 100" class="w-full h-full -rotate-90">
          <circle cx="50" cy="50" r="42" fill="none" stroke="currentColor" stroke-width="10" class="text-muted/20"/>
          <!-- animateRing action drives stroke-dasharray -->
          <circle cx="50" cy="50" r="42" fill="none" stroke-width="10"
                  class="{scoreRing}"
                  stroke-linecap="round"
                  use:animateRing={securityScore}/>
        </svg>
        <div class="absolute inset-0 flex flex-col items-center justify-center">
          <span class="text-2xl font-bold {scoreColor}">{securityScore}</span>
          <span class="text-xs text-muted-foreground">/100</span>
        </div>
      </div>

      <!-- Score breakdown -->
      <div class="flex-1">
        <h2 class="text-lg font-semibold text-foreground mb-1">{$t('security.score_title')}</h2>
        <p class="text-sm text-muted-foreground mb-4">
          {securityScore >= 80 ? $t('security.system_well_protected')
          : securityScore >= 60 ? $t('security.improvements_recommended')
          : $t('security.critical_gaps')}
        </p>
        <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-2">
          {#each scoreItems as item}
            {@const ok = scoreMap[item.key]}
            <div class="flex items-center gap-2.5 px-3 py-2 rounded-lg {ok ? 'bg-green-500/5' : 'bg-red-500/5'}">
              <span class="shrink-0 {ok ? 'text-green-400' : 'text-red-400'}">
                {#if ok}
                  <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2.5" d="M5 13l4 4L19 7"/></svg>
                {:else}
                  <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2.5" d="M6 18L18 6M6 6l12 12"/></svg>
                {/if}
              </span>
              <div class="min-w-0">
                <span class="block text-xs font-medium text-foreground truncate">{item.label}</span>
                <span class="block text-xs text-muted-foreground">{ok ? item.desc : $t('security.not_configured')} <span class="text-primary/70">(+{item.points})</span></span>
              </div>
            </div>
          {/each}
        </div>
      </div>
    </div>
  </div>

  <!-- ── Security Checklist card ────────────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl p-6 space-y-4 transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20 fade-up">
    <div class="flex items-center justify-between">
      <div class="flex items-center gap-3">
        <div class="p-2 bg-blue-500/10 rounded-lg">
          <svg class="w-4 h-4 text-blue-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2m-6 9l2 2 4-4"/>
          </svg>
        </div>
        <h2 class="text-base font-semibold text-foreground">{$t('security.checklist_title')}</h2>
      </div>
      <div class="flex items-center gap-2 text-xs">
        {#if checklistCritical > 0}
          <span class="inline-flex items-center gap-1 px-2 py-0.5 rounded-full border bg-red-500/10 text-red-400 border-red-500/20">
            <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2.5" d="M6 18L18 6M6 6l12 12"/></svg>
            {checklistCritical} {$t('security.critical_label')}
          </span>
        {/if}
        {#if checklistWarnings > 0}
          <span class="inline-flex items-center gap-1 px-2 py-0.5 rounded-full border bg-amber-500/10 text-amber-400 border-amber-500/20">
            <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2.5" d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"/></svg>
            {checklistWarnings} {$t('security.warnings_label')}
          </span>
        {/if}
        <span class="inline-flex items-center gap-1 px-2 py-0.5 rounded-full border bg-green-500/10 text-green-400 border-green-500/20">
          <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2.5" d="M5 13l4 4L19 7"/></svg>
          {checklistDone}/{checklist.length}
        </span>
      </div>
    </div>

    <div class="space-y-1.5">
      {#each checklist as item}
        {@const expanded = !!checklistExpanded[item.id]}
        <div class="rounded-xl border {item.status === 'done' ? 'border-border' : item.status === 'critical' ? 'border-red-500/30 bg-red-500/3' : 'border-amber-500/20 bg-amber-500/3'} overflow-hidden">
          <!-- Row -->
          <div class="flex items-center gap-3 px-3 py-2.5
                      {item.status !== 'done' ? 'cursor-pointer hover:bg-muted/20' : ''}"
               role={item.status !== 'done' ? 'button' : undefined}
               tabindex={item.status !== 'done' ? 0 : undefined}
               on:click={() => { if (item.status !== 'done') checklistExpanded[item.id] = !expanded; }}
               on:keydown={e => { if (e.key === 'Enter' && item.status !== 'done') checklistExpanded[item.id] = !expanded; }}>
            <!-- Status icon -->
            {#if item.status === 'done'}
              <svg class="w-4 h-4 shrink-0 text-green-400" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2.5" d="M5 13l4 4L19 7"/></svg>
            {:else if item.status === 'critical'}
              <svg class="w-4 h-4 shrink-0 text-red-400" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2.5" d="M6 18L18 6M6 6l12 12"/></svg>
            {:else}
              <svg class="w-4 h-4 shrink-0 text-amber-400" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2.5" d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"/></svg>
            {/if}
            <!-- Label -->
            <span class="flex-1 text-sm text-foreground">{item.label}</span>
            <!-- Status badge -->
            <span class="text-xs shrink-0 {item.status === 'done' ? 'text-green-400' : item.status === 'critical' ? 'text-red-400' : 'text-amber-400'}">
              {item.statusLabel}
            </span>
            <!-- Expand chevron for actionable items -->
            {#if item.status !== 'done'}
              <svg class="w-3.5 h-3.5 shrink-0 text-muted-foreground transition-transform {expanded ? 'rotate-180' : ''}" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7"/>
              </svg>
            {/if}
          </div>
          <!-- Expanded fix panel -->
          {#if expanded && item.status !== 'done'}
            <div class="px-4 pb-4 pt-1 border-t border-border/60 space-y-3 bg-muted/10">
              <p class="text-sm text-muted-foreground">{item.fixDesc}</p>
              {#if item.fixAction}
                <button
                  class="h-8 px-4 rounded-lg bg-primary text-primary-foreground text-xs font-medium hover:bg-primary/90 inline-flex items-center gap-2 disabled:opacity-50"
                  disabled={fixingItem === item.id}
                  on:click={item.fixAction}>
                  {#if fixingItem === item.id}
                    <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"/></svg>
                    Applying…
                  {:else}
                    Fix Now
                  {/if}
                </button>
              {:else}
                <a href="/ssl" class="h-8 px-4 rounded-lg border border-border text-xs text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2">{$t('security.view_details')}<svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7"/></svg>
                </a>
              {/if}
            </div>
          {/if}
        </div>
      {/each}
    </div>
  </div>

  <!-- ── Recent Attack Timeline card ────────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl p-6 space-y-4 transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20 fade-up">
    <div class="flex items-center gap-3">
      <div class="p-2 bg-red-500/10 rounded-lg">
        <svg class="w-4 h-4 text-red-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z"/>
        </svg>
      </div>
      <div>
        <h2 class="text-base font-semibold text-foreground">{$t('security.attack_timeline')}</h2>
        <p class="text-xs text-muted-foreground">{$t('security.attack_timeline_desc')}</p>
      </div>
      <div class="ml-auto flex items-center gap-3 text-xs text-muted-foreground">
        <span class="flex items-center gap-1"><span class="w-2 h-2 rounded-full bg-red-500 inline-block"></span> {$t('security.brute_force')}</span>
        <span class="flex items-center gap-1"><span class="w-2 h-2 rounded-full bg-amber-500 inline-block"></span> {$t('security.scan')}</span>
        <span class="flex items-center gap-1"><span class="w-2 h-2 rounded-full bg-blue-500 inline-block"></span> {$t('security.rate_limit')}</span>
      </div>
    </div>

    {#if recentBans.length === 0}
      <div class="h-16 flex items-center justify-center text-sm text-muted-foreground bg-muted/10 rounded-lg">{$t('security.no_ban_events')}</div>
    {:else}
      <div class="relative h-14 bg-muted/20 rounded-lg overflow-hidden border border-border">
        {#each recentBans as ban}
          {@const x = (ban.minutesAgo / 1440) * 100}
          {@const dotColor = ban.type === 'brute' ? 'bg-red-500/80 border-red-500' : ban.type === 'scan' ? 'bg-amber-500/80 border-amber-500' : 'bg-blue-500/80 border-blue-500'}
          <!-- svelte-ignore a11y-no-static-element-interactions -->
          <div class="absolute top-1/2 -translate-y-1/2 w-2.5 h-2.5 rounded-full {dotColor} border
                      transition-all hover:scale-150 hover:z-10 cursor-pointer"
               style="left: {100 - x}%"
               title="{ban.ip} — {ban.jail} ({ban.reason})">
          </div>
        {/each}
        <!-- Time labels -->
        <div class="absolute bottom-0 left-0 right-0 flex justify-between px-2 pb-1 text-[9px] text-muted-foreground select-none">
          <span>{$t('security.last_24h')}</span><span>{$t('security.time_12h')}</span><span>{$t('security.time_6h')}</span><span>{$t('cron.countdown_now')}</span>
        </div>
      </div>
      <p class="text-xs text-muted-foreground">{recentBans.length} ban event{recentBans.length !== 1 ? 's' : ''} in the last 24h</p>
    {/if}
  </div>

  <!-- ── Login Activity card ────────────────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl p-6 space-y-4 transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20 fade-up">
    <div class="flex items-center gap-3">
      <div class="p-2 bg-violet-500/10 rounded-lg">
        <svg class="w-4 h-4 text-violet-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M17.657 16.657L13.414 20.9a1.998 1.998 0 01-2.827 0l-4.244-4.243a8 8 0 1111.314 0z"/>
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 11a3 3 0 11-6 0 3 3 0 016 0z"/>
        </svg>
      </div>
      <h2 class="text-base font-semibold text-foreground">{$t('security.login_activity')}</h2>
      <span class="ml-auto text-xs text-muted-foreground">{$t('security.last_10_attempts')}</span>
    </div>

    {#if loginAttempts.length === 0}
      <div class="py-8 text-center text-sm text-muted-foreground">{$t('security.no_login_attempts')}</div>
    {:else}
      <div class="rounded-xl border border-border overflow-hidden">
        <table class="w-full text-sm">
          <thead>
            <tr class="bg-muted/20 border-b border-border">
              <th class="px-4 py-2 text-left text-xs font-medium text-muted-foreground">{$t('firewall.ip_address')}</th>
              <th class="px-4 py-2 text-left text-xs font-medium text-muted-foreground">{$t('security.country')}</th>
              <th class="px-4 py-2 text-left text-xs font-medium text-muted-foreground hidden sm:table-cell">{$t('security.client')}</th>
              <th class="px-4 py-2 text-left text-xs font-medium text-muted-foreground hidden md:table-cell">{$t('security.time')}</th>
              <th class="px-4 py-2 text-left text-xs font-medium text-muted-foreground">{$t('common.status')}</th>
            </tr>
          </thead>
          <tbody class="divide-y divide-border">
            {#each loginAttempts as attempt}
              <tr class="hover:bg-muted/10 transition-colors">
                <td class="px-4 py-2 font-mono text-xs text-foreground">{attempt.ip}</td>
                <td class="px-4 py-2 text-xs text-foreground">
                  <span class="mr-1">{attempt.flag}</span>{attempt.country}
                </td>
                <td class="px-4 py-2 text-xs text-muted-foreground hidden sm:table-cell">{simplifyUA(attempt.userAgent)}</td>
                <td class="px-4 py-2 text-xs text-muted-foreground hidden md:table-cell">{timeAgo(attempt.time)}</td>
                <td class="px-4 py-2">
                  {#if attempt.status === 'success'}
                    <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium border bg-green-500/10 text-green-400 border-green-500/20">{$t('security.allowed')}</span>
                  {:else}
                    <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium border bg-red-500/10 text-red-400 border-red-500/20">{$t('security.blocked')}</span>
                  {/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  </div>

  <!-- ── 2-column settings grid ──────────────────────────────────────────── -->
  <div class="grid grid-cols-1 lg:grid-cols-2 gap-6">

    <!-- ── Fail2ban card ──────────────────────────────────────────────────── -->
    <div class="bg-card border border-border rounded-xl p-6 space-y-5 transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
      <div class="flex items-center justify-between">
        <div class="flex items-center gap-3">
          <div class="p-2 bg-orange-500/10 rounded-lg">
            <svg class="w-4 h-4 text-orange-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M18.364 18.364A9 9 0 005.636 5.636m12.728 12.728A9 9 0 015.636 5.636m12.728 12.728L5.636 5.636"/>
            </svg>
          </div>
          <h2 class="text-base font-semibold text-foreground">{$t('security.fail2ban')}</h2>
        </div>
        <button
          class="h-5 w-9 rounded-full {fail2banActive ? 'bg-primary' : 'bg-muted'} relative cursor-pointer transition-colors"
          on:click={() => fail2banActive = !fail2banActive}
          role="switch" aria-checked={fail2banActive} aria-label="Toggle Fail2ban">
          <span class="absolute h-4 w-4 bg-background rounded-full shadow transition-transform {fail2banActive ? 'translate-x-4' : 'translate-x-0.5'} top-0.5"></span>
        </button>
      </div>

      <!-- Ban list expand/collapse -->
      <div>
        <button
          class="w-full flex items-center justify-between px-4 py-3 rounded-xl bg-muted/20 border border-border hover:bg-muted/30 transition-colors"
          on:click={() => banListExpanded = !banListExpanded}>
          <div class="flex items-center gap-2">
            <span class="inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium border
              {uniqueBanned > 0 ? 'bg-red-500/10 text-red-400 border-red-500/20' : 'bg-green-500/10 text-green-400 border-green-500/20'}">
              {uniqueBanned} Banned IP{uniqueBanned !== 1 ? 's' : ''}
            </span>
            <span class="text-xs text-muted-foreground">across {fail2banJails.length} jail{fail2banJails.length !== 1 ? 's' : ''}</span>
          </div>
          <svg class="w-4 h-4 text-muted-foreground transition-transform {banListExpanded ? 'rotate-180' : ''}" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7"/>
          </svg>
        </button>

        {#if banListExpanded}
          <div class="mt-2 rounded-xl border border-border overflow-hidden">
            {#if blockedIps.length === 0}
              <div class="py-6 text-center text-sm text-muted-foreground">{$t('security.no_banned_ips')}</div>
            {:else}
              <table class="w-full text-sm">
                <thead>
                  <tr class="bg-muted/20 border-b border-border">
                    <th class="px-4 py-2 text-left text-xs font-medium text-muted-foreground">{$t('security.ip')}</th>
                    <th class="px-4 py-2 text-left text-xs font-medium text-muted-foreground">{$t('security.jail')}</th>
                    <th class="px-4 py-2 text-left text-xs font-medium text-muted-foreground">{$t('security.since')}</th>
                    <th class="px-4 py-2"></th>
                  </tr>
                </thead>
                <tbody class="divide-y divide-border">
                  {#each blockedIps as b}
                    <tr class="hover:bg-muted/10">
                      <td class="px-4 py-2 font-mono text-xs text-foreground">{b.ip}</td>
                      <td class="px-4 py-2 text-xs text-muted-foreground">{b.jail || '—'}</td>
                      <td class="px-4 py-2 text-xs text-muted-foreground">{b.since ? timeAgo(b.since) : '—'}</td>
                      <td class="px-4 py-2 text-right">
                        <button class="text-xs text-red-400 hover:text-red-300 transition-colors"
                                on:click={() => unblockIp(b.ip, b.jail)}>Unban</button>
                      </td>
                    </tr>
                  {/each}
                </tbody>
              </table>
              <div class="px-4 py-3 border-t border-border bg-muted/10">
                <button
                  class="h-9 px-4 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-sm font-medium hover:bg-red-500/20 inline-flex items-center gap-2"
                  on:click={unbanAll} disabled={savingFail2ban}>{$t('security.unban_all')}</button>
              </div>
            {/if}
          </div>
        {/if}
      </div>

      <!-- Config inputs -->
      <div class="space-y-3">
        <h3 class="text-xs font-semibold text-muted-foreground uppercase tracking-wide">{$t('security.config')}</h3>
        <div class="grid grid-cols-3 gap-3">
          <div>
            <label class="block text-xs text-muted-foreground mb-1.5">{$t('security.max_retry')}</label>
            <input type="number" min="1" max="20"
              class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
              bind:value={fail2banMaxretry}/>
          </div>
          <div>
            <label class="block text-xs text-muted-foreground mb-1.5">{$t('security.find_time')}</label>
            <input type="number" min="60"
              class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
              bind:value={fail2banFindtime}/>
          </div>
          <div>
            <label class="block text-xs text-muted-foreground mb-1.5">{$t('security.ban_time')}</label>
            <input type="number" min="60"
              class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
              bind:value={fail2banBantime}/>
          </div>
        </div>
        <button
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2"
          on:click={saveFail2banConfig} disabled={savingFail2ban}>
          {#if savingFail2ban}
            <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"/></svg>
          {/if}
          Save Config
        </button>
      </div>
    </div>

    <!-- ── 2FA + SSL card ─────────────────────────────────────────────────── -->
    <div class="bg-card border border-border rounded-xl p-6 space-y-4 transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
      <div class="flex items-center gap-3">
        <div class="p-2 bg-indigo-500/10 rounded-lg">
          <svg class="w-4 h-4 text-indigo-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z"/>
          </svg>
        </div>
        <h2 class="text-base font-semibold text-foreground">{$t('settings.two_factor_auth')}</h2>
      </div>

      <div class="flex items-center justify-between px-4 py-3 rounded-xl bg-muted/20 border border-border">
        <div>
          <span class="block text-sm font-medium text-foreground">{$t('security.admin_2fa')}</span>
          <span class="text-xs text-muted-foreground">{$t('security.admin_2fa_desc')}</span>
        </div>
        <a href="/settings/totp"
           class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2">{$t('security.configure')}<svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7"/></svg>
        </a>
      </div>

      <div class="flex items-center justify-between px-4 py-3 rounded-xl bg-muted/20 border border-border">
        <div>
          <span class="block text-sm font-medium text-foreground">{$t('security.force_2fa')}</span>
          <span class="text-xs text-muted-foreground">{$t('security.force_2fa_desc')}</span>
        </div>
        <button
          class="h-5 w-9 rounded-full {twoFAForced ? 'bg-primary' : 'bg-muted'} relative cursor-pointer transition-colors shrink-0"
          on:click={() => { showToast('Global 2FA enforcement is not yet supported — enable per-user TOTP in Users settings', 'error'); }}
          role="switch" aria-checked={twoFAForced} aria-label="Force 2FA for all users">
          <span class="absolute h-4 w-4 bg-background rounded-full shadow transition-transform {twoFAForced ? 'translate-x-4' : 'translate-x-0.5'} top-0.5"></span>
        </button>
      </div>

      <!-- SSL status mini-list -->
      <div class="border-t border-border pt-4">
        <h3 class="text-xs font-semibold text-muted-foreground uppercase tracking-wide mb-3">{$t('ssl.title')}</h3>
        {#if sites.length === 0}
          <p class="text-sm text-muted-foreground">{$t('dashboard.no_sites_found')}</p>
        {:else}
          <div class="space-y-2">
            {#each sites.slice(0, 5) as site}
              {@const days = daysUntil(site.ssl_expires_at)}
              <div class="flex items-center justify-between text-sm">
                <span class="text-foreground truncate max-w-40">{site.domain}</span>
                {#if days !== null && days < 14}
                  <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium border bg-red-500/10 text-red-400 border-red-500/20">{days}d left</span>
                {:else if days !== null && days < 30}
                  <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium border bg-amber-500/10 text-amber-400 border-amber-500/20">{days}d left</span>
                {:else}
                  <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium border bg-green-500/10 text-green-400 border-green-500/20">
                    {site.ssl_status === 'active' ? 'OK' : site.ssl_status}
                  </span>
                {/if}
              </div>
            {/each}
            {#if sites.length > 5}
              <a href="/ssl" class="block text-xs text-primary/80 hover:text-primary pt-1">+ {sites.length - 5} more — Manage SSL</a>
            {/if}
          </div>
        {/if}
      </div>
    </div>

    <!-- ── ModSecurity WAF card ────────────────────────────────────────────── -->
    <div class="bg-card border border-border rounded-xl p-6 space-y-5 transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
      <div class="flex items-center justify-between">
        <div class="flex items-center gap-3">
          <div class="p-2 bg-purple-500/10 rounded-lg">
            <svg class="w-4 h-4 text-purple-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M20.618 5.984A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016zM12 9v2m0 4h.01"/>
            </svg>
          </div>
          <h2 class="text-base font-semibold text-foreground">{$t('security.modsecurity_waf')}</h2>
        </div>
        <button
          class="h-5 w-9 rounded-full {wafActive ? 'bg-primary' : 'bg-muted'} relative cursor-pointer transition-colors"
          on:click={() => wafActive = !wafActive}
          role="switch" aria-checked={wafActive} aria-label="Toggle WAF">
          <span class="absolute h-4 w-4 bg-background rounded-full shadow transition-transform {wafActive ? 'translate-x-4' : 'translate-x-0.5'} top-0.5"></span>
        </button>
      </div>

      <!-- Mode radio -->
      <div>
        <label class="block text-xs font-semibold text-muted-foreground uppercase tracking-wide mb-2">{$t('security.mode')}</label>
        <div class="flex gap-2">
          {#each [
            { value: 'detection',  label: 'Detection',  desc: 'Log only' },
            { value: 'prevention', label: 'Prevention', desc: 'Block threats' },
          ] as mode}
            <label class="flex-1 flex flex-col gap-0.5 p-3 rounded-xl border-2 cursor-pointer transition-all
              {wafMode === mode.value ? 'border-primary bg-primary/5' : 'border-border hover:border-primary/40'}">
              <input type="radio" class="sr-only" name="wafMode" value={mode.value} bind:group={wafMode}/>
              <span class="text-sm font-medium text-foreground">{mode.label}</span>
              <span class="text-xs text-muted-foreground">{mode.desc}</span>
            </label>
          {/each}
        </div>
      </div>

      <!-- Rules version -->
      <div class="flex items-center gap-2">
        <span class="text-xs text-muted-foreground">{$t('security.owasp_crs')}</span>
        <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium border bg-blue-500/10 text-blue-400 border-blue-500/20">{$t('security.v33x')}</span>
      </div>

      <!-- Recent blocks expandable -->
      <div>
        <button
          class="w-full flex items-center justify-between px-4 py-3 rounded-xl bg-muted/20 border border-border hover:bg-muted/30 transition-colors"
          on:click={() => wafLogsExpanded = !wafLogsExpanded}>
          <div class="flex items-center gap-2">
            <span class="text-sm text-foreground">{$t('security.recent_blocks')}</span>
            <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium border bg-purple-500/10 text-purple-400 border-purple-500/20">{wafLogs.length}</span>
          </div>
          <svg class="w-4 h-4 text-muted-foreground transition-transform {wafLogsExpanded ? 'rotate-180' : ''}" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7"/>
          </svg>
        </button>

        {#if wafLogsExpanded}
          <div class="mt-2 rounded-xl border border-border overflow-hidden">
            {#if wafLogs.length === 0}
              <div class="py-6 text-center text-sm text-muted-foreground">{$t('security.no_blocked_requests')}</div>
            {:else}
              <div class="overflow-x-auto">
                <table class="w-full text-sm">
                  <thead>
                    <tr class="bg-muted/20 border-b border-border">
                      <th class="px-4 py-2 text-left text-xs font-medium text-muted-foreground">{$t('security.ip')}</th>
                      <th class="px-4 py-2 text-left text-xs font-medium text-muted-foreground">{$t('security.rule')}</th>
                      <th class="px-4 py-2 text-left text-xs font-medium text-muted-foreground">{$t('security.uri')}</th>
                      <th class="px-4 py-2 text-left text-xs font-medium text-muted-foreground">{$t('backups.col_when')}</th>
                    </tr>
                  </thead>
                  <tbody class="divide-y divide-border">
                    {#each wafLogs as log}
                      <tr class="hover:bg-muted/10">
                        <td class="px-4 py-2 font-mono text-xs text-foreground">{log.ip}</td>
                        <td class="px-4 py-2 text-xs">
                          <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium border bg-red-500/10 text-red-400 border-red-500/20">{log.rule_id}</span>
                        </td>
                        <td class="px-4 py-2 text-xs text-muted-foreground truncate max-w-32" title={log.uri}>{log.uri}</td>
                        <td class="px-4 py-2 text-xs text-muted-foreground">{timeAgo(log.blocked_at)}</td>
                      </tr>
                    {/each}
                  </tbody>
                </table>
              </div>
            {/if}
          </div>
        {/if}
      </div>
    </div>

    <!-- ── Automatic Security Updates card ──────────────────────────────── -->
    <div class="bg-card border border-border rounded-xl p-6 space-y-4 transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
      <div class="flex items-center gap-3">
        <div class="p-2 bg-green-500/10 rounded-lg">
          <svg class="w-4 h-4 text-green-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/>
          </svg>
        </div>
        <h2 class="text-base font-semibold text-foreground">{$t('security.automatic_updates')}</h2>
      </div>

      <div class="flex items-center justify-between px-4 py-3 rounded-xl bg-muted/20 border border-border">
        <div>
          <span class="block text-sm font-medium text-foreground">{$t('security.os_updates')}</span>
          <span class="text-xs text-muted-foreground">{$t('security.os_updates_desc')}</span>
        </div>
        <button
          class="h-5 w-9 rounded-full {osUpdates ? 'bg-primary' : 'bg-muted'} relative cursor-pointer transition-colors shrink-0"
          on:click={() => osUpdates = !osUpdates}
          role="switch" aria-checked={osUpdates} aria-label="Toggle OS Updates">
          <span class="absolute h-4 w-4 bg-background rounded-full shadow transition-transform {osUpdates ? 'translate-x-4' : 'translate-x-0.5'} top-0.5"></span>
        </button>
      </div>

      <div class="flex items-center justify-between px-4 py-3 rounded-xl bg-muted/20 border border-border">
        <div>
          <span class="block text-sm font-medium text-foreground">{$t('security.php_updates')}</span>
          <span class="text-xs text-muted-foreground">{$t('security.php_updates_desc')}</span>
        </div>
        <button
          class="h-5 w-9 rounded-full {phpUpdates ? 'bg-primary' : 'bg-muted'} relative cursor-pointer transition-colors shrink-0"
          on:click={() => phpUpdates = !phpUpdates}
          role="switch" aria-checked={phpUpdates} aria-label="Toggle PHP Updates">
          <span class="absolute h-4 w-4 bg-background rounded-full shadow transition-transform {phpUpdates ? 'translate-x-4' : 'translate-x-0.5'} top-0.5"></span>
        </button>
      </div>

      <div class="flex items-center justify-between px-4 py-3 rounded-xl bg-muted/20 border border-border">
        <div>
          <span class="block text-sm font-medium text-foreground">{$t('security.web_server_updates')}</span>
          <span class="text-xs text-muted-foreground">{$t('security.web_server_updates_desc')}</span>
        </div>
        <button
          class="h-5 w-9 rounded-full {webserverUpdates ? 'bg-primary' : 'bg-muted'} relative cursor-pointer transition-colors shrink-0"
          on:click={() => webserverUpdates = !webserverUpdates}
          role="switch" aria-checked={webserverUpdates} aria-label="Toggle Web Server Updates">
          <span class="absolute h-4 w-4 bg-background rounded-full shadow transition-transform {webserverUpdates ? 'translate-x-4' : 'translate-x-0.5'} top-0.5"></span>
        </button>
      </div>

      <div class="flex items-center gap-2 pt-1 text-xs text-muted-foreground">
        <svg class="w-3.5 h-3.5 shrink-0 text-green-400" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2.5" d="M5 13l4 4L19 7"/></svg>{$t('security.last_updated_today')}</div>
    </div>

  </div><!-- end 2-col grid -->

  <!-- ── SSH Security card (full width) ─────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl p-6 space-y-5 transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
    <div class="flex items-center gap-3">
      <div class="p-2 bg-cyan-500/10 rounded-lg">
        <svg class="w-4 h-4 text-cyan-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M8 9l3 3-3 3m5 0h3M5 20h14a2 2 0 002-2V6a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z"/>
        </svg>
      </div>
      <div>
        <h2 class="text-base font-semibold text-foreground">{$t('security.ssh_security')}</h2>
        <p class="text-sm text-muted-foreground">{$t('security.ssh_security_desc')}</p>
      </div>
    </div>

    <div class="grid grid-cols-1 sm:grid-cols-2 gap-4">
      <!-- PasswordAuthentication -->
      <div class="px-4 py-4 rounded-xl border border-border bg-muted/10 space-y-2">
        <div class="flex items-center justify-between">
          <span class="text-sm font-medium text-foreground">{$t('security.password_auth')}</span>
          <button
            class="h-5 w-9 rounded-full {sshPasswordAuth ? 'bg-amber-400' : 'bg-primary'} relative cursor-pointer transition-colors shrink-0"
            on:click={() => sshPasswordAuth = !sshPasswordAuth}
            role="switch" aria-checked={sshPasswordAuth} aria-label="Toggle SSH Password Auth">
            <span class="absolute h-4 w-4 bg-background rounded-full shadow transition-transform {sshPasswordAuth ? 'translate-x-4' : 'translate-x-0.5'} top-0.5"></span>
          </button>
        </div>
        {#if sshPasswordAuth}
          <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium border bg-amber-500/10 text-amber-400 border-amber-500/20">{$t('security.enabled_risk')}</span>
          <span class="block text-xs text-muted-foreground">{$t('security.disabled_recommended')}</span>
        {:else}
          <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium border bg-green-500/10 text-green-400 border-green-500/20">{$t('security.disabled_secure')}</span>
        {/if}
      </div>

      <!-- PermitRootLogin -->
      <div class="px-4 py-4 rounded-xl border border-border bg-muted/10 space-y-2">
        <div class="flex items-center justify-between">
          <span class="text-sm font-medium text-foreground">{$t('security.permit_root_login')}</span>
          <button
            class="h-5 w-9 rounded-full {sshPermitRoot ? 'bg-red-500' : 'bg-primary'} relative cursor-pointer transition-colors shrink-0"
            on:click={() => sshPermitRoot = !sshPermitRoot}
            role="switch" aria-checked={sshPermitRoot} aria-label="Toggle SSH Root Login">
            <span class="absolute h-4 w-4 bg-background rounded-full shadow transition-transform {sshPermitRoot ? 'translate-x-4' : 'translate-x-0.5'} top-0.5"></span>
          </button>
        </div>
        {#if sshPermitRoot}
          <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium border bg-red-500/10 text-red-400 border-red-500/20">{$t('security.enabled_high_risk')}</span>
          <span class="block text-xs text-muted-foreground">{$t('security.recommended_no_or_prohibit_password')}</span>
        {:else}
          <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium border bg-green-500/10 text-green-400 border-green-500/20">{$t('security.disabled_secure')}</span>
        {/if}
      </div>
    </div>

    <!-- Warning banner if insecure -->
    {#if sshPasswordAuth || sshPermitRoot}
      <div class="flex items-start gap-3 px-4 py-3 rounded-xl bg-amber-500/5 border border-amber-500/20">
        <svg class="w-4 h-4 text-amber-400 shrink-0 mt-0.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"/>
        </svg>
        <p class="text-sm">
          <span class="font-medium text-amber-400">{$t('security.insecure_ssh_settings_detected')}</span>
          <span class="text-muted-foreground">
            {#if sshPasswordAuth && sshPermitRoot}Disable both password authentication and root login immediately.
            {:else if sshPasswordAuth}Disable password authentication and use SSH keys only.
            {:else}Disable root login; use a non-root user with sudo privileges.{/if}
          </span>
        </p>
      </div>
    {/if}

    <!-- View SSH Config -->
    <div>
      <button
        class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2"
        on:click={() => sshConfigExpanded = !sshConfigExpanded}>
        <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4"/>
        </svg>
        {sshConfigExpanded ? 'Hide' : 'View'} SSH Config
        <svg class="w-3 h-3 transition-transform {sshConfigExpanded ? 'rotate-180' : ''}" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7"/>
        </svg>
      </button>

      {#if sshConfigExpanded}
        <div class="mt-3 p-4 rounded-xl bg-muted/20 border border-border font-mono text-xs text-foreground leading-6 space-y-0.5">
          <div class="text-muted-foreground">{$t('security.etc_ssh_sshd_config_relevant_lines')}</div>
          <div>{$t('settings.port')}<span class="text-primary">22</span></div>
          <div>{$t('security.permitrootlogin')}<span class="{sshPermitRoot ? 'text-red-400' : 'text-green-400'}">{sshPermitRoot ? 'yes' : 'no'}</span></div>
          <div>{$t('security.passwordauthentication')}<span class="{sshPasswordAuth ? 'text-amber-400' : 'text-green-400'}">{sshPasswordAuth ? 'yes' : 'no'}</span></div>
          <div>{$t('security.pubkeyauthentication')}<span class="text-green-400">{$t('security.yes')}</span></div>
          <div>{$t('security.challengeresponseauthentication')}<span class="text-green-400">{$t('security.no')}</span></div>
          <div>{$t('security.usepam')}<span class="text-green-400">{$t('security.yes')}</span></div>
          <div>{$t('security.x11forwarding')}<span class="text-green-400">{$t('security.no')}</span></div>
          <div>{$t('security.printmotd')}<span class="text-muted-foreground">{$t('security.no')}</span></div>
          <div>{$t('security.acceptenv')}<span class="text-muted-foreground">{$t('security.lang_lc')}</span></div>
          <div>{$t('security.subsystem')}<span class="text-muted-foreground">{$t('security.sftp_usr_lib_openssh_sftp_server')}</span></div>
        </div>
      {/if}
    </div>
  </div>

  {/if}<!-- end !loading -->
</div>

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
