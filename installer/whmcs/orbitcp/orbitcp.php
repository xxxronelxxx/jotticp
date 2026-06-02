<?php
/**
 * OrbitCP WHMCS Provisioning Module
 *
 * Implements all 7 WHMCS module hooks for OrbitCP integration.
 * Place this directory in: /path/to/whmcs/modules/servers/orbitcp/
 *
 * Requires: WHMCS 8.x, PHP 8.1+, ext-curl, ext-json
 * API Version: 1.1
 */

if (!defined("WHMCS")) {
    die("This file cannot be accessed directly");
}

// ── Module metadata ──────────────────────────────────────────────────────────

function orbitcp_MetaData(): array {
    return [
        'DisplayName'    => 'OrbitCP Web Hosting',
        'APIVersion'     => '1.1',
        'RequiresServer' => true,
    ];
}

// ── Configuration options (shown in WHMCS product config) ───────────────────

function orbitcp_ConfigOptions(): array {
    return [
        'package_name' => [
            'FriendlyName' => 'OrbitCP Package Name',
            'Type'         => 'text',
            'Size'         => 25,
            'Default'      => 'basic',
            'Description'  => 'The OrbitCP package/plan name to assign',
        ],
        'disk_gb' => [
            'FriendlyName' => 'Disk Quota (GB)',
            'Type'         => 'text',
            'Size'         => 10,
            'Default'      => '10',
            'Description'  => 'Disk quota in GB',
        ],
        'max_emails' => [
            'FriendlyName' => 'Max Email Accounts',
            'Type'         => 'text',
            'Size'         => 10,
            'Default'      => '50',
        ],
        'max_databases' => [
            'FriendlyName' => 'Max Databases',
            'Type'         => 'text',
            'Size'         => 10,
            'Default'      => '10',
        ],
        'web_server' => [
            'FriendlyName' => 'Web Server',
            'Type'         => 'dropdown',
            'Options'      => 'openlitespeed,nginx,apache',
            'Default'      => 'openlitespeed',
            'Description'  => 'Web server to provision for new sites',
        ],
        'php_version' => [
            'FriendlyName' => 'Default PHP Version',
            'Type'         => 'dropdown',
            'Options'      => '8.4,8.3,8.2,8.1',
            'Default'      => '8.3',
        ],
    ];
}

// ── Internal API helper ──────────────────────────────────────────────────────

/**
 * Make an authenticated request to the OrbitCP REST API.
 *
 * @param  array  $params   WHMCS module $params (provides server credentials)
 * @param  string $method   HTTP verb: GET, POST, PUT, DELETE
 * @param  string $endpoint Path relative to /api/v1/ (e.g. "users")
 * @param  array  $data     Request body (JSON-encoded for non-GET requests)
 * @return array  Decoded JSON response, or ['error' => '...'] on failure
 */
function _orbitcp_api(array $params, string $method, string $endpoint, array $data = []): array {
    $serverUrl = rtrim($params['serverhostname'], '/');
    if (!str_starts_with($serverUrl, 'http')) {
        $serverUrl = 'https://' . $serverUrl . ':2087';
    }

    $url = "{$serverUrl}/api/v1/{$endpoint}";

    $ch = curl_init($url);
    curl_setopt_array($ch, [
        CURLOPT_RETURNTRANSFER => true,
        CURLOPT_SSL_VERIFYPEER => true,
        CURLOPT_SSL_VERIFYHOST => 2,
        CURLOPT_HTTPHEADER     => [
            'Authorization: Bearer ' . $params['serverpassword'],
            'Content-Type: application/json',
            'Accept: application/json',
            'X-OrbitCP-Client: whmcs-module/1.1',
        ],
        CURLOPT_TIMEOUT        => 30,
        CURLOPT_CUSTOMREQUEST  => strtoupper($method),
    ]);

    if (!empty($data) && $method !== 'GET') {
        curl_setopt($ch, CURLOPT_POSTFIELDS, json_encode($data));
    }

    $response = curl_exec($ch);
    $httpCode = curl_getinfo($ch, CURLINFO_HTTP_CODE);
    $curlErr  = curl_error($ch);
    curl_close($ch);

    if ($response === false) {
        return ['error' => "cURL failed: {$curlErr}"];
    }

    $decoded = json_decode($response, true);
    if ($httpCode >= 400) {
        $msg = $decoded['error'] ?? $decoded['message'] ?? "HTTP {$httpCode}";
        return ['error' => "OrbitCP API error ({$httpCode}): {$msg}"];
    }

    return $decoded ?? [];
}

// ── Hook 1 — CreateAccount ───────────────────────────────────────────────────

/**
 * Provision a new hosting account.
 *
 * Creates an OrbitCP user then a site under that user. Stores the OrbitCP
 * user ID in the WHMCS service `username` field for subsequent hooks.
 *
 * @return string "success" or "error: <message>"
 */
function orbitcp_CreateAccount(array $params): string {
    // 1. Create OrbitCP user account
    $userResult = _orbitcp_api($params, 'POST', 'users', [
        'email'    => $params['clientsdetails']['email'],
        'password' => $params['password'],
        'role'     => 'user',
        'plan'     => $params['configoptions']['package_name'] ?? 'basic',
    ]);

    if (isset($userResult['error'])) {
        return 'error: ' . $userResult['error'];
    }

    $userId = $userResult['id'] ?? null;

    // 2. Create the primary site
    $siteResult = _orbitcp_api($params, 'POST', 'sites', [
        'domain'      => $params['domain'],
        'owner_id'    => $userId,
        'disk_quota'  => (int)($params['configoptions']['disk_gb'] ?? 10) * 1024,
        'web_server'  => $params['configoptions']['web_server'] ?? 'openlitespeed',
        'php_version' => $params['configoptions']['php_version'] ?? '8.3',
    ]);

    if (isset($siteResult['error'])) {
        // Attempt rollback: delete the user we just created
        if ($userId) {
            _orbitcp_api($params, 'DELETE', "users/{$userId}");
        }
        return 'error: ' . $siteResult['error'];
    }

    // 3. Persist the OrbitCP user ID in WHMCS so other hooks can reference it
    if ($userId) {
        try {
            Capsule::table('tblhosting')
                ->where('id', $params['serviceid'])
                ->update(['username' => $userId]);
        } catch (\Exception $e) {
            // Non-fatal: log and continue (account was created successfully)
            logModuleCall('orbitcp', 'CreateAccount', $params, $e->getMessage());
        }
    }

    logModuleCall('orbitcp', 'CreateAccount', $params, $userResult, [], []);
    return 'success';
}

// ── Hook 2 — SuspendAccount ──────────────────────────────────────────────────

/**
 * Suspend a hosting account (disable login + site access).
 *
 * @return string "success" or "error: <message>"
 */
function orbitcp_SuspendAccount(array $params): string {
    $userId = $params['username'];
    if (empty($userId)) {
        return 'error: No OrbitCP user ID on record — account may not have been provisioned via this module';
    }

    $result = _orbitcp_api($params, 'POST', "users/{$userId}/suspend", [
        'reason' => $params['suspendreason'] ?? 'Suspended via WHMCS',
    ]);

    logModuleCall('orbitcp', 'SuspendAccount', $params, $result, [], []);
    return isset($result['error']) ? 'error: ' . $result['error'] : 'success';
}

// ── Hook 3 — UnsuspendAccount ────────────────────────────────────────────────

/**
 * Unsuspend a previously suspended account.
 *
 * @return string "success" or "error: <message>"
 */
function orbitcp_UnsuspendAccount(array $params): string {
    $userId = $params['username'];
    if (empty($userId)) {
        return 'error: No OrbitCP user ID on record';
    }

    $result = _orbitcp_api($params, 'POST', "users/{$userId}/unsuspend");

    logModuleCall('orbitcp', 'UnsuspendAccount', $params, $result, [], []);
    return isset($result['error']) ? 'error: ' . $result['error'] : 'success';
}

// ── Hook 4 — TerminateAccount ────────────────────────────────────────────────

/**
 * Permanently delete a hosting account and all its sites.
 *
 * Deletion order: sites → user. This matches OrbitCP's constraint that
 * a user with active sites cannot be deleted.
 *
 * @return string "success" or "error: <message>"
 */
function orbitcp_TerminateAccount(array $params): string {
    $userId = $params['username'];
    if (empty($userId)) {
        return 'error: No OrbitCP user ID on record';
    }

    // 1. List and delete all sites belonging to this user
    $sites = _orbitcp_api($params, 'GET', "users/{$userId}/sites");
    foreach ($sites['sites'] ?? [] as $site) {
        $delResult = _orbitcp_api($params, 'DELETE', "sites/{$site['id']}");
        if (isset($delResult['error'])) {
            logModuleCall('orbitcp', 'TerminateAccount:DeleteSite', $site['id'], $delResult, [], []);
            // Continue attempting to delete remaining sites
        }
    }

    // 2. Delete the user
    $result = _orbitcp_api($params, 'DELETE', "users/{$userId}");

    logModuleCall('orbitcp', 'TerminateAccount', $params, $result, [], []);
    return isset($result['error']) ? 'error: ' . $result['error'] : 'success';
}

// ── Hook 5 — ChangePackage ───────────────────────────────────────────────────

/**
 * Upgrade or downgrade the hosting package.
 *
 * Updates the disk quota and plan name on the OrbitCP user record.
 *
 * @return string "success" or "error: <message>"
 */
function orbitcp_ChangePackage(array $params): string {
    $userId = $params['username'];
    if (empty($userId)) {
        return 'error: No OrbitCP user ID on record';
    }

    $result = _orbitcp_api($params, 'PUT', "users/{$userId}", [
        'plan'       => $params['configoptions']['package_name'] ?? 'basic',
        'disk_quota' => (int)($params['configoptions']['disk_gb'] ?? 10) * 1024,
    ]);

    logModuleCall('orbitcp', 'ChangePackage', $params, $result, [], []);
    return isset($result['error']) ? 'error: ' . $result['error'] : 'success';
}

// ── Hook 6 — UsageUpdate ─────────────────────────────────────────────────────

/**
 * Fetch current resource usage for display in WHMCS.
 *
 * Called by WHMCS nightly to update the usage graph.
 *
 * @return array ['diskusage' => int, 'disklimit' => int, 'bwusage' => int, 'bwlimit' => int]
 */
function orbitcp_UsageUpdate(array $params): array {
    $userId = $params['username'];
    if (empty($userId)) {
        return ['diskusage' => 0, 'disklimit' => 0, 'bwusage' => 0, 'bwlimit' => 0];
    }

    $stats = _orbitcp_api($params, 'GET', "users/{$userId}/stats");

    $diskUsedMb  = (int)(($stats['disk_used_mb'] ?? 0));
    $diskLimitGb = (int)($params['configoptions']['disk_gb'] ?? 10);

    return [
        'diskusage' => $diskUsedMb,
        'disklimit' => $diskLimitGb * 1024,
        'bwusage'   => (int)($stats['bandwidth_used_mb'] ?? 0),
        'bwlimit'   => 0, // OrbitCP does not enforce hard bandwidth caps
    ];
}

// ── Hook 7 — LoginLink ───────────────────────────────────────────────────────

/**
 * Generate a single-sign-on link for the client portal.
 *
 * WHMCS calls this to render a "Login to Control Panel" button on the
 * client area service details page. The link contains a short-lived
 * impersonation token (5-minute TTL) issued by OrbitCP.
 *
 * Outputs an HTML anchor tag and returns an empty string (WHMCS convention).
 *
 * @return string Always ""; HTML is echoed directly.
 */
function orbitcp_LoginLink(array $params): string {
    $userId = $params['username'];
    if (empty($userId)) {
        echo '<span class="text-muted">SSO unavailable — no user ID on record</span>';
        return '';
    }

    $result = _orbitcp_api($params, 'POST', "users/{$userId}/impersonate");

    if (isset($result['error']) || empty($result['token'])) {
        echo '<span class="text-muted">Unable to generate login link</span>';
        return '';
    }

    $serverUrl = rtrim($params['serverhostname'], '/');
    if (!str_starts_with($serverUrl, 'http')) {
        $serverUrl = 'https://' . $serverUrl . ':2087';
    }

    $ssoUrl = htmlspecialchars(
        "{$serverUrl}/sso?token={$result['token']}",
        ENT_QUOTES | ENT_HTML5,
        'UTF-8'
    );

    echo "<a href=\"{$ssoUrl}\" target=\"_blank\" rel=\"noopener noreferrer\" "
       . "class=\"btn btn-default btn-sm\">"
       . "Login to OrbitCP"
       . "</a>";

    return '';
}
