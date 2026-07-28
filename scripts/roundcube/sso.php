<?php
/**
 * JottiCP Webmail SSO Bridge for Roundcube.
 *
 * Flow:
 *   1. Validate one-time token from Valkey (key: jotti:webmail:<token>)
 *   2. GET Roundcube login page to obtain CSRF _token + initial session cookie
 *   3. POST credentials with the captured _token
 *   4. Relay the post-login session cookie back to the browser
 *   5. Redirect to /webmail/
 */

session_name('jotti_webmail');
session_start();

$token = trim($_GET['token'] ?? '');
if ($token === '') {
    http_response_code(400);
    die(renderPage('Invalid Link', 'No SSO token. Return to JottiCP and click Webmail again.', 'error'));
}

// Read Valkey password from www-data readable file
$valkey_pass = trim(@file_get_contents('/etc/jotticp/webmail-valkey-pass') ?: '');

try {
    $redis = new Redis();
    $redis->connect('127.0.0.1', 6379, 3.0);
    if ($valkey_pass !== '') $redis->auth($valkey_pass);
} catch (Throwable $e) {
    error_log('jotti-sso: valkey connect failed: ' . $e->getMessage());
    http_response_code(503);
    die(renderPage('Service Unavailable', 'Cannot reach mail session store.', 'error'));
}

$key  = 'jotti:webmail:' . preg_replace('/[^a-f0-9]/', '', $token);
$data = $redis->get($key);
if ($data === false) {
    http_response_code(401);
    die(renderPage('Link Expired', 'This webmail link has expired or already been used.', 'warn'));
}

$payload  = json_decode($data, true);
$email    = $payload['email']    ?? '';
$password = $payload['password'] ?? '';
$expires  = (int)($payload['expires'] ?? 0);

if ($email === '' || $password === '' || time() > $expires) {
    http_response_code(401);
    die(renderPage('Link Expired', 'This webmail link is invalid or has expired.', 'warn'));
}

// Single-use token
$redis->del($key);

// ── Step 1: GET login page to capture CSRF token + initial session cookie ────
$cookieJar = tempnam(sys_get_temp_dir(), 'rc_sso_');
$ch = curl_init('http://127.0.0.1/webmail/?_task=login');
curl_setopt_array($ch, [
    CURLOPT_RETURNTRANSFER => true,
    CURLOPT_FOLLOWLOCATION => true,
    CURLOPT_HEADER         => true,
    CURLOPT_COOKIEJAR      => $cookieJar,
    CURLOPT_COOKIEFILE     => $cookieJar,
    CURLOPT_HTTPHEADER     => ['Host: ' . ($_SERVER['HTTP_HOST'] ?? 'jotti.ru')],
    CURLOPT_TIMEOUT        => 8,
]);
$loginPage = curl_exec($ch);
$loginCode = curl_getinfo($ch, CURLINFO_HTTP_CODE);
curl_close($ch);

if ($loginCode !== 200 || !preg_match('/name="_token"\s+value="([A-Za-z0-9]+)"/', $loginPage, $m)) {
    @unlink($cookieJar);
    error_log("jotti-sso: cannot parse Roundcube _token (HTTP $loginCode)");
    http_response_code(502);
    die(renderPage('Webmail Error', 'Cannot reach Roundcube login form.', 'error'));
}
$rcToken = $m[1];

// ── Step 2: POST credentials with the captured _token ────────────────────────
$ch = curl_init('http://127.0.0.1/webmail/?_task=login');
curl_setopt_array($ch, [
    CURLOPT_POST            => true,
    CURLOPT_POSTFIELDS      => http_build_query([
        '_token'    => $rcToken,
        '_task'     => 'login',
        '_action'   => 'login',
        '_timezone' => 'UTC',
        '_url'      => '',
        '_user'     => $email,
        '_pass'     => $password,
    ]),
    CURLOPT_RETURNTRANSFER  => true,
    CURLOPT_FOLLOWLOCATION  => false,
    CURLOPT_HEADER          => true,
    CURLOPT_COOKIEJAR       => $cookieJar,
    CURLOPT_COOKIEFILE      => $cookieJar,
    CURLOPT_HTTPHEADER      => [
        'Host: ' . ($_SERVER['HTTP_HOST'] ?? 'jotti.ru'),
        'Content-Type: application/x-www-form-urlencoded',
    ],
    CURLOPT_TIMEOUT         => 8,
]);
$response  = curl_exec($ch);
$httpCode  = curl_getinfo($ch, CURLINFO_HTTP_CODE);
$headerLen = curl_getinfo($ch, CURLINFO_HEADER_SIZE);
curl_close($ch);

$rawHeaders = substr($response ?: '', 0, $headerLen);

// ── Step 3: Relay Set-Cookie headers from Roundcube to the browser ──────────
$relayed = 0;
foreach (explode("\r\n", $rawHeaders) as $line) {
    if (stripos($line, 'set-cookie:') === 0) {
        header('Set-Cookie: ' . trim(substr($line, 12)), false);
        $relayed++;
    }
}
@unlink($cookieJar);

if ($httpCode === 302 || ($httpCode === 200 && $relayed > 0)) {
    header('Location: /webmail/?_task=mail');
    exit;
}

// On failure, fall back to login form so the user can type credentials
error_log("jotti-sso: roundcube login HTTP $httpCode, relayed $relayed cookies");
header('Location: /webmail/');
exit;

function renderPage(string $title, string $msg, string $type): string {
    $color = $type === 'error' ? '#ef4444' : ($type === 'warn' ? '#f59e0b' : '#22c55e');
    $titleHtml = htmlspecialchars($title, ENT_QUOTES);
    $msgHtml   = htmlspecialchars($msg, ENT_QUOTES);
    return <<<HTML
<!DOCTYPE html><html><head><title>{$titleHtml} — JottiCP Webmail</title>
<meta charset=UTF-8><meta name=viewport content="width=device-width,initial-scale=1">
<style>body{font-family:system-ui,sans-serif;background:#0f172a;color:#e2e8f0;
min-height:100vh;display:flex;align-items:center;justify-content:center;padding:1rem}
.c{background:#1e293b;border:1px solid #334155;border-radius:12px;padding:2rem;max-width:420px;text-align:center}
h1{margin:0 0 .5rem;color:{$color};font-size:1.25rem}p{color:#94a3b8;margin:0 0 1rem;line-height:1.5}
a{display:inline-block;margin-top:.5rem;color:#60a5fa;text-decoration:none}</style></head>
<body><div class=c><h1>{$titleHtml}</h1><p>{$msgHtml}</p>
<a href=https://jotti.ru>← Back to JottiCP</a></div></body></html>
HTML;
}
