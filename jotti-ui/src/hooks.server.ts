import type { Handle } from '@sveltejs/kit';

const API_TARGET = 'http://127.0.0.1:2087';

export const handle: Handle = async ({ event, resolve }) => {
  const url = new URL(event.request.url);

  // Proxy API, health, and WebSocket requests to jotti-panel backend
  if (url.pathname.startsWith('/api/') || url.pathname === '/health' || url.pathname.startsWith('/health/') || url.pathname === '/metrics' || url.pathname.startsWith('/ws')) {
    const target = new URL(url.pathname + url.search, API_TARGET);
    const headers = new Headers(event.request.headers);
    headers.delete('connection');

    const proxyReq = new Request(target, {
      method: event.request.method,
      headers,
      body: ['GET', 'HEAD'].includes(event.request.method) ? undefined : event.request.body,
      duplex: 'half',
    });

    return fetch(proxyReq);
  }

  const response = await resolve(event);

  response.headers.set('X-Frame-Options', 'DENY');
  response.headers.set('X-Content-Type-Options', 'nosniff');
  response.headers.set('Referrer-Policy', 'strict-origin-when-cross-origin');
  response.headers.set(
    'Permissions-Policy',
    'camera=(), microphone=(), geolocation=(), payment=(), usb=(), interest-cohort=()',
  );

  const csp = [
    "default-src 'self'",
    "script-src 'self' 'unsafe-inline' https://static.cloudflareinsights.com",
    "style-src 'self' 'unsafe-inline'",
    "img-src 'self' data: blob:",
    "font-src 'self' data:",
    "connect-src 'self' ws: wss:",
    "object-src 'none'",
    "base-uri 'self'",
    "form-action 'self'",
    "frame-ancestors 'none'",
  ].join('; ');

  response.headers.set('Content-Security-Policy', csp);

  return response;
};
