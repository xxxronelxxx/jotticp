<?php
/**
 * OrbitCP Webmail SSO Bridge — 1-click auto-login for SOGo
 * Validates Valkey token, syncs plaintext password to sogo_users,
 * POSTs to SOGo /connect, relays session cookies to browser.
 */

session_name('orbit_webmail');
session_start();

$token = trim($_GET['token'] ?? '');
if (empty($token)) {
    http_response_code(400);
    die(renderPage('Invalid Link', 'No SSO token provided. Return to OrbitCP and click Webmail again.', 'error'));
}

// Connect to Valkey/Redis
try {
    $redis = new Redis();
    $redis->connect('127.0.0.1', 6379, 3.0);
} catch (Exception $e) {
    http_response_code(503);
    die(renderPage('Service Unavailable', 'Cannot reach mail session store. Please try again.', 'error'));
}

$key  = 'orbit:webmail:' . preg_replace('/[^a-f0-9]/', '', $token);
$data = $redis->get($key);

if ($data === false) {
    http_response_code(401);
    die(renderPage('Link Expired', 'This webmail link has expired or already been used. Return to OrbitCP and click Webmail again.', 'warn'));
}

$payload  = json_decode($data, true);
$email    = $payload['email']    ?? '';
$expires  = (int)($payload['expires'] ?? 0);
$password = $payload['password'] ?? null;

if (empty($email) || time() > $expires) {
    http_response_code(401);
    die(renderPage('Link Expired', 'This webmail link has expired. Return to OrbitCP and click Webmail again.', 'warn'));
}

// Single-use: delete token immediately
$redis->del($key);

// ── Sync plaintext password to sogo_users (ensures SOGo can verify it) ────────
if ($password !== null && $password !== '') {
    try {
        $pdo = new PDO('pgsql:host=127.0.0.1;dbname=sogo', 'sogo', 'sogo_pass', [
            PDO::ATTR_ERRMODE => PDO::ERRMODE_EXCEPTION,
            PDO::ATTR_TIMEOUT => 3,
        ]);
        $cn = explode('@', $email)[0];
        $stmt = $pdo->prepare(
            "INSERT INTO sogo_users (c_uid,c_name,c_cn,c_password,c_active,mail)
             VALUES (:u,:u,:cn,:pw,1,:u)
             ON CONFLICT (c_uid) DO UPDATE
             SET c_password=EXCLUDED.c_password, c_active=1, mail=EXCLUDED.mail"
        );
        $stmt->execute([':u' => $email, ':cn' => $cn, ':pw' => $password]);
    } catch (Throwable $e) {
        error_log('OrbitCP SSO: sogo_users sync failed: ' . $e->getMessage());
    }

    // ── Auto-login via SOGo /connect ──────────────────────────────────────────
    $ch = curl_init('http://127.0.0.1:20000/SOGo/connect');
    curl_setopt_array($ch, [
        CURLOPT_POST            => true,
        CURLOPT_POSTFIELDS      => json_encode(['userName' => $email, 'password' => $password]),
        CURLOPT_HTTPHEADER      => ['Content-Type: application/json', 'Accept: application/json'],
        CURLOPT_RETURNTRANSFER  => true,
        CURLOPT_HEADER          => true,
        CURLOPT_FOLLOWLOCATION  => false,
        CURLOPT_TIMEOUT         => 5,
    ]);

    $response  = curl_exec($ch);
    $httpCode  = curl_getinfo($ch, CURLINFO_HTTP_CODE);
    $headerLen = curl_getinfo($ch, CURLINFO_HEADER_SIZE);
    curl_close($ch);

    if ($httpCode === 200) {
        // Relay SOGo session cookies to the browser (path widened to /)
        $rawHeaders = substr($response, 0, $headerLen);
        foreach (explode("\r\n", $rawHeaders) as $line) {
            if (stripos($line, 'set-cookie:') === 0) {
                $cookieLine = preg_replace('/;\s*path=\/SOGo\//i', '; path=/', substr($line, 12));
                header('Set-Cookie: ' . trim($cookieLine), false);
            }
        }
        header('Location: /SOGo/');
        exit;
    }

    error_log('OrbitCP SOGo SSO: login failed for ' . $email . ' (HTTP ' . $httpCode . ')');
}

// ── Fallback: send user to SOGo login form ────────────────────────────────────
$_SESSION['orbit_sso_email']   = $email;
$_SESSION['orbit_sso_expires'] = time() + 300;
header('Location: /SOGo/');
exit;

// ── Helper ────────────────────────────────────────────────────────────────────
function renderPage(string $title, string $message, string $type): string {
    $color = $type === 'error' ? '#ef4444' : ($type === 'warn' ? '#f59e0b' : '#22c55e');
    return <<<HTML
<!DOCTYPE html>
<html lang=en>
<head>
<meta charset=UTF-8>
<meta name=viewport content=width=device-width, initial-scale=1>
<title>{$title} — OrbitCP Webmail</title>
<style>
  * { box-sizing: border-box; margin: 0; padding: 0; }
  body { font-family: system-ui, sans-serif; min-height: 100vh; display: flex; align-items: center;
         justify-content: center; background: #0f172a; color: #e2e8f0; padding: 1rem; }
  .card { background: #1e293b; border: 1px solid #334155; border-radius: 12px;
          padding: 2rem; max-width: 400px; width: 100%; text-align: center; }
  .icon { font-size: 2rem; margin-bottom: 1rem; }
  h1 { font-size: 1.1rem; font-weight: 600; margin-bottom: 0.5rem; color: {$color}; }
  p  { font-size: 0.875rem; color: #94a3b8; line-height: 1.6; }
  a  { display: inline-block; margin-top: 1.5rem; padding: 0.5rem 1.5rem;
       background: #3b82f6; color: #fff; border-radius: 8px; text-decoration: none;
       font-size: 0.875rem; font-weight: 500; }
</style>
</head>
<body>
<div class=card>
  <div class=icon>✉</div>
  <h1>{$title}</h1>
  <p>{$message}</p>
  <a href=/SOGo/>Open Webmail</a>
</div>
</body>
</html>
HTML;
}
