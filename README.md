# bo2w: Black Ops II Zombies (stand-alone build)

This is the `zombies` branch of bo2w, a fan rebuild of Call of Duty: Black Ops II for PC on its own Rust engine. It is
the last stand-alone Zombies build, from before Zombies is merged into the new engine on `main`. The project page,
status and plan are on the [main branch's README](https://gitlab.com/coljax-group/bo2w/-/blob/main/README.md).

**No game files are included.** Source code only. You need your own copy of Black Ops II for PC; it is read
read-only and never patched, copied or redistributed. This is an unofficial fan project, not affiliated with or
endorsed by Activision, Treyarch or Infinity Ward.

## Download

[**bo2w-zombies.zip**](https://gitlab.com/coljax-group/bo2w/-/archive/zombies/bo2w-zombies.zip), or
`git clone https://gitlab.com/coljax-group/bo2w.git` then `git checkout zombies`.

## Build and play (Windows)

1. Install **Rust** ([rustup.rs](https://rustup.rs/)) and the **Visual Studio 2022 Build Tools** with the
   "Desktop development with C++" workload.
2. In the unzipped folder: `cargo build -p launcher --profile play` (10-20 minutes the first time). The game is
   `target\play\iw4l.exe`.
3. Copy it into a play folder with a `.env` file next to it:
   `IW4L_GAMES="C:/Program Files (x86)/Steam/steamapps/common/Call of Duty Black Ops II"`
4. Run `.\iw4l.exe frontend t6:zm_nuked`, then ONLINE > SOLO PLAY opens BO2's globe.

Everything else (map progress, features, every setting, the change list): [ZOMBIES.md](ZOMBIES.md).

## Licence

Apache License 2.0 ([LICENSE](LICENSE)); it started from [IW4L](https://github.com/vladtrc/iw4L), credited in
[NOTICE](NOTICE) with the bundled wgpu-hal and fonts. Call of Duty and Black Ops II are trademarks of Activision.
