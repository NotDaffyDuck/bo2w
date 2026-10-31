# Running your own master

`bo2mp-master` is a relay and server browser, not a game server: the match is
simulated by the host's client. The crate has no engine dependency, so a VPS
needs neither game assets nor a GPU. For shipping our own builds, see
the page [`DEPLOY.md`](DEPLOY.md).

## Installing it

Run these from the machine that holds the clone; the VPS only has to accept ssh.

```bash
cargo xtask master install   root@203.0.113.7
cargo xtask master logs      root@203.0.113.7 --since 10min
```

`install` first creates a CA and a server certificate under `~/.bo2mp/ca`
(override with `--ca DIR` or `BO2MP_MASTER_CA_DIR`), builds a static binary, and
puts the binary under `/usr/local/lib/bo2mp/` and the certificates under
`/etc/bo2mp/`. It then writes the systemd unit via `bo2mp-master print-unit` and
runs `enable --now`. The CA key stays on the local machine. Further verbs are
`update`, `status` and `uninstall`; each accepts `--channel prod|dev` and
`--ca DIR`. `uninstall` leaves the certificates and the CA in place, so a later
reinstall is still trusted.

## No domain needed

Clients dial an address but verify the certificate against a separate, fixed
label, `BO2MP_MASTER_SERVER_NAME`, so nothing in the certificate depends on the
host. When `BO2MP_MASTER_CA_CERT` is set, the client loads only that PEM into an
empty `RootCertStore` (`tls_for` in `crates/net/src/net_links/lobby_link/quic.rs`)
and skips the platform verifier. `install` signs with `San::Labels` only, so the
certificate is made once per user rather than once per server, and a new IP or
VPS keeps working with it. Ports: prod 4433, dev 4434 (UDP).

## What it stores

Nothing on disk. Rooms, the advertised match name, peers and membership exist
only in the running process and vanish when a room closes or the process
restarts. There are no accounts, history, analytics or telemetry. The only
trace is the systemd journal (startup and failures, never matches or players),
kept under the VPS's own `journald` retention. While a connection is open, peer
IPs are visible to the kernel and to any packet capture on that host, as with
any server. If you run it for other people, that is the accurate summary: it
forwards packets and vouches for no one.

## What players need

The CA file `bo2mp-ca.pem` plus the block below. Trust rests entirely on that
pinned file, so pass it over a channel you already trust.

```dotenv
BO2MP_MASTER_ADDR=1.2.3.4:4433
BO2MP_MASTER_SERVER_NAME=bo2mp-prod
BO2MP_MASTER_CA_CERT=/path/to/bo2mp-ca.pem
```

The host starts a match with `BO2MP_MASTER_HOST_NAME='name' make map mp_raid`;
everyone else joins from `make menu`. A room holds 2 to 18 players
(`BO2MP_MASTER_MAX_PLAYERS`).

## Hosting on a LAN, with no master

The hosting PC can be the lobby service itself. Start the match with a room
name and no address (`BO2MP_MASTER_HOST_NAME='name' bo2mp map mp_raid`), or put
`BO2MP_MASTER_ADDR=lan` (`lan:<port>` for another port) in its `.env` to host
from the menus. The game then opens UDP 4433 on every interface, dials it
itself, and logs `LAN host: lobby service listening on UDP 0.0.0.0:4433`.
Windows asks once whether `bo2mp.exe` may use the network; allow it on private
networks.

The first time, it makes a certificate for the name `bo2mp-lan` and keeps it
in `lan/` beside the exe (`bo2mp-lan.pem`, and the key, which never leaves the
host). Copy `bo2mp-lan.pem` to each other PC once, then give those PCs:

```dotenv
BO2MP_MASTER_ADDR=<the host's LAN address>:4433
BO2MP_MASTER_SERVER_NAME=bo2mp-lan
BO2MP_MASTER_CA_CERT=/path/to/bo2mp-lan.pem
```

and they find the room in the menus' server browser.
