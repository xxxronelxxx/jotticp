#!/bin/bash
# Syncs /etc/dovecot/users to sogo.sogo_users table
# Dovecot format: user@domain:{SHA512-CRYPT}$6$hash:::/var/mail/...
# SOGo crypt algorithm expects bare $6$hash (no prefix)

PGHOST=127.0.0.1
PGDB=sogo
PGUSER=sogo
export PGPASSWORD=sogo_pass

DOVECOT_FILE=/etc/dovecot/users

if [ ! -f "$DOVECOT_FILE" ]; then exit 0; fi

while IFS=: read -r email hash rest; do
    [ -z "$email" ] && continue
    # Strip {SHA512-CRYPT} prefix - SOGo crypt algo needs bare $6$...
    bare_hash="${hash#\{SHA512-CRYPT\}}"
    cn="${email%%@*}"
    # Escape single quotes for SQL
    email_esc="${email//\'/\'\'}"
    bare_hash_esc="${bare_hash//\'/\'\'}"
    cn_esc="${cn//\'/\'\'}"

    psql -h "$PGHOST" -U "$PGUSER" -d "$PGDB" -c "
        INSERT INTO sogo_users (c_uid, c_name, c_cn, c_password, c_active, mail)
        VALUES ('$email_esc', '$email_esc', '$cn_esc', '$bare_hash_esc', 1, '$email_esc')
        ON CONFLICT (c_uid) DO UPDATE
          SET c_password = EXCLUDED.c_password,
              c_active   = 1,
              mail       = EXCLUDED.mail;
    " > /dev/null 2>&1
done < "$DOVECOT_FILE"

# Deactivate removed users
ALL_EMAILS=$(grep -o '^[^:]*' "$DOVECOT_FILE" | paste -sd "','" | sed "s/^/'/" | sed "s/$/'/")
if [ -n "$ALL_EMAILS" ]; then
    psql -h "$PGHOST" -U "$PGUSER" -d "$PGDB" -c "
        UPDATE sogo_users SET c_active=0 WHERE c_uid NOT IN ($ALL_EMAILS);
    " > /dev/null 2>&1
fi
