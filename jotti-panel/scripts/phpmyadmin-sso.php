<?php
/**
 * JottiCP DB Manager SSO Bridge (Adminer) — auto-login via POST.
 * Reads token from Valkey -> auto-submits Adminer's login form with the DB credentials.
 */
session_name('jotti_pma');
session_start();

$token = preg_replace('/[^a-f0-9\-]/', '', trim($_GET['token'] ?? ''));
if ($token === '') { http_response_code(400); die(render('Invalid Link', 'No SSO token provided.', 'error')); }

$valkey_pass = trim(@file_get_contents('/etc/jottiecp/webmail-valkey-pass') ?: '');
try {
    $redis = new Redis();
    $redis->connect('127.0.0.1', 6379, 3.0);
    if ($valkey_pass !== '') $redis->auth($valkey_pass);
} catch (Throwable $e) { http_response_code(503); die(render('Service Unavailable', 'Cannot reach session store.', 'error')); }

$data = $redis->get('jotti:pma:' . $token);
if ($data === false) { http_response_code(401); die(render('Link Expired', 'This DB manager link has expired. Click "Open DB Manager" again.', 'warn')); }

$payload  = json_decode($data, true);
$db_name  = $payload['db_name']     ?? '';
$db_user  = $payload['db_user']     ?? '';
$db_pass  = $payload['db_password'] ?? '';
if ($db_name === '' || $db_user === '') { http_response_code(401); die(render('Link Expired', 'Session data is incomplete.', 'error')); }

$redis->del('jotti:pma:' . $token); // single-use

// Adminer ignores arbitrary session vars — it authenticates from a POSTed login form.
// Render a self-submitting form that logs the browser into Adminer (sets its cookie).
$action = '/phpmyadmin/?server=' . rawurlencode('127.0.0.1') . '&username=' . rawurlencode($db_user) . '&db=' . rawurlencode($db_name);
$h = fn($s) => htmlspecialchars((string)$s, ENT_QUOTES, 'UTF-8');
header('Content-Type: text/html; charset=UTF-8');
echo '<!DOCTYPE html><html><head><meta charset=UTF-8><title>Opening DB Manager…</title>'
   . '<style>body{font-family:system-ui;background:#0f172a;color:#94a3b8;min-height:100vh;display:flex;align-items:center;justify-content:center}</style></head>'
   . '<body onload="document.forms[0].submit()">'
   . '<form method="post" action="' . $h($action) . '">'
   . '<input type="hidden" name="auth[driver]" value="server">'
   . '<input type="hidden" name="auth[server]" value="127.0.0.1">'
   . '<input type="hidden" name="auth[username]" value="' . $h($db_user) . '">'
   . '<input type="hidden" name="auth[password]" value="' . $h($db_pass) . '">'
   . '<input type="hidden" name="auth[db]" value="' . $h($db_name) . '">'
   . '<input type="hidden" name="auth[permanent]" value="1">'
   . '<noscript><button type="submit">Open DB Manager</button></noscript>'
   . '</form><p>Opening database manager…</p></body></html>';
exit;

function render(string $title, string $msg, string $type): string {
    $color = $type === 'error' ? '#ef4444' : ($type === 'warn' ? '#f59e0b' : '#22c55e');
    return '<!DOCTYPE html><html><head><meta charset=UTF-8><title>' . htmlspecialchars($title) . ' — JottiCP</title>'
        . '<style>body{font-family:system-ui;background:#0f172a;color:#e2e8f0;min-height:100vh;display:flex;align-items:center;justify-content:center}'
        . '.c{background:#1e293b;border:1px solid #334155;border-radius:12px;padding:2rem;max-width:400px;text-align:center}'
        . 'h1{color:' . $color . ';margin:0 0 .5rem}p{color:#94a3b8;margin:0 0 1rem}a{color:#60a5fa}</style></head><body><div class=c>'
        . '<h1>' . htmlspecialchars($title) . '</h1><p>' . htmlspecialchars($msg) . '</p>'
        . '<a href=https://jottiecp.dev-spb.ru>← Back to JottiCP</a></div></body></html>';
}
