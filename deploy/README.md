# Deploying Board

One DigitalOcean droplet (Ubuntu), Board bound to localhost, Caddy in front
for TLS. Replace `board.example.com` with your domain throughout.

## 1. Build

On the droplet (or any Linux x86_64 machine with Rust and Node 22):

```sh
git clone https://github.com/Chrisgarlick/board.git && cd board
(cd web && npm ci && npm run build)
cargo build --release
sudo install -m 755 target/release/board /usr/local/bin/board
sudo install -m 755 scripts/backup.sh /usr/local/bin/board-backup.sh
```

## 2. User, directory and config

```sh
sudo useradd --system --home /var/lib/board --shell /usr/sbin/nologin board
sudo install -d -o board -g board -m 750 /var/lib/board
sudo -u board tee /var/lib/board/board.toml >/dev/null <<'TOML'
bind = "127.0.0.1:7070"
database = "/var/lib/board/board.db"
public_origin = "https://board.example.com"
TOML
```

The first run adds a random `server_secret` to this file and makes it
readable only by the `board` user.

## 3. Service

```sh
sudo cp deploy/board.service /etc/systemd/system/board.service
sudo systemctl daemon-reload
sudo systemctl enable --now board
systemctl status board        # should be active (running)
curl -s localhost:7070/healthz
```

## 4. Caddy (TLS)

Point the domain's DNS at the droplet, add `deploy/Caddyfile`'s block to
`/etc/caddy/Caddyfile`, then `sudo systemctl reload caddy`.
`https://board.example.com/healthz` should answer.

## 5. Log in, and a token for scripts

```sh
sudo -u board board --config /var/lib/board/board.toml login
# open the printed link in your browser (works once, within 10 minutes)

sudo -u board board --config /var/lib/board/board.toml token create --name laptop --scope admin --actor api
# shown once: store it in your password manager
```

## 6. Backups

```sh
sudo -u board crontab -e
# add (set BACKUP_REMOTE to somewhere off this droplet):
# 15 3 * * * BACKUP_REMOTE=backup@nas.example.com:board/ /usr/local/bin/board-backup.sh
```

The `board` user needs an SSH key that the remote accepts. Run the script once
by hand to check it works:

```sh
sudo -u board /usr/local/bin/board-backup.sh
```

### Test a restore (do this once)

```sh
gunzip -c /var/lib/board/backups/board-<stamp>.db.gz > /tmp/restore.db
printf 'database = "/tmp/restore.db"\nbind = "127.0.0.1:7071"\n' > /tmp/restore.toml
board --config /tmp/restore.toml serve     # then: curl localhost:7071/healthz
```

To restore for real: `sudo systemctl stop board`, replace
`/var/lib/board/board.db` with the unzipped backup (and delete any
`board.db-wal` and `board.db-shm` next to it), then `sudo systemctl start board`.

## Updating

```sh
cd board && git pull
(cd web && npm ci && npm run build) && cargo build --release
sudo install -m 755 target/release/board /usr/local/bin/board
sudo systemctl restart board
```

Migrations run automatically on start.
