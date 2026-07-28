export const sitesHelp = {
  title: "Websites",
  content: `## Websites
Manage all your hosted websites. Each website gets:
- Isolated PHP-FPM process (no cross-site access)
- Dedicated cgroup limits (memory + CPU)
- Automatic SSL certificate from Let's Encrypt

**Create a site:** Click "Add Website", enter your domain, choose PHP version and web server.

**Common tasks:**
- Change PHP version: Site → PHP tab → select version → Apply
- Force SSL renewal: Site → SSL tab → Renew Now
- View access logs: Site → Logs tab

**Tip:** JottiCP issues the SSL certificate during site creation — your site is HTTPS-ready immediately.`
};

export const emailHelp = {
  title: "Email",
  content: `## Email Accounts
Manage mailboxes across all your domains. Each account uses Dovecot IMAP/POP3 with Postfix SMTP.

**Create a mailbox:** Select a domain, click "Add Account", enter a username and password.

**Common tasks:**
- Change password: click the key icon next to an account
- View quota usage: shown in the usage bar per account
- Add a mail alias: click "Add Alias" — aliases forward to any address

**Protocols supported:**
- IMAP: port 993 (SSL)
- POP3: port 995 (SSL)
- SMTP submission: port 587 (STARTTLS)

**Tip:** SPF, DKIM, and DMARC records are auto-configured. Check the DNS tab if mail delivery fails.`
};

export const dnsHelp = {
  title: "DNS",
  content: `## DNS Management
Manage DNS zones and records for all your domains using JottiCP's built-in PowerDNS server.

**Add a record:** Select a zone, click "Add Record", fill in the type and value.

**Record types supported:** A, AAAA, MX, TXT, CNAME, NS, SRV, CAA

**Common tasks:**
- Add subdomain: Add an A record for sub.yourdomain.com
- Set up email: MX record pointing to your mail server
- Verify domain: Add TXT record with the verification string

**Propagation check:** Click "Check Propagation" to query 8.8.8.8 and 1.1.1.1 for each record.

**Tip:** TTL of 300 seconds is recommended during setup. Increase to 3600 once DNS is stable.`
};

export const sslHelp = {
  title: "SSL Certificates",
  content: `## SSL Certificates
All SSL certificates are issued automatically via Let's Encrypt ACME v2.

**Certificate lifecycle:**
- Issued within seconds of site creation
- Auto-renewed 30 days before expiry
- HSTS headers applied automatically

**Common tasks:**
- Force renewal: click "Renew Now" on any certificate
- Upload custom cert: click "Upload Custom Cert" (Pro)
- Wildcard certs: use DNS-01 challenge (requires DNS managed by JottiCP)

**Status indicators:**
- Active (green): valid, more than 30 days remaining
- Expiring soon (amber): fewer than 30 days remaining
- Expired (red): certificate has expired — renew immediately

**Tip:** If renewal fails, check that your domain's A record points to this server.`
};

export const databasesHelp = {
  title: "Databases",
  content: `## Databases
Manage all databases across all sites from one place.

**Supported database engines:**
- MySQL / MariaDB — standard relational DB
- PostgreSQL — advanced relational DB
- FerretDB — MongoDB-compatible on PostgreSQL
- SurrealDB — multi-model DB
- Valkey — Redis-compatible in-memory cache/store

**Create a database:** Select a site, click "Add Database", choose the engine.

**Common tasks:**
- Get connection string: click the copy icon next to any database
- Open phpMyAdmin: click "Open in PhpMyAdmin" (MySQL/MariaDB only)
- Check size: shown in the usage bar for each database

**Tip:** Database credentials are auto-generated. Use the "Copy connection string" button to get them into your app's .env file.`
};

export const fileManagerHelp = {
  title: "File Manager",
  content: `## File Manager
Browse, edit, upload, and manage files for all your sites.

**Navigation:**
- Left panel: directory tree
- Right panel: file list with size and modification date
- Click a text file to open it in the built-in editor

**Common tasks:**
- Upload files: drag & drop or use the Upload button
- Edit a file: click it to open in the code editor
- Create directory: right-click in the file list
- Set permissions: right-click → Permissions

**Supported editors:** HTML, CSS, JavaScript, PHP, Python, shell scripts, and all plain text files.

**Tip:** Large files (over 10 MB) cannot be edited in-browser. Use SFTP for large transfers.`
};

export const backupsHelp = {
  title: "Backups",
  content: `## Backups
Automated and on-demand backups for all your sites.

**Backup types:**
- Full: all files + databases
- Files only: site files without databases
- DB only: databases only

**Schedule:** Configure per-site backup schedules in the site detail → Backups tab.

**Common tasks:**
- Run backup now: click "Backup Now" for any site
- Restore: click "Restore" on any completed backup
- Download: click "Download" to save the backup archive locally

**Retention:** Backups are retained for 30 days by default. Adjust in Settings → Limits.

**Tip:** Test your restore procedure before you need it. Run a restore to a staging site monthly.`
};

export const cronHelp = {
  title: "Cron Jobs",
  content: `## Cron Jobs
Schedule recurring tasks to run automatically on your server.

**Cron syntax:** Standard 5-field cron expression: minute hour day month weekday.

**Common schedules:**
- \`0 * * * *\` — every hour
- \`0 0 * * *\` — daily at midnight
- \`*/15 * * * *\` — every 15 minutes
- \`0 0 * * 0\` — weekly on Sunday

**Create a job:** Click "Add Cron Job", enter the command and schedule.

**Commands:** Relative paths are resolved from the site's document root. Use full paths for system commands.

**Tip:** Use \`php /path/to/artisan schedule:run\` to integrate with Laravel Scheduler.`
};

export const phpHelp = {
  title: "PHP",
  content: `## PHP Management
Configure PHP versions and extensions per site.

**PHP versions available:** 7.4, 8.0, 8.1, 8.2, 8.3 (multiple versions installed via PHP-FPM)

**Common tasks:**
- Change PHP version: select from the dropdown and click Apply
- Toggle extensions: use the extension toggles list
- View phpinfo(): click "Show phpinfo()" to see the full configuration
- Flush OPcache: click "Flush OPcache" to clear cached bytecode

**Performance settings per site:**
- memory_limit: up to the plan limit
- max_execution_time: configurable per site
- upload_max_filesize: configurable per site

**Tip:** PHP-FPM runs each site under its own Unix user for complete process isolation.`
};

export const cacheHelp = {
  title: "Cache",
  content: `## Cache Management
Manage OPcache, Valkey (Redis-compatible), and browser cache settings.

**OPcache (PHP bytecode cache):**
- View hit rate, memory used, and cached script count
- Flush all cached bytecode with "Flush OPcache"
- Each site has its own OPcache namespace

**Valkey (application cache):**
- View keys count and memory usage per site
- Flush all keys for a site
- Connection string available for your app's .env

**Browser cache presets:**
- Aggressive: static assets cached 1 year (JS, CSS, images)
- Balanced: 1 week for static, no-cache for HTML
- Disabled: all responses set to no-store

**Tip:** Enable OPcache + Valkey for maximum PHP application performance (up to 10x faster).`
};

export const firewallHelp = {
  title: "Firewall",
  content: `## Firewall & Security
Manage nftables firewall rules and fail2ban intrusion prevention.

**Firewall rules:**
- View active nftables ruleset
- Add whitelist or blacklist entries by IP or CIDR range

**fail2ban jails:**
- SSH brute force: blocks IPs after 5 failed attempts
- Web auth: blocks IPs hitting login pages repeatedly
- Webmail: protects mail login endpoints

**Common tasks:**
- Unban an IP: enter the IP in the unban field and click "Unban"
- Whitelist your office IP: add it to the permanent whitelist
- View currently banned IPs: see the "Currently Blocked" table

**Tip:** Always whitelist your own static IP before enabling strict fail2ban rules to avoid locking yourself out.`
};

export const serversHelp = {
  title: "Servers",
  content: `## Servers
Add and manage remote servers from this panel. Each server runs the orbit-agent daemon.

**Adding a server:**
1. Click "Add Server", enter the hostname and IP
2. Copy the enrollment token shown
3. On the remote server: \`orbit-agent enroll --token <token> --panel <panel-url>\`
4. The server appears as Online within 30 seconds

**Monitoring:**
- CPU, RAM, and disk usage shown in real-time
- Click a server card to see detailed metrics

**Common tasks:**
- Test connectivity: click "Test Connection"
- View server logs: click "Logs"
- Remove a server: click "Remove" (does not uninstall orbit-agent)

**Tip:** One orbit-panel can manage unlimited servers. Add all your VPS instances here.`
};

export const usersHelp = {
  title: "Users",
  content: `## Users
Manage users and resellers. Use 'Login as User' to troubleshoot a user's account.

**User roles:**
- Admin: full access to all features and servers
- Reseller: can create sub-users, has resource limits
- User: accesses only their own sites and resources

**Common tasks:**
- Create user: click "Add User", enter email and role
- Suspend account: click "Suspend" — user cannot log in
- Login as user: click "Login As" to enter a user's session
- Return to admin: click the yellow banner → "Return to Admin"

**Reseller quotas:** Set disk, bandwidth, and site count limits per reseller in the edit form.

**Tip:** Use impersonation to debug user issues without asking for their password.`
};
