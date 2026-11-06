#!/bin/sh
# Installs or updates Board on the Kritano droplet, as root, from files
# copied to /tmp/board-deploy (deploy/deploy.sh does both). Safe to run again:
# it never touches the database or an existing board.toml.
set -eu
SRC=/tmp/board-deploy
DOMAIN=board.kritano.com

id board >/dev/null 2>&1 || useradd --system --home /var/lib/board --shell /usr/sbin/nologin board
install -d -o board -g board -m 0750 /var/lib/board
if [ ! -f /var/lib/board/board.toml ]; then
  # The first start adds a random server_secret and makes the file private.
  printf 'bind = "127.0.0.1:7070"\ndatabase = "/var/lib/board/board.db"\npublic_origin = "https://%s"\n' "$DOMAIN" \
    | install -o board -g board -m 0600 /dev/stdin /var/lib/board/board.toml
fi
command -v sqlite3 >/dev/null || apt-get install -y -q sqlite3 >/dev/null

install -m 0755 "$SRC/board" /usr/local/bin/board
install -m 0755 "$SRC/backup.sh" /usr/local/bin/board-backup.sh
install -m 0644 "$SRC/board.service" "$SRC/board-backup.service" "$SRC/board-backup.timer" /etc/systemd/system/

# nginx: installed once, then certbot adds TLS to the live file. Later
# template changes are flagged, not applied, so certbot's edits survive.
CONF=/etc/nginx/sites-available/$DOMAIN
if [ ! -d /etc/letsencrypt/live/$DOMAIN ]; then
  install -m 0644 "$SRC/$DOMAIN.conf" "$CONF"
  ln -sf "$CONF" /etc/nginx/sites-enabled/$DOMAIN
  nginx -t && systemctl reload nginx
  certbot --nginx -d $DOMAIN --redirect -n
elif ! cmp -s "$SRC/$DOMAIN.conf" "$CONF.template"; then
  echo "note: deploy/$DOMAIN.conf has changed since the site was set up; merge it into $CONF by hand" >&2
fi
install -m 0644 "$SRC/$DOMAIN.conf" "$CONF.template"

systemctl daemon-reload
systemctl enable board >/dev/null
systemctl enable --now board-backup.timer >/dev/null
systemctl restart board
for _ in 1 2 3 4 5 6 7 8 9 10; do
  curl -sf 127.0.0.1:7070/healthz >/dev/null && break
  sleep 0.5
done
systemctl is-active board
curl -sf 127.0.0.1:7070/healthz && echo
curl -sf -o /dev/null -w "https://$DOMAIN/healthz: %{http_code}\n" https://$DOMAIN/healthz || true
