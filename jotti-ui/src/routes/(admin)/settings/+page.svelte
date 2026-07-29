<script lang="ts">
  import { onMount } from 'svelte';
  import { get } from 'svelte/store';
  import { api } from '$api/client';
  import type { ApiKey } from '$api/client';
  import { t, setLanguage } from '$lib/i18n';

  // ── State ──────────────────────────────────────────────────────────────────
  type TabKey = 'general' | 'branding' | 'smtp' | 'smtp-limits' | 'security' | 'api-keys' | 'advanced';
  let activeTab = $state<TabKey>('general');
  let loading = $state(true);

  // ── General ────────────────────────────────────────────────────────────────
  let settings = $state({
    panel_domain: '',
    panel_email: '',
    company_name: '',
    timezone: 'UTC',
    default_php: '8.3',
    default_webserver: 'nginx' as 'nginx' | 'apache' | 'ols',
    language: 'en',
    date_format: 'MM/DD/YYYY',
  });
  let generalLoading = $state(false);

  // ── Branding ───────────────────────────────────────────────────────────────
  let branding = $state({
    panel_name: 'JottiCP',
    primary_color: '#6366f1',
  });
  let logoPreview = $state('');
  let faviconPreview = $state('');
  let brandingLoading = $state(false);

  // ── SMTP ───────────────────────────────────────────────────────────────────
  let smtpConfig = $state({
    host: '',
    port: 587,
    user: '',
    password: '',
    from: '',
    encryption: 'starttls' as 'none' | 'tls' | 'starttls',
  });
  let showSmtpPassword = $state(false);
  let smtpTestEmail = $state('');
  let smtpTestResult = $state<{ success: boolean; message: string } | null>(null);
  let smtpTestLoading = $state(false);
  let smtpSaveLoading = false;

  // ── Security ───────────────────────────────────────────────────────────────
  let security = {
    session_timeout: '8hr',
    max_login_attempts: 5,
    totp_policy: 'optional' as 'optional' | 'admin_required' | 'all_required',
    ip_whitelist: '',
  };
  let securityLoading = false;

  // ── API Keys ───────────────────────────────────────────────────────────────
  let apiKeys: ApiKey[] = [];
  let apiKeysLoading = false;
  let apiKeysLoaded = false;
  let showNewKeyModal = false;
  let newKeyForm = {
    name: '',
    scopes: [] as string[],
    expires_at: '',
  };
  let newKeyLoading = false;
  let newKeyError = '';
  let createdKey: string | null = null;
  let createdKeyCopied = false;

  // ── SMTP Limits ────────────────────────────────────────────────────────────
  let smtpLimits = {
    max_recipients: 50,
    max_message_size_mb: 25,
    connection_timeout: 60,
    max_retry_attempts: 3,
    retry_interval_min: 30,
    queue_lifetime_hours: 72,
    allow_open_relay: false,
    require_smtp_auth: true,
    allow_auth_plaintext: false,
    tls_requirement: 'required' as 'optional' | 'required' | 'strict',
    rate_limit_hourly_per_ip: 100,
    rate_limit_daily_per_domain: 500,
    fail2ban_enabled: true,
    abuse_threshold: 5,
    whitelist: '',
    blacklist: '',
  };
  let smtpLimitsSaveLoading = false;
  let smtpLimitsTestLoading = false;
  let smtpLimitsTestResult: { success: boolean; message: string } | null = null;
  let smtpQueueCount: number | null = null;
  let smtpQueueLoading = false;
  let smtpFlushLoading = false;
  let smtpQueueRows: Array<{ id: string; from: string; to: string; queued_at: string; retries: number }> = [];

  // ── Login Audit ────────────────────────────────────────────────────────────
  interface LoginAuditEntry {
    ip: string;
    country: string;
    ua: string;
    time: string;
    success: boolean;
  }
  let loginAudit: LoginAuditEntry[] = [];
  let loginAuditLoading = false;
  let loginAuditLoaded = false;

  // ── Maintenance Mode ───────────────────────────────────────────────────────
  let maintenance = {
    enabled: false,
    message: 'This panel is undergoing scheduled maintenance. Please check back shortly.',
    whitelist_ips: '',
  };
  let maintenanceLoading = false;
  let showMaintenancePreview = false;

  // ── License ────────────────────────────────────────────────────────────────
  let license = {
    tier: 'community',
    domain: '',
    expires_at: null as string | null,
    features: [] as string[],
  };
  let licenseKey = '';
  let licenseActivating = false;

  // ── SSH Keys ───────────────────────────────────────────────────────────────
  interface SshKey {
    id: string;
    label: string;
    fingerprint: string;
    key_type: string;
    added_at: string;
    last_used_at: string | null;
  }
  let sshKeys: SshKey[] = [];
  let sshKeysLoading = false;
  let sshKeysLoaded = false;
  let newSshPubkey = '';
  let newSshLabel = '';
  let sshAddLoading = false;
  let sshAddError = '';
  let showDisablePasswordConfirm = false;
  let disablePwdLoading = false;

  // Inline confirm state
  let confirmFlushQueue = false;
  let confirmRevokeKeyId: string | null = null;
  let confirmDeleteSshKeyId: string | null = null;

  // Toast
  let toastMessage = '';
  let toastType: 'success' | 'error' = 'success';
  let toastTimer: ReturnType<typeof setTimeout> | null = null;

  function authHeaders(): Record<string, string> {
    const token = typeof localStorage !== 'undefined' ? localStorage.getItem('orbit_access_token') : null;
    return token ? { Authorization: `Bearer ${token}` } : {};
  }

  onMount(async () => {
    loading = true;
    try {
      const [genRes, brandRes, lic] = await Promise.all([
        fetch('/api/v1/settings', { headers: authHeaders() }).then(r => r.ok ? r.json() as Promise<Record<string, unknown>> : Promise.resolve({})).catch(() => ({})),
        fetch('/api/v1/branding', { headers: authHeaders() }).then(r => r.ok ? r.json() as Promise<Record<string, unknown>> : Promise.resolve({})).catch(() => ({})),
        api.license.get().catch(() => null),
      ]);
      const data = genRes as Record<string, unknown>;
      if (data.panel_domain) settings.panel_domain = String(data.panel_domain);
      if (data.admin_email) settings.panel_email = String(data.admin_email);
      if (data.default_web_server)
        settings.default_webserver = String(data.default_web_server) as typeof settings.default_webserver;
      if (data.language) {
        settings.language = String(data.language);
        setLanguage(String(data.language) as any);
      }
      if (data.timezone) settings.timezone = String(data.timezone);
      if (data.date_format) settings.date_format = String(data.date_format);
      if (data.default_php) settings.default_php = String(data.default_php);
      const bd = brandRes as Record<string, unknown>;
      if (bd.panel_name) branding.panel_name = String(bd.panel_name);
      if (bd.primary_color) branding.primary_color = String(bd.primary_color);
      if (lic) {
        license.tier = lic.tier;
        license.domain = lic.domain ?? '';
        license.expires_at = lic.expires_at;
        license.features = lic.features;
      }
    } catch {
      /* use defaults */
    }
    loading = false;
  });

  // ── Tab switching ───────────────────────────────────────────────────────────

  async function switchTab(tab: TabKey) {
    activeTab = tab;
    if (tab === 'api-keys' && !apiKeysLoaded) await loadApiKeys();
    if (tab === 'advanced' && !sshKeysLoaded) await loadSshKeys();
    if (tab === 'security' && !loginAuditLoaded) await loadLoginAudit();
    if (tab === 'smtp-limits') await loadSmtpLimits();
  }

  // ── SMTP Limits ─────────────────────────────────────────────────────────────

  async function loadSmtpLimits() {
    try {
      const r = await fetch('/api/v1/settings/email-limits', { headers: authHeaders() });
      if (r.ok) {
        const data = await r.json() as { per_hour?: number; per_day?: number; enabled?: boolean };
        if (data.per_hour != null) smtpLimits.rate_limit_hourly_per_ip = data.per_hour;
        if (data.per_day != null) smtpLimits.rate_limit_daily_per_domain = data.per_day;
      }
    } catch { /* use defaults */ }
  }

  async function saveSmtpLimits(e: SubmitEvent) {
    e.preventDefault();
    smtpLimitsSaveLoading = true;
    try {
      const token = typeof localStorage !== 'undefined' ? localStorage.getItem('orbit_access_token') : null;
      const r = await fetch('/api/v1/settings/email-limits', {
        method: 'PUT',
        headers: {
          'Content-Type': 'application/json',
          ...(token ? { Authorization: `Bearer ${token}` } : {}),
        },
        body: JSON.stringify({
          per_hour: smtpLimits.rate_limit_hourly_per_ip,
          per_day: smtpLimits.rate_limit_daily_per_domain,
          per_month: smtpLimits.rate_limit_daily_per_domain * 30,
          enabled: smtpLimits.fail2ban_enabled,
        }),
      });
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      showToast(get(t)('settings.smtp_limits_saved'), 'success');
    } catch {
      showToast(get(t)('settings.smtp_limits_save_failed'), 'error');
    } finally {
      smtpLimitsSaveLoading = false;
    }
  }

  async function testOutboundSmtp() {
    smtpLimitsTestLoading = true;
    smtpLimitsTestResult = null;
    try {
      const r = await fetch('/api/v1/settings/smtp/test', {
        method: 'POST',
        headers: authHeaders(),
      });
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      const data = await r.json() as { success: boolean; message: string };
      smtpLimitsTestResult = data;
    } catch {
      smtpLimitsTestResult = { success: false, message: get(t)('settings.test_failed') };
    } finally {
      smtpLimitsTestLoading = false;
    }
  }

  async function viewSmtpQueue() {
    smtpQueueLoading = true;
    try {
      const r = await fetch('/api/v1/settings/smtp/queue', { headers: authHeaders() });
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      const data = await r.json() as { count: number; items: typeof smtpQueueRows };
      smtpQueueCount = data.count;
      smtpQueueRows = data.items ?? [];
    } catch {
      smtpQueueCount = 0;
    } finally {
      smtpQueueLoading = false;
    }
  }

  async function flushSmtpQueue() {
    confirmFlushQueue = false;
    smtpFlushLoading = true;
    try {
      const r = await fetch('/api/v1/settings/smtp/queue', {
        method: 'DELETE',
        headers: authHeaders(),
      });
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      smtpQueueCount = 0;
      smtpQueueRows = [];
      showToast(get(t)('settings.queue_flushed'), 'success');
    } catch {
      showToast(get(t)('settings.queue_flush_failed'), 'error');
    } finally {
      smtpFlushLoading = false;
    }
  }

  // ── Login Audit ─────────────────────────────────────────────────────────────

  async function loadLoginAudit() {
    loginAuditLoading = true;
    try {
      const r = await fetch('/api/v1/audit-log?limit=50', { headers: authHeaders() });
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      const raw = await r.json() as Array<{
        action: string;
        ip_address?: string;
        created_at: string;
        user_email?: string;
      }>;
      loginAudit = (Array.isArray(raw) ? raw : [])
        .filter(e => e.action?.includes('/auth/login'))
        .slice(0, 10)
        .map(e => ({
          ip:      e.ip_address ?? 'Unknown',
          country: '–',
          ua:      '–',
          time:    e.created_at,
          success: true,
        }));
      loginAuditLoaded = true;
    } catch {
      loginAuditLoaded = true;
    } finally {
      loginAuditLoading = false;
    }
  }

  function simplifyUA(ua: string): string {
    if (!ua) return 'Unknown';
    if (ua.includes('Chrome')) return 'Chrome';
    if (ua.includes('Firefox')) return 'Firefox';
    if (ua.includes('Safari')) return 'Safari';
    if (ua.includes('Edge')) return 'Edge';
    return ua.slice(0, 24);
  }

  // ── Maintenance Mode ─────────────────────────────────────────────────────────

  async function saveMaintenance() {
    maintenanceLoading = true;
    try {
      const r = await fetch('/api/v1/settings', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json', ...authHeaders() },
        body: JSON.stringify({ maintenance_mode: maintenance.enabled }),
      });
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      showToast(get(t)('settings.maintenance_saved'), 'success');
    } catch {
      showToast(get(t)('settings.maintenance_save_failed'), 'error');
    } finally {
      maintenanceLoading = false;
    }
  }

  // ── Save General ───────────────────────────────────────────────────────────

  async function saveGeneral(e: SubmitEvent) {
    e.preventDefault();
    generalLoading = true;
    try {
      const r = await fetch('/api/v1/settings', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json', ...authHeaders() },
        body: JSON.stringify({
          panel_domain: settings.panel_domain,
          admin_email: settings.panel_email,
          default_web_server: settings.default_webserver,
          language: settings.language,
          timezone: settings.timezone,
          date_format: settings.date_format,
          default_php: settings.default_php,
        }),
      });
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      showToast(get(t)('settings.saved_success'), 'success');
    } catch {
      showToast(get(t)('settings.save_failed'), 'error');
    } finally {
      generalLoading = false;
    }
  }

  // ── SMTP ───────────────────────────────────────────────────────────────────

  async function saveSmtp(e: SubmitEvent) {
    e.preventDefault();
    smtpSaveLoading = true;
    try {
      const r = await fetch('/api/v1/settings', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json', ...authHeaders() },
        body: JSON.stringify({ smtp_from: smtpConfig.from }),
      });
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      showToast(get(t)('settings.smtp_from_saved'), 'success');
    } catch {
      showToast(get(t)('settings.smtp_save_failed'), 'error');
    } finally {
      smtpSaveLoading = false;
    }
  }

  async function testSmtp() {
    if (!smtpTestEmail) return;
    smtpTestLoading = true;
    smtpTestResult = null;
    try {
      const r = await fetch('/api/v1/settings/smtp/test', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', ...authHeaders() },
        body: JSON.stringify({ ...smtpConfig, to: smtpTestEmail }),
      });
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      smtpTestResult = await r.json() as { success: boolean; message: string };
    } catch (err: unknown) {
      const e = err as { message?: string };
        smtpTestResult = {
          success: false,
          message: e.message ?? get(t)('settings.test_failed'),
        };
    } finally {
      smtpTestLoading = false;
    }
  }

  // ── Security ───────────────────────────────────────────────────────────────

  async function saveSecurity(e: SubmitEvent) {
    e.preventDefault();
    showToast(get(t)('settings.security_policy_not_configurable'), 'error');
  }

  // ── API Keys ───────────────────────────────────────────────────────────────

  async function loadApiKeys() {
    apiKeysLoading = true;
    try {
      apiKeys = await api.apiKeys.list();
      apiKeysLoaded = true;
    } catch {
      showToast(get(t)('settings.api_keys_load_failed'), 'error');
    } finally {
      apiKeysLoading = false;
    }
  }

  async function createApiKey(e: SubmitEvent) {
    e.preventDefault();
    newKeyLoading = true;
    newKeyError = '';
    try {
      const result = await api.apiKeys.create({
        name: newKeyForm.name,
        scopes: newKeyForm.scopes.length ? newKeyForm.scopes : ['read'],
        expires_at: newKeyForm.expires_at || undefined,
      });
      createdKey = result.full_key;
      apiKeys = [...apiKeys, result];
    } catch (err: unknown) {
      const e = err as { message?: string };
      newKeyError = e.message ?? get(t)('settings.api_key_create_failed');
    } finally {
      newKeyLoading = false;
    }
  }

  async function revokeApiKey(key: ApiKey) {
    confirmRevokeKeyId = null;
    try {
      await api.apiKeys.revoke(key.id);
      apiKeys = apiKeys.filter((k) => k.id !== key.id);
      showToast(get(t)('settings.api_key_revoked'), 'success');
    } catch {
      showToast(get(t)('settings.api_key_revoke_failed'), 'error');
    }
  }

  async function copyCreatedKey() {
    if (!createdKey) return;
    await navigator.clipboard.writeText(createdKey);
    createdKeyCopied = true;
    setTimeout(() => {
      createdKeyCopied = false;
    }, 2000);
  }

  function closeNewKeyModal() {
    showNewKeyModal = false;
    newKeyForm = { name: '', scopes: [], expires_at: '' };
    newKeyError = '';
    createdKey = null;
  }

  // ── SSH Keys ───────────────────────────────────────────────────────────────

  async function loadSshKeys() {
    sshKeysLoading = true;
    try {
      sshKeys = await api.sshKeys.list();
      sshKeysLoaded = true;
    } catch {
      showToast(get(t)('settings.ssh_key_load_failed'), 'error');
    } finally {
      sshKeysLoading = false;
    }
  }

  async function addSshKey(e: SubmitEvent) {
    e.preventDefault();
    if (!newSshPubkey.trim() || !newSshLabel.trim()) return;
    sshAddLoading = true;
    sshAddError = '';
    try {
      const key = await api.sshKeys.add(newSshPubkey.trim(), newSshLabel.trim());
      sshKeys = [
        ...sshKeys,
        {
          id: key.id,
          label: newSshLabel.trim(),
          fingerprint: key.fingerprint,
          key_type: newSshPubkey.trim().split(' ')[0] ?? 'unknown',
          added_at: new Date().toISOString(),
          last_used_at: null,
        },
      ];
      newSshPubkey = '';
      newSshLabel = '';
      showToast(get(t)('settings.ssh_key_added'), 'success');
    } catch (err: unknown) {
      const e = err as { message?: string };
      sshAddError = e.message ?? get(t)('settings.ssh_key_add_failed');
    } finally {
      sshAddLoading = false;
    }
  }

  async function deleteSshKey(key: SshKey) {
    confirmDeleteSshKeyId = null;
    try {
      await api.sshKeys.delete(key.id);
      sshKeys = sshKeys.filter((k) => k.id !== key.id);
      showToast(get(t)('settings.ssh_key_deleted'), 'success');
    } catch {
      showToast(get(t)('settings.ssh_key_delete_failed'), 'error');
    }
  }

  async function disablePasswordLogin() {
    disablePwdLoading = true;
    try {
      await api.sshKeys.disablePasswordLogin();
      showDisablePasswordConfirm = false;
      showToast(get(t)('settings.password_login_disabled'), 'success');
    } catch (err: unknown) {
      const e = err as { message?: string };
      showToast(e.message ?? get(t)('settings.password_login_disable_failed'), 'error');
    } finally {
      disablePwdLoading = false;
    }
  }

  // ── License ────────────────────────────────────────────────────────────────

  async function activateLicense(e: SubmitEvent) {
    e.preventDefault();
    if (!licenseKey.trim()) return;
    licenseActivating = true;
    try {
      const result = await api.license.activate(licenseKey.trim());
      license.tier = result.tier;
      license.domain = result.domain;
      license.expires_at = result.expires_at;
      licenseKey = '';
      showToast(get(t)('settings.license_activated'), 'success');
    } catch (err: unknown) {
      const e = err as { message?: string };
      showToast(e.message ?? get(t)('settings.license_activate_failed'), 'error');
    } finally {
      licenseActivating = false;
    }
  }

  // ── Helpers ─────────────────────────────────────────────────────────────────

  function showToast(msg: string, type: 'success' | 'error') {
    if (toastTimer) clearTimeout(toastTimer);
    toastMessage = msg;
    toastType = type;
    toastTimer = setTimeout(() => {
      toastMessage = '';
    }, 4000);
  }

  function formatDateShort(dateStr: string): string {
    return new Date(dateStr).toLocaleDateString();
  }

  function formatRelative(dateStr: string | null): string {
    if (!dateStr) return 'Never';
    const d = new Date(dateStr);
    const diff = Date.now() - d.getTime();
    if (diff < 86400000) return `${Math.floor(diff / 3600000)}h ago`;
    if (diff < 604800000) return `${Math.floor(diff / 86400000)}d ago`;
    return d.toLocaleDateString();
  }

  let tabs = $derived.by(() => {
    const tr = get(t);
    return [
      { key: 'general', label: tr('settings.general'), icon: 'M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z M15 12a3 3 0 11-6 0 3 3 0 016 0z' },
      { key: 'branding', label: tr('settings.branding'), icon: 'M4 16l4.586-4.586a2 2 0 012.828 0L16 16m-2-2l1.586-1.586a2 2 0 012.828 0L20 14m-6-6h.01M6 20h12a2 2 0 002-2V6a2 2 0 00-2-2H6a2 2 0 00-2 2v12a2 2 0 002 2z' },
      { key: 'smtp', label: tr('settings.smtp'), icon: 'M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z' },
      { key: 'smtp-limits', label: tr('settings.smtp_limits'), icon: 'M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z' },
      { key: 'security', label: tr('settings.security'), icon: 'M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z' },
      { key: 'api-keys', label: tr('settings.api_keys'), icon: 'M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z' },
      { key: 'advanced', label: tr('settings.advanced'), icon: 'M12 6V4m0 2a2 2 0 100 4m0-4a2 2 0 110 4m-6 8a2 2 0 100-4m0 4a2 2 0 110-4m0 4v2m0-6V4m6 6v10m6-2a2 2 0 100-4m0 4a2 2 0 110-4m0 4v2m0-6V4' },
    ];
  });

  // Reactive branding preview values
  let primaryColor = $derived(branding.primary_color);
  let panelName = $derived(branding.panel_name);

  const timezones = [
    'UTC',
    'America/New_York',
    'America/Chicago',
    'America/Denver',
    'America/Los_Angeles',
    'America/Sao_Paulo',
    'Europe/London',
    'Europe/Paris',
    'Europe/Berlin',
    'Europe/Moscow',
    'Asia/Dubai',
    'Asia/Kolkata',
    'Asia/Bangkok',
    'Asia/Singapore',
    'Asia/Tokyo',
    'Asia/Seoul',
    'Australia/Sydney',
    'Pacific/Auckland',
  ];

  const apiScopes = ['read', 'write', 'sites', 'databases', 'email', 'dns', 'backups', 'users'];
</script>

<svelte:head>
  <title>{$t('settings.title')} — JottiCP</title>
</svelte:head>

<div class="p-4 lg:p-6">
  <div class="mb-5">
    <h1 class="text-xl font-semibold text-foreground">{$t('settings.title')}</h1>
    <p class="text-sm text-muted-foreground mt-0.5">{$t('settings.subtitle')}</p>
  </div>

  <div class="flex flex-col lg:flex-row gap-6">
    <!-- Sidebar (desktop) / top tabs (mobile) -->
    <aside class="lg:w-52 shrink-0">
      <!-- Mobile: horizontal scroll -->
      <div class="flex gap-1 overflow-x-auto pb-1 lg:hidden">
        {#each tabs as tab}
          <button
            on:click={() => switchTab(tab.key)}
            class="flex-shrink-0 h-9 px-3 rounded-lg text-sm font-medium transition-colors whitespace-nowrap
                   {activeTab === tab.key
                     ? 'bg-primary/10 text-primary'
                     : 'text-muted-foreground hover:bg-muted hover:text-foreground'}"
          >
            {tab.label}
          </button>
        {/each}
      </div>

      <!-- Desktop: vertical nav -->
      <nav class="hidden lg:flex flex-col gap-0.5">
        {#each tabs as tab}
          <button
            on:click={() => switchTab(tab.key)}
            class="w-full h-9 px-3 rounded-lg text-sm font-medium text-left flex items-center gap-2.5 transition-all duration-150
                   {activeTab === tab.key
                     ? 'bg-primary/10 text-primary'
                     : 'text-muted-foreground hover:bg-muted hover:text-foreground'}"
          >
            <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.75" d={tab.icon} />
            </svg>
            {tab.label}
          </button>
        {/each}
      </nav>
    </aside>

    <!-- Content -->
    <div class="flex-1 min-w-0">
      {#if loading}
        <div class="space-y-3">
          {#each [1, 2, 3, 4] as _}
            <div class="h-12 bg-muted rounded-xl animate-pulse"></div>
          {/each}
        </div>

        <!-- ── General ──────────────────────────────────────────────────────── -->
      {:else if activeTab === 'general'}
        <form on:submit={saveGeneral} class="max-w-xl settings-section">
          <div class="bg-card border border-border rounded-xl overflow-hidden mb-4">
            <div class="px-4 py-3 border-b border-border bg-muted/30">
              <h2 class="text-sm font-semibold text-foreground">{$t('settings.panel')}</h2>
            </div>

            <div class="border-b border-border py-4 px-4 flex items-center justify-between last:border-0 transition-colors duration-150 hover:bg-muted/20">
              <div class="mr-4">
                <div class="flex items-center gap-2">
                  <svg class="w-3.5 h-3.5 text-muted-foreground shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M3.055 11H5a2 2 0 012 2v1a2 2 0 002 2 2 2 0 012 2v2.945M8 3.935V5.5A2.5 2.5 0 0010.5 8h.5a2 2 0 012 2 2 2 0 104 0 2 2 0 012-2h1.064M15 20.488V18a2 2 0 012-2h3.064" /></svg>
                  <span class="text-sm font-medium text-foreground">{$t('settings.panel_domain')}</span>
                </div>
                <p class="text-xs text-muted-foreground mt-0.5">{$t('settings.panel_domain_desc')}</p>
              </div>
              <input
                type="text"
                bind:value={settings.panel_domain}
                placeholder={$t('settings.panel_domain_placeholder')}
                class="w-52 h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary transition-colors duration-150 focus:bg-background"
              />
            </div>

            <div class="border-b border-border py-4 px-4 flex items-center justify-between last:border-0 transition-colors duration-150 hover:bg-muted/20">
              <div class="mr-4">
                <div class="flex items-center gap-2">
                  <svg class="w-3.5 h-3.5 text-muted-foreground shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z" /></svg>
                  <span class="text-sm font-medium text-foreground">{$t('settings.admin_email')}</span>
                </div>
                <p class="text-xs text-muted-foreground mt-0.5">{$t('settings.admin_email_desc')}</p>
              </div>
              <input
                type="email"
                bind:value={settings.panel_email}
                placeholder={$t('settings.admin_email_placeholder')}
                 class="w-52 h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary transition-colors duration-150 focus:bg-background"
               />
             </div>

             <div class="py-4 px-4 flex items-center justify-between last:border-0 transition-colors duration-150 hover:bg-muted/20">
               <div class="mr-4">
                 <div class="flex items-center gap-2">
                   <svg class="w-3.5 h-3.5 text-muted-foreground shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 21V5a2 2 0 00-2-2H7a2 2 0 00-2 2v16m14 0h2m-2 0h-5m-9 0H3m2 0h5M9 7h1m-1 4h1m4-4h1m-1 4h1m-5 10v-5a1 1 0 011-1h2a1 1 0 011 1v5m-4 0h4" /></svg>
                   <span class="text-sm font-medium text-foreground">{$t('settings.company_name')}</span>
                 </div>
                 <p class="text-xs text-muted-foreground mt-0.5">{$t('settings.company_name_desc')}</p>
              </div>
              <input
                type="text"
                bind:value={settings.company_name}
                placeholder={$t('settings.company_name_placeholder')}
                class="w-52 h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary transition-colors duration-150 focus:bg-background"
              />
            </div>
          </div>

          <div class="bg-card border border-border rounded-xl overflow-hidden mb-4">
            <div class="px-4 py-3 border-b border-border bg-muted/30">
              <h2 class="text-sm font-semibold text-foreground">{$t('settings.defaults')}</h2>
            </div>

            <div class="border-b border-border py-4 px-4 flex items-center justify-between last:border-0 transition-colors duration-150 hover:bg-muted/20">
              <div class="mr-4">
                <div class="flex items-center gap-2">
                  <svg class="w-3.5 h-3.5 text-muted-foreground shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4" /></svg>
                  <span class="text-sm font-medium text-foreground">{$t('settings.default_php')}</span>
                </div>
                <p class="text-xs text-muted-foreground mt-0.5">{$t('settings.default_php_desc')}</p>
              </div>
              <select
                bind:value={settings.default_php}
                class="w-40 h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground appearance-none focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary transition-colors duration-150"
              >
                {#each ['8.3', '8.2', '8.1', '8.0', '7.4'] as ver}
                  <option value={ver}>PHP {ver}</option>
                {/each}
              </select>
            </div>

            <div class="border-b border-border py-4 px-4 last:border-0 transition-colors duration-150 hover:bg-muted/20">
              <div class="mb-3">
                <div class="flex items-center gap-2">
                  <svg class="w-3.5 h-3.5 text-muted-foreground shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2m-2-4h.01M17 16h.01" /></svg>
                  <span class="text-sm font-medium text-foreground">{$t('settings.default_webserver')}</span>
                </div>
                <p class="text-xs text-muted-foreground mt-0.5">{$t('settings.default_webserver_desc')}</p>
              </div>
              <div class="flex gap-2">
                {#each [
                  { value: 'nginx', label: 'Nginx' },
                  { value: 'apache', label: 'Apache' },
                  { value: 'ols', label: 'OpenLiteSpeed' },
                ] as opt}
                  <label class="cursor-pointer">
                    <input
                      type="radio"
                      bind:group={settings.default_webserver}
                      value={opt.value}
                      class="sr-only"
                    />
                    <div
                      class="h-8 px-3 rounded-lg border text-xs font-medium flex items-center transition-all
                                    {settings.default_webserver === opt.value
                        ? 'border-primary bg-primary/10 text-primary'
                        : 'border-border text-muted-foreground hover:bg-muted/50'}"
                    >
                      {opt.label}
                    </div>
                  </label>
                {/each}
              </div>
            </div>

            <div class="border-b border-border py-4 px-4 flex items-center justify-between last:border-0 transition-colors duration-150 hover:bg-muted/20">
              <div class="mr-4">
                <div class="flex items-center gap-2">
                  <svg class="w-3.5 h-3.5 text-muted-foreground shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z" /></svg>
                  <span class="text-sm font-medium text-foreground">{$t('settings.timezone')}</span>
                </div>
                <p class="text-xs text-muted-foreground mt-0.5">{$t('settings.timezone_desc')}</p>
              </div>
              <select
                bind:value={settings.timezone}
                class="w-52 h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground appearance-none focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary transition-colors duration-150"
              >
                {#each timezones as tz}
                  <option value={tz}>{tz}</option>
                {/each}
              </select>
            </div>

            <div class="border-b border-border py-4 px-4 flex items-center justify-between last:border-0 transition-colors duration-150 hover:bg-muted/20">
              <div class="mr-4">
                <div class="flex items-center gap-2">
                  <svg class="w-3.5 h-3.5 text-muted-foreground shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 12a9 9 0 01-9 9m9-9a9 9 0 00-9-9m9 9H3m9 9a9 9 0 01-9-9m9 9c1.657 0 3-4.03 3-9s-1.343-9-3-9m0 18c-1.657 0-3-4.03-3-9s1.343-9 3-9m-9 9a9 9 0 019-9" /></svg>
                  <span class="text-sm font-medium text-foreground">{$t('settings.language')}</span>
                </div>
                <p class="text-xs text-muted-foreground mt-0.5">{$t('settings.language_desc')}</p>
              </div>
              <select
                bind:value={settings.language}
                on:change={() => setLanguage(settings.language as any)}
                class="w-40 h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground appearance-none focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary transition-colors duration-150"
              >
                <option value="en">English</option>
                <option value="ru">Русский</option>
                <option value="es">Spanish</option>
                <option value="de">German</option>
                <option value="fr">French</option>
                <option value="pt">Portuguese</option>
              </select>
            </div>

            <div class="py-4 px-4 flex items-center justify-between last:border-0 transition-colors duration-150 hover:bg-muted/20">
              <div class="mr-4">
                <div class="flex items-center gap-2">
                  <svg class="w-3.5 h-3.5 text-muted-foreground shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M8 7V3m8 4V3m-9 8h10M5 21h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z" /></svg>
                  <span class="text-sm font-medium text-foreground">{$t('settings.date_format')}</span>
                </div>
                <p class="text-xs text-muted-foreground mt-0.5">{$t('settings.date_format_desc')}</p>
              </div>
              <select
                bind:value={settings.date_format}
                class="w-40 h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground appearance-none focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary transition-colors duration-150"
              >
                <option value="MM/DD/YYYY">MM/DD/YYYY</option>
                <option value="DD/MM/YYYY">DD/MM/YYYY</option>
                <option value="YYYY-MM-DD">YYYY-MM-DD</option>
              </select>
            </div>
          </div>

          <button
            type="submit"
            disabled={generalLoading}
            class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 transition-all duration-150 active:scale-95 disabled:opacity-50"
          >
            {generalLoading ? get(t)('common.saving') : get(t)('settings.save_settings')}
          </button>
        </form>

        <!-- ── Branding ──────────────────────────────────────────────────────── -->
      {:else if activeTab === 'branding'}
        <div class="max-w-3xl space-y-4 settings-section">
          <div class="flex flex-col lg:flex-row gap-4">
          <div class="flex-1 min-w-0">
          <div class="bg-card border border-border rounded-xl overflow-hidden">
            <div class="px-4 py-3 border-b border-border bg-muted/30">
              <h2 class="text-sm font-semibold text-foreground">{$t('settings.branding')}</h2>
            </div>

            <div class="border-b border-border py-4 px-4 flex items-center justify-between last:border-0 transition-colors duration-150 hover:bg-muted/20">
              <div class="mr-4">
                <p class="text-sm font-medium text-foreground">{$t('settings.panel_name')}</p>
                <p class="text-xs text-muted-foreground">{$t('settings.panel_name_desc')}</p>
              </div>
              <input
                type="text"
                bind:value={branding.panel_name}
                class="w-44 h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary transition-colors duration-150 focus:bg-background"
              />
            </div>

            <div class="border-b border-border py-4 px-4 flex items-center justify-between last:border-0 transition-colors duration-150 hover:bg-muted/20">
              <div class="mr-4">
                <p class="text-sm font-medium text-foreground">{$t('settings.primary_color')}</p>
                <p class="text-xs text-muted-foreground">{$t('settings.primary_color_desc')}</p>
              </div>
              <div class="flex items-center gap-2">
                <input
                  type="color"
                  bind:value={branding.primary_color}
                  class="w-9 h-9 rounded-lg border border-border bg-background cursor-pointer p-0.5"
                />
                <input
                  type="text"
                  bind:value={branding.primary_color}
                  class="w-28 h-9 rounded-lg border border-border bg-background px-3 text-sm font-mono text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary transition-colors duration-150 focus:bg-background"
                />
              </div>
            </div>

            <div class="border-b border-border py-4 px-4 flex items-start justify-between last:border-0">
              <div class="mr-4">
                <p class="text-sm font-medium text-foreground">{$t('settings.logo')}</p>
                <p class="text-xs text-muted-foreground">{$t('settings.logo_desc')}</p>
              </div>
              <div class="shrink-0 text-right space-y-2">
                {#if logoPreview}
                  <img
                    src={logoPreview}
                    alt="Logo preview"
                    class="h-10 w-auto rounded border border-border ml-auto"
                  />
                {/if}
                <label
                  class="h-9 px-3 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2 transition-colors cursor-pointer"
                >
                  <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      stroke-width="2"
                      d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-8l-4-4m0 0L8 8m4-4v12"
                    />
                  </svg>
                  {$t('common.upload')}
                  <input
                    type="file"
                    accept="image/*"
                    class="sr-only"
                    on:change={(e) => {
                      const f = (e.target as HTMLInputElement).files?.[0];
                      if (f) {
                        const r = new FileReader();
                        r.onload = (ev) => (logoPreview = ev.target?.result as string);
                        r.readAsDataURL(f);
                      }
                    }}
                  />
                </label>
              </div>
            </div>

            <div class="py-4 px-4 flex items-start justify-between last:border-0">
              <div class="mr-4">
                <p class="text-sm font-medium text-foreground">{$t('settings.favicon')}</p>
                <p class="text-xs text-muted-foreground">{$t('settings.favicon_desc')}</p>
              </div>
              <div class="shrink-0 text-right space-y-2">
                {#if faviconPreview}
                  <img
                    src={faviconPreview}
                    alt="Favicon preview"
                    class="h-8 w-8 rounded border border-border ml-auto"
                  />
                {/if}
                <label
                  class="h-9 px-3 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2 transition-colors cursor-pointer"
                >
                  <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      stroke-width="2"
                      d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-8l-4-4m0 0L8 8m4-4v12"
                    />
                  </svg>
                  {$t('common.upload')}
                  <input
                    type="file"
                    accept="image/*,.ico"
                    class="sr-only"
                    on:change={(e) => {
                      const f = (e.target as HTMLInputElement).files?.[0];
                      if (f) {
                        const r = new FileReader();
                        r.onload = (ev) => (faviconPreview = ev.target?.result as string);
                        r.readAsDataURL(f);
                      }
                    }}
                  />
                </label>
              </div>
            </div>
          </div>

          <button
            disabled={brandingLoading}
            on:click={async () => {
              brandingLoading = true;
              try {
                const r = await fetch('/api/v1/branding', {
                  method: 'PUT',
                  headers: { 'Content-Type': 'application/json', ...authHeaders() },
                  body: JSON.stringify({
                    panel_name: branding.panel_name,
                    primary_color: branding.primary_color,
                  }),
                });
                if (!r.ok) throw new Error(`HTTP ${r.status}`);
                showToast(get(t)('settings.branding_saved'), 'success');
              } catch {
                showToast(get(t)('settings.branding_save_failed'), 'error');
              } finally {
                brandingLoading = false;
              }
            }}
            class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 transition-all duration-150 active:scale-95 disabled:opacity-50"
          >
            {brandingLoading ? get(t)('common.saving') : get(t)('settings.save_branding')}
          </button>
          </div><!-- end flex-1 -->

          <!-- Live preview -->
          <div class="lg:w-56 shrink-0">
            <p class="text-xs font-medium text-muted-foreground mb-2 uppercase tracking-wide">{$t('settings.live_preview')}</p>
            <div class="rounded-xl border border-border overflow-hidden bg-background p-0 fade-up">
              <div class="h-1.5 w-full" style="background: {primaryColor}"></div>
              <div class="flex h-40">
                <!-- Mini sidebar -->
                <div class="w-24 bg-card border-r border-border p-2 flex flex-col gap-1">
                  <p class="text-[8px] font-bold truncate mb-1" style="color: {primaryColor}">{panelName || 'JottiCP'}</p>
                  {#each [get(t)('nav.dashboard'), get(t)('nav.websites'), get(t)('nav.email')] as item}
                    <div class="h-4 rounded bg-muted/50 flex items-center px-1">
                      <span class="text-[7px] text-muted-foreground">{item}</span>
                    </div>
                  {/each}
                </div>
                <!-- Mini content area -->
                <div class="flex-1 p-2">
                  <div class="h-3 bg-muted/30 rounded w-3/4 mb-1"></div>
                  <div class="h-2 bg-muted/20 rounded w-1/2"></div>
                  <div class="mt-2 h-2 bg-muted/15 rounded w-2/3"></div>
                  <div class="mt-1 h-2 bg-muted/10 rounded w-1/3"></div>
                </div>
              </div>
            </div>
            <p class="text-[10px] text-muted-foreground mt-1.5 text-center">{$t('settings.updates_as_you_type')}</p>
          </div>
        </div><!-- end flex row -->
        </div>

        <!-- ── SMTP ──────────────────────────────────────────────────────────── -->
      {:else if activeTab === 'smtp'}
        <form on:submit={saveSmtp} class="max-w-xl space-y-4">
          <div class="bg-card border border-border rounded-xl overflow-hidden">
            <div class="px-4 py-3 border-b border-border bg-muted/30">
              <h2 class="text-sm font-semibold text-foreground">{$t('settings.smtp_config')}</h2>
            </div>

            <div class="p-4 space-y-3">
              <div class="grid grid-cols-2 gap-3">
                <div class="col-span-2 sm:col-span-1">
                  <label class="block text-sm font-medium text-foreground mb-1.5">{$t('settings.smtp_host')}</label>
                  <input
                    type="text"
                    bind:value={smtpConfig.host}
                    placeholder={$t('settings.smtp_host_placeholder')}
                    class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
                  />
                </div>
                <div>
                  <label class="block text-sm font-medium text-foreground mb-1.5">Port</label>
                  <input
                    type="number"
                    bind:value={smtpConfig.port}
                    class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm font-mono text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
                  />
                </div>
                <div>
                  <label class="block text-sm font-medium text-foreground mb-1.5">Username</label>
                  <input
                    type="text"
                    bind:value={smtpConfig.user}
                    autocomplete="off"
                    class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
                  />
                </div>
                <div>
                  <label class="block text-sm font-medium text-foreground mb-1.5">Password</label>
                  <div class="relative">
                    <input
                      type={showSmtpPassword ? 'text' : 'password'}
                      bind:value={smtpConfig.password}
                      autocomplete="off"
                      class="w-full h-9 rounded-lg border border-border bg-background px-3 pr-9 text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
                    />
                    <button
                      type="button"
                      on:click={() => (showSmtpPassword = !showSmtpPassword)}
                      class="absolute right-2.5 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                    >
                      {#if showSmtpPassword}
                        <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor"
                          ><path
                            stroke-linecap="round"
                            stroke-linejoin="round"
                            stroke-width="2"
                            d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M9.88 9.88l-3.29-3.29m7.532 7.532l3.29 3.29M3 3l3.59 3.59m0 0A9.953 9.953 0 0112 5c4.478 0 8.268 2.943 9.543 7a10.025 10.025 0 01-4.132 5.411m0 0L21 21"
                          /></svg
                        >
                      {:else}
                        <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor"
                          ><path
                            stroke-linecap="round"
                            stroke-linejoin="round"
                            stroke-width="2"
                            d="M15 12a3 3 0 11-6 0 3 3 0 016 0z M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z"
                          /></svg
                        >
                      {/if}
                    </button>
                  </div>
                </div>
                <div class="col-span-2">
                  <label class="block text-sm font-medium text-foreground mb-1.5">Encryption</label>
                  <div class="flex gap-2">
                    {#each [
                      { value: 'none', label: 'None' },
                      { value: 'starttls', label: 'STARTTLS' },
                      { value: 'tls', label: 'TLS/SSL' },
                    ] as opt}
                      <label class="cursor-pointer">
                        <input
                          type="radio"
                          bind:group={smtpConfig.encryption}
                          value={opt.value}
                          class="sr-only"
                        />
                        <div
                          class="h-8 px-3 rounded-lg border text-xs font-medium flex items-center transition-all
                                          {smtpConfig.encryption === opt.value
                            ? 'border-primary bg-primary/10 text-primary'
                            : 'border-border text-muted-foreground hover:bg-muted/50'}"
                        >
                          {opt.label}
                        </div>
                      </label>
                    {/each}
                  </div>
                </div>
                <div class="col-span-2">
                  <label class="block text-sm font-medium text-foreground mb-1.5">From Address</label>
                  <input
                    type="email"
                    bind:value={smtpConfig.from}
                    placeholder="noreply@yourdomain.com"
                    class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
                  />
                </div>
              </div>
            </div>
          </div>

          <div class="flex gap-2">
            <button
              type="submit"
              disabled={smtpSaveLoading}
              class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 transition-colors disabled:opacity-50"
            >
              {smtpSaveLoading ? 'Saving...' : 'Save SMTP'}
            </button>
          </div>

          <!-- Test email -->
          <div class="bg-card border border-border rounded-xl p-4 space-y-3">
            <h3 class="text-sm font-medium text-foreground">Test Email</h3>
            <div class="flex gap-2">
              <input
                type="email"
                bind:value={smtpTestEmail}
                placeholder="test@example.com"
                class="flex-1 h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
              />
              <button
                type="button"
                on:click={testSmtp}
                disabled={smtpTestLoading || !smtpTestEmail}
                class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2 transition-colors disabled:opacity-50 whitespace-nowrap"
              >
                {smtpTestLoading ? 'Sending...' : 'Send Test'}
              </button>
            </div>
            {#if smtpTestResult}
              <div
                class="flex items-start gap-2 rounded-lg px-3 py-2.5 text-sm
                          {smtpTestResult.success
                  ? 'bg-emerald-500/10 border border-emerald-500/20 text-emerald-400'
                  : 'bg-red-500/10 border border-red-500/20 text-red-400'}"
              >
                {#if smtpTestResult.success}
                  <svg class="w-4 h-4 mt-0.5 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor"
                    ><path
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      stroke-width="2"
                      d="M5 13l4 4L19 7"
                    /></svg
                  >
                {:else}
                  <svg class="w-4 h-4 mt-0.5 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor"
                    ><path
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      stroke-width="2"
                      d="M6 18L18 6M6 6l12 12"
                    /></svg
                  >
                {/if}
                {smtpTestResult.message}
              </div>
            {/if}
          </div>
        </form>

        <!-- ── SMTP Limits ───────────────────────────────────────────────────── -->
      {:else if activeTab === 'smtp-limits'}
        <form on:submit={saveSmtpLimits} class="max-w-2xl space-y-4 settings-section">

          <!-- Outbound SMTP -->
          <div class="bg-card border border-border rounded-xl overflow-hidden">
            <div class="px-4 py-3 border-b border-border bg-muted/30">
              <h2 class="text-sm font-semibold text-foreground">Outbound SMTP</h2>
            </div>
            <div class="divide-y divide-border">
              {#each [
                { label: 'Max recipients per message', key: 'max_recipients', min: 1, max: 500 },
                { label: 'Max message size (MB)', key: 'max_message_size_mb', min: 1, max: 100 },
                { label: 'Connection timeout (seconds)', key: 'connection_timeout', min: 5, max: 300 },
                { label: 'Max retry attempts', key: 'max_retry_attempts', min: 0, max: 10 },
                { label: 'Retry interval (minutes)', key: 'retry_interval_min', min: 1, max: 1440 },
                { label: 'Queue lifetime (hours)', key: 'queue_lifetime_hours', min: 1, max: 720 },
              ] as row}
                <div class="py-3 px-4 flex items-center justify-between transition-colors duration-150 hover:bg-muted/20">
                  <p class="text-sm font-medium text-foreground">{row.label}</p>
                  <input
                    type="number"
                    bind:value={smtpLimits[row.key as keyof typeof smtpLimits] as number}
                    min={row.min}
                    max={row.max}
                    class="w-24 h-8 rounded-lg border border-border bg-background px-3 text-sm font-mono text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary transition-colors duration-150 focus:bg-background"
                  />
                </div>
              {/each}
            </div>
          </div>

          <!-- Relay & Auth -->
          <div class="bg-card border border-border rounded-xl overflow-hidden">
            <div class="px-4 py-3 border-b border-border bg-muted/30">
              <h2 class="text-sm font-semibold text-foreground">Relay &amp; Authentication</h2>
            </div>
            <div class="divide-y divide-border">
              <div class="py-3 px-4 flex items-center justify-between transition-colors duration-150 hover:bg-muted/20">
                <div>
                  <p class="text-sm font-medium text-foreground">Allow open relay</p>
                  <p class="text-xs text-amber-400 mt-0.5">⚠ Never enable on production</p>
                </div>
                <button
                  type="button"
                  on:click={() => (smtpLimits.allow_open_relay = !smtpLimits.allow_open_relay)}
                  class="relative inline-flex h-5 w-9 items-center rounded-full transition-colors duration-200 focus:outline-none {smtpLimits.allow_open_relay ? 'bg-amber-500' : 'bg-muted'}"
                  role="switch"
                  aria-checked={smtpLimits.allow_open_relay}
                >
                  <span class="inline-block h-3.5 w-3.5 rounded-full bg-white shadow transition-transform duration-200 {smtpLimits.allow_open_relay ? 'translate-x-4' : 'translate-x-0.5'}"></span>
                </button>
              </div>
              <div class="py-3 px-4 flex items-center justify-between transition-colors duration-150 hover:bg-muted/20">
                <p class="text-sm font-medium text-foreground">Require SMTP auth</p>
                <button
                  type="button"
                  on:click={() => (smtpLimits.require_smtp_auth = !smtpLimits.require_smtp_auth)}
                  class="relative inline-flex h-5 w-9 items-center rounded-full transition-colors duration-200 focus:outline-none {smtpLimits.require_smtp_auth ? 'bg-primary' : 'bg-muted'}"
                  role="switch"
                  aria-checked={smtpLimits.require_smtp_auth}
                >
                  <span class="inline-block h-3.5 w-3.5 rounded-full bg-white shadow transition-transform duration-200 {smtpLimits.require_smtp_auth ? 'translate-x-4' : 'translate-x-0.5'}"></span>
                </button>
              </div>
              <div class="py-3 px-4 flex items-center justify-between transition-colors duration-150 hover:bg-muted/20">
                <div>
                  <p class="text-sm font-medium text-foreground">Allow AUTH over plaintext</p>
                  <p class="text-xs text-amber-400 mt-0.5">⚠ Exposes credentials without TLS</p>
                </div>
                <button
                  type="button"
                  on:click={() => (smtpLimits.allow_auth_plaintext = !smtpLimits.allow_auth_plaintext)}
                  class="relative inline-flex h-5 w-9 items-center rounded-full transition-colors duration-200 focus:outline-none {smtpLimits.allow_auth_plaintext ? 'bg-amber-500' : 'bg-muted'}"
                  role="switch"
                  aria-checked={smtpLimits.allow_auth_plaintext}
                >
                  <span class="inline-block h-3.5 w-3.5 rounded-full bg-white shadow transition-transform duration-200 {smtpLimits.allow_auth_plaintext ? 'translate-x-4' : 'translate-x-0.5'}"></span>
                </button>
              </div>
              <div class="py-3 px-4 transition-colors duration-150 hover:bg-muted/20">
                <p class="text-sm font-medium text-foreground mb-2">TLS requirement</p>
                <div class="flex gap-2 flex-wrap">
                  {#each [
                    { value: 'optional', label: 'Optional' },
                    { value: 'required', label: 'Required (recommended)' },
                    { value: 'strict', label: 'Strict' },
                  ] as opt}
                    <label class="cursor-pointer">
                      <input type="radio" bind:group={smtpLimits.tls_requirement} value={opt.value} class="sr-only" />
                      <div class="h-8 px-3 rounded-lg border text-xs font-medium flex items-center transition-all {smtpLimits.tls_requirement === opt.value ? 'border-primary bg-primary/10 text-primary' : 'border-border text-muted-foreground hover:bg-muted/50'}">
                        {opt.label}
                      </div>
                    </label>
                  {/each}
                </div>
              </div>
            </div>
          </div>

          <!-- Rate Limiting -->
          <div class="bg-card border border-border rounded-xl overflow-hidden">
            <div class="px-4 py-3 border-b border-border bg-muted/30">
              <h2 class="text-sm font-semibold text-foreground">Rate Limiting</h2>
            </div>
            <div class="divide-y divide-border">
              <div class="py-3 px-4 flex items-center justify-between transition-colors duration-150 hover:bg-muted/20">
                <p class="text-sm font-medium text-foreground">Global hourly limit per IP</p>
                <input type="number" bind:value={smtpLimits.rate_limit_hourly_per_ip} min="1" class="w-24 h-8 rounded-lg border border-border bg-background px-3 text-sm font-mono text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary transition-colors duration-150 focus:bg-background" />
              </div>
              <div class="py-3 px-4 flex items-center justify-between transition-colors duration-150 hover:bg-muted/20">
                <p class="text-sm font-medium text-foreground">Global daily limit per domain</p>
                <input type="number" bind:value={smtpLimits.rate_limit_daily_per_domain} min="1" class="w-24 h-8 rounded-lg border border-border bg-background px-3 text-sm font-mono text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary transition-colors duration-150 focus:bg-background" />
              </div>
              <div class="py-3 px-4 flex items-center justify-between transition-colors duration-150 hover:bg-muted/20">
                <p class="text-sm font-medium text-foreground">Fail2ban integration</p>
                <button
                  type="button"
                  on:click={() => (smtpLimits.fail2ban_enabled = !smtpLimits.fail2ban_enabled)}
                  class="relative inline-flex h-5 w-9 items-center rounded-full transition-colors duration-200 focus:outline-none {smtpLimits.fail2ban_enabled ? 'bg-primary' : 'bg-muted'}"
                  role="switch"
                  aria-checked={smtpLimits.fail2ban_enabled}
                >
                  <span class="inline-block h-3.5 w-3.5 rounded-full bg-white shadow transition-transform duration-200 {smtpLimits.fail2ban_enabled ? 'translate-x-4' : 'translate-x-0.5'}"></span>
                </button>
              </div>
              <div class="py-3 px-4 flex items-center justify-between transition-colors duration-150 hover:bg-muted/20">
                <p class="text-sm font-medium text-foreground">Abuse threshold (failed auths before ban)</p>
                <input type="number" bind:value={smtpLimits.abuse_threshold} min="1" max="50" class="w-20 h-8 rounded-lg border border-border bg-background px-3 text-sm font-mono text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary transition-colors duration-150 focus:bg-background" />
              </div>
            </div>
          </div>

          <!-- Blacklist / Whitelist -->
          <div class="bg-card border border-border rounded-xl overflow-hidden">
            <div class="px-4 py-3 border-b border-border bg-muted/30">
              <h2 class="text-sm font-semibold text-foreground">Blacklist / Whitelist</h2>
            </div>
            <div class="p-4 grid grid-cols-1 sm:grid-cols-2 gap-4">
              <div>
                <label class="block text-xs font-medium text-foreground mb-1.5">
                  Whitelist
                  <span class="text-muted-foreground font-normal ml-1">Always allow (one per line)</span>
                </label>
                <textarea
                  bind:value={smtpLimits.whitelist}
                  placeholder={"example.com\n192.168.1.1"}
                  class="font-mono text-xs bg-background text-foreground border border-border rounded-lg p-3 h-32 resize-none w-full focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
                ></textarea>
              </div>
              <div>
                <label class="block text-xs font-medium text-foreground mb-1.5">
                  Blacklist
                  <span class="text-muted-foreground font-normal ml-1">Always block (one per line)</span>
                </label>
                <textarea
                  bind:value={smtpLimits.blacklist}
                  placeholder={"spammer.com\n1.2.3.4"}
                  class="font-mono text-xs bg-background text-foreground border border-border rounded-lg p-3 h-32 resize-none w-full focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
                ></textarea>
              </div>
            </div>
          </div>

          <!-- Save -->
          <button
            type="submit"
            disabled={smtpLimitsSaveLoading}
            class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 transition-all duration-150 active:scale-95 disabled:opacity-50"
          >
            {smtpLimitsSaveLoading ? 'Saving...' : 'Save SMTP Limits'}
          </button>

          <!-- Diagnostics -->
          <div class="bg-card border border-border rounded-xl p-4 space-y-3">
            <h3 class="text-sm font-semibold text-foreground">Test &amp; Diagnostics</h3>
            <div class="flex flex-wrap gap-2">
              <button
                type="button"
                on:click={testOutboundSmtp}
                disabled={smtpLimitsTestLoading}
                class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2 transition-all duration-150 active:scale-95 disabled:opacity-50"
              >
                {smtpLimitsTestLoading ? 'Sending...' : 'Test outbound SMTP'}
              </button>
              <button
                type="button"
                on:click={viewSmtpQueue}
                disabled={smtpQueueLoading}
                class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2 transition-all duration-150 active:scale-95 disabled:opacity-50"
              >
                {smtpQueueLoading ? 'Loading...' : 'View SMTP queue'}
              </button>
              {#if smtpQueueCount !== null && smtpQueueCount > 0}
                {#if confirmFlushQueue}
                  <div class="flex items-center gap-1.5">
                    <span class="text-xs text-destructive">Clear all queued messages?</span>
                    <button class="text-xs px-2 py-1 rounded bg-destructive text-white hover:bg-destructive/90" on:click={flushSmtpQueue}>Yes</button>
                    <button class="text-xs px-2 py-1 rounded bg-muted" on:click={() => confirmFlushQueue = false}>No</button>
                  </div>
                {:else}
                  <button
                    type="button"
                    on:click={() => confirmFlushQueue = true}
                    disabled={smtpFlushLoading}
                    class="h-9 px-4 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-sm font-medium hover:bg-red-500/20 inline-flex items-center gap-2 transition-all duration-150 active:scale-95 disabled:opacity-50"
                  >
                    {smtpFlushLoading ? 'Flushing...' : 'Flush queue'}
                  </button>
                {/if}
              {/if}
            </div>
            {#if smtpLimitsTestResult}
              <div class="flex items-start gap-2 rounded-lg px-3 py-2.5 text-sm {smtpLimitsTestResult.success ? 'bg-emerald-500/10 border border-emerald-500/20 text-emerald-400' : 'bg-red-500/10 border border-red-500/20 text-red-400'} fade-up">
                {smtpLimitsTestResult.message}
              </div>
            {/if}
            {#if smtpQueueCount !== null}
              <div class="rounded-lg bg-muted/30 border border-border px-3 py-2 text-sm fade-up">
                <span class="font-medium text-foreground">{smtpQueueCount}</span>
                <span class="text-muted-foreground ml-1">message{smtpQueueCount === 1 ? '' : 's'} in queue</span>
              </div>
            {/if}
            {#if smtpQueueRows.length > 0}
              <div class="overflow-x-auto rounded-lg border border-border fade-up">
                <table class="w-full text-xs">
                  <thead class="bg-muted/30">
                    <tr>
                      <th class="px-3 py-2 text-left text-muted-foreground font-medium">From</th>
                      <th class="px-3 py-2 text-left text-muted-foreground font-medium">To</th>
                      <th class="px-3 py-2 text-left text-muted-foreground font-medium">Queued</th>
                      <th class="px-3 py-2 text-right text-muted-foreground font-medium">Retries</th>
                    </tr>
                  </thead>
                  <tbody class="divide-y divide-border">
                    {#each smtpQueueRows as row (row.id)}
                      <tr class="hover:bg-muted/20 transition-colors duration-150">
                        <td class="px-3 py-2 font-mono text-foreground">{row.from}</td>
                        <td class="px-3 py-2 font-mono text-foreground">{row.to}</td>
                        <td class="px-3 py-2 text-muted-foreground">{formatRelative(row.queued_at)}</td>
                        <td class="px-3 py-2 text-right text-muted-foreground">{row.retries}</td>
                      </tr>
                    {/each}
                  </tbody>
                </table>
              </div>
            {/if}
          </div>
        </form>

        <!-- ── Security ──────────────────────────────────────────────────────── -->
      {:else if activeTab === 'security'}
        <form on:submit={saveSecurity} class="max-w-xl space-y-4 settings-section">
          <div class="bg-card border border-border rounded-xl overflow-hidden">
            <div class="px-4 py-3 border-b border-border bg-muted/30">
              <h2 class="text-sm font-semibold text-foreground">Login & Sessions</h2>
            </div>

            <div class="border-b border-border py-4 px-4 flex items-center justify-between last:border-0">
              <div class="mr-4">
                <p class="text-sm font-medium text-foreground">Session Timeout</p>
                <p class="text-xs text-muted-foreground">Inactive sessions are invalidated after this period</p>
              </div>
              <select
                bind:value={security.session_timeout}
                class="w-36 h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground appearance-none focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
              >
                <option value="15min">15 minutes</option>
                <option value="30min">30 minutes</option>
                <option value="1hr">1 hour</option>
                <option value="8hr">8 hours</option>
                <option value="24hr">24 hours</option>
              </select>
            </div>

            <div class="py-4 px-4 flex items-center justify-between last:border-0">
              <div class="mr-4">
                <p class="text-sm font-medium text-foreground">Max Login Attempts</p>
                <p class="text-xs text-muted-foreground">Account gets locked after this many failures</p>
              </div>
              <input
                type="number"
                bind:value={security.max_login_attempts}
                min="1"
                max="20"
                class="w-20 h-9 rounded-lg border border-border bg-background px-3 text-sm font-mono text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
              />
            </div>
          </div>

          <div class="bg-card border border-border rounded-xl overflow-hidden">
            <div class="px-4 py-3 border-b border-border bg-muted/30">
              <h2 class="text-sm font-semibold text-foreground">Two-Factor Authentication</h2>
            </div>

            <div class="border-b border-border py-4 px-4 last:border-0">
              <p class="text-sm font-medium text-foreground mb-1">2FA Policy</p>
              <p class="text-xs text-muted-foreground mb-3">
                Determine who must use two-factor authentication
              </p>
              <div class="space-y-2">
                {#each [
                  {
                    value: 'optional',
                    label: 'Optional',
                    desc: 'Users can choose to enable 2FA',
                  },
                  {
                    value: 'admin_required',
                    label: 'Required for admins',
                    desc: 'All admin accounts must use 2FA',
                  },
                  {
                    value: 'all_required',
                    label: 'Required for all',
                    desc: 'Every account must have 2FA enabled',
                  },
                ] as opt}
                  <label
                    class="flex items-start gap-3 p-3 rounded-lg border cursor-pointer transition-colors
                                  {security.totp_policy === opt.value
                      ? 'border-primary bg-primary/5'
                      : 'border-border hover:bg-muted/30'}"
                  >
                    <input
                      type="radio"
                      bind:group={security.totp_policy}
                      value={opt.value}
                      class="mt-0.5 text-primary focus:ring-primary/50"
                    />
                    <div>
                      <p class="text-sm font-medium text-foreground">{opt.label}</p>
                      <p class="text-xs text-muted-foreground">{opt.desc}</p>
                    </div>
                  </label>
                {/each}
              </div>
            </div>

            <div class="py-4 px-4 flex items-center justify-between last:border-0">
              <div>
                <p class="text-sm font-medium text-foreground">Your 2FA Setup</p>
                <p class="text-xs text-muted-foreground">Configure TOTP for your own admin account</p>
              </div>
              <a
                href="/settings/totp"
                class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2 transition-colors"
              >
                Setup 2FA
                <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7" />
                </svg>
              </a>
            </div>
          </div>

          <div class="bg-card border border-border rounded-xl overflow-hidden">
            <div class="px-4 py-3 border-b border-border bg-muted/30">
              <h2 class="text-sm font-semibold text-foreground">Access Control</h2>
            </div>
            <div class="py-4 px-4">
              <p class="text-sm font-medium text-foreground mb-1">IP Allowlist for Admin Panel</p>
              <p class="text-xs text-muted-foreground mb-2">
                Only allow access from these IP addresses (one per line). Leave blank to allow all.
              </p>
              <textarea
                bind:value={security.ip_whitelist}
                rows="4"
                placeholder={'192.168.1.0/24\n10.0.0.1\n2001:db8::1'}
                class="w-full rounded-lg border border-border bg-background px-3 py-2 text-sm font-mono text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary resize-none"
              ></textarea>
            </div>
          </div>

          <button
            type="submit"
            disabled={securityLoading}
            class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 transition-all duration-150 active:scale-95 disabled:opacity-50"
          >
            {securityLoading ? 'Saving...' : 'Save Security Settings'}
          </button>

          <!-- Recent Login Activity -->
          <div class="bg-card border border-border rounded-xl overflow-hidden">
            <div class="px-4 py-3 border-b border-border bg-muted/30 flex items-center justify-between">
              <h2 class="text-sm font-semibold text-foreground">Recent Login Activity</h2>
              <span class="text-xs text-muted-foreground">Last 5 logins</span>
            </div>
            {#if loginAuditLoading}
              <div class="p-4 space-y-2">
                {#each [1,2,3] as _}
                  <div class="h-10 bg-muted rounded-lg animate-pulse"></div>
                {/each}
              </div>
            {:else if loginAudit.length > 0}
              <div class="overflow-x-auto">
                <table class="w-full text-xs">
                  <thead class="bg-muted/20">
                    <tr>
                      <th class="px-4 py-2 text-left text-muted-foreground font-medium">IP</th>
                      <th class="px-4 py-2 text-left text-muted-foreground font-medium">Country</th>
                      <th class="px-4 py-2 text-left text-muted-foreground font-medium">Browser</th>
                      <th class="px-4 py-2 text-left text-muted-foreground font-medium">Time</th>
                      <th class="px-4 py-2 text-right text-muted-foreground font-medium">Status</th>
                    </tr>
                  </thead>
                  <tbody class="divide-y divide-border">
                    {#each loginAudit as entry}
                      <tr class="hover:bg-muted/20 transition-colors duration-150">
                        <td class="px-4 py-2.5 font-mono text-foreground">{entry.ip}</td>
                        <td class="px-4 py-2.5 text-muted-foreground">{entry.country}</td>
                        <td class="px-4 py-2.5 text-muted-foreground">{simplifyUA(entry.ua)}</td>
                        <td class="px-4 py-2.5 text-muted-foreground">{formatRelative(entry.time)}</td>
                        <td class="px-4 py-2.5 text-right">
                          {#if entry.success}
                            <span class="inline-flex items-center px-1.5 py-0.5 rounded-full text-[10px] font-medium bg-emerald-500/10 text-emerald-400 border border-emerald-500/20">OK</span>
                          {:else}
                            <span class="inline-flex items-center px-1.5 py-0.5 rounded-full text-[10px] font-medium bg-red-500/10 text-red-400 border border-red-500/20">Failed</span>
                          {/if}
                        </td>
                      </tr>
                    {/each}
                  </tbody>
                </table>
              </div>
            {:else}
              <div class="p-6 text-center">
                <p class="text-sm text-muted-foreground">Audit log coming soon</p>
              </div>
            {/if}
          </div>
        </form>

        <!-- ── API Keys ──────────────────────────────────────────────────────── -->
      {:else if activeTab === 'api-keys'}
        <div class="max-w-2xl space-y-4 settings-section">
          <div class="flex items-center justify-between">
            <div>
              <h2 class="text-sm font-semibold text-foreground">API Keys</h2>
              <p class="text-xs text-muted-foreground mt-0.5">Keys for programmatic access to JottiCP</p>
            </div>
            <button
              on:click={() => (showNewKeyModal = true)}
              class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
            >
              <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4" />
              </svg>
              Generate Key
            </button>
          </div>

          {#if apiKeysLoading}
            <div class="space-y-2">
              {#each [1, 2, 3] as _}
                <div class="h-16 bg-muted rounded-xl animate-pulse"></div>
              {/each}
            </div>
          {:else if apiKeys.length === 0}
            <div class="bg-card border border-border rounded-xl p-10 text-center">
              <svg
                class="w-8 h-8 text-muted-foreground mx-auto mb-3"
                fill="none"
                viewBox="0 0 24 24"
                stroke="currentColor"
              >
                <path
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  stroke-width="1.5"
                  d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z"
                />
              </svg>
              <p class="text-sm font-medium text-foreground mb-1">No API keys</p>
              <p class="text-xs text-muted-foreground">Generate a key to access the JottiCP API.</p>
            </div>
          {:else}
            <div class="bg-card border border-border rounded-xl overflow-hidden">
              {#each apiKeys as key (key.id)}
                <div
                  class="group relative flex items-center justify-between gap-4 px-4 py-3 border-b border-border last:border-0 transition-colors duration-150 hover:bg-muted/20"
                >
                  <div class="min-w-0 flex-1">
                    <div class="flex items-center gap-2 flex-wrap">
                      <p class="text-sm font-medium text-foreground truncate">{key.name}</p>
                      {#if !key.is_active}
                        <span class="text-xs text-red-400 bg-red-500/10 border border-red-500/20 px-1.5 py-0.5 rounded-full">Revoked</span>
                      {/if}
                    </div>
                    <p class="text-xs font-mono text-muted-foreground mt-0.5">{key.key_prefix}••••••••</p>
                    {#if key.scopes?.length}
                      <div class="flex flex-wrap gap-1 mt-1">
                        {#each key.scopes as scope}
                          <span class="inline-flex items-center px-1.5 py-0.5 rounded text-[10px] font-medium
                            {scope === 'read' ? 'bg-sky-500/10 text-sky-400 border border-sky-500/20' :
                             scope === 'write' ? 'bg-violet-500/10 text-violet-400 border border-violet-500/20' :
                             scope === 'sites' ? 'bg-emerald-500/10 text-emerald-400 border border-emerald-500/20' :
                             scope === 'databases' ? 'bg-amber-500/10 text-amber-400 border border-amber-500/20' :
                             scope === 'email' ? 'bg-blue-500/10 text-blue-400 border border-blue-500/20' :
                             scope === 'dns' ? 'bg-teal-500/10 text-teal-400 border border-teal-500/20' :
                             scope === 'backups' ? 'bg-orange-500/10 text-orange-400 border border-orange-500/20' :
                             'bg-muted text-muted-foreground border border-border'}">{scope}</span>
                        {/each}
                      </div>
                    {/if}
                    <p class="text-xs text-muted-foreground mt-0.5">
                      Created {formatDateShort(key.created_at)}
                      {#if key.last_used_at} · Last used {formatRelative(key.last_used_at)}{/if}
                      {#if key.expires_at} · Expires {formatDateShort(key.expires_at)}{/if}
                    </p>
                  </div>
                  <!-- Hover actions slide in from right -->
                  <div class="flex items-center gap-2 shrink-0 opacity-0 group-hover:opacity-100 transition-all duration-150 translate-x-2 group-hover:translate-x-0">
                    {#if confirmRevokeKeyId === key.id}
                      <div class="flex items-center gap-1.5">
                        <span class="text-xs text-destructive">Revoke?</span>
                        <button class="text-xs px-2 py-1 rounded bg-destructive text-white hover:bg-destructive/90" on:click={() => revokeApiKey(key)}>Yes</button>
                        <button class="text-xs px-2 py-1 rounded bg-muted" on:click={() => confirmRevokeKeyId = null}>No</button>
                      </div>
                    {:else}
                      <button
                        on:click={() => confirmRevokeKeyId = key.id}
                        class="h-8 px-3 rounded-lg border border-red-500/20 text-xs font-medium text-red-400 hover:bg-red-500/10 transition-colors whitespace-nowrap"
                      >
                        Revoke
                      </button>
                    {/if}
                  </div>
                </div>
              {/each}
            </div>
          {/if}
        </div>

        <!-- ── Advanced ──────────────────────────────────────────────────────── -->
      {:else if activeTab === 'advanced'}
        <div class="max-w-2xl space-y-5 settings-section">
          <!-- License -->
          <div class="bg-card border border-border rounded-xl overflow-hidden">
            <div class="px-4 py-3 border-b border-border bg-muted/30">
              <h2 class="text-sm font-semibold text-foreground">License</h2>
            </div>
            <div class="p-4 space-y-4">
              <div class="flex items-center gap-4 flex-wrap">
                <div>
                  <p class="text-xs text-muted-foreground mb-1">Tier</p>
                  <span
                    class="inline-flex items-center px-2.5 py-1 rounded-full text-xs font-semibold
                               {license.tier === 'community'
                      ? 'bg-muted text-muted-foreground'
                      : 'bg-primary/10 text-primary'}"
                  >
                    {license.tier === 'community' ? 'Community (Free)' : license.tier}
                  </span>
                </div>
                {#if license.domain}
                  <div>
                    <p class="text-xs text-muted-foreground mb-1">Domain</p>
                    <p class="text-sm font-medium text-foreground">{license.domain}</p>
                  </div>
                {/if}
                <div>
                  <p class="text-xs text-muted-foreground mb-1">Expires</p>
                  <p class="text-sm font-medium text-foreground">
                    {license.expires_at ? new Date(license.expires_at).toLocaleDateString() : 'Never'}
                  </p>
                </div>
              </div>
              <form on:submit={activateLicense} class="flex gap-2">
                <input
                  type="text"
                  bind:value={licenseKey}
                  placeholder="ORBIT-XXXX-XXXX-XXXX-XXXX"
                  class="flex-1 h-9 rounded-lg border border-border bg-background px-3 text-sm font-mono text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
                />
                <button
                  type="submit"
                  disabled={licenseActivating || !licenseKey.trim()}
                  class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 transition-colors disabled:opacity-50"
                >
                  {licenseActivating ? 'Activating...' : 'Activate'}
                </button>
              </form>
            </div>
          </div>

          <!-- SSH Keys -->
          <div class="bg-card border border-border rounded-xl overflow-hidden">
            <div class="px-4 py-3 border-b border-border bg-muted/30">
              <h2 class="text-sm font-semibold text-foreground">SSH Keys</h2>
            </div>
            <div class="p-4 space-y-4">
              <form on:submit={addSshKey} class="space-y-3">
                <div>
                  <label class="block text-sm font-medium text-foreground mb-1.5">Label</label>
                  <input
                    type="text"
                    bind:value={newSshLabel}
                    required
                    placeholder="e.g. Work Laptop"
                    class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
                  />
                </div>
                <div>
                  <label class="block text-sm font-medium text-foreground mb-1.5">Public Key</label>
                  <textarea
                    bind:value={newSshPubkey}
                    required
                    rows="3"
                    placeholder="ssh-ed25519 AAAA... or ssh-rsa AAAA..."
                    class="w-full rounded-lg border border-border bg-background px-3 py-2 text-sm font-mono text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary resize-none"
                  ></textarea>
                </div>
                {#if sshAddError}
                  <p class="text-sm text-red-400">{sshAddError}</p>
                {/if}
                <button
                  type="submit"
                  disabled={sshAddLoading || !newSshPubkey.trim() || !newSshLabel.trim()}
                  class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 transition-colors disabled:opacity-50"
                >
                  {sshAddLoading ? 'Adding...' : 'Add Key'}
                </button>
              </form>

              {#if sshKeysLoading}
                <div class="space-y-2">
                  {#each [1, 2] as _}
                    <div class="h-14 bg-muted rounded-xl animate-pulse"></div>
                  {/each}
                </div>
              {:else if sshKeys.length > 0}
                <div class="border border-border rounded-xl overflow-hidden">
                  {#each sshKeys as key (key.id)}
                    <div
                      class="flex items-start justify-between gap-4 px-4 py-3 border-b border-border last:border-0"
                    >
                      <div class="min-w-0">
                        <p class="text-sm font-medium text-foreground">{key.label}</p>
                        <p class="text-xs font-mono text-muted-foreground mt-0.5 truncate">
                          {key.fingerprint}
                        </p>
                        <p class="text-xs text-muted-foreground mt-0.5">
                          {key.key_type} · Added {formatDateShort(key.added_at)}
                          {#if key.last_used_at}
                            · Last used {formatDateShort(key.last_used_at)}
                          {/if}
                        </p>
                      </div>
                      {#if confirmDeleteSshKeyId === key.id}
                        <div class="flex items-center gap-1.5 shrink-0">
                          <span class="text-xs text-destructive">Delete?</span>
                          <button class="text-xs px-2 py-1 rounded bg-destructive text-white hover:bg-destructive/90" on:click={() => deleteSshKey(key)}>Yes</button>
                          <button class="text-xs px-2 py-1 rounded bg-muted" on:click={() => confirmDeleteSshKeyId = null}>No</button>
                        </div>
                      {:else}
                        <button
                          on:click={() => confirmDeleteSshKeyId = key.id}
                          class="shrink-0 h-8 px-3 rounded-lg border border-red-500/20 text-xs font-medium text-red-400 hover:bg-red-500/10 transition-colors"
                        >
                          Delete
                        </button>
                      {/if}
                    </div>
                  {/each}
                </div>
              {:else}
                <div class="py-8 text-center">
                  <svg class="w-8 h-8 mx-auto text-muted-foreground mb-2" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
                      d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z" />
                  </svg>
                  <p class="text-sm font-medium text-foreground">No SSH keys added</p>
                  <p class="text-xs text-muted-foreground mt-0.5">Add a public key above to enable key-based authentication</p>
                </div>
              {/if}
            </div>
          </div>

          <!-- Maintenance Mode -->
          <div class="bg-card border border-border rounded-xl overflow-hidden">
            <div class="px-4 py-3 border-b border-border bg-muted/30">
              <h2 class="text-sm font-semibold text-foreground">Maintenance Mode</h2>
            </div>
            <div class="p-4 space-y-4">
              <div class="flex items-center justify-between transition-colors duration-150 hover:bg-muted/20 -mx-4 px-4 py-2 rounded">
                <div>
                  <p class="text-sm font-medium text-foreground">Enable maintenance mode</p>
                  <p class="text-xs text-muted-foreground mt-0.5">Shows maintenance page to all non-admin visitors</p>
                </div>
                <button
                  type="button"
                  on:click={() => (maintenance.enabled = !maintenance.enabled)}
                  class="relative inline-flex h-5 w-9 items-center rounded-full transition-colors duration-200 focus:outline-none {maintenance.enabled ? 'bg-amber-500' : 'bg-muted'}"
                  role="switch"
                  aria-checked={maintenance.enabled}
                >
                  <span class="inline-block h-3.5 w-3.5 rounded-full bg-white shadow transition-transform duration-200 {maintenance.enabled ? 'translate-x-4' : 'translate-x-0.5'}"></span>
                </button>
              </div>
              {#if maintenance.enabled}
                <div class="space-y-3 fade-up">
                  <div>
                    <label class="block text-sm font-medium text-foreground mb-1.5">Maintenance message</label>
                    <textarea
                      bind:value={maintenance.message}
                      rows="3"
                      class="w-full rounded-lg border border-border bg-background px-3 py-2 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary resize-none transition-colors duration-150 focus:bg-background"
                    ></textarea>
                  </div>
                  <div>
                    <label class="block text-sm font-medium text-foreground mb-1.5">Whitelist admin IPs <span class="text-xs text-muted-foreground font-normal">(comma separated)</span></label>
                    <input
                      type="text"
                      bind:value={maintenance.whitelist_ips}
                      placeholder="192.168.1.1, 10.0.0.1"
                      class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm font-mono text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary transition-colors duration-150 focus:bg-background"
                    />
                  </div>
                  <div class="flex gap-2">
                    <button
                      type="button"
                      on:click={saveMaintenance}
                      disabled={maintenanceLoading}
                      class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 transition-all duration-150 active:scale-95 disabled:opacity-50"
                    >
                      {maintenanceLoading ? 'Saving...' : 'Save Maintenance Settings'}
                    </button>
                    <button
                      type="button"
                      on:click={() => (showMaintenancePreview = !showMaintenancePreview)}
                      class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
                    >
                      Preview maintenance page
                    </button>
                  </div>
                  {#if showMaintenancePreview}
                    <div class="rounded-xl border border-amber-500/30 bg-amber-500/5 p-6 text-center fade-up">
                      <div class="text-amber-400 text-2xl mb-2">🔧</div>
                      <p class="text-sm font-semibold text-foreground mb-1">Maintenance in progress</p>
                      <p class="text-xs text-muted-foreground">{maintenance.message}</p>
                    </div>
                  {/if}
                </div>
              {:else}
                <button
                  type="button"
                  on:click={saveMaintenance}
                  disabled={maintenanceLoading}
                  class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 transition-all duration-150 active:scale-95 disabled:opacity-50"
                >
                  {maintenanceLoading ? 'Saving...' : 'Save Maintenance Settings'}
                </button>
              {/if}
            </div>
          </div>

          <!-- Disable password login -->
          <div class="bg-amber-500/5 border border-amber-500/20 rounded-xl p-4">
            <h3 class="text-sm font-semibold text-amber-400 mb-1">Disable Password Login</h3>
            <p class="text-xs text-amber-400/70 mb-3">
              Require SSH key authentication only. Ensure you have a working key above before proceeding.
              <strong>This cannot be undone from the panel.</strong>
            </p>
            {#if !showDisablePasswordConfirm}
              <button
                type="button"
                on:click={() => (showDisablePasswordConfirm = true)}
                class="h-9 px-4 rounded-lg bg-amber-500/20 text-amber-400 border border-amber-500/30 text-sm font-medium hover:bg-amber-500/30 inline-flex items-center gap-2 transition-colors"
              >
                Disable Password Login
              </button>
            {:else}
              <div class="space-y-2">
                <p class="text-sm font-medium text-amber-400">
                  Are you sure? You will be locked out if your SSH key fails.
                </p>
                <div class="flex gap-2">
                  <button
                    type="button"
                    on:click={() => (showDisablePasswordConfirm = false)}
                    class="h-9 px-4 rounded-lg border border-amber-500/30 text-sm font-medium text-amber-400 hover:bg-amber-500/10 transition-colors"
                  >
                    Cancel
                  </button>
                  <button
                    type="button"
                    on:click={disablePasswordLogin}
                    disabled={disablePwdLoading}
                    class="h-9 px-4 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-sm font-medium hover:bg-red-500/20 inline-flex items-center gap-2 transition-colors disabled:opacity-50"
                  >
                    {disablePwdLoading ? 'Applying...' : 'Yes, Disable Password Login'}
                  </button>
                </div>
              </div>
            {/if}
          </div>
        </div>
      {/if}
    </div>
  </div>
</div>

<!-- ── New API Key Modal ──────────────────────────────────────────────────── -->
{#if showNewKeyModal}
  <!-- svelte-ignore a11y-click-events-have-key-events -->
  <!-- svelte-ignore a11y-no-static-element-interactions -->
  <div
    class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
    on:click|self={closeNewKeyModal}
    role="dialog"
    aria-modal="true"
  >
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-lg shadow-2xl">
      <div class="flex items-center justify-between mb-5">
        <h2 class="text-base font-semibold text-foreground">
          {createdKey ? 'API Key Created' : 'Generate API Key'}
        </h2>
        <button
          on:click={closeNewKeyModal}
          class="w-8 h-8 rounded-lg flex items-center justify-center text-muted-foreground hover:bg-muted hover:text-foreground transition-colors"
        >
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      {#if createdKey}
        <div class="space-y-4">
          <div
            class="bg-emerald-500/10 border border-emerald-500/20 rounded-lg px-3 py-2.5 text-sm text-emerald-400"
          >
            Copy this key now — it will not be shown again.
          </div>
          <div class="flex gap-2">
            <code
              class="flex-1 rounded-lg border border-border bg-muted px-3 py-2 text-xs font-mono text-foreground break-all"
            >
              {createdKey}
            </code>
            <button
              on:click={copyCreatedKey}
              class="h-auto px-3 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground transition-colors shrink-0 whitespace-nowrap self-start py-2"
            >
              {createdKeyCopied ? 'Copied!' : 'Copy'}
            </button>
          </div>
          <button
            on:click={closeNewKeyModal}
            class="w-full h-9 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center justify-center gap-2 transition-colors"
          >
            Done
          </button>
        </div>
      {:else}
        <form on:submit={createApiKey} class="space-y-4">
          <div>
            <label class="block text-sm font-medium text-foreground mb-1.5">Key Name</label>
            <input
              type="text"
              bind:value={newKeyForm.name}
              required
              placeholder="e.g. Deploy Bot, Monitoring"
              class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
            />
          </div>

          <div>
            <p class="text-sm font-medium text-foreground mb-2">Scopes</p>
            <div class="flex flex-wrap gap-1.5">
              {#each apiScopes as scope}
                <label class="cursor-pointer">
                  <input
                    type="checkbox"
                    bind:group={newKeyForm.scopes}
                    value={scope}
                    class="sr-only"
                  />
                  <span
                    class="inline-flex items-center h-7 px-2.5 rounded-lg border text-xs font-medium transition-colors
                               {newKeyForm.scopes.includes(scope)
                      ? 'border-primary bg-primary/10 text-primary'
                      : 'border-border text-muted-foreground hover:bg-muted/50'}"
                  >
                    {scope}
                  </span>
                </label>
              {/each}
            </div>
          </div>

          <div>
            <label class="block text-sm font-medium text-foreground mb-1.5">
              Expires
              <span class="text-xs text-muted-foreground font-normal ml-1">(leave blank for no expiry)</span>
            </label>
            <input
              type="date"
              bind:value={newKeyForm.expires_at}
              class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
            />
          </div>

          {#if newKeyError}
            <div
              role="alert"
              class="text-sm text-red-400 bg-red-500/10 border border-red-500/20 rounded-lg px-3 py-2.5"
            >
              {newKeyError}
            </div>
          {/if}

          <div class="flex gap-2">
            <button
              type="button"
              on:click={closeNewKeyModal}
              class="flex-1 h-9 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center justify-center gap-2 transition-colors"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={newKeyLoading || !newKeyForm.name.trim()}
              class="flex-1 h-9 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center justify-center gap-2 transition-colors disabled:opacity-50"
            >
              {newKeyLoading ? 'Generating...' : 'Generate Key'}
            </button>
          </div>
        </form>
      {/if}
    </div>
  </div>
{/if}

<!-- ── Toast ─────────────────────────────────────────────────────────────── -->
{#if toastMessage}
  <div
    class="fixed bottom-4 right-4 z-50 bg-card border border-border rounded-xl px-4 py-3 text-sm shadow-lg
              flex items-center gap-2 fade-up"
    role="status"
    aria-live="polite"
  >
    {#if toastType === 'success'}
      <svg class="w-4 h-4 text-emerald-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7" />
      </svg>
    {:else}
      <svg class="w-4 h-4 text-red-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
      </svg>
    {/if}
    <span class="text-foreground">{toastMessage}</span>
  </div>
{/if}

<style>
  @keyframes fadeUp {
    from { opacity: 0; transform: translateY(6px); }
    to   { opacity: 1; transform: none; }
  }
  .fade-up       { animation: fadeUp 0.25s ease-out both; }
  .settings-section { animation: fadeUp 0.2s ease-out both; }
</style>
