<?php
// JottiCP overrides for Roundcube — loaded after config.inc.php.

// Force TLS to local Dovecot via STARTTLS on 143 (cheaper than 993 SSL on loopback).
$config["imap_host"]            = "tls://localhost:143";
$config["smtp_host"]            = "tls://localhost:587";

// We do not want users to see the host selector.
$config["default_host"]         = "tls://localhost:143";
$config["default_port"]         = 143;
$config["smtp_port"]            = 587;
$config["smtp_user"]            = "%u";
$config["smtp_pass"]            = "%p";

// Self-signed-friendly TLS verification for loopback only.
$config["imap_conn_options"] = [
  "ssl" => [
    "verify_peer"       => false,
    "verify_peer_name"  => false,
    "allow_self_signed" => true,
  ],
];
$config["smtp_conn_options"] = $config["imap_conn_options"];

// Plugins — managesieve is included in roundcube-core.
$config["plugins"] = ["archive", "zipdownload", "managesieve", "contextmenu"];

// Sieve via Dovecot ManageSieve on 4190.
$config["managesieve_host"]            = "tls://localhost:4190";
$config["managesieve_conn_options"]    = $config["imap_conn_options"];
$config["managesieve_default_headers"] = ["From", "To", "Cc", "Subject", "Sender", "List-Id"];

// Branding
$config["product_name"]   = "JottiCP Webmail";
$config["support_url"]    = "";
$config["skin"]           = "elastic";

// Reduce session lifetime to 30 minutes for control-panel context.
$config["session_lifetime"] = 30;

// Trust Cloudflare/nginx X-Forwarded-* (we are behind a reverse proxy).
$config["use_https"]      = false;
$config["force_https"]    = false;

// Auth/security
$config["des_key"]        = "7e28a42b9c439d3fd8fd3e76";  // CHANGE BELOW

// Logging
$config["log_driver"]     = "file";
$config["log_dir"]        = "/var/log/roundcube/";

// Disable signup/registration — accounts are managed by panel.
$config["enable_installer"] = false;
