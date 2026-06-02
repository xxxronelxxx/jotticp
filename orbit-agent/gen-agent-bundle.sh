#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# OrbitCP — generate a SECURE orbit-agent install bundle for a new managed server.
#
# Run this ON THE PANEL host. It produces a self-contained tarball you copy to
# the target server and run. The agent talks to the panel over mutual TLS
# (gRPC, port 7443) using a server cert signed by the panel's agent CA.
#
#   Usage:  ./gen-agent-bundle.sh <server-ip> [label]
#
# After installing on the target, in the panel UI:
#   1. Servers → Add Server (enter the same IP)
#   2. Enroll  (generates a single-use token)
#   3. Test Connectivity  (should now succeed on :7443)
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail

IP="${1:?usage: gen-agent-bundle.sh <server-ip> [label]}"
LABEL="${2:-$IP}"
PKI=/etc/orbitcp/agent
AGENT_BIN=/opt/orbitcp/src/target/release/orbit-agent
OUT="/tmp/orbit-agent-bundle-$IP"

[ -f "$PKI/ca.pem" ]     || { echo "ERROR: panel agent CA missing at $PKI/ca.pem"; exit 1; }
[ -f "$PKI/ca-key.pem" ] || { echo "ERROR: panel agent CA key missing at $PKI/ca-key.pem"; exit 1; }
[ -f "$AGENT_BIN" ]      || { echo "ERROR: agent binary missing at $AGENT_BIN (build it first)"; exit 1; }

rm -rf "$OUT"; mkdir -p "$OUT/agent"

# Server cert for the agent, SAN = target IP, signed by the panel CA.
openssl ecparam -name prime256v1 -genkey -noout -out "$OUT/agent/agent-key.pem"
openssl req -new -key "$OUT/agent/agent-key.pem" -out /tmp/_agent.csr -subj "/CN=orbit-agent-$IP" 2>/dev/null
printf "subjectAltName=IP:%s\nextendedKeyUsage=serverAuth\n" "$IP" > /tmp/_agent.cnf
openssl x509 -req -in /tmp/_agent.csr -CA "$PKI/ca.pem" -CAkey "$PKI/ca-key.pem" \
    -CAcreateserial -out "$OUT/agent/agent.pem" -days 825 -extfile /tmp/_agent.cnf 2>/dev/null
cp "$PKI/ca.pem" "$OUT/agent/ca.pem"
cp "$AGENT_BIN"  "$OUT/orbit-agent"
chmod 600 "$OUT/agent/agent-key.pem"
rm -f /tmp/_agent.csr /tmp/_agent.cnf

# install.sh — runs ON THE TARGET server.
cat > "$OUT/install.sh" <<'INSTALL'
#!/usr/bin/env bash
set -euo pipefail
[ "$(id -u)" = 0 ] || { echo "run as root"; exit 1; }
DIR="$(cd "$(dirname "$0")" && pwd)"
install -m755 "$DIR/orbit-agent" /usr/local/bin/orbit-agent
mkdir -p /etc/orbitcp/agent /var/lib/orbitcp/agent
install -m644 "$DIR/agent/ca.pem"  /etc/orbitcp/agent/ca.pem
install -m644 "$DIR/agent/agent.pem" /etc/orbitcp/agent/agent.pem
install -m600 "$DIR/agent/agent-key.pem" /etc/orbitcp/agent/agent-key.pem

# DB-admin credentials the agent uses to provision databases (CreateDatabase RPC).
# The agent runs `mysql --defaults-file=/etc/orbitcp/mysql-admin.cnf`. Default to unix_socket
# auth (works out-of-box on most MariaDB/MySQL where root uses the socket plugin). If this
# server's root requires a password, edit the file and set `password=...`.
if [ ! -f /etc/orbitcp/mysql-admin.cnf ]; then
  SOCK="$(mysqladmin --silent var 2>/dev/null | awk '/ socket /{print $4; exit}')"
  SOCK="${SOCK:-/run/mysqld/mysqld.sock}"
  printf '[client]\nuser=root\nsocket=%s\n# If root needs a password, uncomment & set:\n# password=CHANGE_ME\n' "$SOCK" \
    > /etc/orbitcp/mysql-admin.cnf
  chmod 600 /etc/orbitcp/mysql-admin.cnf
  echo "ℹ  wrote /etc/orbitcp/mysql-admin.cnf (socket auth). Set a password there if 'mysql --defaults-file=/etc/orbitcp/mysql-admin.cnf -e \"SELECT 1\"' is denied."
fi

# PowerDNS REST API endpoint the agent uses for DNS zone/record management. Auto-detect
# the webserver port + api-key from this host's pdns.conf (PowerDNS defaults to :8081, but
# many installs move it — e.g. :8053 — to avoid clashing with a web server on :8081).
PDNS_PORT="$(grep -hiE '^webserver-port[[:space:]]*=' /etc/powerdns/pdns.conf /etc/powerdns/pdns.d/*.conf 2>/dev/null | tail -1 | cut -d= -f2 | tr -d '[:space:]')"
PDNS_KEY="$(grep -hiE '^api-key[[:space:]]*=' /etc/powerdns/pdns.conf /etc/powerdns/pdns.d/*.conf 2>/dev/null | tail -1 | cut -d= -f2 | tr -d '[:space:]')"
PDNS_PORT="${PDNS_PORT:-8081}"
PDNS_ENV=""
if [ -n "$PDNS_KEY" ]; then
  PDNS_ENV=$'\n'"Environment=PDNS_API_URL=http://127.0.0.1:${PDNS_PORT}"$'\n'"Environment=PDNS_API_KEY=${PDNS_KEY}"
  echo "ℹ  detected PowerDNS API on :${PDNS_PORT} — wired PDNS_API_URL/PDNS_API_KEY into the agent service."
else
  echo "ℹ  no PowerDNS api-key found in pdns.conf — if this server runs PowerDNS, set api=yes + api-key, then add PDNS_API_URL/PDNS_API_KEY to the agent service."
fi

cat > /etc/systemd/system/orbit-agent.service <<EOF
[Unit]
Description=OrbitCP Agent (mTLS gRPC, port 7443)
After=network-online.target
Wants=network-online.target
[Service]
Type=simple
ExecStart=/usr/local/bin/orbit-agent
Restart=on-failure
RestartSec=5
Environment=RUST_LOG=info${PDNS_ENV}
[Install]
WantedBy=multi-user.target
EOF
systemctl daemon-reload
systemctl enable --now orbit-agent
sleep 2
if systemctl is-active --quiet orbit-agent && ss -ltn | grep -q ':7443'; then
  echo "✅ orbit-agent running on :7443 (mTLS)."
  echo "   Now in the panel: Servers → Add Server (this IP) → Enroll → Test Connectivity."
else
  echo "❌ orbit-agent failed to start — check: journalctl -u orbit-agent -n 30"; exit 1
fi
INSTALL
chmod +x "$OUT/install.sh"

TARBALL="/tmp/orbit-agent-$IP.tar.gz"
tar -czf "$TARBALL" -C /tmp "orbit-agent-bundle-$IP"
echo "✅ Bundle created: $TARBALL  (label: $LABEL)"
echo
echo "Deploy to the new server:"
echo "  scp $TARBALL root@$IP:/tmp/"
echo "  ssh root@$IP 'cd /tmp && tar xzf orbit-agent-$IP.tar.gz && orbit-agent-bundle-$IP/install.sh'"
