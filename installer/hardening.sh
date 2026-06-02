#!/bin/bash
# CIS Level 1 hardening for OrbitCP servers
# Part of OrbitCP Step 6.6 — sysctl + IO scheduler hardening
set -euo pipefail

echo "Applying CIS Level 1 sysctl hardening..."

cat > /etc/sysctl.d/99-orbitcp-hardening.conf << 'EOF'
# TCP BBR congestion control
net.core.default_qdisc=fq
net.ipv4.tcp_congestion_control=bbr

# SYN flood protection
net.ipv4.tcp_syncookies=1
net.ipv4.tcp_max_syn_backlog=2048
net.ipv4.tcp_synack_retries=2

# IP spoofing protection
net.ipv4.conf.all.rp_filter=1
net.ipv4.conf.default.rp_filter=1

# ICMP redirect protection
net.ipv4.conf.all.accept_redirects=0
net.ipv6.conf.all.accept_redirects=0
net.ipv4.conf.all.send_redirects=0

# No IP source routing
net.ipv4.conf.all.accept_source_route=0
net.ipv6.conf.all.accept_source_route=0

# Dmesg restriction
kernel.dmesg_restrict=1
kernel.kptr_restrict=2

# ASLR
kernel.randomize_va_space=2

# Disable magic sysrq
kernel.sysrq=0

# Protect hardlinks/symlinks
fs.protected_hardlinks=1
fs.protected_symlinks=1
EOF

sysctl -p /etc/sysctl.d/99-orbitcp-hardening.conf

echo "Applying IO scheduler udev rules..."

# IO scheduler optimization
cat > /etc/udev/rules.d/60-scheduler.rules << 'EOF'
ACTION=="add|change", KERNEL=="sd[a-z]|vd[a-z]|nvme[0-9]n[0-9]", ATTR{queue/rotational}=="0", ATTR{queue/scheduler}="mq-deadline"
ACTION=="add|change", KERNEL=="sd[a-z]|vd[a-z]", ATTR{queue/rotational}=="1", ATTR{queue/scheduler}="bfq"
EOF

# Apply udev rules immediately (without waiting for reboot)
udevadm control --reload-rules
udevadm trigger --type=subsystems

echo "CIS Level 1 hardening applied"
