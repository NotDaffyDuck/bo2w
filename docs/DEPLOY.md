# How a release ships

This page covers what `make deploy` does, what lands in the Windows folder with
nobody touching it, and what does not. The folder itself is described in
[`WINDOWS.md`](WINDOWS.md); running a master of your own is in
[`MASTER.md`](MASTER.md).

The make goals, with `prod` or `dev` after each one that ships:

* `make setup-windows` once, for the rustup target and cargo-xwin.
* `make release` builds into a local directory and leaves the server alone.
* `make publish` uploads a release that is already built (name it with `RELEASE=`).
* `make deploy` is release then publish, with the play profile unless `PROFILE=` says otherwise
  (`PROFILE=release` for the optimised build).
* `make provision` sets up the TLS certs, Caddy, the systemd units and the users.
* `make logs` shows the service log; `SINCE=2h` picks how far back.

Every recipe boils down to one `cargo xtask` command (`windows`, `release`,
`publish`, `logs`, `certs` and the provisioning one); `make` passes on only `PROFILE` and
`RELEASE`, and `cargo xtask` with no arguments prints the list. `prod` and `dev`
name the publication channel and have nothing to do with the compilation
profile. The target host and root come from `BO2MP_DEPLOY_HOST` and
`BO2MP_DEPLOY_ROOT` in `.env`. Prod uses udp/4433 and the `/prod/` path on port 8443;
dev uses 4434 and `/dev/`. Git, protocol and SHA are frozen into
`deployment.json` when `release` runs, not when `publish` does.

A routine publish compiles nothing, runs no zstd, leaves `/etc` alone, and does
not touch the master if its SHA is unchanged. The uncompressed `bo2mp.exe` stays
on the local disk; what goes up is `bo2mp-<sha>.exe.zst`. The previous archive on
the VPS is kept, and the manifest is swapped in by a rename.

Bumping `master_protocol::PROTOCOL_VERSION` also changes `ALPN`, so an old client
is turned away at the handshake instead of misreading a changed format. The
master and the clients of such a release therefore ship together, and `publish`
restarts the master because its SHA differs.

`make launcher windows` produces the portable ZIPs in `dist/windows/`. That is
not a deploy: a fix to the updater reaches a player only if `bo2mp-launcher.exe`
is copied over by hand. What the player's launcher does with `BO2MP_UPDATE_URL`
is in [`WINDOWS.md`](WINDOWS.md).

## A public release, step by step

Tag it `v0.1.0-demo.N`, title it `bo2mp Technical Demo N`, and set the
**pre-release** flag. The notes are worth writing only if they hold four things: the
commit it was built from; the platforms the archive was actually **run** on; the
demo scenario (which game, version and map, and what to do); and the known
limitations. A target that merely compiled is not a verified platform, and
neither a Linux nor a Windows release waits for the other.

Every archive holds `LICENSE`, `NOTICE` and both font licence files
(`release.rs::LEGAL_FILES`) and no game data. Inspect the archive itself, not
only the tracked files. Before any push, the tracked tree is searched by hand for
retail addresses and decompiler placeholder names. Published tags are never
moved, and history is never recreated once anything has been pushed.
