# The portable Windows folder

`make launcher windows` writes `dist/windows/bo2mp-windows-dev.zip` and
`dist/windows/bo2mp-windows-prod.zip`. Both use the password `contextrot` and
carry everything, the game binary included, so the folder runs without any
network access.

```text
bo2mp-portable/
├── bo2mp-launcher.exe    from the archive
├── bo2mp.exe             from the archive; replaced only by `bo2mp-launcher update`
├── bo2mp-ca.pem          public trust anchor
├── LICENSE NOTICE OFL-Oxanium.txt COPYING-FreeFont.txt
├── .env                  shipped in the archive, but anything the player changes there wins
├── <your shortcuts>.lnk  added by the player, pointing at an install folder or exe
└── bo2mp-artifacts/      saves, caches, demos, captures, log files
```

The four licence files matter: `bo2mp.exe` embeds both fonts with
`include_bytes!`, so any archive that ships the binary ships their licences.
`release.rs::LEGAL_FILES` is the list, and a build that goes out without them is
a bug.

Unpack into a dedicated folder and add ordinary Windows `.lnk` shortcuts that
point at an installed game folder or executable. The launcher starts the game
without changing those installs.

## Updating is opt-in

Running `bo2mp-launcher.exe` plain starts the `bo2mp.exe` already in the folder
and nothing more. To fetch a newer build, run `bo2mp-launcher.exe update`, or set
`BO2MP_UPDATE=1` for a shortcut that cannot pass an argument; either needs
`BO2MP_UPDATE_URL` in the portable `.env`. No build replaces its own binary by
itself. Publishing is covered in [`DEPLOY.md`](DEPLOY.md).

## Working folder and paths

On Windows the folder that holds the executables becomes the working directory,
even when Explorer starts the program from somewhere else. `BO2MP_GAMES`
defaults to that folder, and the targets of `.lnk` files inside it serve as extra
read-only search roots. `.env` is read before the release-manifest defaults; it
is created only when absent and never overwritten.

`.env` carries `BO2MP_UPDATE_URL=https://host:8443/prod` and the master keys
described in [`MASTER.md`](MASTER.md). A master that cannot be reached does not
stop the launch; the browser simply keeps retrying. All writable state sits
under `bo2mp-artifacts/`, including `settings.cfg` unless `BO2MP_SETTINGS_PATH`
points elsewhere. Shortcut targets are never used as output paths.
