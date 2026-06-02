/**
 * OrbitCP API client — typed wrappers for every REST endpoint.
 *
 * Usage:
 *   import { api } from '$api/client';
 *   const sites = await api.sites.list({ server_id: '...' });
 */

// ── Types ─────────────────────────────────────────────────────────────────────

export interface Site {
  id:             string;
  domain:         string;
  status:         'provisioning' | 'active' | 'suspended' | 'deleting' | 'error';
  php_version:    string;
  web_server:     'ols' | 'nginx' | 'apache' | 'lse';
  unix_user:      string;
  disk_mb:        number;
  server_id:      string | null;
  server_label:   string | null;
  ssl_status:     'active' | 'pending' | 'expired' | 'none';
  ssl_expires_at: string | null;
  created_at:     string;
  suspended_at:   string | null;
}

export interface CreateSitePayload {
  domain:             string;
  php_version:        string;
  web_server:         string;
  server_id?:         string;
  plan?:              string;
  disk_quota_mb?:     number;
  bandwidth_quota_gb?: number;
}

export interface Server {
  id:             string;
  label:          string;
  hostname:       string;
  ip:             string;
  status:         'active' | 'offline' | 'error';
  os_version:     string | null;
  cpu_count:      number | null;
  ram_total_mb:   number | null;
  disk_total_gb:  number | null;
  last_seen_at:   string | null;
  created_at:     string;
  // Live metrics (populated from WebSocket stream)
  cpu_pct?:       number;
  ram_pct?:       number;
  disk_pct?:      number;
  load_1?:        number;
  site_count?:    number;
  uptime_seconds?: number;
}

export interface Database {
  id:         string;
  site_id:    string | null;
  db_type:    'mysql' | 'mariadb' | 'postgresql' | 'ferretdb' | 'surrealdb';
  db_name:    string;
  db_user:    string;
  db_host:    string;
  db_port:    number;
  size_mb:    number;
  created_at: string;
}

export interface CreateDatabasePayload {
  db_type:    string;
  db_name:    string;
  site_id?:   string;
  server_id?: string;
}

export interface EmailDomain {
  id:           string;
  domain:       string;
  site_id:      string | null;
  dkim_selector: string;
  spf_record:   string | null;
  dmarc_record: string | null;
  status:       string;
  created_at:   string;
}

export interface EmailAccount {
  id:          string;
  site_id:     string | null;
  domain_id:   string | null;
  address:     string | null;
  local_part:  string | null;
  quota_mb:    number;
  used_mb:     number;
  status:      string;
  created_at:  string;
}

export interface DnsZone {
  id:         string;
  zone:       string;
  site_id:    string | null;
  serial:     number;
  created_at: string;
  records?:   DnsRecord[];
}

export interface DnsRecord {
  id:       string;
  zone_id:  string;
  name:     string;
  type:     'A' | 'AAAA' | 'MX' | 'TXT' | 'CNAME' | 'NS' | 'SRV' | 'CAA';
  content:  string;
  ttl:      number;
  priority: number | null;
  disabled: boolean;
}

export interface Backup {
  id:           string;
  site_id:      string;
  type:         string;  // 'full' | 'files' | 'database'
  status:       'pending' | 'running' | 'completed' | 'failed' | 'expired';
  size_bytes:   number | null;
  destination:  string | null;
  started_at:   string | null;
  completed_at: string | null;
  created_at:   string;
}

export interface BackupSettings {
  site_id:         string;
  daily_time:      string;
  retention_days:  number;
  retention_weeks: number;
  destination:     string;
  s3_bucket:       string | null;
  s3_region:       string | null;
  sftp_host:       string | null;
  sftp_user:       string | null;
  sftp_path:       string | null;
}

export interface EmailForwarder {
  id:          string;
  site_id:     string | null;
  source:      string;
  destination: string;
  created_at:  string;
}

export interface SmtpLimitsResponse {
  id:             string;
  site_id:        string | null;
  domain:         string;
  hourly_limit:   number;
  daily_limit:    number;
  rate_action:    'queue' | 'reject' | 'bounce';
  relay_enabled:  boolean;
  require_spf:    boolean;
  require_dkim:   boolean;
  enabled:        boolean;
  sent_this_hour: number;
  sent_today:     number;
}

export interface SmtpLimitsRequest {
  hourly_limit?:  number;
  daily_limit?:   number;
  rate_action?:   'queue' | 'reject' | 'bounce';
  relay_enabled?: boolean;
  require_spf?:   boolean;
  require_dkim?:  boolean;
  enabled?:       boolean;
}

export interface AppInstall {
  id:                string;
  site_id:           string;
  manifest_slug:     string;
  installed_version: string;
  latest_version:    string | null;
  install_path:      string;
  status:            'active' | 'updating' | 'failed' | 'removed';
  installed_at:      string;
}

export interface User {
  id:               string;
  email:            string;
  role:             'admin' | 'reseller' | 'user';
  display_name:     string | null;
  full_name:        string | null;
  status:           'active' | 'suspended';
  totp_enabled:     boolean;
  wizard_complete:  boolean;
  last_login_at:    string | null;
  created_at:       string;
}

export interface ApiKey {
  id:                 string;
  name:               string;
  key_prefix:         string;
  scopes:             string[];
  rate_limit_per_min: number;
  last_used_at:       string | null;
  expires_at:         string | null;
  is_active:          boolean;
  created_at:         string;
}

export interface Notification {
  id:         string;
  type:       string;
  title:      string;
  body:       string;
  action_url: string | null;
  read_at:    string | null;
  created_at: string;
  site_id:    string | null;
  server_id:  string | null;
}

export interface SiteStats {
  site_id:          string;
  domain:           string;
  memory_bytes:     number;
  cpu_usage_pct:    number;
  disk_bytes:       number;
  db_bytes:         number;
  request_count_1h: number;
}

export interface StagingSite {
  id:          string;
  site_id:     string;
  domain:      string;
  status:      'active' | 'provisioning' | 'error';
  created_at:  string;
}

export interface MigrationJob {
  id:          string;
  source_url:  string;
  target_site_id: string;
  status:      'pending' | 'running' | 'completed' | 'failed';
  progress_pct: number;
  error_message: string | null;
  created_at:  string;
  completed_at: string | null;
}

export interface PerformanceReport {
  site_id:        string;
  domain:         string;
  ttfb_ms:        number;
  lcp_ms:         number;
  cls:            number;
  fcp_ms:         number;
  score:          number;
  tested_at:      string;
}

export interface EmailIpPool {
  id:          string;
  ip:          string;
  rdns:        string | null;
  is_primary:  boolean;
  active:      boolean;
  created_at:  string;
}

export interface LoginResponse {
  interim_token:   string;
  totp_required:   boolean;
  // Populated only when totp_required = false
  access_token?:   string;
  expires_in?:     number;
  user_id?:        string;
  email?:          string;
  role?:           string;
  wizard_complete?: boolean;
}

export interface AuthResponse {
  access_token:    string;
  expires_in:      number;
  user_id:         string;
  email:           string;
  role:            string;
  wizard_complete: boolean;
}

export interface ApiError {
  error:   string;
  message: string;
  [key: string]: unknown;
}

export interface CronJob {
  id:             string;
  site_id:        string;
  label:          string;
  schedule:       string;
  command:        string;
  enabled:        boolean;
  last_run_at:    string | null;
  last_exit_code: number | null;
  created_at:     string;
}

// ── Client implementation ─────────────────────────────────────────────────────

class OrbitApiClient {
  private baseUrl: string;
  private token: string | null = null;

  // ── In-memory GET cache ──────────────────────────────────────────────────────
  private _reqCache = new Map<string, { data: unknown; expiry: number }>();
  private readonly CACHE_TTL = 30_000; // 30 seconds

  private getCached<T>(key: string): T | null {
    const entry = this._reqCache.get(key);
    if (entry && Date.now() < entry.expiry) return entry.data as T;
    this._reqCache.delete(key);
    return null;
  }

  private setCached<T>(key: string, data: T): void {
    this._reqCache.set(key, { data, expiry: Date.now() + this.CACHE_TTL });
  }

  public invalidateCache(pattern?: string): void {
    if (!pattern) { this._reqCache.clear(); return; }
    for (const key of this._reqCache.keys()) {
      if (key.includes(pattern)) this._reqCache.delete(key);
    }
  }

  /** Warm the cache by pre-fetching a list of GET paths in parallel. */
  async prefetch(paths: string[]): Promise<void> {
    await Promise.allSettled(paths.map(p => this.request('GET', p)));
  }

  constructor(baseUrl = '') {
    this.baseUrl = baseUrl;
  }

  setToken(token: string | null) {
    this.token = token;
    // Clear cache on token change to avoid stale data from previous session
    this.invalidateCache();
  }

  private get headers(): Record<string, string> {
    const h: Record<string, string> = {
      'Content-Type': 'application/json',
      'Accept':       'application/json',
    };
    if (this.token) h['Authorization'] = `Bearer ${this.token}`;
    return h;
  }

  private async request<T>(
    method:  string,
    path:    string,
    body?:   unknown,
    params?: Record<string, string | number | boolean | undefined>,
  ): Promise<T> {
    let url = `${this.baseUrl}${path}`;

    if (params) {
      const qs = new URLSearchParams();
      for (const [k, v] of Object.entries(params)) {
        if (v !== undefined && v !== null) qs.set(k, String(v));
      }
      const str = qs.toString();
      if (str) url += '?' + str;
    }

    // Cache key includes query string so different param combos are cached separately
    const cacheKey = method === 'GET' ? url.replace(this.baseUrl, '') : null;
    if (cacheKey) {
      const cached = this.getCached<T>(cacheKey);
      if (cached !== null) return cached;
    }

    const res = await fetch(url, {
      method,
      headers: this.headers,
      body:    body ? JSON.stringify(body) : undefined,
    });

    if (!res.ok) {
      // Auto-redirect to login on 401 Unauthorized
      if (res.status === 401 && typeof window !== 'undefined' && !path.startsWith('/api/v1/auth/')) {
        // Clear token so the auth guard triggers
        this.token = null;
        if (typeof localStorage !== 'undefined') {
          localStorage.removeItem('orbit_access_token');
          localStorage.removeItem('orbit_token_expires');
          localStorage.removeItem('orbit_user');
        }
        window.location.href = '/login?reason=session_expired';
        // Throw so the calling code doesn't continue
        throw Object.assign(new Error('Session expired'), { error: 'unauthorized', message: 'Session expired' });
      }
      let errBody: ApiError = { error: 'unknown', message: `HTTP ${res.status}` };
      try { errBody = await res.json(); } catch { /* ignore */ }
      throw Object.assign(new Error(errBody.message), errBody);
    }

    let data: T;
    if (res.status === 204 || res.status === 202) {
      data = undefined as T;
    } else {
      const ct = res.headers.get('content-type') ?? '';
      data = (!ct.includes('application/json') && !ct.includes('text/json'))
        ? undefined as T
        : await res.json() as T;
    }

    // Cache successful GET responses
    if (cacheKey) this.setCached<T>(cacheKey, data);

    // Invalidate related resource caches on mutations (POST/PUT/DELETE)
    if (method !== 'GET') {
      const segments = path.split('/');
      // e.g. /api/v1/sites → 'sites', /api/v1/databases/123 → 'databases'
      const resource = segments[3];
      if (resource) this.invalidateCache(resource);
    }

    return data;
  }

  // ── Auth ────────────────────────────────────────────────────────────────────

  readonly auth = {
    login: (email: string, password: string) =>
      this.request<LoginResponse>('POST', '/api/v1/auth/login', { email, password }),

    verifyTotp: (token: string, code: string) =>
      this.request<AuthResponse>('POST', '/api/v1/auth/totp', { token, code }),

    refresh: (refreshToken: string) =>
      this.request<AuthResponse>('POST', '/api/v1/auth/refresh', { refresh_token: refreshToken }),

    logout: () =>
      this.request<void>('POST', '/api/v1/auth/logout'),

    setupTotp: (totp_code: string) =>
      this.request<{ secret: string; provisioning_uri: string; backup_codes: string[] }>(
        'POST', '/api/v1/auth/totp/setup', { totp_code }),

    verifyBackupCode: (token: string, code: string) =>
      this.request<AuthResponse>('POST', '/api/v1/auth/totp/backup', { token, code }),
  };

  // ── Sites ───────────────────────────────────────────────────────────────────

  readonly sites = {
    list: (params?: { server_id?: string; status?: string; search?: string; limit?: number; offset?: number }) =>
      this.request<Site[]>('GET', '/api/v1/sites', undefined, params as Record<string, string | number | undefined>),

    get: (id: string) =>
      this.request<Site>('GET', `/api/v1/sites/${id}`),

    create: (payload: CreateSitePayload) =>
      this.request<Site>('POST', '/api/v1/sites', payload),

    update: (id: string, payload: Partial<CreateSitePayload>) =>
      this.request<Site>('PUT', `/api/v1/sites/${id}`, payload),

    delete: (id: string) =>
      this.request<void>('DELETE', `/api/v1/sites/${id}`),

    suspend: (id: string) =>
      this.request<void>('POST', `/api/v1/sites/${id}/suspend`),

    unsuspend: (id: string) =>
      this.request<void>('POST', `/api/v1/sites/${id}/unsuspend`),

    issueSSL: (id: string) =>
      this.request<{ status: string; message: string }>('POST', `/api/v1/sites/${id}/ssl`),

    previewUrl: (siteId: string) =>
      this.request<{ preview_url: string; expires_at: string }>('GET', `/api/v1/sites/${siteId}/preview-url`),

    stats: (id: string) =>
      this.request<SiteStats>('GET', `/api/v1/sites/${id}/stats`),

    changePhpVersion: (id: string, version: string) =>
      this.request<void>('PUT', `/api/v1/sites/${id}/php-version`, { php_version: version }),

    logs: (siteId: string, type: 'access' | 'error' | 'php', lines: number) =>
      this.request<{ content: string; path: string }>(
        'GET', `/api/v1/sites/${siteId}/logs`,
        undefined,
        { type, lines } as Record<string, string | number>
      ),
  };

  // ── Servers ─────────────────────────────────────────────────────────────────

  readonly servers = {
    list: () => this.request<Server[]>('GET', '/api/v1/servers'),
    get:  (id: string) => this.request<Server>('GET', `/api/v1/servers/${id}`),

    create: (payload: { label: string; hostname: string; ip: string; ssh_port?: number }) =>
      this.request<Server>('POST', '/api/v1/servers', payload),

    delete: (id: string) =>
      this.request<void>('DELETE', `/api/v1/servers/${id}`),

    testConnection: (id: string) =>
      this.request<{ success: boolean; message: string }>('POST', `/api/v1/servers/${id}/test`),

    metrics: (id: string) =>
      this.request<{
        cpu_pct: number; ram_pct: number; disk_pct: number;
        load_1: number; load_5: number; load_15: number;
        network_rx_mb: number; network_tx_mb: number;
        uptime_seconds: number; site_count: number;
        recorded_at: string;
      }>('GET', `/api/v1/servers/${id}/metrics`),

    enroll: (id: string, token: string) =>
      this.request<{ success: boolean; message: string }>(
        'POST', `/api/v1/servers/${id}/enroll`, { token }),
  };

  // ── Databases ───────────────────────────────────────────────────────────────

  readonly databases = {
    list: (params?: { site_id?: string }) =>
      this.request<Database[]>('GET', '/api/v1/databases', undefined, params as Record<string, string | undefined>),

    get: (id: string) =>
      this.request<Database>('GET', `/api/v1/databases/${id}`),

    create: (payload: CreateDatabasePayload) =>
      this.request<Database>('POST', '/api/v1/databases', payload),

    delete: (id: string) =>
      this.request<void>('DELETE', `/api/v1/databases/${id}`),

    changePassword: (id: string, password: string) =>
      this.request<void>('POST', `/api/v1/databases/${id}/password`, { password }),

    stats: (id: string) =>
      this.request<{ id: string; db_name: string; db_type: string; size_bytes: number; table_count: number; connection_count: number }>(
        'GET', `/api/v1/databases/${id}/stats`),

    getPhpMyAdminToken: (id: string) =>
      this.request<{ token: string; url: string }>('POST', `/api/v1/databases/${id}/phpmyadmin-token`),
  };

  // ── Email ───────────────────────────────────────────────────────────────────

  readonly email = {
    domains: {
      list: (params?: { site_id?: string }) =>
        this.request<EmailDomain[]>('GET', '/api/v1/email/domains', undefined, params as Record<string, string | undefined>),
      create: (payload: { domain: string; site_id?: string }) =>
        this.request<EmailDomain>('POST', '/api/v1/email/domains', payload),
      delete: (id: string) =>
        this.request<void>('DELETE', `/api/v1/email/domains/${id}`),
    },

    accounts: {
      list: (params?: { domain_id?: string; site_id?: string }) =>
        this.request<EmailAccount[]>('GET', '/api/v1/email/accounts', undefined, params as Record<string, string | undefined>),
      create: (payload: { domain_id: string; local_part: string; password: string; quota_mb?: number }) =>
        this.request<EmailAccount>('POST', '/api/v1/email/accounts', payload),
      delete: (id: string) =>
        this.request<void>('DELETE', `/api/v1/email/accounts/${id}`),
      changePassword: (id: string, password: string) =>
        this.request<void>('POST', `/api/v1/email/accounts/${id}/password`, { password }),
    },

    forwarders: {
      list: (params?: { site_id?: string }) =>
        this.request<EmailForwarder[]>('GET', '/api/v1/email/forwarders', undefined, params as Record<string, string | undefined>),
      create: (payload: { site_id: string; source: string; destination: string }) =>
        this.request<EmailForwarder>('POST', '/api/v1/email/forwarders', payload),
      delete: (id: string) =>
        this.request<void>('DELETE', `/api/v1/email/forwarders/${id}`),
    },

    webmailSso: (accountId: string) =>
      this.request<{ sso_url: string }>('POST', `/api/v1/email/mailboxes/${accountId}/webmail-sso`),

    smtpLimits: {
      list: () =>
        this.request<SmtpLimitsResponse[]>('GET', '/api/v1/email/smtp-limits'),
      get: (domain: string) =>
        this.request<SmtpLimitsResponse>('GET', `/api/v1/email/smtp-limits/${encodeURIComponent(domain)}`),
      upsert: (domain: string, payload: SmtpLimitsRequest) =>
        this.request<SmtpLimitsResponse>('PUT', `/api/v1/email/smtp-limits/${encodeURIComponent(domain)}`, payload),
      resetCounters: (domain: string) =>
        this.request<void>('POST', `/api/v1/email/smtp-limits/${encodeURIComponent(domain)}/reset`),
    },
  };

  // ── DNS ─────────────────────────────────────────────────────────────────────

  readonly dns = {
    zones: {
      list: () => this.request<DnsZone[]>('GET', '/api/v1/dns/zones'),
      get:  (id: string) => this.request<DnsZone>('GET', `/api/v1/dns/zones/${id}`),
      create: (payload: { domain: string; site_id?: string }) =>
        this.request<DnsZone>('POST', '/api/v1/dns/zones', payload),
      delete: (id: string) =>
        this.request<void>('DELETE', `/api/v1/dns/zones/${id}`),
    },

    records: {
      list: (zoneId: string) =>
        this.request<DnsRecord[]>('GET', `/api/v1/dns/zones/${zoneId}/records`),
      create: (zoneId: string, payload: Omit<DnsRecord, 'id' | 'zone_id'>) =>
        this.request<DnsRecord>('POST', `/api/v1/dns/zones/${zoneId}/records`, payload),
      update: (zoneId: string, recordId: string, payload: Partial<DnsRecord>) =>
        this.request<DnsRecord>('PUT', `/api/v1/dns/zones/${zoneId}/records/${recordId}`, payload),
      delete: (zoneId: string, recordId: string) =>
        this.request<void>('DELETE', `/api/v1/dns/zones/${zoneId}/records/${recordId}`),
    },
  };

  // ── Backups ─────────────────────────────────────────────────────────────────

  readonly backups = {
    // Admin list (all sites)
    listAll: (params?: { site_id?: string }) =>
      this.request<Backup[]>('GET', '/api/v1/backups', undefined, params as Record<string, string | undefined>),

    // Per-site list (correct endpoint)
    list: (siteId: string) =>
      this.request<Backup[]>('GET', `/api/v1/backups/${siteId}`),

    // Trigger backup for site
    create: (siteId: string, type: string = 'full') =>
      this.request<Backup>('POST', `/api/v1/backups/${siteId}`, { type }),

    getSettings: (siteId: string) =>
      this.request<BackupSettings>('GET', `/api/v1/backups/settings/${siteId}`),

    updateSettings: (siteId: string, payload: Record<string, unknown>) =>
      this.request<BackupSettings>('PUT', `/api/v1/backups/settings/${siteId}`, payload),

    restore: (backupId: string) =>
      this.request<{ job_id: string }>('POST', `/api/v1/backup-jobs/${backupId}/restore`, {}),

    delete: (backupId: string) =>
      this.request<void>('DELETE', `/api/v1/backup-jobs/${backupId}`),

    download: (backupId: string) =>
      `/api/v1/backup-jobs/${backupId}/download`,
  };

  // ── App Installer ───────────────────────────────────────────────────────────

  readonly apps = {
    list: () =>
      this.request<Array<{ id: string; name: string; description: string; version: string; category: string; icon_url: string | null; requires_pro: boolean }>>(
        'GET', '/api/v1/apps'),

    listInstalled: (siteId?: string) =>
      this.request<Array<{ id: string; site_id: string; app_id: string; version: string; status: string; installed_at: string | null; admin_url: string | null }>>(
        'GET', '/api/v1/apps/installed', undefined, siteId ? { site_id: siteId } as Record<string, string> : undefined),

    install: (payload: { app_id: string; site_id: string; params?: Record<string, string> }) =>
      this.request<{ job_id: string }>('POST', '/api/v1/apps/install', payload),

    uninstall: (installId: string) =>
      this.request<void>('DELETE', `/api/v1/apps/installed/${installId}`),

    update: (installId: string) =>
      this.request<{ job_id: string }>('POST', `/api/v1/apps/installed/${installId}/update`),
  };

  // ── Users ───────────────────────────────────────────────────────────────────

  readonly users = {
    list: () => this.request<User[]>('GET', '/api/v1/users'),
    get:  (id: string) => this.request<User>('GET', `/api/v1/users/${id}`),
    me:   () => this.request<User>('GET', '/api/v1/users/me'),

    create: (payload: { email: string; role: string; password?: string; full_name?: string }) =>
      this.request<User>('POST', '/api/v1/users', payload),

    update: (id: string, payload: { email?: string; full_name?: string; role?: string }) =>
      this.request<User>('PUT', `/api/v1/users/${id}`, payload),

    suspend: (id: string) =>
      this.request<void>('POST', `/api/v1/users/${id}/suspend`),

    unsuspend: (id: string) =>
      this.request<void>('POST', `/api/v1/users/${id}/unsuspend`),

    impersonate: (id: string) =>
      this.request<{ token: string }>('POST', `/api/v1/users/${id}/impersonate`),

    delete: (id: string) =>
      this.request<void>('DELETE', `/api/v1/users/${id}`),

    changePassword: (id: string, payload: { current_password?: string; new_password: string }) =>
      this.request<void>('PUT', `/api/v1/users/${id}/password`, payload),

    auditLog: (params?: {
      action?: string; target?: string; user_id?: string;
      date_from?: string; date_to?: string; limit?: number; offset?: number;
    }) => {
      const q = new URLSearchParams();
      if (params?.action)    q.set('action',    params.action);
      if (params?.target)    q.set('target',    params.target);
      if (params?.user_id)   q.set('user_id',   params.user_id);
      if (params?.date_from) q.set('date_from', params.date_from);
      if (params?.date_to)   q.set('date_to',   params.date_to);
      if (params?.limit)     q.set('limit',     String(params.limit));
      if (params?.offset)    q.set('offset',    String(params.offset));
      return this.request<Array<{
        id: string; user_id: string | null; user_email: string | null;
        action: string; resource_type: string | null; resource_id: string | null;
        ip_address: string | null; created_at: string;
      }>>('GET', `/api/v1/audit-log?${q}`);
    },

    activity: (id: string) =>
      this.request<Array<{
        id: number; action: string; resource_type: string | null;
        resource_id: string | null; ip_address: string | null;
        created_at: string; metadata: Record<string, unknown> | null;
      }>>('GET', `/api/v1/users/${id}/activity`),
  };

  // ── API Keys ────────────────────────────────────────────────────────────────
  // NOT YET IMPLEMENTED IN BACKEND — these methods will return 404 until the
  // backend exposes /api/v1/settings/api-keys. Tracked on roadmap.

  readonly apiKeys = {
    list: () => this.request<ApiKey[]>('GET', '/api/v1/settings/api-keys'),

    create: (payload: { name: string; scopes: string[]; rate_limit_per_min?: number; expires_at?: string }) =>
      this.request<ApiKey & { full_key: string }>('POST', '/api/v1/settings/api-keys', payload),

    revoke: (id: string) =>
      this.request<void>('DELETE', `/api/v1/settings/api-keys/${id}`),
  };

  // ── Notifications ───────────────────────────────────────────────────────────

  readonly notifications = {
    list: (params?: { unread_only?: boolean }) =>
      this.request<Notification[]>('GET', '/api/v1/notifications', undefined, params as Record<string, boolean | undefined>),

    markRead: (id: string) =>
      this.request<void>('PUT', `/api/v1/notifications/${id}/read`),

    markAllRead: () =>
      this.request<void>('POST', '/api/v1/notifications/mark-all-read'),
  };

  // ── Settings ────────────────────────────────────────────────────────────────

  readonly settings = {
    get: () =>
      this.request<Record<string, unknown>>('GET', '/api/v1/settings'),

    update: (payload: Record<string, unknown>) =>
      this.request<Record<string, unknown>>('PUT', '/api/v1/settings', payload),

    wizardComplete: () =>
      this.request<void>('POST', '/api/v1/settings/wizard-complete'),

    getEmailLimits: () =>
      this.request<{ per_hour: number; per_day: number; per_month: number; enabled: boolean }>(
        'GET', '/api/v1/settings/email-limits'),

    updateEmailLimits: (limits: { per_hour?: number; per_day?: number; per_month?: number; enabled?: boolean }) =>
      this.request<{ per_hour: number; per_day: number; per_month: number; enabled: boolean }>(
        'PUT', '/api/v1/settings/email-limits', limits),
  };

  // ── Usage Reports ───────────────────────────────────────────────────────────
  // NOT YET IMPLEMENTED IN BACKEND — these methods will return 404 until the
  // backend exposes /api/v1/reports/. Tracked on roadmap.

  readonly reports = {
    usage: (params: { site_id?: string; from: string; to: string; granularity?: 'hourly' | 'daily' | 'monthly' }) =>
      this.request<unknown[]>('GET', '/api/v1/reports/usage', undefined, params as Record<string, string | undefined>),

    exportCsv: (params: Record<string, string>) =>
      `/api/v1/reports/usage/export?format=csv&` + new URLSearchParams(params).toString(),
  };

  // ── Firewall ────────────────────────────────────────────────────────────────

  readonly firewall = {
    blockedIps: async () => {
      const data = await this.request<{ ips: string[] }>('GET', '/api/v1/firewall/blocked-ips');
      return (data.ips ?? []).map(ip => ({ ip, reason: '', jail: 'sshd', since: '', attempts: 0 }));
    },

    blockIp: (ip: string, reason?: string) =>
      this.request<void>('POST', '/api/v1/firewall/blocked-ips', { ip, reason }),

    unblockIp: (ip: string) =>
      this.request<void>('DELETE', `/api/v1/firewall/blocked-ips/${encodeURIComponent(ip)}`),

    whitelist: async () => {
      const data = await this.request<{ ips: string[] }>('GET', '/api/v1/firewall/whitelist');
      return (data.ips ?? []).map(ip => ({ ip, comment: undefined as string | undefined, added_at: '' }));
    },

    addToWhitelist: (ip: string, comment?: string) =>
      this.request<void>('POST', '/api/v1/firewall/whitelist', { ip, comment }),

    removeFromWhitelist: (ip: string) =>
      this.request<void>('DELETE', `/api/v1/firewall/whitelist/${encodeURIComponent(ip)}`),
  };

  // ── fail2ban ────────────────────────────────────────────────────────────────

  readonly fail2ban = {
    jails: async () => {
      const data = await this.request<{ jails: string[] }>('GET', '/api/v1/fail2ban/jails');
      return (data.jails ?? []).map(name => ({
        name,
        enabled: true,
        banned_count: 0,
        maxretry: 5,
        findtime: 600,
        bantime: 3600,
        banned_ips: [] as string[],
      }));
    },

    unban: (jail: string, ip: string) =>
      this.request<void>('POST', `/api/v1/fail2ban/jails/${jail}/unban`, { ip }),

    updateConfig: (jail: string, config: { maxretry?: number; findtime?: number; bantime?: number }) =>
      this.request<void>('PUT', `/api/v1/fail2ban/jails/${jail}/maxretry`, config),
  };

  // ── WAF ─────────────────────────────────────────────────────────────────────

  readonly waf = {
    status: async (siteId?: string) => {
      const data = await this.request<{ enabled_globally: boolean; enabled_sites: string[] }>(
        'GET', '/api/v1/waf/status', undefined, siteId ? { site_id: siteId } : undefined);
      // Preserve enabled_globally (the security score reads it); enabled_sites = per-vhost matches.
      return {
        enabled_globally: data.enabled_globally === true,
        enabled_sites: (data.enabled_sites ?? []).map(id => String(id)),
      };
    },

    enable: (siteId: string) =>
      this.request<void>('POST', `/api/v1/waf/enable/${siteId}`),

    disable: (siteId: string) =>
      this.request<void>('POST', `/api/v1/waf/disable/${siteId}`),

    logs: async (params?: { site_id?: string; limit?: number }) => {
      const data = await this.request<{ entries: Array<{
        timestamp: string; client_ip: string; method: string; uri: string;
        rule_id: string; message: string; severity: string; raw_line: string;
      }> }>('GET', '/api/v1/waf/logs', undefined, params as Record<string, string | number | undefined>);
      return (data.entries ?? []).map(e => ({
        id:         e.raw_line,
        site_id:    '',
        domain:     '',
        ip:         e.client_ip,
        rule_id:    e.rule_id,
        rule_msg:   e.message,
        uri:        e.uri,
        blocked_at: e.timestamp,
      }));
    },

    addWhitelistRule: (siteId: string, rule: string) =>
      this.request<void>('POST', '/api/v1/waf/rules', { site_id: siteId, rule }),
  };

  // ── DB Manager ──────────────────────────────────────────────────────────────

  readonly dbmanager = {
    schemas: (dbId: string) =>
      this.request<string[]>('GET', `/api/v1/databases/${dbId}/schemas`),

    tables: (dbId: string, schema?: string) =>
      this.request<Array<{ name: string; row_count: number; size_bytes: number }>>(
        'GET', `/api/v1/databases/${dbId}/tables`, undefined, schema ? { schema } : undefined),

    rows: (dbId: string, table: string, limit = 50, offset = 0, schema?: string) =>
      this.request<{ columns: string[]; rows: unknown[][]; total: number }>(
        'GET', `/api/v1/databases/${dbId}/tables/${encodeURIComponent(table)}/rows`,
        undefined, { limit, offset, ...(schema ? { schema } : {}) } as Record<string, string | number | undefined>),

    query: (dbId: string, sql: string) =>
      this.request<{ columns: string[]; rows: unknown[][]; affected_rows: number; duration_ms: number }>(
        'POST', `/api/v1/databases/${dbId}/query`, { sql }),

    exportCsv: (dbId: string, table: string) =>
      `/api/v1/databases/${dbId}/tables/${encodeURIComponent(table)}/export?format=csv`,

    importSql: (dbId: string, file: File, token?: string | null) => {
      const form = new FormData();
      form.append('file', file);
      const storedToken = token ?? (typeof localStorage !== 'undefined' ? localStorage.getItem('orbit_access_token') : null);
      return fetch(`/api/v1/databases/${dbId}/import`, {
        method:  'POST',
        headers: storedToken ? { Authorization: `Bearer ${storedToken}` } : {},
        body:    form,
      }).then(r => r.ok ? r.json() : r.json().then((e: { message?: string }) => Promise.reject(new Error(e.message ?? `HTTP ${r.status}`))));
    },

    updateRow: (dbId: string, table: string, pk: string, data: Record<string, unknown>) => {
      const headers: Record<string, string> = { 'Content-Type': 'application/json' };
      const storedToken = this.token ?? (typeof localStorage !== 'undefined' ? localStorage.getItem('orbit_access_token') : null);
      if (storedToken) headers['Authorization'] = `Bearer ${storedToken}`;
      return fetch(`/api/v1/databases/${dbId}/tables/${encodeURIComponent(table)}/rows/${encodeURIComponent(pk)}`, {
        method: 'PUT',
        headers,
        body: JSON.stringify(data),
      }).then(r => r.ok ? r.json() : r.json().then((e: { message?: string }) => Promise.reject(new Error(e.message ?? `HTTP ${r.status}`))));
    },
  };

  // ── Reseller ────────────────────────────────────────────────────────────────

  readonly reseller = {
    clients: () =>
      this.request<Array<{
        id: string;
        email: string;
        display_name: string | null;
        sites_count: number;
        package_name: string | null;
        status: 'active' | 'suspended';
        created_at: string;
      }>>('GET', '/api/v1/reseller/clients'),

    suspendClient: (id: string) =>
      this.request<void>('POST', `/api/v1/reseller/clients/${id}/suspend`),

    unsuspendClient: (id: string) =>
      this.request<void>('POST', `/api/v1/reseller/clients/${id}/unsuspend`),

    removeClient: (id: string) =>
      this.request<void>('DELETE', `/api/v1/reseller/clients/${id}`),

    loginAsClient: (id: string) =>
      this.request<{
        access_token: string; user_id: string; email: string;
        role: string; expires_in: number; wizard_complete: boolean;
      }>('POST', `/api/v1/reseller/clients/${id}/login-as`),

    packages: () =>
      this.request<Array<{
        id: string;
        name: string;
        max_sites: number;
        max_disk_gb: number;
        max_email_accounts: number;
        max_databases: number;
        price_monthly: number;
        clients_count: number;
        features: Record<string, boolean>;
      }>>('GET', '/api/v1/reseller/packages'),

    createPackage: (data: {
      name: string;
      max_sites: number;
      max_disk_gb: number;
      max_email_accounts: number;
      max_databases: number;
      price_monthly: number;
      features?: Record<string, boolean>;
    }) => this.request<{ id: string }>('POST', '/api/v1/reseller/packages', data),

    updatePackage: (id: string, data: Record<string, unknown>) =>
      this.request<void>('PUT', `/api/v1/reseller/packages/${id}`, data),

    deletePackage: (id: string) =>
      this.request<void>('DELETE', `/api/v1/reseller/packages/${id}`),

    stats: () =>
      this.request<{
        total_clients: number;
        active_clients: number;
        total_sites: number;
        total_disk_used_gb: number;
        total_revenue_monthly: number;
      }>('GET', '/api/v1/reseller/stats'),
  };

  // ── Git Deploy ──────────────────────────────────────────────────────────────

  readonly gitDeploy = {
    status: (siteId: string) =>
      this.request<{
        enabled: boolean;
        remote_url: string | null;
        host: string;
        deploy_key_pub: string | null;
      }>('GET', `/api/v1/sites/${siteId}/git-deploy/status`),

    enable: (siteId: string) =>
      this.request<{ remote_url: string; deploy_key_pub: string }>(
        'POST', `/api/v1/sites/${siteId}/git-deploy/enable`),

    disable: (siteId: string) =>
      this.request<void>('POST', `/api/v1/sites/${siteId}/git-deploy/disable`),

    history: (siteId: string) =>
      this.request<Array<{
        id: string;
        commit_hash: string;
        commit_message: string;
        deployed_at: string;
        deployed_by: string;
        status: 'success' | 'failed';
      }>>('GET', `/api/v1/sites/${siteId}/git-deploy/history`),

    deployNow: (siteId: string) =>
      this.request<{ job_id: string }>('POST', `/api/v1/sites/${siteId}/git-deploy/trigger`),
  };

  // ── SSH Keys ────────────────────────────────────────────────────────────────

  readonly sshKeys = {
    list: () =>
      this.request<Array<{
        id: string;
        label: string;
        fingerprint: string;
        key_type: string;
        added_at: string;
        last_used_at: string | null;
      }>>('GET', '/api/v1/ssh-keys'),

    add: (pubkey: string, label: string) =>
      this.request<{ id: string; fingerprint: string }>('POST', '/api/v1/ssh-keys', { pubkey, label }),

    delete: (id: string) =>
      this.request<void>('DELETE', `/api/v1/ssh-keys/${id}`),

    /** Disable password login for SSH. Requires the id of an SSH key that is already registered. */
    disablePasswordLogin: (keyId: string) =>
      this.request<void>('POST', `/api/v1/ssh-keys/${keyId}/disable-password`),
  };

  // ── Client portal ───────────────────────────────────────────────────────────

  readonly client = {
    sites: () =>
      this.request<Site[]>('GET', '/api/v1/client/sites'),

    profile: () =>
      this.request<{
        id: string;
        email: string;
        display_name: string | null;
        package: {
          name: string;
          max_sites: number;
          max_disk_gb: number;
          max_email_accounts: number;
          max_databases: number;
        } | null;
        usage: {
          sites_count: number;
          disk_used_gb: number;
          email_accounts: number;
          databases: number;
        };
      }>('GET', '/api/v1/client/profile'),

    changePassword: (current_password: string, new_password: string) =>
      this.request<void>('PUT', '/api/v1/client/profile/password', { current_password, new_password }),
  };

  // ── Email IP Pools ──────────────────────────────────────────────────────────

  readonly emailIps = {
    list: () =>
      this.request<EmailIpPool[]>('GET', '/api/v1/email/ip-pools'),

    add: (ip: string, rdns?: string) =>
      this.request<EmailIpPool>('POST', '/api/v1/email/ip-pools', { ip, rdns }),

    remove: (id: string) =>
      this.request<void>('DELETE', `/api/v1/email/ip-pools/${id}`),
    // Note: setPrimary not yet implemented in backend
  };

  // ── Staging ─────────────────────────────────────────────────────────────────

  readonly staging = {
    list: (siteId: string) =>
      this.request<StagingSite[]>('GET', `/api/v1/sites/${siteId}/staging`),

    create: (siteId: string) =>
      this.request<StagingSite>('POST', `/api/v1/sites/${siteId}/staging/create`),

    promote: (siteId: string, _stagingId: string) =>
      this.request<{ job_id: string }>('POST', `/api/v1/sites/${siteId}/staging/push-to-live`),

    delete: (siteId: string, _stagingId: string) =>
      this.request<void>('DELETE', `/api/v1/sites/${siteId}/staging`),
  };

  // ── Migration ────────────────────────────────────────────────────────────────

  readonly migration = {
    list: () =>
      this.request<MigrationJob[]>('GET', '/api/v1/migration/jobs'),

    start: (payload: {
      source_url:     string;
      source_type:    'cpanel' | 'plesk' | 'directadmin' | 'generic';
      target_site_id: string;
      include_email?: boolean;
      include_db?:    boolean;
    }) => this.request<MigrationJob>('POST', '/api/v1/migration/import', payload),

    get: (id: string) =>
      this.request<MigrationJob>('GET', `/api/v1/migration/jobs/${id}`),
    // Note: cancel endpoint not yet implemented in backend
  };

  // ── Performance ──────────────────────────────────────────────────────────────

  readonly performance = {
    report: (siteId: string) =>
      this.request<PerformanceReport>('GET', `/api/v1/sites/${siteId}/performance`),

    history: (siteId: string, params?: { from?: string; to?: string }) =>
      this.request<PerformanceReport[]>('GET', `/api/v1/sites/${siteId}/performance/history`,
        undefined, params as Record<string, string | undefined>),

    runTest: (siteId: string) =>
      this.request<{ job_id: string }>('POST', `/api/v1/sites/${siteId}/performance/test`),
  };

  // ── MCP ─────────────────────────────────────────────────────────────────────

  // NOT YET IMPLEMENTED IN BACKEND — orbit-panel does not yet expose /api/v1/mcp.
  // orbit-mcp is a separate process. Tracked on roadmap.
  readonly mcp = {
    /** List configured MCP server connections */
    list: () =>
      this.request<Array<{
        id:         string;
        name:       string;
        url:        string;
        is_active:  boolean;
        created_at: string;
      }>>('GET', '/api/v1/mcp/servers'),

    add: (payload: { name: string; url: string; api_key?: string }) =>
      this.request<{ id: string; name: string; url: string }>('POST', '/api/v1/mcp/servers', payload),

    remove: (id: string) =>
      this.request<void>('DELETE', `/api/v1/mcp/servers/${id}`),

    test: (id: string) =>
      this.request<{ success: boolean; message: string }>('POST', `/api/v1/mcp/servers/${id}/test`),
  };

  // ── SMTP (save + test) ───────────────────────────────────────────────────────

  readonly smtp = {
    /** NOT YET IMPLEMENTED — backend only has POST /smtp/test (no persistent save endpoint). */
    save: (payload: { host: string; port: number; user: string; password: string; from: string }) =>
      this.request<void>('PUT', '/api/v1/settings/smtp', payload),

    test: (payload: { host: string; port: number; user: string; password: string; from: string; to: string }) =>
      this.request<{ success: boolean; message: string }>('POST', '/api/v1/settings/smtp/test', payload),
  };

  // ── License ──────────────────────────────────────────────────────────────────

  readonly license = {
    get: () =>
      this.request<{
        tier:       string;
        domain:     string | null;
        expires_at: string | null;
        features:   string[];
      }>('GET', '/api/v1/settings/license'),

    activate: (key: string) =>
      this.request<{ tier: string; domain: string; expires_at: string | null }>('POST', '/api/v1/settings/license', { key }),
  };

  // ── Cron ────────────────────────────────────────────────────────────────────

  readonly cron = {
    list: () =>
      this.request<CronJob[]>('GET', '/api/v1/cron'),

    listForSite: (siteId: string) =>
      this.request<CronJob[]>('GET', `/api/v1/cron/site/${siteId}`),

    create: (payload: { site_id: string; label?: string; schedule: string; command: string; enabled?: boolean }) =>
      this.request<CronJob>('POST', '/api/v1/cron', payload),

    update: (id: string, payload: { enabled?: boolean; label?: string; schedule?: string; command?: string }) =>
      this.request<CronJob>('PUT', `/api/v1/cron/${id}`, payload),

    delete: (id: string) =>
      this.request<void>('DELETE', `/api/v1/cron/${id}`),

    runNow: (id: string) =>
      this.request<{ output: string; exit_code: number }>('POST', `/api/v1/cron/${id}/run`),
  };

  // ── Cache ────────────────────────────────────────────────────────────────────

  readonly cache = {
    opcacheStats: (siteId?: string) =>
      siteId
        ? this.request<{ enabled: boolean; hit_rate: number; memory_used_mb: number; memory_free_mb: number; cached_scripts: number; jit_enabled: boolean }>('GET', `/api/v1/cache/opcache/${siteId}`)
        : this.request<{ enabled: boolean; hit_rate: number; memory_used_mb: number; memory_free_mb: number; cached_scripts: number; jit_enabled: boolean }>('GET', '/api/v1/cache/opcache'),

    valkeyStats: (siteId?: string) =>
      siteId
        ? this.request<{ connected: boolean; keys: number; memory_used_mb: number; hit_rate: number; connected_clients: number; uptime_seconds: number; version: string }>('GET', `/api/v1/cache/valkey/${siteId}`)
        : this.request<{ connected: boolean; keys: number; memory_used_mb: number; hit_rate: number; connected_clients: number; uptime_seconds: number; version: string }>('GET', '/api/v1/cache/valkey'),

    flushOpcache: (siteId: string) =>
      this.request<void>('POST', `/api/v1/cache/opcache/${siteId}/flush`),

    flushValkey: (siteId: string) =>
      this.request<{ deleted_keys: number }>('POST', `/api/v1/cache/valkey/${siteId}/flush`),

    setCachePreset: (siteId: string, preset: string) =>
      this.request<void>('PUT', `/api/v1/cache/headers/${siteId}`, { preset }),
  };

  // ── File Manager ─────────────────────────────────────────────────────────────

  readonly filemanager = {
    list: (params: { site_id: string; path: string }) =>
      this.request<{ entries: Array<{ name: string; path: string; is_dir: boolean; size: number; modified_at: string }>; path: string }>(
        'GET', '/api/v1/files', undefined, params as Record<string, string>),

    downloadUrl: (siteId: string, filePath: string) =>
      `/api/v1/files/download?site_id=${encodeURIComponent(siteId)}&path=${encodeURIComponent(filePath)}`,

    delete: (params: { site_id: string; path: string }) =>
      this.request<void>('DELETE', '/api/v1/files', params),

    mkdir: (params: { site_id: string; path: string }) =>
      this.request<void>('POST', '/api/v1/files/mkdir', params),

    write: (params: { site_id: string; path: string; content: string }) =>
      this.request<void>('POST', '/api/v1/files/write', params),

    rename: (params: { site_id: string; path: string; new_path: string }) =>
      this.request<void>('POST', '/api/v1/files/rename', params),

    upload: (params: { site_id: string; path: string; file: File }) => {
      const form = new FormData();
      form.append('file', params.file);
      form.append('site_id', params.site_id);
      form.append('path', params.path);
      const storedToken = this.token ?? (typeof localStorage !== 'undefined' ? localStorage.getItem('orbit_access_token') : null);
      const headers: Record<string, string> = {};
      if (storedToken) headers['Authorization'] = `Bearer ${storedToken}`;
      return fetch('/api/v1/files/upload', { method: 'POST', headers, body: form })
        .then(r => r.ok ? r.json() : r.json().then((e: { message?: string }) => Promise.reject(new Error(e.message ?? `HTTP ${r.status}`))));
    },
  };

  // ── PHP ──────────────────────────────────────────────────────────────────────

  readonly php = {
    extensions: (siteId: string) =>
      this.request<Array<{ name: string; enabled: boolean; description: string; dev_only: boolean; category: string; special_note?: string }>>('GET', `/api/v1/php/extensions/${siteId}`),

    toggleExtension: (siteId: string, extName: string, enabled: boolean) =>
      this.request<void>('PUT', `/api/v1/php/extensions/${siteId}/${extName}`, { enabled }),

    updateExtensions: (siteId: string, payload: { extensions: Record<string, boolean> }) =>
      this.request<void>('PUT', `/api/v1/php/extensions/${siteId}/batch`, payload),

    opcacheStats: (siteId: string) =>
      this.request<{ enabled: boolean; hit_rate: number; memory_used_mb: number; memory_free_mb: number; cached_scripts: number; jit_enabled: boolean; jit_buffer_size: number }>('GET', `/api/v1/php/opcache/${siteId}`),

    flushOpcache: (siteId: string) =>
      this.request<void>('POST', `/api/v1/php/opcache/${siteId}/flush`),

    toggleJit: (siteId: string, enabled: boolean) =>
      this.request<void>('PUT', `/api/v1/php/opcache/${siteId}/jit`, { enabled }),

    getSettings: (siteId: string) =>
      this.request<Record<string, string>>('GET', `/api/v1/php/settings/${siteId}`),

    saveSettings: (siteId: string, settings: Record<string, string>) =>
      this.request<void>('PUT', `/api/v1/php/settings/${siteId}`, { settings }),

    phpInfo: (siteId: string) =>
      this.request<{ html: string }>('GET', `/api/v1/php/info/${siteId}`),
  };

  // ── Graceful fetch helper ─────────────────────────────────────────────────
  // Returns emptyValue for 404/501 (feature not deployed), throws for other errors.

  private async safeFetch<T>(path: string, emptyValue: T): Promise<T> {
    const res = await fetch(`${this.baseUrl}${path}`, { headers: this.headers });
    if (res.status === 404 || res.status === 501) return emptyValue;
    if (!res.ok) {
      let errBody: ApiError = { error: 'unknown', message: `HTTP ${res.status}` };
      try { errBody = await res.json(); } catch { /* ignore */ }
      throw Object.assign(new Error(errBody.message), errBody);
    }
    if (res.status === 204) return emptyValue;
    return res.json() as Promise<T>;
  }

  // ── Site SSH Keys (per-site) ──────────────────────────────────────────────
  // These call new endpoints added to orbit-panel (may return 501 if not yet deployed)

  readonly siteSshKeys = {
    list: (siteId: string) =>
      this.safeFetch<Array<{ id: string; label: string; public_key: string; fingerprint: string; created_at: string }>>(
        `/api/v1/sites/${siteId}/ssh-keys`, []),

    add: (siteId: string, payload: { label: string; public_key: string }) =>
      this.request<{ id: string; label: string; fingerprint: string; created_at: string }>(
        'POST', `/api/v1/sites/${siteId}/ssh-keys`, payload),

    delete: (siteId: string, keyId: string) =>
      this.request<void>('DELETE', `/api/v1/sites/${siteId}/ssh-keys/${keyId}`),
  };

  // ── Site FTP Users ────────────────────────────────────────────────────────

  readonly ftpUsers = {
    list: (siteId: string) =>
      this.request<Array<{ id: string; username: string; home_dir: string; created_at: string }>>(
        'GET', `/api/v1/sites/${siteId}/ftp-users`),

    create: (siteId: string, payload: { username: string; password: string }) =>
      this.request<{ id: string; username: string; home_dir: string; created_at: string }>(
        'POST', `/api/v1/sites/${siteId}/ftp-users`, payload),

    changePassword: (siteId: string, userId: string, password: string) =>
      this.request<void>('PUT', `/api/v1/sites/${siteId}/ftp-users/${userId}/password`, { password }),

    delete: (siteId: string, userId: string) =>
      this.request<void>('DELETE', `/api/v1/sites/${siteId}/ftp-users/${userId}`),
  };

  // ── Database Users ────────────────────────────────────────────────────────

  readonly dbUsers = {
    list: (dbId: string) =>
      this.safeFetch<Array<{ id: string; username: string; privileges: string[]; created_at: string }>>(
        `/api/v1/databases/${dbId}/users`, []),

    create: (dbId: string, payload: { username: string; password: string; privileges: string[] }) =>
      this.request<{ id: string; username: string; privileges: string[]; created_at: string }>(
        'POST', `/api/v1/databases/${dbId}/users`, payload),

    delete: (dbId: string, userId: string) =>
      this.request<void>('DELETE', `/api/v1/databases/${dbId}/users/${userId}`),
  };

  // ── WordPress (WP-CLI) ───────────────────────────────────────────────────────
  readonly wordpress = {
    core: (siteId: string) =>
      this.request<{ version: string; update_available: boolean; update_version: string | null }>(
        'GET', `/api/v1/sites/${siteId}/wordpress/core`),

    updateCore: (siteId: string) =>
      this.request<{ updated: boolean; old_version: string; new_version: string }>(
        'POST', `/api/v1/sites/${siteId}/wordpress/core/update`),

    plugins: (siteId: string) =>
      this.request<Array<{ name: string; slug: string; version: string; status: 'active' | 'inactive'; update_available: boolean; update_version: string | null }>>(
        'GET', `/api/v1/sites/${siteId}/wordpress/plugins`),

    installPlugin: (siteId: string, slug: string) =>
      this.request<{ slug: string; name: string; version: string; status: string }>(
        'POST', `/api/v1/sites/${siteId}/wordpress/plugins`, { slug }),

    activatePlugin: (siteId: string, slug: string) =>
      this.request<void>('PUT', `/api/v1/sites/${siteId}/wordpress/plugins/${encodeURIComponent(slug)}`, { activate: true }),

    deactivatePlugin: (siteId: string, slug: string) =>
      this.request<void>('PUT', `/api/v1/sites/${siteId}/wordpress/plugins/${encodeURIComponent(slug)}`, { activate: false }),

    deletePlugin: (siteId: string, slug: string) =>
      this.request<void>('DELETE', `/api/v1/sites/${siteId}/wordpress/plugins/${encodeURIComponent(slug)}`),

    themes: (siteId: string) =>
      this.request<Array<{ name: string; slug: string; version: string; status: 'active' | 'inactive' }>>(
        'GET', `/api/v1/sites/${siteId}/wordpress/themes`),

    activateTheme: (siteId: string, slug: string) =>
      this.request<void>('POST', `/api/v1/sites/${siteId}/wordpress/themes/${encodeURIComponent(slug)}/activate`),

    deleteTheme: (siteId: string, slug: string) =>
      this.request<void>('DELETE', `/api/v1/sites/${siteId}/wordpress/themes/${encodeURIComponent(slug)}`),

    users: (siteId: string) =>
      this.request<Array<{ id: number; login: string; email: string; display_name: string; role: string }>>(
        'GET', `/api/v1/sites/${siteId}/wordpress/users`),

    loginUrl: (siteId: string, userId: number) =>
      this.request<{ login_url: string }>('POST', `/api/v1/sites/${siteId}/wordpress/users/${userId}/login-url`),

    updateAll: (siteId: string) =>
      this.request<{ core: boolean; plugins: number; themes: number; errors: string[] }>(
        'POST', `/api/v1/sites/${siteId}/wordpress/update-all`),
  };

  // ── Cloudflare ───────────────────────────────────────────────────────────────
  readonly cloudflare = {
    tokens: () =>
      this.request<Array<{ id: string; label: string; zone_id: string; domain: string; proxy: boolean; created_at: string }>>(
        'GET', '/api/v1/cloudflare/tokens'),

    addToken: (payload: { label: string; api_token: string; zone_id: string; domain: string; proxy?: boolean }) =>
      this.request<{ id: string; label: string; zone_id: string; domain: string; proxy: boolean; created_at: string }>(
        'POST', '/api/v1/cloudflare/tokens', payload),

    deleteToken: (id: string) =>
      this.request<void>('DELETE', `/api/v1/cloudflare/tokens/${id}`),

    syncZone: (id: string, proxy?: boolean) =>
      this.request<{ synced: number; errors: number; message: string }>(
        'POST', `/api/v1/cloudflare/tokens/${id}/sync`, proxy !== undefined ? { proxy } : {}),

    setProxy: (id: string, proxy: boolean) =>
      this.request<{ proxy: boolean; message: string }>('PUT', `/api/v1/cloudflare/tokens/${id}/proxy`, { proxy }),

    zones: () =>
      this.request<Array<{ id: string | null; name: string | null; status: string | null }>>(
        'GET', '/api/v1/cloudflare/zones'),
  };

  // ── Webhooks ─────────────────────────────────────────────────────────────────
  readonly webhooks = {
    list: () =>
      this.request<Array<{ id: string; url: string; events: string[]; enabled: boolean; secret: string; created_at: string }>>(
        'GET', '/api/v1/webhooks'),

    create: (payload: { url: string; events: string[]; secret?: string }) =>
      this.request<{ id: string; url: string; events: string[]; enabled: boolean; secret: string; created_at: string }>(
        'POST', '/api/v1/webhooks', payload),

    delete: (id: string) =>
      this.request<void>('DELETE', `/api/v1/webhooks/${id}`),

    test: (id: string) =>
      this.request<{ success: boolean; status_code: number; response_ms: number }>(
        'POST', `/api/v1/webhooks/${id}/test`),

    toggle: (id: string, enabled: boolean) =>
      this.request<void>('PUT', `/api/v1/webhooks/${id}`, { enabled }),
  };

  // ── Site Clone + Tags ────────────────────────────────────────────────────────
  readonly siteExtras = {
    clone: (siteId: string, targetDomain: string, copyDb: boolean) =>
      this.request<{ clone_id: string; status: string; message: string }>(
        'POST', `/api/v1/sites/${siteId}/clone`, { target_domain: targetDomain, copy_db: copyDb }),

    getTags: (siteId: string) =>
      this.request<{ tags: string[] }>('GET', `/api/v1/sites/${siteId}/tags`),

    setTags: (siteId: string, tags: string[]) =>
      this.request<{ tags: string[] }>('PUT', `/api/v1/sites/${siteId}/tags`, { tags }),
  };

  // ── Runtime ──────────────────────────────────────────────────────────────────
  readonly runtime = {
    get: (siteId: string) =>
      this.request<{ runtime: string; version: string; entry_point: string | null }>(
        'GET', `/api/v1/sites/${siteId}/runtime`),

    set: (siteId: string, runtime: string, version: string) =>
      this.request<{ runtime: string; version: string; message: string }>(
        'POST', `/api/v1/sites/${siteId}/runtime`, { runtime, version }),

    nodejsStatus: (siteId: string) =>
      this.request<{
        name: string; status: string; pid: number | null;
        uptime_ms: number | null; restarts: number;
        cpu_pct: number | null; memory_mb: number | null;
      }>('GET', `/api/v1/sites/${siteId}/nodejs/status`),

    nodejsProcess: (siteId: string, action: 'start' | 'stop' | 'restart' | 'delete') =>
      this.request<{ status: string; message: string }>(
        'POST', `/api/v1/sites/${siteId}/nodejs/process`, { action }),

    nodejsLogs: (siteId: string) =>
      this.request<Array<{ ts: string; level: 'stdout' | 'stderr'; line: string }>>(
        'GET', `/api/v1/sites/${siteId}/nodejs/logs`),
  };
}

// Singleton instance — configured with token from auth store

export const api = new OrbitApiClient();
export default api;
