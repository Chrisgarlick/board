# Deploying Board

Board runs on the Kritano droplet (`167.71.134.177`) next to Clients and
Motion: a systemd service on `127.0.0.1:7070`, data in `/var/lib/board`,
nginx in front at `https://board.kritano.com` (certbot for TLS).

| File | Goes to |
|---|---|
| `board` (built by the Release workflow) | `/usr/local/bin/board` |
| `board.service`, `board-backup.service`, `board-backup.timer` | `/etc/systemd/system/` |
| `board.kritano.com.conf` | `/etc/nginx/sites-available/board.kritano.com` (first time only) |
| `../scripts/backup.sh` | `/usr/local/bin/board-backup.sh` |
| (made on first install) | `/var/lib/board/board.toml` |

## Deploy (first time and every update)

From your Mac, in `~/dev/board`, with main committed and pushed:

```sh
deploy/deploy.sh            # builds on GitHub Actions, then installs
deploy/deploy.sh --latest   # skip the build; install the newest successful one
```

It runs the **Release** workflow (a static `x86_64-unknown-linux-musl` binary
with the frontend embedded), downloads it, copies it and the files above to
`/tmp/board-deploy` on the droplet, and runs `install.sh` there. `install.sh`
is safe to repeat: it creates the `board` user and config once, gets the
certificate once, never touches the database, installs the binary and
units, and restarts the service. Migrations run at start-up.

The build happens on GitHub because the droplet (2 GB, shared with live
services) shouldn't compile Rust, and the Mac would need a Linux cross
toolchain.

## After the first deploy

On the droplet (`ssh -i ~/.ssh/keys/kritano root@167.71.134.177`):

Sign in with a passkey (Face ID, Touch ID, a fingerprint). The first time,
there's no passkey yet, so get in with a one-time link and add one from the
board list (Passkeys). After that, `board login` is only the way back in if
every passkey is lost.

```sh
B="sudo -u board board --config /var/lib/board/board.toml"
$B login                                                     # one-time link for your browser
$B token create --name laptop --scope admin --actor api      # scripts
$B token create --name claude-mcp --scope admin --actor claude
$B token create --name design --scope read --actor api       # Design reads the model
```

Each token is shown once; keep it in your password manager. Then connect
Claude Code:

```sh
claude mcp add --transport http board https://board.kritano.com/mcp \
  --header "Authorization: Bearer brd_…"
```

## Planning PDFs (Typeset)

Board renders PDFs through Typeset with its own Clients caller key. Once:

```sh
# on the droplet: a caller key for Board (it may render for any client)
KEY=$(clients-admin keys create board --kind caller)
printf 'typeset_url = "https://typeset.chrisgarlick.com"\ntypeset_key = "%s"\n' "$KEY" \
  >> /var/lib/board/board.toml && unset KEY
systemctl restart board
```

Then set each board's client (Settings or the PDF panel) so its PDF is in their brand.

## Day to day

```sh
journalctl -u board -n 100            # logs
systemctl list-timers board-backup    # next backup
```

## Backups

`board-backup.timer` runs `board-backup.sh` nightly at 03:15: a consistent
copy of `board.db` (SQLite's online backup), checked and gzipped, in
`/var/lib/board/backups/`, 14 days kept. To also copy it off the droplet, put
`BACKUP_REMOTE=user@host:board/` in `/etc/board-backup.env` and give the
`board` user an SSH key the remote accepts.

### Test a restore (do this once)

```sh
gunzip -c /var/lib/board/backups/board-<stamp>.db.gz > /tmp/restore.db
printf 'database = "/tmp/restore.db"\nbind = "127.0.0.1:7071"\n' > /tmp/restore.toml
board --config /tmp/restore.toml serve     # then: curl localhost:7071/healthz
```

To restore for real: `systemctl stop board`, replace `/var/lib/board/board.db`
with the unzipped backup (and delete any `board.db-wal` and `board.db-shm`
next to it), then `systemctl start board`.
