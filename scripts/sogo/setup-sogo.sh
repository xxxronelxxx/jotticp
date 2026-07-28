#!/bin/bash
# SOGo groupware setup script for JottiCP
# Run as root on the target server (Ubuntu 24.04 / noble)
set -e

echo "=== SOGo Groupware Setup for JottiCP ==="

# ── 1. Add SOGo repository ────────────────────────────────────────────────────
cat > /etc/apt/sources.list.d/sogo.list << EOF
deb [trusted=yes] https://packages.sogo.nu/nightly/5/ubuntu noble noble
EOF
apt-get update -qq

# ── 2. Install SOGo and dependencies ─────────────────────────────────────────
DEBIAN_FRONTEND=noninteractive apt-get install -y \
    sogo \
    sope4.9-gdl1-postgresql \
    memcached \
    postfix

# ── 3. PostgreSQL: create sogo DB + user ─────────────────────────────────────
sudo -u postgres psql -c "CREATE USER sogo WITH PASSWORD 'sogo_pass';" 2>/dev/null || true
sudo -u postgres psql -c "CREATE DATABASE sogo OWNER sogo ENCODING 'UTF8';" 2>/dev/null || true
sudo -u postgres psql sogo -c "GRANT ALL PRIVILEGES ON DATABASE sogo TO sogo;" 2>/dev/null || true
sudo -u postgres psql sogo -c "GRANT ALL ON SCHEMA public TO sogo;" 2>/dev/null || true

# ── 4. Create sogo_users table ────────────────────────────────────────────────
PGPASSWORD=sogo_pass psql -h 127.0.0.1 -U sogo -d sogo << 'SQL'
CREATE TABLE IF NOT EXISTS sogo_users (
    c_uid       VARCHAR(255) PRIMARY KEY,
    c_name      VARCHAR(255) NOT NULL,
    c_cn        VARCHAR(255) NOT NULL,
    c_password  TEXT        NOT NULL,
    c_active    SMALLINT    NOT NULL DEFAULT 1,
    mail        VARCHAR(255)
);
CREATE INDEX IF NOT EXISTS idx_sogo_users_mail ON sogo_users(mail);
SQL

# ── 5. Write sogo.conf ────────────────────────────────────────────────────────
cp "$(dirname "$0")/sogo.conf" /etc/sogo/sogo.conf
chown sogo:sogo /etc/sogo/sogo.conf
chmod 640 /etc/sogo/sogo.conf

# ── 6. Create runtime directories ────────────────────────────────────────────
mkdir -p /var/run/sogo /var/log/sogo /var/spool/sogo
chown sogo:sogo /var/run/sogo /var/log/sogo /var/spool/sogo
chmod 750 /var/run/sogo /var/log/sogo /var/spool/sogo

# ── 7. systemd service ────────────────────────────────────────────────────────
cat > /etc/systemd/system/sogo.service << 'UNIT'
[Unit]
Description=SOGo Groupware Server
After=network-online.target postgresql.service memcached.service
Wants=network-online.target

[Service]
Type=forking
User=sogo
Group=sogo
ExecStart=/usr/sbin/sogod
ExecStop=/bin/kill -TERM $MAINPID
PIDFile=/run/sogo/sogo.pid
Restart=on-failure
RestartSec=5s
RuntimeDirectory=sogo
RuntimeDirectoryMode=0750
StandardOutput=journal
StandardError=journal
SyslogIdentifier=sogo
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
UNIT

systemctl daemon-reload
systemctl enable sogo
systemctl start sogo

# ── 8. Install sync script ────────────────────────────────────────────────────
cp "$(dirname "$0")/sogo-sync-users.sh" /usr/local/bin/sogo-sync-users.sh
chmod +x /usr/local/bin/sogo-sync-users.sh

cat > /etc/cron.d/sogo-sync << 'EOF'
*/5 * * * * root /usr/local/bin/sogo-sync-users.sh >> /var/log/sogo/sync.log 2>&1
EOF

# ── 9. Install SSO bridge ─────────────────────────────────────────────────────
cp "$(dirname "$0")/sso.php" /var/www/webmail/sso.php

# ── 10. Configure auto-updates ────────────────────────────────────────────────
DEBIAN_FRONTEND=noninteractive apt-get install -y unattended-upgrades
cat >> /etc/apt/apt.conf.d/50unattended-upgrades << 'EOF'

Unattended-Upgrade::Origins-Pattern {
    "o=Inverse Ubuntu distribution,a=noble";
};
EOF

# ── 11. Initial user sync ─────────────────────────────────────────────────────
/usr/local/bin/sogo-sync-users.sh

sleep 3
systemctl is-active sogo && echo "SOGo is running OK" || echo "SOGo FAILED - check: journalctl -u sogo"
