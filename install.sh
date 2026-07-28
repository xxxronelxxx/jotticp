#!/usr/bin/env bash
# ============================================================
# JottiCP — One-Command Installer
# https://jottiecp.dev-spb.ru
# ============================================================
set -euo pipefail

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
BLUE='\033[0;34m'; BOLD='\033[1m'; NC='\033[0m'

log()  { echo -e "${GREEN}[JottiCP]${NC} $*"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $*"; }
err()  { echo -e "${RED}[ERROR]${NC} $*"; exit 1; }

JOTTICP_VERSION="0.0.1"
INSTALL_DIR="/opt/jottiecp"
DATA_DIR="/var/lib/jottiecp"
BACKUP_DIR="/var/backups/jottiecp"
LOG_DIR="/var/log/jottiecp"

banner() {
cat << 'BANNER'
  ___       _     _ _  ____  ____
 / _ \ _ __| |__ (_) |/ ___||  _ \
| | | | '__| '_ \| | | |    | |_) |
| |_| | |  | |_) | | | |___ |  __/
 \___/|_|  |_.__/|_|_|\____||_|
BANNER
echo -e "${BLUE}JottiCP v${JOTTICP_VERSION} — Rust-Powered Web Hosting Panel${NC}"
echo -e "${YELLOW}https://dev-spb.ru/products/jottiecp${NC}"
echo ""
}

check_root() {
  [ "$(id -u)" -eq 0 ] || err "Run as root: sudo bash install.sh"
}

check_os() {
  . /etc/os-release 2>/dev/null || err "Cannot detect OS"
  case "$ID" in
    ubuntu) [[ "$VERSION_ID" =~ ^(22|24) ]] || warn "Ubuntu 22.04/24.04 recommended" ;;
    debian) [[ "$VERSION_ID" =~ ^(11|12) ]] || warn "Debian 11/12 recommended" ;;
    *) warn "Untested OS: $ID $VERSION_ID — proceeding anyway" ;;
  esac
  log "OS: $PRETTY_NAME"
}

install_deps() {
  log "Installing system dependencies..."
  apt-get update -qq
  apt-get install -y -qq \
    curl wget git build-essential pkg-config \
    libssl-dev libpq-dev \
    postgresql postgresql-client \
    redis-server \
    nginx \
    certbot python3-certbot-nginx \
    nftables \
    ca-certificates gnupg lsb-release 2>&1 | tail -5
  log "Dependencies installed"
}

install_rust() {
  if command -v cargo &>/dev/null; then
    log "Rust already installed: $(cargo --version)"
    return
  fi
  log "Installing Rust..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
  source "$HOME/.cargo/env"
  log "Rust installed: $(cargo --version)"
}

setup_database() {
  log "Setting up PostgreSQL database..."
  local DB_PASS
  DB_PASS=$(openssl rand -base64 24 | tr -dc 'a-zA-Z0-9' | head -c 24)

  systemctl start postgresql
  systemctl enable postgresql

  sudo -u postgres psql -c "CREATE USER jottiecp WITH PASSWORD '${DB_PASS}';" 2>/dev/null || true
  sudo -u postgres psql -c "CREATE DATABASE jottiecp OWNER jottiecp;" 2>/dev/null || true
  sudo -u postgres psql -c "GRANT ALL PRIVILEGES ON DATABASE jottiecp TO jottiecp;" 2>/dev/null || true

  echo "DATABASE_URL=postgresql://jottiecp:${DB_PASS}@127.0.0.1:5432/jottiecp"
}

setup_valkey() {
  log "Configuring Redis/Valkey..."
  systemctl start redis-server 2>/dev/null || systemctl start redis 2>/dev/null || true
  systemctl enable redis-server 2>/dev/null || systemctl enable redis 2>/dev/null || true
}

build_jottiecp() {
  log "Building JottiCP from source..."
  source "$HOME/.cargo/env" 2>/dev/null || true

  [ -d "${INSTALL_DIR}" ] || git clone https://dev-spb.ru/jottiecp.git "${INSTALL_DIR}"
  cd "${INSTALL_DIR}"
  cargo build --release --workspace 2>&1 | tail -10
  log "Build complete"
}

setup_env() {
  local DB_URL="$1"
  local JWT_SECRET
  JWT_SECRET=$(openssl rand -base64 48)
  local PDNS_KEY
  PDNS_KEY=$(openssl rand -hex 16)

  cat > "${INSTALL_DIR}/.env" << EOF
DATABASE_URL=${DB_URL}
VALKEY_URL=redis://127.0.0.1:6379
JWT_SECRET=${JWT_SECRET}
JOTTI_LISTEN=127.0.0.1:2087
JOTTI_LOG_LEVEL=info
JOTTI_APPS_DIR=${DATA_DIR}/apps
JOTTI_BACKUPS_DIR=${BACKUP_DIR}
JOTTI_PLAN=community
PDNS_API_URL=http://127.0.0.1:8053
PDNS_API_KEY=${PDNS_KEY}
EOF
  chmod 600 "${INSTALL_DIR}/.env"
  log "Environment configured (secrets auto-generated)"
}

setup_systemd() {
  log "Installing systemd services..."

  cat > /etc/systemd/system/orbit-panel.service << 'SVC'
[Unit]
Description=JottiCP Panel API
After=network.target postgresql.service redis.service
Requires=postgresql.service

[Service]
Type=exec
User=jottiecp
WorkingDirectory=/opt/jottiecp
EnvironmentFile=/opt/jottiecp/.env
ExecStart=/opt/jottiecp/target/release/jotti-panel
Restart=on-failure
RestartSec=5
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
SVC

  systemctl daemon-reload
  systemctl enable jotti-panel
}

setup_nginx() {
  local DOMAIN="${1:-localhost}"
  log "Configuring nginx for domain: ${DOMAIN}..."

  cat > /etc/nginx/sites-available/jottiecp << EOF
server {
    listen 80;
    server_name ${DOMAIN};

    location /api/ {
        proxy_pass http://127.0.0.1:2087;
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto \$scheme;
    }

    location / {
        proxy_pass http://127.0.0.1:3000;
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_http_version 1.1;
        proxy_set_header Upgrade \$http_upgrade;
        proxy_set_header Connection "upgrade";
    }
}
EOF
  ln -sf /etc/nginx/sites-available/jottiecp /etc/nginx/sites-enabled/jottiecp
  nginx -t && systemctl reload nginx
}

print_success() {
  echo ""
  echo -e "${GREEN}${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
  echo -e "${GREEN}${BOLD}  ✅  JottiCP Installation Complete!${NC}"
  echo -e "${GREEN}${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
  echo ""
  echo -e "  Panel URL:   ${BLUE}http://${DOMAIN:-localhost}${NC}"
  echo -e "  First login: ${YELLOW}Create admin at /setup${NC}"
  echo -e "  Logs:        ${BLUE}journalctl -u jotti-panel -f${NC}"
  echo -e "  Docs:        ${BLUE}https://dev-spb.ru/docs/jottiecp/${NC}"
  echo ""
  echo -e "  ${YELLOW}⚠  14-day trial active. Activate at: https://dev-spb.ru/products/jottiecp${NC}"
  echo ""
}

main() {
  banner
  check_root
  check_os

  DOMAIN="${1:-}"

  install_deps
  install_rust
  DB_URL=$(setup_database)
  setup_valkey
  build_jottiecp
  setup_env "$DB_URL"
  setup_systemd

  [ -n "$DOMAIN" ] && setup_nginx "$DOMAIN"

  mkdir -p "${DATA_DIR}" "${BACKUP_DIR}" "${LOG_DIR}"
  useradd -r -s /bin/false jottiecp 2>/dev/null || true
  chown -R jottiecp:jottiecp "${INSTALL_DIR}" "${DATA_DIR}" "${BACKUP_DIR}" "${LOG_DIR}"

  systemctl start jotti-panel

  print_success
}

main "$@"
