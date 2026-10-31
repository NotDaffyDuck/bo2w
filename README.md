# bo2w: the Black Ops II fan engine (work in progress)

bo2w is a free fan remake of Call of Duty: Black Ops II for PC. Its goal is a 1:1 Black Ops II: **Multiplayer against
bots** and **Zombies**, played from your own copy of the game, on a new engine written in Rust (on
[bevy](https://bevyengine.org/) and [wgpu](https://wgpu.rs/)). Black Ops II's own scripts, menus and bots run inside
it, so matches follow the game's real rules without touching the original executable. It is never sold.

> [!IMPORTANT]
> **This repository contains NO Black Ops II assets, and you NEED Black Ops II to use it.**
>
> - **No game files of any kind are included:** no maps, models, textures, sounds, music, scripts, menus, videos or
>   executables from Black Ops II (or any other game), and no pictures of them. It is source code only.
> - **You must own Call of Duty: Black Ops II for PC** (Steam, with Multiplayer installed; the Zombies part for
>   Zombies). bo2w loads everything it shows and plays from your own installed copy. Without the game it has nothing
>   to run.
> - bo2w reads your game files read-only. It never patches, copies or redistributes them, and never touches the
>   original executable.
> - This is an unofficial, free fan project, not affiliated with or endorsed by Activision, Treyarch or Infinity Ward.
>   Buy Black Ops II to support the people who made it.

**Last updated:** 2026-10-31

> **Work in progress.** Matches against Black Ops II's own bots play on every multiplayer map, started from BO2's own
> menus, and Nuketown Zombies plays to a real game over. It is not finished: the list under [What's next](#whats-next)
> is still being worked through, one item at a time, each checked side by side with Black Ops II.

## A fresh start (2026-10-31)

This repository used to hold an older snapshot (2026-10-24) of an earlier engine plan. On 2026-10-31 `main` started
over as the fan engine on its own, 1:1 with Black Ops II as the only goal. The old snapshot is kept on the branch
`old-2026-10-24`; the `zombies` branch is unchanged.

## What works

- **Every multiplayer map and mode** from BO2's own menus: the map's windows, breakable props, moving parts, sounds and
  end-of-match nuke; whole Search and Destroy and Demolition rounds on the base maps and most DLC maps.
- **BO2's own bots**, run by the game's bot scripts: routes, jumps, ladders, mantles and windows, BO2's stuck rescue
  and avoidance, cars and vans they meet as solid.
- **Guns the BO2 way**: BO2's weapon rules, sway, bob, sprint, dive and dive get-up, hit markers with the hit sound and
  head crack, bullets through walls as in BO2 (up to five walls), and BO2's lag compensation (400 ms, with the 50 ms skip).
- **Scorestreaks** call in and work, with BO2's ragdolls and explosion deaths.
- **Killcam and final killcam** with BO2's match flags and slow motion.
- **Zombies**: Nuketown Zombies to a real game over, with the Zombies menus and map picker.

The full feature list, map table and changelog are in [MULTIPLAYER.md](MULTIPLAYER.md).

## Build and run

There are no releases yet: you build it from this tree. You need Windows x64, your own Black Ops II for PC (Steam) with
Multiplayer installed, [Rust](https://rustup.rs/) (stable) and Visual Studio 2022 Build Tools with the C++ workload.

1. Clone this repository.
2. Build: `cargo build --profile play -p launcher` (set `CARGO_TARGET_DIR` to a drive with tens of GB free).
3. Copy `<target-dir>\play\bo2mp.exe` into a play folder, and put a `.env` next to it with
   `BO2MP_GAMES="C:/Program Files (x86)/Steam/steamapps/common/Call of Duty Black Ops II"` (your install folder).
4. Run `bo2mp.exe`.

More detail (settings, mods, how it works) is in [MULTIPLAYER.md](MULTIPLAYER.md#build-and-run).

## What's next

- Bots: no more stalls in Pod's doorway or at Drone's jump spots; every map's climbs measured.
- Maps: Concert's and Hijacked's water matching BO2; side-by-side pictures of every map.
- Scorestreaks: each one checked side by side with BO2.
- A full play-test build, then the polish list in [MULTIPLAYER.md](MULTIPLAYER.md#coming-soon).

## Recent changes

- 2026-10-31: fresh start of `main` as the fan engine: bullets through walls as in BO2, BO2's lag compensation for
  players and moving map parts, killcam match flags, bots on Nuketown's jump spots and Frostbite's routes, ladders,
  water edges, and the lobby's game type setting the match rules.

## Credits and license

Licensed under the [Apache License 2.0](LICENSE); [`NOTICE`](NOTICE) lists the third-party material. Call of Duty,
Black Ops and their assets, trademarks and intellectual property belong to their owners (Activision). No game files
are included. More credits are in [MULTIPLAYER.md](MULTIPLAYER.md#credits-and-license).
