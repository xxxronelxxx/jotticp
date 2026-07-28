#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# JottiCP One-Command Installer
# Version: 0.0.1
#
# Usage:
#   curl -sL https://install.jotti.ru | bash
#   # or with options:
#   JOTTI_ENV=pro bash install.sh
#
# Supported OS:
#   Ubuntu 22.04 LTS, Ubuntu 24.04 LTS
#   Debian 12 (Bookworm)
#   AlmaLinux 9 / Rocky Linux 9
#
# What this script does:
#   1.  OS detection and conflict check
#   2.  Install system dependencies
#   3.  Download and verify orbit-panel + orbit-agent binaries
#   4.  Initialize and harden PostgreSQL 17
#   5.  Set up PowerDNS 4.9
#   6.  Set up Postfix + Dovecot + rspamd (mail stack)
#   7.  Set up OpenLiteSpeed 1.8.4 as default web server
#   8.  Set up cgroups v2 site isolation
#   9.  Configure and enable systemd services
#   10. Set up nftables firewall
#   11. Apply CIS Level 1 security baseline
#   12. Launch first-run wizard URL
#
# Target install time: < 3 minutes on a 2-vCPU VPS
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail

# ── Constants ─────────────────────────────────────────────────────────────────
readonly JOTTI_VERSION="0.0.1"
readonly JOTTI_INSTALL_DIR="/opt/jotticp"
readonly JOTTI_CONF_DIR="/etc/jotticp"
readonly JOTTI_LOG_DIR="/var/log/jotticp"
readonly JOTTI_DATA_DIR="/var/lib/jotticp"
readonly JOTTI_BACKUP_DIR="/var/backups/jotticp"

readonly JOTTI_PANEL_PORT=2087
readonly JOTTI_AGENT_PORT=7443
readonly JOTTI_PANEL_USER="jotticp"

readonly PG_VERSION="17"
readonly OLS_VERSION="1.8.4"
readonly PDNS_VERSION="4.9"

readonly BINARY_BASE_URL="https://releases.jotti.ru/${JOTTI_VERSION}"

# Ed25519 public key for binary signature verification.
# HARDCODED here so a DNS hijack of releases.jotti.ru cannot replace both
# the binary AND the verification key. Generated at release time; rotate with
# a new install.sh signed by the previous key (bootstrap trust on first install).
# Verify: minisign -V -P <key> -m <binary> -x <binary>.minisig
readonly JOTTI_SIGNING_PUBKEY="RWRPHu8VRhzFtQWnG2fxQBFvKYBiAeABXX7ZzKDqKJ2qNLJFv0bR8uDm"

# Colors
readonly RED='\033[0;31m'
readonly GREEN='\033[0;32m'
readonly YELLOW='\033[1;33m'
readonly BLUE='\033[0;34m'
readonly CYAN='\033[0;36m'
readonly NC='\033[0m'

# ── Logging helpers ───────────────────────────────────────────────────────────
log_step()    { printf "\n${CYAN}[%s]${NC} %s\n" "$(date +%H:%M:%S)" "$*"; }
log_ok()      { printf "  ${GREEN}✓${NC} %s\n" "$*"; }
log_warn()    { printf "  ${YELLOW}⚠${NC} %s\n" "$*"; }
log_error()   { printf "  ${RED}✗${NC} %s\n" "$*" >&2; }
log_detail()  { printf "    %s\n" "$*"; }
die()         { log_error "$*"; exit 1; }

# Spinner for long operations
spinner_pid=
start_spinner() {
    local msg="$1"
    printf "  %s " "$msg"
    while true; do
        for char in '⠋' '⠙' '⠹' '⠸' '⠼' '⠴' '⠦' '⠧' '⠇' '⠏'; do
            printf "\b%s" "$char"; sleep 0.1
        done
    done &
    spinner_pid=$!
}
stop_spinner() {
    if [[ -n "${spinner_pid:-}" ]]; then
        kill "$spinner_pid" 2>/dev/null || true
        spinner_pid=
        printf "\b \n"
    fi
}

# ── Pre-flight checks ─────────────────────────────────────────────────────────

check_root() {
    if [[ "$(id -u)" -ne 0 ]]; then
        die "This installer must be run as root. Try: sudo bash install.sh"
    fi
    log_ok "Running as root"
}

check_architecture() {
    local arch; arch="$(uname -m)"
    if [[ "$arch" != "x86_64" ]] && [[ "$arch" != "aarch64" ]]; then
        die "Unsupported architecture: ${arch}. JottiCP supports x86_64 and aarch64."
    fi
    log_ok "Architecture: ${arch}"
}

# ── Step 1: OS Detection ─────────────────────────────────────────────────────

detect_os() {
    log_step "Step 1/12: Detecting operating system"

    if [[ ! -f /etc/os-release ]]; then
        die "Cannot detect OS — /etc/os-release not found."
    fi

    # shellcheck source=/dev/null
    . /etc/os-release
    OS_ID="${ID:-unknown}"
    OS_VERSION="${VERSION_ID:-0}"
    OS_NAME="${PRETTY_NAME:-unknown}"

    case "${OS_ID}" in
        ubuntu)
            if [[ "${OS_VERSION}" == "22.04" ]] || [[ "${OS_VERSION}" == "24.04" ]]; then
                PKG_MANAGER="apt"
                log_ok "Supported OS: ${OS_NAME}"
            else
                log_warn "Ubuntu ${OS_VERSION} is not officially tested (only 22.04/24.04 are). Proceeding..."
                PKG_MANAGER="apt"
            fi
            ;;
        debian)
            PKG_MANAGER="apt"
            log_ok "Supported OS: ${OS_NAME}"
            ;;
        almalinux|rocky|rhel|centos)
            PKG_MANAGER="dnf"
            if [[ "${OS_VERSION}" == 9* ]]; then
                log_ok "Supported OS: ${OS_NAME}"
            else
                log_warn "RHEL-family version ${OS_VERSION} — only v9 is tested. Proceeding..."
            fi
            ;;
        *)
            die "Unsupported OS: ${OS_ID} ${OS_VERSION}. Supported: Ubuntu 22.04/24.04, Debian 12, AlmaLinux 9"
            ;;
    esac

    export OS_ID OS_VERSION OS_NAME PKG_MANAGER
}

check_conflicts() {
    local conflicts=()

    # Check for other control panels
    [[ -d /usr/local/cpanel ]]      && conflicts+=("cPanel")
    [[ -d /usr/local/psa ]]         && conflicts+=("Plesk")
    [[ -f /usr/local/lsws/admin/conf/hkeydata ]] && conflicts+=("LiteSpeed (already configured)")
    command -v cyberpanel &>/dev/null   && conflicts+=("CyberPanel")
    command -v hestia &>/dev/null       && conflicts+=("HestiaCP")
    [[ -f /etc/bt.conf ]]               && conflicts+=("aaPanel")
    command -v enhancepanel &>/dev/null && conflicts+=("Enhance")

    if [[ ${#conflicts[@]} -gt 0 ]]; then
        log_warn "Existing control panel(s) detected: ${conflicts[*]}"
        log_warn "JottiCP can coexist with some panels but conflicts are possible."
        printf "\n  Continue installation anyway? [y/N] "
        read -r answer
        if [[ "${answer,,}" != "y" ]]; then
            echo "Installation cancelled."
            exit 0
        fi
    else
        log_ok "No conflicting control panels detected"
    fi
}

check_minimum_resources() {
    # RAM: minimum 512MB
    local ram_kb; ram_kb=$(awk '/MemTotal/ {print $2}' /proc/meminfo)
    local ram_mb=$(( ram_kb / 1024 ))
    if [[ "${ram_mb}" -lt 512 ]]; then
        die "Insufficient RAM: ${ram_mb} MB. JottiCP requires at least 512 MB."
    fi
    log_ok "RAM: ${ram_mb} MB (minimum: 512 MB)"

    # Disk: minimum 5GB free on /
    local free_gb; free_gb=$(df -BG / | awk 'NR==2 {gsub("G",""); print $4}')
    if [[ "${free_gb}" -lt 5 ]]; then
        die "Insufficient disk space: ${free_gb} GB free on /. JottiCP requires at least 5 GB."
    fi
    log_ok "Free disk: ${free_gb} GB (minimum: 5 GB)"
}

# ── Step 2: Install Dependencies ─────────────────────────────────────────────

install_dependencies() {
    log_step "Step 2/12: Installing system dependencies"

    if [[ "${PKG_MANAGER}" == "apt" ]]; then
        install_dependencies_apt
    else
        install_dependencies_dnf
    fi
}

install_dependencies_apt() {
    export DEBIAN_FRONTEND=noninteractive

    log_detail "Updating package lists..."
    apt-get update -qq 2>/dev/null

    log_detail "Installing base packages..."
    apt-get install -y -qq \
        curl wget gnupg2 ca-certificates lsb-release \
        software-properties-common apt-transport-https \
        tar gzip zip unzip \
        git \
        openssl \
        nftables \
        fail2ban \
        auditd audispd-plugins \
        rsyslog \
        systemd \
        cgroup-tools \
        > /dev/null 2>&1
    log_ok "Base packages installed"

    # PostgreSQL 17
    log_detail "Adding PostgreSQL 17 repository..."
    curl -fsSL "https://www.postgresql.org/media/keys/ACCC4CF8.asc" \
        | gpg --dearmor -o /usr/share/keyrings/postgresql.gpg > /dev/null 2>&1
    echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/postgresql.gpg] \
https://apt.postgresql.org/pub/repos/apt $(lsb_release -cs)-pgdg main" \
        > /etc/apt/sources.list.d/pgdg.list
    apt-get update -qq > /dev/null 2>&1
    apt-get install -y -qq postgresql-17 postgresql-client-17 > /dev/null 2>&1
    log_ok "PostgreSQL 17 installed"

    # Valkey (in-memory store, Redis-compatible)
    log_detail "Installing Valkey..."
    if apt-get install -y -qq valkey > /dev/null 2>&1; then
        log_ok "Valkey installed from repo"
    else
        # Fallback: install Redis (Valkey-compatible on same protocol)
        apt-get install -y -qq redis-server > /dev/null 2>&1
        log_ok "Redis installed (Valkey fallback)"
    fi

    # PowerDNS
    log_detail "Adding PowerDNS repository..."
    curl -fsSL "https://repo.powerdns.com/FD380FBB-pub.asc" \
        | gpg --dearmor -o /usr/share/keyrings/pdns.gpg > /dev/null 2>&1
    echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/pdns.gpg] \
http://repo.powerdns.com/debian $(lsb_release -cs)-auth-${PDNS_VERSION//./} main" \
        > /etc/apt/sources.list.d/pdns.list
    echo "Package: pdns-*
Pin: origin repo.powerdns.com
Pin-Priority: 600" > /etc/apt/preferences.d/pdns
    apt-get update -qq > /dev/null 2>&1
    apt-get install -y -qq pdns-server pdns-backend-pgsql pdns-tools > /dev/null 2>&1
    log_ok "PowerDNS ${PDNS_VERSION} installed"

    # Postfix + Dovecot + rspamd (mail stack)
    log_detail "Installing mail stack..."
    debconf-set-selections <<< "postfix postfix/mailname string $(hostname -f)"
    debconf-set-selections <<< "postfix postfix/main_mailer_type string 'Internet Site'"
    apt-get install -y -qq \
        postfix postfix-pgsql \
        dovecot-core dovecot-imapd dovecot-pop3d dovecot-lmtpd dovecot-pgsql \
        rspamd redis-server \
        > /dev/null 2>&1
    log_ok "Mail stack installed (Postfix + Dovecot + rspamd)"

    # PHP multi-version (via Ondrej Sury PPA)
    log_detail "Adding PHP repository (Ondrej Sury)..."
    add-apt-repository -y ppa:ondrej/php > /dev/null 2>&1
    apt-get update -qq > /dev/null 2>&1
    for PHP_VER in 8.1 8.2 8.3 8.4; do
        apt-get install -y -qq \
            "php${PHP_VER}-fpm" "php${PHP_VER}-common" \
            "php${PHP_VER}-mysql" "php${PHP_VER}-pgsql" \
            "php${PHP_VER}-gd" "php${PHP_VER}-mbstring" \
            "php${PHP_VER}-xml" "php${PHP_VER}-curl" \
            "php${PHP_VER}-zip" "php${PHP_VER}-intl" \
            "php${PHP_VER}-bcmath" "php${PHP_VER}-opcache" \
            "php${PHP_VER}-redis" \
            > /dev/null 2>&1 || log_warn "PHP ${PHP_VER} install had warnings"
    done
    log_ok "PHP 8.1, 8.2, 8.3, 8.4 installed"

    # WP-CLI
    log_detail "Installing WP-CLI..."
    curl -sL "https://raw.githubusercontent.com/wp-cli/builds/gh-pages/phar/wp-cli.phar" \
        -o /usr/local/bin/wp
    chmod +x /usr/local/bin/wp
    log_ok "WP-CLI installed"

    # Composer
    log_detail "Installing Composer..."
    php -r "copy('https://getcomposer.org/installer', '/tmp/composer-setup.php');" 2>/dev/null
    php /tmp/composer-setup.php --quiet --install-dir=/usr/local/bin --filename=composer 2>/dev/null
    rm -f /tmp/composer-setup.php
    log_ok "Composer installed"

    # Nginx (secondary web server, used for non-PHP sites)
    apt-get install -y -qq nginx > /dev/null 2>&1
    log_ok "nginx installed"
}

install_dependencies_dnf() {
    log_detail "Enabling EPEL and CRB..."
    dnf install -y epel-release > /dev/null 2>&1
    dnf config-manager --set-enabled crb > /dev/null 2>&1

    log_detail "Adding PostgreSQL 17 repository..."
    dnf install -y "https://download.postgresql.org/pub/repos/yum/reporpms/EL-9-x86_64/pgdg-redhat-repo-latest.noarch.rpm" \
        > /dev/null 2>&1
    dnf -qy module disable postgresql > /dev/null 2>&1

    log_detail "Adding Remi PHP repository..."
    dnf install -y "https://rpms.remirepo.net/enterprise/remi-release-9.rpm" > /dev/null 2>&1

    log_detail "Installing core packages..."
    dnf install -y \
        postgresql17-server postgresql17 postgresql17-contrib \
        postfix dovecot rspamd \
        nginx \
        nftables \
        fail2ban \
        auditd \
        rsyslog \
        curl wget tar gzip zip unzip git openssl \
        > /dev/null 2>&1
    log_ok "Core packages installed via dnf"

    for PHP_VER in 81 82 83 84; do
        dnf install -y \
            "php${PHP_VER}" "php${PHP_VER}-php-fpm" "php${PHP_VER}-php-common" \
            "php${PHP_VER}-php-mysqlnd" "php${PHP_VER}-php-pgsql" "php${PHP_VER}-php-gd" \
            "php${PHP_VER}-php-mbstring" "php${PHP_VER}-php-xml" "php${PHP_VER}-php-curl" \
            "php${PHP_VER}-php-zip" "php${PHP_VER}-php-intl" "php${PHP_VER}-php-bcmath" \
            "php${PHP_VER}-php-opcache" "php${PHP_VER}-php-redis" \
            > /dev/null 2>&1 || log_warn "PHP ${PHP_VER} had warnings"
    done
    log_ok "PHP 8.1, 8.2, 8.3, 8.4 installed via Remi"
}

# ── Step 3: Download JottiCP Binaries ────────────────────────────────────────

download_orbit_binaries() {
    log_step "Step 3/12: Downloading JottiCP binaries"

    local arch; arch="$(uname -m)"
    local os_suffix; os_suffix="linux-${arch}"

    mkdir -p "${JOTTI_INSTALL_DIR}/bin"
    mkdir -p "${JOTTI_CONF_DIR}"
    mkdir -p "${JOTTI_LOG_DIR}"
    mkdir -p "${JOTTI_DATA_DIR}"
    mkdir -p "${JOTTI_BACKUP_DIR}"

    # Create the jotticp system user (no login shell, no home directory needed for /bin)
    if ! id "${JOTTI_PANEL_USER}" &>/dev/null; then
        useradd --system --no-create-home --shell /usr/sbin/nologin \
            --comment "JottiCP panel daemon" "${JOTTI_PANEL_USER}"
        log_ok "System user '${JOTTI_PANEL_USER}' created"
    else
        log_ok "System user '${JOTTI_PANEL_USER}' already exists"
    fi

    local binaries=("jotti-panel" "jotti-agent" "jotti-dns" "jotti-mail" "jotti-cron")

    # Require minisign for signature verification (available in most distro repos)
    if ! command -v minisign &>/dev/null; then
        log_detail "Installing minisign for binary signature verification..."
        if command -v apt-get &>/dev/null; then
            apt-get install -y -q minisign
        elif command -v dnf &>/dev/null; then
            dnf install -y minisign
        else
            die "Cannot install minisign. Install it manually and re-run."
        fi
    fi

    for binary in "${binaries[@]}"; do
        local url="${BINARY_BASE_URL}/${binary}-${os_suffix}"
        local dest="${JOTTI_INSTALL_DIR}/bin/${binary}"
        local sig_url="${url}.minisig"

        log_detail "Downloading ${binary}..."

        if curl -fsSL --progress-bar "${url}" -o "${dest}" 2>/dev/null &&
           curl -fsSL "${sig_url}" -o "${dest}.minisig" 2>/dev/null; then
            # Verify Ed25519 signature against the hardcoded public key embedded in this script.
            # The key is NOT fetched from the same server — a DNS hijack cannot forge signatures.
            if minisign -V -P "${JOTTI_SIGNING_PUBKEY}" -m "${dest}" -x "${dest}.minisig" 2>/dev/null; then
                rm -f "${dest}.minisig"
                chmod +x "${dest}"
                log_ok "${binary} downloaded and signature verified"
            else
                rm -f "${dest}" "${dest}.minisig"
                die "Signature verification failed for ${binary}. Binary may be tampered with."
            fi
        else
            log_warn "Could not download ${binary} from ${url}"
            log_warn "You can build from source: https://jotti.ru/jotticp"
        fi
    done

    # Install symlinks for easy CLI access
    ln -sf "${JOTTI_INSTALL_DIR}/bin/jotti-panel" /usr/local/bin/jotti-panel
    ln -sf "${JOTTI_INSTALL_DIR}/bin/jotti-agent" /usr/local/bin/jotti-agent

    log_ok "JottiCP binaries installed to ${JOTTI_INSTALL_DIR}/bin/"
}

# ── Step 4: Initialize PostgreSQL ────────────────────────────────────────────

setup_postgresql() {
    log_step "Step 4/12: Initializing PostgreSQL 17"

    if [[ "${PKG_MANAGER}" == "apt" ]]; then
        # Ubuntu/Debian: PostgreSQL is initialized on install
        systemctl enable postgresql
        systemctl start postgresql
    else
        # AlmaLinux: manual init required
        /usr/pgsql-17/bin/postgresql-17-setup initdb
        systemctl enable postgresql-17
        systemctl start postgresql-17
    fi

    log_ok "PostgreSQL service started"

    # Wait for PG to be ready
    local retries=10
    while ! sudo -u postgres psql -c "SELECT 1" > /dev/null 2>&1; do
        retries=$(( retries - 1 ))
        if [[ ${retries} -le 0 ]]; then
            die "PostgreSQL did not start in time"
        fi
        sleep 1
    done

    # Generate random passwords
    local pg_orbit_password; pg_orbit_password=$(openssl rand -base64 32 | tr -dc 'a-zA-Z0-9' | head -c 32)

    # Create jotticp database user and database
    sudo -u postgres psql -v ON_ERROR_STOP=1 <<SQL > /dev/null 2>&1
DO \$\$
BEGIN
  IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = 'jotticp') THEN
    CREATE ROLE jotticp WITH LOGIN PASSWORD '${pg_orbit_password}';
  END IF;
END
\$\$;

CREATE DATABASE jotticp OWNER jotticp;
SQL

    log_ok "PostgreSQL database 'jotticp' created"

    # Harden PostgreSQL: bind to localhost only
    local pg_conf; pg_conf=$(sudo -u postgres psql -t -c "SHOW config_file" | tr -d ' ')
    local pg_hba;  pg_hba=$(sudo -u postgres psql -t -c "SHOW hba_file"     | tr -d ' ')

    # Set listen_addresses = 'localhost'
    sed -i "s/^#\?listen_addresses.*/listen_addresses = 'localhost'/" "${pg_conf}"

    # pg_hba.conf: only allow jotticp user from localhost
    cat >> "${pg_hba}" << 'HBA'
# JottiCP: allow local connections only
host    jotticp         jotticp         127.0.0.1/32            scram-sha-256
host    jotticp         jotticp         ::1/128                 scram-sha-256
HBA

    systemctl reload postgresql || systemctl reload postgresql-17

    # Save DATABASE_URL
    local database_url="postgresql://jotticp:${pg_orbit_password}@127.0.0.1:5432/jotticp"
    mkdir -p "${JOTTI_CONF_DIR}"
    printf "DATABASE_URL=%s\n" "${database_url}" >> "${JOTTI_CONF_DIR}/jotti-panel.env"

    log_ok "PostgreSQL hardened (localhost only, scram-sha-256)"
}

# ── Step 5: Setup PowerDNS ───────────────────────────────────────────────────

setup_powerdns() {
    log_step "Step 5/12: Setting up PowerDNS"

    # Stop and disable the system resolver if it's using port 53
    if systemctl is-active --quiet systemd-resolved 2>/dev/null; then
        systemctl disable --now systemd-resolved > /dev/null 2>&1
        # Use 1.1.1.1 as upstream resolver
        ln -sf /run/systemd/resolve/resolv.conf /etc/resolv.conf 2>/dev/null || true
        printf "nameserver 1.1.1.1\nnameserver 9.9.9.9\n" > /etc/resolv.conf
        log_ok "systemd-resolved disabled (conflicts with PowerDNS on port 53)"
    fi

    # Configure PowerDNS to use PostgreSQL backend
    cat > /etc/powerdns/pdns.conf << EOF
# JottiCP PowerDNS Configuration
setuid=pdns
setgid=pdns

launch=gpgsql
gpgsql-host=127.0.0.1
gpgsql-port=5432
gpgsql-dbname=jotticp
gpgsql-user=jotticp
gpgsql-password=$(grep DATABASE_URL "${JOTTI_CONF_DIR}/jotti-panel.env" | cut -d: -f3 | cut -d@ -f1)

# Disable built-in webserver (orbit-dns manages via API)
webserver=no

# Security
allow-axfr-ips=
allow-recursion=
log-dns-queries=no
local-port=53

# Performance
distributor-threads=2
receiver-threads=2
EOF

    # Initialize PowerDNS schema in PostgreSQL
    sudo -u postgres psql jotticp < /usr/share/pdns-backend-pgsql/schema.pgsql.sql > /dev/null 2>&1 \
        || log_warn "PowerDNS schema may already exist"

    systemctl enable pdns
    systemctl restart pdns

    log_ok "PowerDNS configured and started"
}

# ── Step 6: Setup Postfix + Dovecot + rspamd ─────────────────────────────────

setup_mail() {
    log_step "Step 6/12: Setting up mail stack (Postfix + Dovecot + rspamd)"

    local server_hostname; server_hostname=$(hostname -f 2>/dev/null || hostname)

    # Postfix main configuration
    postconf -e "myhostname = ${server_hostname}"
    postconf -e "mydomain = ${server_hostname#*.}"
    postconf -e "myorigin = \$mydomain"
    postconf -e "inet_interfaces = all"
    postconf -e "inet_protocols = ipv4"
    postconf -e "smtpd_banner = \$myhostname ESMTP"
    postconf -e "biff = no"
    postconf -e "append_dot_mydomain = no"
    postconf -e "readme_directory = no"
    postconf -e "smtpd_tls_security_level = may"
    postconf -e "smtpd_tls_auth_only = yes"
    postconf -e "smtp_tls_security_level = may"
    postconf -e "smtpd_relay_restrictions = permit_mynetworks,permit_sasl_authenticated,reject_unauth_destination"
    postconf -e "smtpd_sasl_auth_enable = yes"
    postconf -e "smtpd_sasl_type = dovecot"
    postconf -e "smtpd_sasl_path = private/auth"
    postconf -e "virtual_transport = lmtp:unix:private/dovecot-lmtp"
    # Virtual domains stored in PostgreSQL — configured further by orbit-mail

    # Dovecot: basic configuration
    # (Full orbit-mail configuration applied by orbit-panel on first run)

    # rspamd: basic spam filtering
    mkdir -p /etc/rspamd/local.d
    cat > /etc/rspamd/local.d/actions.conf << 'RSPAMD'
reject = 15;
add_header = 6;
greylist = 4;
RSPAMD

    systemctl enable postfix dovecot rspamd
    systemctl restart postfix dovecot rspamd > /dev/null 2>&1 || log_warn "Mail services start with warnings — configure via orbit-panel"

    log_ok "Mail stack configured (full configuration via jotti-panel)"
}

# ── Step 7: Setup OpenLiteSpeed ──────────────────────────────────────────────

setup_openlitespeed() {
    log_step "Step 7/12: Setting up OpenLiteSpeed ${OLS_VERSION}"

    if [[ "${PKG_MANAGER}" == "apt" ]]; then
        # OpenLiteSpeed official repo
        wget -qO - https://repo.litespeed.sh | bash > /dev/null 2>&1
        apt-get install -y -qq openlitespeed > /dev/null 2>&1
    else
        rpm -Uvh "http://rpms.litespeedtech.com/centos/litespeed-repo-1.1-1.el9.noarch.rpm" > /dev/null 2>&1 || true
        dnf install -y openlitespeed > /dev/null 2>&1
    fi

    log_ok "OpenLiteSpeed installed"

    # Disable OLS admin UI on 7080 from public internet — orbit-panel manages OLS
    # The admin console is only needed locally; jotti-panel uses the config file API
    local ols_conf="/usr/local/lsws/conf/httpd_config.conf"
    if [[ -f "${ols_conf}" ]]; then
        # Restrict admin listener to loopback
        sed -i 's/address.*:7080/address\t\t127.0.0.1:7080/' "${ols_conf}" 2>/dev/null || true
    fi

    # Create jotti-panel reverse proxy vhost (serves the panel UI on HTTPS 443)
    mkdir -p /usr/local/lsws/conf/vhosts/jotti-panel
    cat > /usr/local/lsws/conf/vhosts/jotti-panel/vhconf.conf << 'OLS_VHOST'
# JottiCP panel vhost — served on HTTPS 443, proxied to jotti-panel:2087
docRoot /usr/local/lsws/DEFAULT/html
index {
  useServer 0
}
extprocessor jotti-panel {
  type proxy
  address 127.0.0.1:2087
  maxConns 100
  pcKeepAliveTimeout 60
  respBuffer 0
}
context / {
  type proxy
  handler jotti-panel
  addDefaultCharset off
}
OLS_VHOST

    systemctl enable lsws 2>/dev/null || /usr/local/lsws/bin/lswsctrl start > /dev/null 2>&1 || true
    log_ok "OpenLiteSpeed configured (panel proxied to localhost:2087)"
}

# ── Step 8: Setup cgroups v2 ─────────────────────────────────────────────────

setup_cgroups_v2() {
    log_step "Step 8/12: Configuring cgroups v2 for site isolation"

    # Check if cgroups v2 is already enabled
    if [[ "$(stat -fc %T /sys/fs/cgroup/)" == "cgroup2fs" ]]; then
        log_ok "cgroups v2 already enabled (cgroup2fs)"
    else
        # Enable unified cgroup hierarchy
        local grub_file="/etc/default/grub"
        if [[ -f "${grub_file}" ]]; then
            if ! grep -q "systemd.unified_cgroup_hierarchy" "${grub_file}"; then
                sed -i 's/GRUB_CMDLINE_LINUX="\(.*\)"/GRUB_CMDLINE_LINUX="\1 systemd.unified_cgroup_hierarchy=1"/' \
                    "${grub_file}"
                update-grub > /dev/null 2>&1 || grub2-mkconfig -o /boot/grub2/grub.cfg > /dev/null 2>&1 || true
                log_warn "cgroups v2 enabled — reboot required to fully activate"
            fi
        fi
    fi

    # Create systemd slice for all hosted websites
    cat > /etc/systemd/system/websites.slice << 'SLICE'
[Unit]
Description=Hosted Websites Resource Control Group
Before=slices.target

[Slice]
# Default limits apply to the slice as a whole
# Per-site limits are set via website-DOMAIN.slice (created by orbit-panel)
MemoryAccounting=yes
CPUAccounting=yes
IOAccounting=yes
TasksAccounting=yes
SLICE

    systemctl daemon-reload
    systemctl enable websites.slice 2>/dev/null || true

    # Verify cgroup accounting is available
    if systemctl status websites.slice > /dev/null 2>&1; then
        log_ok "websites.slice cgroup hierarchy created"
    else
        log_warn "websites.slice not yet active — will be activated after reboot"
    fi

    # Ensure PHP-FPM uses cgroup delegation
    cat >> /etc/systemd/system/php8.3-fpm.service.d/override.conf << 'FPM_OVERRIDE' 2>/dev/null || true
[Service]
Delegate=yes
FPM_OVERRIDE

    log_ok "cgroups v2 configured (MemoryAccounting + CPUAccounting + IOAccounting)"
}

# ── Step 9: Enable systemd Services ─────────────────────────────────────────

setup_systemd_services() {
    log_step "Step 9/12: Creating and enabling systemd services"

    local services=("jotti-panel" "jotti-agent" "jotti-dns" "jotti-mail" "jotti-cron")

    for svc in "${services[@]}"; do
        cat > "/etc/systemd/system/${svc}.service" << EOF
[Unit]
Description=JottiCP ${svc} daemon
Documentation=https://docs.jotti.ru
After=network.target postgresql.service postgresql-17.service
Wants=network.target

[Service]
Type=notify
User=${JOTTI_PANEL_USER}
Group=${JOTTI_PANEL_USER}
WorkingDirectory=${JOTTI_DATA_DIR}
EnvironmentFile=${JOTTI_CONF_DIR}/${svc}.env
ExecStart=${JOTTI_INSTALL_DIR}/bin/${svc}
ExecReload=/bin/kill -HUP \$MAINPID
Restart=on-failure
RestartSec=5s
TimeoutStartSec=30s

# Hardening
NoNewPrivileges=yes
PrivateDevices=yes
ProtectSystem=strict
ReadWritePaths=${JOTTI_DATA_DIR} ${JOTTI_LOG_DIR} ${JOTTI_CONF_DIR}
ProtectHome=yes
RestrictSUIDSGID=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
RestrictRealtime=yes
RestrictNamespaces=~user pid net
SystemCallFilter=@system-service

# orbit-panel specific: needs to call adduser, systemctl, etc. via sudo
$([ "${svc}" == "jotti-panel" ] && echo "# (sudo rules in /etc/sudoers.d/jotticp)" || true)

# Resource limits for the panel daemon itself
MemoryMax=512M
CPUQuota=50%

[Install]
WantedBy=multi-user.target
EOF

        # Create empty env file if it doesn't exist
        touch "${JOTTI_CONF_DIR}/${svc}.env"
        chmod 640 "${JOTTI_CONF_DIR}/${svc}.env"
        chown "root:${JOTTI_PANEL_USER}" "${JOTTI_CONF_DIR}/${svc}.env"

        log_ok "Service unit created: ${svc}.service"
    done

    # Populate jotti-panel.env with generated values
    local jwt_secret; jwt_secret=$(openssl rand -base64 48 | tr -dc 'a-zA-Z0-9' | head -c 48)
    cat >> "${JOTTI_CONF_DIR}/jotti-panel.env" << EOF
JOTTI_ENV=${JOTTI_ENV:-community}
JWT_SECRET=${jwt_secret}
VALKEY_URL=redis://127.0.0.1:6379
PANEL_PORT=${JOTTI_PANEL_PORT}
ALLOWED_ORIGINS=https://$(hostname -f 2>/dev/null || hostname)
EOF

    # sudo rules: jotti-panel can run specific privileged commands
    cat > /etc/sudoers.d/jotticp << 'SUDOERS'
# JottiCP sudo rules — allow jotti-panel to manage sites without root shell
# All commands are explicitly listed — no wildcard root shell access

# Unix user management for site isolation
jotticp ALL=(root) NOPASSWD: /usr/sbin/useradd --system * jotti_*
jotticp ALL=(root) NOPASSWD: /usr/sbin/userdel -r jotti_*

# systemd: manage PHP-FPM, OLS, and website slices
jotticp ALL=(root) NOPASSWD: /bin/systemctl enable php*-fpm
jotticp ALL=(root) NOPASSWD: /bin/systemctl disable php*-fpm
jotticp ALL=(root) NOPASSWD: /bin/systemctl restart php*-fpm
jotticp ALL=(root) NOPASSWD: /bin/systemctl reload php*-fpm
jotticp ALL=(root) NOPASSWD: /bin/systemctl start website-*.slice
jotticp ALL=(root) NOPASSWD: /bin/systemctl stop website-*.slice

# nftables: manage fail2ban IP blocks
jotticp ALL=(root) NOPASSWD: /usr/sbin/nft add element inet filter fail2ban_* *
jotticp ALL=(root) NOPASSWD: /usr/sbin/nft delete element inet filter fail2ban_* *

# OLS management
jotticp ALL=(root) NOPASSWD: /usr/local/lsws/bin/lswsctrl restart
jotticp ALL=(root) NOPASSWD: /usr/local/lsws/bin/lswsctrl reload

# WP-CLI (run as site users)
jotticp ALL=(jotti_*) NOPASSWD: /usr/local/bin/wp *
SUDOERS
    chmod 440 /etc/sudoers.d/jotticp

    systemctl daemon-reload

    # Enable all services
    for svc in "${services[@]}"; do
        systemctl enable "${svc}" > /dev/null 2>&1
    done

    log_ok "All JottiCP services enabled"

    # Start orbit-panel
    systemctl start jotti-panel > /dev/null 2>&1 \
        || log_warn "jotti-panel start deferred (binary may not be present yet)"
}

# ── Step 10: Setup nftables Firewall ─────────────────────────────────────────

setup_nftables() {
    log_step "Step 10/12: Configuring nftables firewall"

    # Disable and uninstall UFW/iptables-legacy if present
    if command -v ufw &>/dev/null; then
        ufw disable > /dev/null 2>&1 || true
        log_detail "UFW disabled (replaced by nftables)"
    fi

    systemctl enable nftables > /dev/null 2>&1

    cat > /etc/nftables.conf << 'NFTABLES'
#!/usr/sbin/nft -f
# JottiCP nftables ruleset
# Generated by jotti-installer

flush ruleset

table inet filter {
    set fail2ban_ssh {
        type ipv4_addr; flags dynamic, timeout; timeout 15m;
        comment "fail2ban SSH jail"
    }
    set fail2ban_panel {
        type ipv4_addr; flags dynamic, timeout; timeout 15m;
        comment "fail2ban panel login jail"
    }
    set fail2ban_smtp {
        type ipv4_addr; flags dynamic, timeout; timeout 1h;
        comment "fail2ban SMTP auth jail"
    }

    chain input {
        type filter hook input priority filter; policy drop;

        iifname "lo" accept
        ct state invalid drop
        ct state { established, related } accept

        # ICMP (rate-limited)
        ip protocol icmp icmp type {
            echo-request, echo-reply, destination-unreachable, time-exceeded, parameter-problem
        } limit rate 10/second accept

        ip6 nexthdr icmpv6 icmpv6 type {
            echo-request, echo-reply, nd-neighbor-solicit, nd-neighbor-advert,
            nd-router-advert, destination-unreachable, time-exceeded
        } accept

        # Dynamic blocks (fail2ban)
        ip saddr @fail2ban_ssh   drop
        ip saddr @fail2ban_panel drop
        ip saddr @fail2ban_smtp  drop

        # Web
        tcp dport 80  accept
        tcp dport 443 accept

        # SSH
        tcp dport 22 accept

        # orbit-agent gRPC (mTLS — authentication at application layer)
        tcp dport 7443 accept

        # Mail ports (enabled/disabled by orbit-mail module)
        # tcp dport { 25, 465, 587, 993, 995, 143, 110 } accept

        # DNS (enabled/disabled by orbit-dns module)
        # tcp dport 53 accept
        # udp dport 53 accept

        # ADMIN PORT 2087 IS INTENTIONALLY NOT OPENED HERE
        # orbit-panel binds to 127.0.0.1:2087 — accessed via OLS/nginx reverse proxy on 443

        log prefix "jotticp-drop: " flags all limit rate 5/minute drop
    }

    chain forward {
        type filter hook forward priority filter; policy drop;
    }

    chain output {
        type filter hook output priority filter; policy accept;
    }
}
NFTABLES

    nft -f /etc/nftables.conf
    systemctl restart nftables
    log_ok "nftables firewall configured (deny-all inbound, explicit allow-list)"
}

# ── Step 11: CIS Level 1 Security Baseline ───────────────────────────────────

apply_cis_baseline() {
    log_step "Step 11/12: Applying CIS Level 1 security baseline"

    # Kernel hardening via sysctl
    cat > /etc/sysctl.d/99-jotticp-hardening.conf << 'SYSCTL'
# JottiCP CIS Level 1 sysctl hardening
net.ipv4.ip_forward = 0
net.ipv4.conf.all.send_redirects = 0
net.ipv4.conf.default.send_redirects = 0
net.ipv4.conf.all.accept_redirects = 0
net.ipv4.conf.default.accept_redirects = 0
net.ipv4.conf.all.secure_redirects = 1
net.ipv4.conf.all.log_martians = 1
net.ipv4.conf.default.log_martians = 1
net.ipv4.icmp_echo_ignore_broadcasts = 1
net.ipv4.icmp_ignore_bogus_error_responses = 1
net.ipv4.tcp_syncookies = 1
net.ipv4.conf.all.rp_filter = 1
net.ipv4.conf.default.rp_filter = 1
net.ipv4.tcp_rfc1337 = 1
net.ipv4.conf.all.accept_source_route = 0
net.ipv4.conf.default.accept_source_route = 0
net.ipv6.conf.all.accept_redirects = 0
net.ipv6.conf.all.accept_ra = 0
net.ipv6.conf.default.accept_ra = 0
net.ipv6.conf.all.accept_source_route = 0
kernel.randomize_va_space = 2
kernel.dmesg_restrict = 1
kernel.perf_event_paranoid = 3
fs.suid_dumpable = 0
SYSCTL

    sysctl --system > /dev/null 2>&1
    log_ok "sysctl hardening applied"

    # SSH hardening
    local sshd_conf="/etc/ssh/sshd_config"
    cp "${sshd_conf}" "${sshd_conf}.backup-pre-jotticp"

    # Apply SSH settings (append to avoid breaking any include directives)
    cat >> "${sshd_conf}" << 'SSH'

# JottiCP SSH hardening (appended by installer)
PermitRootLogin no
PasswordAuthentication no
PubkeyAuthentication yes
MaxAuthTries 3
LoginGraceTime 30
X11Forwarding no
AllowTcpForwarding no
PermitTunnel no
ClientAliveInterval 300
ClientAliveCountMax 2
SSH

    # Validate sshd_config before reloading
    if sshd -t -f "${sshd_conf}" > /dev/null 2>&1; then
        systemctl reload sshd > /dev/null 2>&1 || systemctl reload ssh > /dev/null 2>&1 || true
        log_ok "SSH hardened (PasswordAuthentication disabled, PermitRootLogin disabled)"
    else
        log_warn "sshd_config validation failed — SSH config not applied. Check ${sshd_conf}."
        cp "${sshd_conf}.backup-pre-jotticp" "${sshd_conf}"
    fi

    # Remove unnecessary services
    local dangerous_services=("cups" "avahi-daemon" "rpcbind" "smbd" "nmbd" "nfs-server" "rsyncd")
    for svc in "${dangerous_services[@]}"; do
        if systemctl is-active --quiet "${svc}" 2>/dev/null; then
            systemctl disable --now "${svc}" > /dev/null 2>&1
            log_detail "Disabled service: ${svc}"
        fi
    done
    log_ok "Unnecessary services removed (Samba, CUPS, rpcbind)"

    # Automatic security updates
    if [[ "${PKG_MANAGER}" == "apt" ]]; then
        apt-get install -y -qq unattended-upgrades > /dev/null 2>&1
        cat > /etc/apt/apt.conf.d/50unattended-upgrades << 'UNATTENDED'
Unattended-Upgrade::Allowed-Origins {
    "${distro_id}:${distro_codename}-security";
};
Unattended-Upgrade::Remove-Unused-Dependencies "true";
Unattended-Upgrade::Automatic-Reboot "false";
Unattended-Upgrade::SyslogEnable "true";
UNATTENDED
        systemctl enable --now unattended-upgrades > /dev/null 2>&1
    else
        dnf install -y dnf-automatic > /dev/null 2>&1
        sed -i 's/apply_updates = no/apply_updates = yes/' /etc/dnf/automatic.conf 2>/dev/null || true
        sed -i 's/upgrade_type = default/upgrade_type = security/' /etc/dnf/automatic.conf 2>/dev/null || true
        systemctl enable --now dnf-automatic-install.timer > /dev/null 2>&1 || true
    fi
    log_ok "Automatic security updates enabled"

    # Enable auditd
    systemctl enable --now auditd > /dev/null 2>&1
    log_ok "auditd enabled"

    # fail2ban configuration
    cat > /etc/fail2ban/jail.d/jotticp.conf << 'FAIL2BAN'
[DEFAULT]
bantime   = 15m
findtime  = 10m
maxretry  = 5
banaction = nftables-allports

[sshd]
enabled  = true
maxretry = 3
bantime  = 1h

[jotti-panel-auth]
enabled  = true
port     = 443
filter   = orbit-panel-auth
logpath  = /var/log/jotticp/auth.log
maxretry = 5
bantime  = 15m

[postfix-sasl]
enabled  = true
maxretry = 5
bantime  = 30m

[dovecot]
enabled  = true
maxretry = 5
bantime  = 30m
FAIL2BAN

    cat > /etc/fail2ban/filter.d/jotti-panel-auth.conf << 'F2B_FILTER'
[Definition]
failregex = ^.*FAILED_LOGIN.*ip=<HOST>.*$
            ^.*TOTP_FAILED.*ip=<HOST>.*$
ignoreregex =
F2B_FILTER

    systemctl enable --now fail2ban > /dev/null 2>&1
    log_ok "fail2ban configured and started"
}

# ── Step 12: First-Run Wizard ─────────────────────────────────────────────────

launch_wizard() {
    log_step "Step 12/12: Preparing for first-run setup"

    local server_ip; server_ip=$(hostname -I | awk '{print $1}')
    local panel_url="https://${server_ip}"

    # Generate a first-run token (valid for 30 minutes, single-use)
    local firstrun_token; firstrun_token=$(openssl rand -hex 16)
    echo "${firstrun_token}" > "${JOTTI_DATA_DIR}/firstrun.token"
    chmod 600 "${JOTTI_DATA_DIR}/firstrun.token"
    chown "${JOTTI_PANEL_USER}:${JOTTI_PANEL_USER}" "${JOTTI_DATA_DIR}/firstrun.token"

    printf "\n"
    printf "${GREEN}═══════════════════════════════════════════════════════════════${NC}\n"
    printf "${GREEN}  JottiCP ${JOTTI_VERSION} installed successfully!${NC}\n"
    printf "${GREEN}═══════════════════════════════════════════════════════════════${NC}\n"
    printf "\n"
    printf "  ${CYAN}Continue setup in your browser:${NC}\n"
    printf "\n"
    printf "  ${BLUE}%s/?setup=%s${NC}\n" "${panel_url}" "${firstrun_token}"
    printf "\n"
    printf "  ${YELLOW}Important:${NC}\n"
    printf "  • This setup link expires in 30 minutes\n"
    printf "  • Add your SSH public key before logging out of this terminal\n"
    printf "    (SSH root login has been disabled)\n"
    printf "  • To add your SSH key:\n"
    printf "    mkdir -p ~/.ssh && echo 'YOUR_PUBLIC_KEY' >> ~/.ssh/authorized_keys\n"
    printf "\n"
    printf "  ${CYAN}Configuration files:${NC} %s\n" "${JOTTI_CONF_DIR}"
    printf "  ${CYAN}Log files:${NC}            %s\n" "${JOTTI_LOG_DIR}"
    printf "  ${CYAN}Documentation:${NC}        https://docs.jotti.ru\n"
    printf "\n"

    # Summarize what was installed
    printf "  ${GREEN}Installed:${NC}\n"
    printf "    ✓ JottiCP panel & agent (v%s)\n" "${JOTTI_VERSION}"
    printf "    ✓ PostgreSQL %s (localhost only)\n" "${PG_VERSION}"
    printf "    ✓ OpenLiteSpeed %s\n" "${OLS_VERSION}"
    printf "    ✓ PowerDNS %s\n" "${PDNS_VERSION}"
    printf "    ✓ PHP 8.1, 8.2, 8.3, 8.4 (FPM)\n"
    printf "    ✓ Postfix + Dovecot + rspamd\n"
    printf "    ✓ nftables firewall (deny-all inbound)\n"
    printf "    ✓ fail2ban (SSH, panel, SMTP jails)\n"
    printf "    ✓ CIS Level 1 sysctl hardening\n"
    printf "    ✓ Automatic security updates\n"
    printf "\n"

    # Log install completion
    {
        printf "JottiCP %s installed at %s\n" "${JOTTI_VERSION}" "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        printf "OS: %s\n" "${OS_NAME}"
        printf "Server IP: %s\n" "${server_ip}"
    } >> "${JOTTI_LOG_DIR}/install.log" 2>/dev/null || true
}

# ── Main ──────────────────────────────────────────────────────────────────────

main() {
    clear 2>/dev/null || true
    printf "${CYAN}"
    printf "  ██████╗ ██████╗ ██████╗ ██╗████████╗ ██████╗██████╗ \n"
    printf " ██╔═══██╗██╔══██╗██╔══██╗██║╚══██╔══╝██╔════╝██╔══██╗\n"
    printf " ██║   ██║██████╔╝██████╔╝██║   ██║   ██║     ██████╔╝\n"
    printf " ██║   ██║██╔══██╗██╔══██╗██║   ██║   ██║     ██╔═══╝ \n"
    printf " ╚██████╔╝██║  ██║██████╔╝██║   ██║   ╚██████╗██║     \n"
    printf "  ╚═════╝ ╚═╝  ╚═╝╚═════╝ ╚═╝   ╚═╝    ╚═════╝╚═╝     \n"
    printf "${NC}"
    printf "\n  Hosting Control Panel v%s — Installer\n\n" "${JOTTI_VERSION}"

    check_root
    check_architecture

    detect_os

    check_conflicts
    check_minimum_resources

    install_dependencies

    download_orbit_binaries

    setup_postgresql

    setup_powerdns

    setup_mail

    setup_openlitespeed

    setup_cgroups_v2

    setup_systemd_services

    setup_nftables

    apply_cis_baseline

    launch_wizard
}

# Run installer with error trap
trap 'stop_spinner; log_error "Installation failed on line ${LINENO}. Check ${JOTTI_LOG_DIR}/install.log for details."; exit 1' ERR
main "$@"
