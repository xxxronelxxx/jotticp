/**
 * Auth store — manages JWT session, user identity, permissions, and
 * auto-refresh of access tokens before expiry.
 */
import { writable, derived, get } from 'svelte/store';
import { browser } from '$app/environment';
import { goto } from '$app/navigation';
import { api } from '$api/client';

// ── Types ─────────────────────────────────────────────────────────────────────

export type UserRole = 'admin' | 'reseller' | 'user';

export interface AuthUser {
  id:              string;
  email:           string;
  role:            UserRole;
  wizard_complete: boolean;
}

export interface AuthState {
  user:            AuthUser | null;
  token:           string | null;
  expires_at:      number | null;   // unix timestamp (ms)
  is_loading:      boolean;
  is_initialized:  boolean;
}

// ── Storage keys ─────────────────────────────────────────────────────────────

const TOKEN_KEY         = 'orbit_access_token';
const TOKEN_EXPIRES_KEY = 'orbit_token_expires';
const USER_KEY          = 'orbit_user';

// ── Token helpers ─────────────────────────────────────────────────────────────

/**
 * Decode the JWT payload and check whether the `exp` claim is in the past.
 * Returns `true` (expired) on any parse error so callers fail-closed.
 */
export function isTokenExpired(token: string): boolean {
  try {
    // JWTs use base64url (- and _ instead of + and /); atob requires standard base64
    const b64 = token.split('.')[1].replace(/-/g, '+').replace(/_/g, '/');
    const padded = b64.padEnd(Math.ceil(b64.length / 4) * 4, '=');
    const payload = JSON.parse(atob(padded));
    return payload.exp * 1000 < Date.now();
  } catch {
    return true;
  }
}

// ── Store ─────────────────────────────────────────────────────────────────────

function createAuthStore() {
  const initial: AuthState = {
    user:           null,
    token:          null,
    expires_at:     null,
    is_loading:     false,
    is_initialized: false,
  };

  const { subscribe, set, update } = writable<AuthState>(initial);

  // ── Initialization ──────────────────────────────────────────────────────────

  let focusListenerAttached = false;

  function init() {
    if (!browser) return;

    const token      = localStorage.getItem(TOKEN_KEY);
    const expires_at = Number(localStorage.getItem(TOKEN_EXPIRES_KEY) ?? '0');
    const userJson   = localStorage.getItem(USER_KEY);

    if (token && expires_at > Date.now() && userJson) {
      const user = JSON.parse(userJson) as AuthUser;
      api.setToken(token);
      set({ user, token, expires_at, is_loading: false, is_initialized: true });

      // Schedule proactive token refresh (5 minutes before expiry)
      scheduleRefresh(expires_at);
    } else {
      // Stale or missing — clear everything
      clearStorage();
      set({ ...initial, is_initialized: true });
    }

    // Re-check token validity whenever the tab regains focus. Only add once.
    if (!focusListenerAttached) {
      focusListenerAttached = true;
      window.addEventListener('focus', () => {
        const t = get({ subscribe }).token;
        if (t && isTokenExpired(t)) {
          clearSession();
          goto('/login?reason=session_expired');
        }
      });
    }
  }

  // ── Login ───────────────────────────────────────────────────────────────────

  async function login(email: string, password: string) {
    update(s => ({ ...s, is_loading: true }));
    try {
      const response = await api.auth.login(email, password);
      return response;
    } finally {
      update(s => ({ ...s, is_loading: false }));
    }
  }

  async function verifyTotp(interimToken: string, code: string) {
    update(s => ({ ...s, is_loading: true }));
    try {
      const response = await api.auth.verifyTotp(interimToken, code);
      setSession(response);
      return response;
    } finally {
      update(s => ({ ...s, is_loading: false }));
    }
  }

  async function verifyBackupCode(interimToken: string, code: string) {
    update(s => ({ ...s, is_loading: true }));
    try {
      const response = await api.auth.verifyBackupCode(interimToken, code);
      setSession(response);
      return response;
    } finally {
      update(s => ({ ...s, is_loading: false }));
    }
  }

  function setSession(response: {
    access_token: string;
    expires_in: number;
    user_id: string;
    email: string;
    role: string;
    wizard_complete: boolean;
  }) {
    const expires_at = Date.now() + response.expires_in * 1000;
    const user: AuthUser = {
      id:              response.user_id,
      email:           response.email,
      role:            response.role as UserRole,
      wizard_complete: response.wizard_complete,
    };

    api.setToken(response.access_token);

    if (browser) {
      localStorage.setItem(TOKEN_KEY,         response.access_token);
      localStorage.setItem(TOKEN_EXPIRES_KEY, String(expires_at));
      localStorage.setItem(USER_KEY,          JSON.stringify(user));
    }

    set({ user, token: response.access_token, expires_at, is_loading: false, is_initialized: true });
    scheduleRefresh(expires_at);
  }

  // ── Logout ──────────────────────────────────────────────────────────────────

  async function logout() {
    try {
      await api.auth.logout();
    } catch {
      // Ignore errors — clear session regardless
    }
    clearSession();
    goto('/login');
  }

  function clearSession() {
    api.setToken(null);
    clearStorage();
    set({ ...initial, is_initialized: true });
    if (refreshTimer) clearTimeout(refreshTimer);
  }

  function clearStorage() {
    if (!browser) return;
    localStorage.removeItem(TOKEN_KEY);
    localStorage.removeItem(TOKEN_EXPIRES_KEY);
    localStorage.removeItem(USER_KEY);
  }

  // ── Token refresh ───────────────────────────────────────────────────────────

  let refreshTimer: ReturnType<typeof setTimeout> | null = null;

  function scheduleRefresh(expires_at: number) {
    if (refreshTimer) clearTimeout(refreshTimer);

    // Refresh 5 minutes before expiry
    const refreshAt  = expires_at - 5 * 60 * 1000;
    const msUntilRefresh = refreshAt - Date.now();

    if (msUntilRefresh <= 0) {
      // Token is expired or about to expire — refresh immediately
      void refreshToken();
    } else {
      refreshTimer = setTimeout(() => void refreshToken(), msUntilRefresh);
    }
  }

  async function refreshToken() {
    // No refresh token in this flow — redirect to login on expiry
    // (TOTP 2FA requirement makes silent refresh complex; user must re-login)
    const state = get({ subscribe });
    if (!state.token) return;

    if (state.expires_at && state.expires_at < Date.now()) {
      clearSession();
      goto('/login?reason=session_expired');
    }
  }

  // ── Wizard complete ─────────────────────────────────────────────────────────

  async function markWizardComplete() {
    await api.settings.wizardComplete();
    update(s => ({
      ...s,
      user: s.user ? { ...s.user, wizard_complete: true } : null,
    }));
    if (browser) {
      try {
        const user = JSON.parse(localStorage.getItem(USER_KEY) ?? '{}');
        user.wizard_complete = true;
        localStorage.setItem(USER_KEY, JSON.stringify(user));
      } catch { /* ignore parse errors — store state is already updated */ }
    }
  }

  return {
    subscribe,
    init,
    login,
    verifyTotp,
    verifyBackupCode,
    setSession,
    logout,
    markWizardComplete,
    refreshToken,
  };
}

// ── Derived stores ────────────────────────────────────────────────────────────

export const auth = createAuthStore();

/** True if a valid, non-expired session exists */
export const isAuthenticated = derived(auth, $auth =>
  $auth.token !== null &&
  $auth.is_initialized &&
  ($auth.expires_at === null || $auth.expires_at > Date.now())
);

/** True if the user has admin role */
export const isAdmin = derived(auth, $auth => $auth.user?.role === 'admin');

/** True if the user has reseller role or above */
export const isReseller = derived(auth, $auth =>
  $auth.user?.role === 'admin' || $auth.user?.role === 'reseller');

/** Current user or null */
export const currentUser = derived(auth, $auth => $auth.user);

/** Current user ID or empty string */
export const userId = derived(auth, $auth => $auth.user?.id ?? '');

/** Permission check helper */
export const permissions = derived(auth, $auth => ({
  canManageServers:  $auth.user?.role === 'admin',
  canManageUsers:    $auth.user?.role === 'admin',
  canViewReports:    $auth.user?.role === 'admin' || $auth.user?.role === 'reseller',
  canManageBranding: $auth.user?.role === 'admin' || $auth.user?.role === 'reseller',
  canUseApiKeys:     $auth.user?.role !== null,
  isAdmin:           $auth.user?.role === 'admin',
  isReseller:        $auth.user?.role === 'reseller',
}));
