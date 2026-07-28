// JottiCP Service Worker
// Strategy:
//   - API calls (/api/):  network-first, no caching (always fresh data)
//   - WebSocket (ws:/wss:): passthrough (SW cannot intercept)
//   - Static assets:       cache-first, fall back to network
//   - HTML pages:          network-first, fall back to cache (stale-while-revalidate)

const CACHE_NAME    = 'jottiecp-v2';
const STATIC_ASSETS = [
    '/',
    '/dashboard',
    '/manifest.json',
];

// ── Install: pre-cache critical shell assets ──────────────────────────────────

self.addEventListener('install', (event) => {
    event.waitUntil(
        caches.open(CACHE_NAME).then((cache) => {
            return cache.addAll(STATIC_ASSETS).catch((err) => {
                // Non-fatal: some assets may not exist at install time (SSR generated)
                console.warn('SW: pre-cache partial failure (ok):', err);
            });
        })
    );
    // Activate immediately — don't wait for old SW to be idle
    self.skipWaiting();
});

// ── Activate: clean up old caches ─────────────────────────────────────────────

self.addEventListener('activate', (event) => {
    event.waitUntil(
        caches.keys().then((keys) =>
            Promise.all(
                keys
                    .filter((key) => key !== CACHE_NAME)
                    .map((key) => caches.delete(key))
            )
        ).then(() => self.clients.claim())
    );
});

// ── Fetch: routing strategy ───────────────────────────────────────────────────

self.addEventListener('fetch', (event) => {
    const url = new URL(event.request.url);

    // Only intercept same-origin requests
    if (url.origin !== self.location.origin) {
        return;
    }

    // API calls: always network, never cache
    if (url.pathname.startsWith('/api/')) {
        event.respondWith(fetch(event.request));
        return;
    }

    // Static assets (JS/CSS/images/fonts): cache-first
    if (
        url.pathname.match(/\.(js|css|png|jpg|jpeg|svg|ico|woff2?|ttf)$/)
    ) {
        event.respondWith(
            caches.match(event.request).then(
                (cached) => cached || fetchAndCache(event.request)
            )
        );
        return;
    }

    // HTML navigation: network-first, fall back to cache
    if (event.request.mode === 'navigate') {
        event.respondWith(
            fetch(event.request)
                .then((response) => {
                    // Cache a fresh copy
                    const clone = response.clone();
                    caches.open(CACHE_NAME).then((cache) => cache.put(event.request, clone));
                    return response;
                })
                .catch(() => caches.match(event.request))
        );
        return;
    }

    // Default: network with cache fallback
    event.respondWith(
        fetch(event.request).catch(() => caches.match(event.request))
    );
});

// ── Push notifications ────────────────────────────────────────────────────────

self.addEventListener('push', (event) => {
    if (!event.data) return;

    let data;
    try {
        data = event.data.json();
    } catch {
        data = { title: 'JottiCP', body: event.data.text() };
    }

    const options = {
        body:    data.body   || '',
        icon:    '/icon-192.png',
        badge:   '/icon-192.png',
        tag:     data.tag    || 'jottiecp',
        data:    { url: data.url || '/dashboard' },
        actions: data.actions || [],
    };

    event.waitUntil(
        self.registration.showNotification(data.title || 'JottiCP Alert', options)
    );
});

self.addEventListener('notificationclick', (event) => {
    event.notification.close();
    const targetUrl = event.notification.data?.url || '/dashboard';

    event.waitUntil(
        clients.matchAll({ type: 'window', includeUncontrolled: true }).then((clientList) => {
            for (const client of clientList) {
                if (client.url === targetUrl && 'focus' in client) {
                    return client.focus();
                }
            }
            if (clients.openWindow) {
                return clients.openWindow(targetUrl);
            }
        })
    );
});

// ── Helpers ───────────────────────────────────────────────────────────────────

async function fetchAndCache(request) {
    const response = await fetch(request);
    if (response.ok) {
        const cache = await caches.open(CACHE_NAME);
        cache.put(request, response.clone());
    }
    return response;
}
