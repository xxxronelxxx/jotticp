/// Returns the full list of MCP tool definitions with JSON Schema for inputSchema.
pub fn tool_definitions() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "name": "list_sites",
            "description": "List all websites managed by OrbitCP",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "search": {
                        "type": "string",
                        "description": "Optional search/filter term (matches domain name)"
                    }
                }
            }
        }),
        serde_json::json!({
            "name": "create_site",
            "description": "Create a new website / virtual host in OrbitCP",
            "inputSchema": {
                "type": "object",
                "required": ["domain", "php_version", "web_server", "plan"],
                "properties": {
                    "domain": {
                        "type": "string",
                        "description": "Primary domain name for the site (e.g. example.com)"
                    },
                    "php_version": {
                        "type": "string",
                        "description": "PHP version to use (e.g. '8.3', '8.2')"
                    },
                    "web_server": {
                        "type": "string",
                        "enum": ["ols", "nginx", "apache"],
                        "description": "Web server backend"
                    },
                    "plan": {
                        "type": "string",
                        "enum": ["community", "pro"],
                        "description": "Hosting plan"
                    },
                    "server_id": {
                        "type": "string",
                        "description": "UUID of the target server (omit for local server)"
                    },
                    "disk_quota_mb": {
                        "type": "integer",
                        "description": "Disk quota in megabytes"
                    }
                }
            }
        }),
        serde_json::json!({
            "name": "delete_site",
            "description": "Delete a website and all its resources from OrbitCP",
            "inputSchema": {
                "type": "object",
                "required": ["site_id"],
                "properties": {
                    "site_id": {
                        "type": "string",
                        "description": "UUID of the site to delete"
                    }
                }
            }
        }),
        serde_json::json!({
            "name": "get_site_stats",
            "description": "Get current CPU, RAM, and disk usage statistics for a site",
            "inputSchema": {
                "type": "object",
                "required": ["site_id"],
                "properties": {
                    "site_id": {
                        "type": "string",
                        "description": "UUID of the site"
                    }
                }
            }
        }),
        serde_json::json!({
            "name": "list_email_accounts",
            "description": "List all email accounts managed by OrbitCP",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "domain": {
                        "type": "string",
                        "description": "Filter by domain (e.g. example.com)"
                    }
                }
            }
        }),
        serde_json::json!({
            "name": "create_email_account",
            "description": "Create a new email account (mailbox)",
            "inputSchema": {
                "type": "object",
                "required": ["local_part", "domain", "password"],
                "properties": {
                    "local_part": {
                        "type": "string",
                        "description": "Local part of the email address (before the @)"
                    },
                    "domain": {
                        "type": "string",
                        "description": "Email domain"
                    },
                    "password": {
                        "type": "string",
                        "description": "Mailbox password (min 12 chars)"
                    },
                    "quota_mb": {
                        "type": "integer",
                        "description": "Mailbox quota in megabytes (default: 1024)"
                    }
                }
            }
        }),
        serde_json::json!({
            "name": "delete_email_account",
            "description": "Delete an email account and its mailbox",
            "inputSchema": {
                "type": "object",
                "required": ["account_id"],
                "properties": {
                    "account_id": {
                        "type": "string",
                        "description": "UUID of the email account to delete"
                    }
                }
            }
        }),
        serde_json::json!({
            "name": "list_dns_zones",
            "description": "List all DNS zones managed by OrbitCP",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "search": {
                        "type": "string",
                        "description": "Optional search term to filter zone names"
                    }
                }
            }
        }),
        serde_json::json!({
            "name": "create_dns_record",
            "description": "Create a new DNS record in a zone",
            "inputSchema": {
                "type": "object",
                "required": ["zone_id", "type", "name", "content"],
                "properties": {
                    "zone_id": {
                        "type": "string",
                        "description": "UUID of the DNS zone"
                    },
                    "type": {
                        "type": "string",
                        "enum": ["A", "AAAA", "CNAME", "MX", "TXT", "SRV", "CAA", "PTR"],
                        "description": "DNS record type"
                    },
                    "name": {
                        "type": "string",
                        "description": "Record name (relative to zone, or FQDN)"
                    },
                    "content": {
                        "type": "string",
                        "description": "Record value/content"
                    },
                    "ttl": {
                        "type": "integer",
                        "description": "TTL in seconds (default: 300)"
                    },
                    "priority": {
                        "type": "integer",
                        "description": "Record priority (MX/SRV only)"
                    }
                }
            }
        }),
        serde_json::json!({
            "name": "list_ssl_certs",
            "description": "List all SSL/TLS certificates managed by OrbitCP",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "expiring_soon": {
                        "type": "boolean",
                        "description": "If true, only return certs expiring within 30 days"
                    }
                }
            }
        }),
        serde_json::json!({
            "name": "renew_ssl",
            "description": "Trigger immediate SSL certificate renewal for a site",
            "inputSchema": {
                "type": "object",
                "required": ["cert_id"],
                "properties": {
                    "cert_id": {
                        "type": "string",
                        "description": "UUID of the SSL certificate to renew"
                    }
                }
            }
        }),
        serde_json::json!({
            "name": "list_databases",
            "description": "List all databases managed by OrbitCP",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "site_id": {
                        "type": "string",
                        "description": "Filter by site UUID"
                    }
                }
            }
        }),
        serde_json::json!({
            "name": "create_database",
            "description": "Create a new database for a site",
            "inputSchema": {
                "type": "object",
                "required": ["site_id", "db_type", "db_name"],
                "properties": {
                    "site_id": {
                        "type": "string",
                        "description": "UUID of the site that owns this database"
                    },
                    "db_type": {
                        "type": "string",
                        "enum": ["mysql", "mariadb", "postgresql"],
                        "description": "Database engine"
                    },
                    "db_name": {
                        "type": "string",
                        "description": "Database name (will be prefixed with site unix user)"
                    }
                }
            }
        }),
        serde_json::json!({
            "name": "trigger_backup",
            "description": "Trigger an immediate backup of a site",
            "inputSchema": {
                "type": "object",
                "required": ["site_id"],
                "properties": {
                    "site_id": {
                        "type": "string",
                        "description": "UUID of the site to back up"
                    },
                    "type": {
                        "type": "string",
                        "enum": ["full", "files", "database"],
                        "description": "Type of backup (default: full)"
                    }
                }
            }
        }),
        serde_json::json!({
            "name": "get_server_stats",
            "description": "Get current resource usage statistics for all managed servers",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "server_id": {
                        "type": "string",
                        "description": "UUID of a specific server (omit for all servers)"
                    }
                }
            }
        }),
        serde_json::json!({
            "name": "install_app",
            "description": "Install a one-click application (WordPress, Nextcloud, etc.) on a site",
            "inputSchema": {
                "type": "object",
                "required": ["site_id", "app_id"],
                "properties": {
                    "site_id": {
                        "type": "string",
                        "description": "UUID of the site to install the app on"
                    },
                    "app_id": {
                        "type": "string",
                        "description": "Application identifier (e.g. 'wordpress', 'nextcloud', 'gitea')"
                    },
                    "admin_user": {
                        "type": "string",
                        "description": "Admin username for the new app installation"
                    },
                    "admin_email": {
                        "type": "string",
                        "description": "Admin email for the new app installation"
                    },
                    "admin_password": {
                        "type": "string",
                        "description": "Admin password (min 12 chars). Omit to auto-generate."
                    }
                }
            }
        }),
        serde_json::json!({
            "name": "list_installed_apps",
            "description": "List all installed one-click applications",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "site_id": {
                        "type": "string",
                        "description": "Filter by site UUID"
                    }
                }
            }
        }),
        serde_json::json!({
            "name": "list_cron_jobs",
            "description": "List all cron jobs configured in OrbitCP",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "site_id": {
                        "type": "string",
                        "description": "Filter cron jobs by site UUID"
                    }
                }
            }
        }),
        serde_json::json!({
            "name": "create_cron_job",
            "description": "Create a new cron job for a site",
            "inputSchema": {
                "type": "object",
                "required": ["site_id", "schedule", "command"],
                "properties": {
                    "site_id": {
                        "type": "string",
                        "description": "UUID of the site that owns this cron job"
                    },
                    "schedule": {
                        "type": "string",
                        "description": "Cron schedule expression (e.g. '0 * * * *' for hourly)"
                    },
                    "command": {
                        "type": "string",
                        "description": "Command to execute (relative to site home or absolute)"
                    },
                    "label": {
                        "type": "string",
                        "description": "Human-friendly label for this job"
                    }
                }
            }
        }),
        serde_json::json!({
            "name": "get_system_logs",
            "description": "Retrieve recent system audit log entries",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of entries to return (default: 50, max: 200)"
                    },
                    "action": {
                        "type": "string",
                        "description": "Filter by action type (e.g. 'site.create', 'backup.trigger')"
                    },
                    "user_id": {
                        "type": "string",
                        "description": "Filter by user UUID"
                    }
                }
            }
        }),
    ]
}
