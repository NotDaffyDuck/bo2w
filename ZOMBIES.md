# bo2zm: Black Ops II Zombies on IW4L

bo2zm is a fan rebuild that plays Call of Duty: Black Ops II Zombies, starting with Nuketown Zombies, from your own
PC copy of the game, in a new engine written in Rust. It is a fork of [IW4L](https://github.com/vladtrc/iw4L), an
open-source Call of Duty runtime on [bevy](https://bevyengine.org/) and [wgpu](https://wgpu.rs/) that reads Modern
Warfare 2's files the same way. bo2zm teaches it Black Ops II's file formats, compiled game scripts, zombie AI and
UI scripts, so Nuketown runs by the game's own rules without touching the original executable. The `bo2mp`
branches use the same code to rebuild Black Ops II multiplayer.

**Status:** every Zombies map loads from BO2's globe (TranZit and its Bus Depot, Town, Farm; Nuketown; Die Rise; Mob
of the Dead; Buried; Origins), and so do the nine Black Ops maps from the map pack; Nuketown is the most finished; play-testing fixes; multiplayer ships as its own app ·
**Visibility:** private for now · **Last updated:** 2026-10-12

**No game files are included.** This repository has no Black Ops II (or Modern Warfare 2) files: no maps, models,
textures, sounds, scripts or executables. You need your own copy of Black Ops II for PC; bo2zm reads its files
read-only and never patches, copies or redistributes them. This is an unofficial fan project, not affiliated with or
endorsed by Activision, Treyarch or Infinity Ward.

## Latest update (2026-10-12)

Build zm-cloud11 was played on every map in one sitting on 2026-10-11: the six Black Ops II maps, Town, and the nine
Black Ops maps.

- **A cheat menu for solo games.** Num 1 opens it and pauses the game (Black Ops II's pause menu stays shut); Up and
  Down move, Enter picks, Num 1 or Esc closes it. +1000 Points (pick it again and again), God Mode, No Clip (fly
  where you look, Shift for faster), Zombies Ignore You, Infinite Ammo, End Round, Drop Power-Up, Power On, Open All
  Doors, Pack-a-Punch Gun, All Perks, and a Special Weapons tab: every wonder weapon, staff, craftable, shield and
  special grenade the map has, by name; the one you pick goes straight into your hands (Esc goes back). Only in a
  game with one player; Public Match and co-op never get it. Tested
  in play on 2026-10-12.
- **Mob of the Dead: feeding Cerberus.** The wolf heads show, hide and move as the game says; a zombie killed at a
  head is pulled into its mouth and bitten, with blood at the mouth.
- **Mob of the Dead: the Afterlife shock strikes once.** A revive in Afterlife no longer leaves its electricity
  striking over and over.
- **Every map played through**, the nine Black Ops maps included (Nacht der Untoten, Verrückt, Shi No Numa,
  Der Riese, Kino der Toten, Five, Ascension, Shangri-La, Moon).

| | |
|---|---|
|  |  |
| Mob of the Dead: a Cerberus head bites the zombie it pulled in | Mob of the Dead: Afterlife, shocking a panel |
|  |  |
| The cheat menu (Num 1), at TranZit's bus depot | Origins: a giant robot walks over the dig site |

Also in this build, in the code and waiting for a check in play: zombie voices and the announcer as loud as in
Black Ops II, boards that turn with the zombie pulling them, box guns with their magazines in place, Buried's Ray Gun
Mark II blast and a fired Ballistic Knife that hurts, TranZit's bus windows turning with the bus, a smooth Die Rise
lift ride, Mob of the Dead's Afterlife time bar, and more of Black Ops II's own HUD (the part you carry, the
tomahawk, the Vulture meter, Buried's Time Bomb). Everything older is in [Recent changes](#recent-changes).

## What's new since the 2026-10-09 release

Everything that went in after the 2026-10-09 release and its gameplay video, grouped by map. Each line is in
[Recent changes](#recent-changes) with the full detail.

**Every map**
- A cheat menu for solo games on Num 1 that pauses the game (points, God Mode, No Clip, power, doors,
  Pack-a-Punch, perks and more).
- Barricades: the "Hold F to rebuild" prompt shows at a window the zombies are tearing down.
- A dead zombie's eyes go dark.
- Going down turns the screen grey and it clears on revive; after a solo Quick Revive you stand back up.
- You stand back up after a revive even with a zombie or another player on top of you.
- Wall buys (guns, grenades, the Claymore, the Bowie Knife, the Galvaknuckles) slide straight out of the wall.
- Perk machines light the walls and floor around them in their own colour once the power is on.
- An empty gun switches to your next one.

**Movement, by Black Ops II's own numbers**
- The dolphin dive step for step: dive in any direction, aim and fire through it, a quicker get-up.
- Black Ops II's PC keys: F use, G grenade, 4 tactical, C stance, CTRL prone. Omni movement is off by default.
- Jumps, falls, climbing and slides use the game's own numbers; Stamin-Up and PhD Flopper work.
- Stairs and slick ground slow you, landing sounds follow the fall height, and hits kick the view.
- Faster first shots on the HAMR, AN-94 and B23R.

**TranZit**
- The bus: its loop, the stop at each station, riding inside and on the roof, the roof hatch, the plow.
- Richtofen's side of the easter egg works end to end; the Denizen portals at the lamp posts carry you.
- The Navcomputer, the Jet Gun (spins up, heats, overheats), the Zombie Shield.
- Juggernaut takes you to 250 health; Grief at the Bus Depot.

**Die Rise**
- Who's Who, the Sliquifier and the bank.
- Easter egg step 2 (the lion balls), the dragon's sparks and fireball.
- Zombies climb between the buildings along the map's own routes.
- The Mystery Box, the elevator shafts, Pack-a-Punch, the dragon rooftop and the start spot match Black Ops II
  side by side.

**Buried**
- The Time Bomb, the Paralyzer and the turbine; a stuck Claymore arms.
- Step 3 of the lantern quest; the ghosts checked against Black Ops II.

**Mob of the Dead**
- Brutus follows his real route, locks perk machines, swings, and throws smoke when his helmet comes off.
- In Afterlife you walk through the ghost-only doors; alive, they stop you.
- Feeding Cerberus: a zombie killed at a wolf head is pulled into its mouth and bitten.
- The Afterlife shock from a revive strikes once instead of over and over.

**Origins**
- The Panzer Soldat uses his flamethrower, his faceplate comes off, and head shots finish him.

**More maps and modes**
- Turned on Diner is on the menus: you play the zombie.
- Nuketown's sky, the Bus Depot glow and Shi No Numa's swamp water fixed.
- The nine Black Ops maps all played through in one sitting (2026-10-11).

**Online play** (being finished last)
- Custom Games with a friend (host, join, kick); Public Match finds or hosts a game.
- After bleeding out you watch a teammate until the next round.

**Still being fixed.** Play-testing found these still wrong, and they are next: the TranZit bus (zombies and the
bus, its route, the tyres) and its part spots, the Buried ballistic knife and Ray Gun Mark II, the Die Rise elevator
ride and one doorway, the Mob of the Dead Afterlife time bar, the dive get-up, and boards glitching while zombies pull
them. Fixes for several of these are in build zm-cloud11 and wait for a check in play. See
[Coming soon](#coming-soon).

## Contents

- [Latest update](#latest-update-2026-10-12)
- [What's new since the 2026-10-09 release](#whats-new-since-the-2026-10-09-release)
- [Map progress](#map-progress)
- [Features](#features)
- [Requirements](#requirements)
- [Install](#install)
- [Run / Play](#run--play)
- [Configuration](#configuration)
- [How it works](#how-it-works)
- [Project layout](#project-layout)
- [Development](#development)
- [Coming soon](#coming-soon)
- [Recent changes](#recent-changes)
- [Credits and license](#credits-and-license)

## Map progress

<!-- map-bars:start -->
```text
                    Mechanics         Visuals
Nuketown            ██████████ 100%   █░░░░░░░░░   7%
TranZit             ██████████ 100%   ░░░░░░░░░░   0%
Die Rise            ██████████  98%   █████████░  89%
Mob of the Dead     ██████████  97%   █░░░░░░░░░   6%
Buried              █████████░  94%   ██░░░░░░░░  19%
Origins             █████████░  94%   █░░░░░░░░░   6%
Nacht der Untoten   ███░░░░░░░  27%   not compared yet
Verrückt            ██░░░░░░░░  15%   not compared yet
Shi No Numa         ██░░░░░░░░  17%   not compared yet
Der Riese           ████░░░░░░  36%   not compared yet
Kino der Toten      █░░░░░░░░░  14%   not compared yet
Five                █░░░░░░░░░  14%   not compared yet
Ascension           ████░░░░░░  43%   not compared yet
Shangri-La          █░░░░░░░░░  14%   not compared yet
Moon                █░░░░░░░░░  14%   not compared yet
All maps            ██████░░░░  58%   █░░░░░░░░░  14%
```

Mechanics: each map's systems and modes work by Black Ops II's rules. Visuals: set spots compared side by side with Black Ops II. Scored in [`docs/maps.json`](docs/maps.json) (pass or match 1, part or close half, not yet 0), last updated 2026-10-10; drawn by [`docs/make_bars.py`](docs/make_bars.py).
<!-- map-bars:end -->

## Features

- **Boots from Black Ops II alone.** `map t6:zm_nuked` loads Nuketown Zombies from the BO2 install; no MW2 files
  are needed for it. All 196 BO2 zone files read to the exact byte.
- **Looks and sounds like BO2.** The map with its textures, baked lighting, sky, fog, shine, glass, bloom and
  effects; BO2's sound banks; every Nuketown gun in first person with flashes, brass, tracers, bullet holes,
  grenades, the knife, dual wield and Pack-a-Punch camos; BO2's fonts and chalk round marks.
- **Plays by the game's own scripts.** BO2's compiled server scripts run in the engine's script VM: zombies rise
  from the ground and climb fences, rounds speed up from walk to sprint, points, wall buys, a Mystery Box that
  moves, perks dropping from the sky, Pack-a-Punch, power-ups, going down and game over, then a new game.
- **Zombie AI rebuilt.** Actors with BO2's animation states, A* over the map's path nodes with jump and climb
  links, body-blocking crowds, doors and debris that cut paths, BO2's "bad path" fallback to the player's trail.
- **BO2's own HUD and menus.** A HavokScript (BO2's UI Lua) reader and VM runs BO2's own HUD scripts; Esc opens BO2's
  pause menu (Resume, Restart Level, Options, End Game); Options cover key binds (including aim-down-sight hold),
  FOV, resolution, display mode, shadows, volume and mouse. Keyboard, mouse and controller in the menus.
- **The nine Black Ops maps** from the map pack sit on the globe next to Black Ops II's own: Nacht der Untoten,
  Verrückt, Shi No Numa, Der Riese, Kino der Toten, Five, Ascension, Shangri-La and Moon.
- **A cheat menu for solo games** (Num 1) that pauses the game, to reach any part of a map fast.
- **Everything IW4L already had:** walking, sprinting, prone, mantling, collision, a fixed-step simulation,
  settings and logs kept next to the binary.

## Requirements

- **Your own Call of Duty: Black Ops II for PC** (Steam), with the Zombies part installed. The `bo2mp` branches
  also need Multiplayer installed (Steam app 202990).
- **Windows x64** for the BO2 build (built and played on Windows 11 only so far). Upstream IW4L also builds for
  Linux and macOS (MW2 only).
- **Rust** (stable toolchain via [rustup](https://rustup.rs/); the code uses the 2024 edition) and
  **Visual Studio 2022 Build Tools** with the C++ workload (MSVC).
- A DirectX 12 or Vulkan capable GPU. Development and testing used an RTX 4070 Ti at 1920x1080.
- Tens of GB free for the Rust build output (many small files; a hard drive or a roomy SSD).

## Install

There are no releases: you build it from this tree. These steps were checked on 2026-10-09 from a fresh clone of the
public copy into an empty folder, on the author's PC; they have not been tried on another PC yet.

1. Install Rust and the Visual Studio 2022 Build Tools (C++ workload).
2. Clone the repository and check out the branch: `git checkout zombies`.
3. Build the game (about 12 minutes the first time):

   ```powershell
   cargo build -p launcher --profile play
   ```

   The binary is `target\play\iw4l.exe`.
4. Make a play folder anywhere, copy `iw4l.exe` into it, and create a `.env` file next to it that points at your
   Black Ops II folder itself (quoted, with forward slashes), for example:

   ```text
   IW4L_GAMES="C:/Program Files (x86)/Steam/steamapps/common/Call of Duty Black Ops II"
   ```

   bo2zm finds the game's zone files inside that folder (`common_zm.ff`, or `common_mp.ff` for multiplayer).
   Point it at the Black Ops II folder, not at the whole Steam library: discovery walks every file under the
   folder it is given (once per game it looks for), which takes many minutes on a big library. The game folder
   is only read.

## Run / Play

From the play folder:

```powershell
.\iw4l.exe frontend t6:zm_nuked
```

This opens on BO2's own main menu: ONLINE > SOLO PLAY opens BO2's globe. Turn it with Left/Right and press Enter on
a map, pick a start and a mode, then START MATCH (or CUSTOM GAMES for the starting round and difficulty):

| Map on the globe | Starts and modes |
|---|---|
| TranZit | TranZit (story), Bus Depot (Survival), Town (Grief, Survival), Farm (Grief, Survival), Diner (Turned) |
| Nuketown | Survival |
| Die Rise | Die Rise (story) |
| Mob of the Dead | Mob of the Dead (story), Cell Block (Grief) |
| Buried | Buried (story), Borough (Grief, Turned) |
| Origins | Origins (story) |
 After a game it goes back to the lobby. `.\iw4l.exe map t6:zm_nuked` skips
the menu and goes straight into Nuketown.

A Windows shortcut with that target and "Start in" set to the play folder works the same way.

The first start after a reboot is slow: it reads the game's image packs from disk, which took about 3 minutes with
Black Ops II on a hard drive (materials 73 s, weapons/effects/sound 86 s on the 10-09 fresh-clone run). Later starts
are much quicker.

- Keys follow BO2's PC defaults: WASD, Space to jump, G to throw a grenade, 4 for the Monkey Bomb, 2/3/5 for the
  d-pad slots, Esc for the pause menu.
- Change keys, FOV, resolution, display mode (borderless only), shadows, volume and mouse in Esc > Options.
- Settings, caches, logs and demos go to `iw4l-artifacts\` next to `iw4l.exe`.

## Configuration

- `.env` next to the binary: `IW4L_GAMES` (required), plus upstream's optional master-server settings (see
  `.env.example`; not needed for solo play).
- `iw4l-artifacts\settings.cfg`: saved settings (`fullscreen`, resolution, `vsync`, `bo2_quality=full`).
- `BO2ZM_LUI=0` brings back the simple hand-made HUD instead of BO2's own.
- Test and debug switches (`IW4L_T6_*`, `IW4L_SOUND_LOG`, ...) are listed in `docs/BO2ZM.md`.

## How it works

IW4L's design, unchanged by the fork:

| | |
|---|---|
| assets | Game zones read natively into one intermediate form (`asset_iw4` IR); MW2, MW3 and Black Ops 1 upstream, Black Ops II added here |
| shaders | MW2's D3D9 shaders translated to WGSL, so no DirectX at runtime; BO2 surfaces use WGSL written for BO2's material families and draw states |
| rendering | One sorted draw-surface list |
| physics | Fixed 17 ms step on its own accumulator; frame rate changes nothing about how a body falls |
| simulation | One `TickInput -> sim::step -> Snapshot` funnel for server, prediction and replay |
| network | Custom peer-to-peer protocol over UDP; IW4L clients only talk to IW4L clients |

What bo2zm adds, mostly as new crates so upstream merges stay clean:

| Crate | What it does |
|---|---|
| `fastfile_t6` | BO2 zones: envelope, XChunk streams with the public PC key, deflate, and a generated walk over all 60 asset types |
| `ipak_t6`, `sab_t6` | BO2 image packs and sound banks |
| `asset_t6` | the BO2 world, materials, models, weapons, effects and fonts captured into the engine's form |
| `gsc_t6` | BO2's compiled game scripts, translated into IW4L's script VM |
| `hks_t6` | HavokScript (BO2's UI Lua) reader, VM and LUI widgets for the HUD and menus |
| `sim` (`t6` modules) | zombie actors, path nodes and A*, risers, the Mystery Box, last stand, BO2's engine functions |
| `bo2zm_tools` | `t6zone` (zone checker), `t6census` and other probes |

The full notes are in `docs/BO2ZM.md`; upstream's notes start at [`docs/INDEX.md`](docs/INDEX.md).

## Project layout

| Path | What lives there |
|---|---|
| `crates/` | the engine: about 70 crates (assets, render, sim, net, ui, ...) including the BO2 ones above |
| `docs/` | one short note per area; `BO2ZM.md` for the fork |
| `xtask/`, `scripts/`, `Makefile` | upstream's build, release and run helpers (Linux-first) |
| `.env.example` | the environment file template |

## Development

- Build: `cargo build -p launcher --profile play`; tools: `cargo build -p bo2zm_tools --profile play`.
- Check that BO2 zones read to the byte: `t6zone --walk --nuketown "<Black Ops II folder>"`.
- Tests: `cargo test -p fastfile_t6` (and `cargo test -p <crate>` for the others).
- `crates/fastfile_t6/src/walk_gen.rs` is generated by `crates/fastfile_t6/gen/t6gen.py` from structure facts in a
  local [OpenAssetTools](https://github.com/Laupetin/OpenAssetTools) checkout; do not edit it by hand.
- Upstream's `make` recipes, Linux packages and Windows cross-build are in [`docs/BUILD.md`](docs/BUILD.md),
  [`docs/RUN.md`](docs/RUN.md) and [`docs/WINDOWS.md`](docs/WINDOWS.md).
- Upstream is pinned at IW4L `11514ea` (2026-09-30). Merges from upstream happen on purpose, never automatically;
  the `upstream` remote has pushing disabled.
- Like upstream IW4L, this code is written by an LLM (Claude), directed and play-tested by the author.

## Coming soon

> Keep this list current: when an item ships, move it to Recent changes with the date, then add what is next.

Specialty weapons, still to match side by side:

- [ ] The Special Weapons tab on every map: the 31-79 JGb215 shrinking zombies away from Shangri-La (it fires and kills, but they stay full size); a check of the Gersh Device, QED and Matryoshka Dolls away from their own maps; the Portal Gun in the tab (it works in Minecraft Zombies and multiplayer)
- [ ] The G-Strike throwing the zombies it kills; the Galvaknuckles lighting up the zombie they hit; the shrink ray's blue puff; the Gersh Device portal's white-cyan core (pink here)

Zombies, from the play-test of 2026-10-10 (each ships only once it matches Black Ops II side by side):

- [ ] Every map: zombies tearing boards off a window without glitching (the one-frame jump is removed; awaiting a
  check against real Black Ops II)
- [ ] Every map: zombie voices as loud as in Black Ops II (a fix is in build zm-cloud11, to check in play)
- [ ] Every map: box guns with their magazines in the right place
- [ ] Every map: standing up after a dive at Black Ops II's speed
- [ ] Buried: the Ray Gun Mark II's damage and blast; the box less bright (its question marks visible); patchy lighting;
  the Ballistic Knife (the stab, its place in the hand, the fired blade hurting zombies); zombies that find
  their way to you from every spot
- [ ] TranZit: zombies climbing onto the bus; the bus's route, its barricades, its driving feel and turning wheels; parts in random spots each game
- [ ] Die Rise: sound cut-outs (to recheck on an idle PC); riding the lift smoothly; a doorway after the lift drop that
  blocks only the player
- [ ] Mob of the Dead: knifing zombies through windows; zombie heads drawn white; the Afterlife time bar (in build
  zm-cloud11, to check in play); colours; feeding Cerberus and the one-strike Afterlife shock checked side by side
  with Black Ops II (both work in play since 2026-10-11)
- [ ] Origins, after a full side-by-side look at the map: planes moving in the sky, colours, the invisible wall at
  Generator 2, the tank the player walks through, pipes and props out of place
- [ ] Testing: the real Black Ops II capture rig cannot turn the view yet, so window shots wait for it

Also coming:

- [ ] The play-test fixes of 2026-10-04 confirmed (grenade key, zombies unstuck, menu blur, downed camera)
- [ ] Front-end details: film playback in Theater, the Sound Device row switching audio output, a rank earned in play
- [ ] Games inside games: a Minecraft world and a skate mode in the same engine, as in community IW4L mash-ups
- [ ] Nuketown: the play-test of 2026-10-07 confirmed (flying mannequin heads, both song eggs, the new sounds, Fire Sale)
- [ ] Nuketown: the 2026-10-08 fixes confirmed in play (dive sounds, pain grunts, controller ammo, cupboard colour)
- [ ] Nuketown: the black smoke cloud at the start confirmed gone in play
- [ ] Die Rise: the cloud floor under the rooftops (cloud shapes below the roofs, clear dark sky above them)
- [ ] Die Rise: the red light on the back wall of the Jugger-Nog lift
- [ ] Zombies go limp and fall when they die (ragdolls); Die Rise's Trample Steam throws them
- [ ] Nuketown: no "0/0" ammo while drinking a perk or using Pack-a-Punch, confirmed in play
- [ ] Nuketown full play-through to 100% on the finished map
- [ ] Survival: Town confirmed in play (pick Tranzit on the globe, then Town, Survival, Start Match)
- [ ] Every map on the globe confirmed in play (2026-10-08 build: TranZit, Die Rise, Mob of the Dead, Buried, Origins)
- [ ] Mob of the Dead: Afterlife's own meter in place of the gun's ammo count
- [ ] Grief and Turned: the round scoreboard drawn on the sign in the world, like BO2
- [ ] Grief: the start text checked in play (no "You have been given another chance" ghost copy)
- [ ] Die Rise, Mob of the Dead, Buried and Origins: their loading movies (the painted loading picture shows for now)
- [ ] The TranZit bus and the other maps' own quests, one map at a time
- [ ] TranZit: Maxis's side of the easter egg
- [ ] The nine Black Ops maps, one at a time: box, perks, power and Pack-a-Punch checked where the map has them (each
  map played through on 2026-10-11)
- [ ] Black Ops II movement confirmed in play (sprint, dive to prone, falls, gun sway)
- [ ] Standing back up after a revive confirmed in play
- [ ] Multiplayer (`bo2mp` branches): Team Deathmatch against bots on Nuketown 2025
- [ ] Multiplayer: BO2's menus and player profile, every mode, scorestreaks, every map
- [ ] Last, after everything above: Zombies co-op for 2-4 players over the home network (Public Match, Custom Games
  with Friends, the party following its leader; tested so far with two copies on one PC)

## Recent changes

- 2026-10-15 Public release v2026.10.15: a new Windows build with every change below, and the public README rewritten (download first, the full special weapons list)
- 2026-10-15 Minecraft Zombies' guests in the Special Weapons tab on every map: the Gravity Gun and Super Gravity Gun, the Bow and Enchanted Bow, and the Enchanted Stick
- 2026-10-15 Black Ops II's gun rules: your gun stays in your hands over low climbs and goes away for high ones, with the gun's own climb animation; a slow grenade settles where it lands; sticky grenades stick to a riot shield; test logs no longer stutter the game when the disk is busy
- 2026-10-15 More specialty weapons on any map from the Special Weapons tab: TranZit's riot shield, Origins' G-Strike (its robot now fires on every map) and One Inch Punch, and Black Ops I's Wunderwaffe DG-2, Winter's Howl and Zap Guns
- 2026-10-15 More specialty weapons on any map from the Special Weapons tab: TranZit's EMP grenade, turbine, turret and electric trap (the turret and trap run off a turbine placed beside them), Buried's Subwoofer, Trample Steam and Head Chopper (place it on a wall; it cuts standing zombies' heads off on maps whose zombies are shorter for touch checks too), and Buried's Time Bomb (throw it with the tactical key; firing the detonator later takes the game back to that moment: the round, your points and your guns)
- 2026-10-15 More specialty weapons on any map from the Special Weapons tab: TranZit's Jet Gun (zombies sprint into it with TranZit's own pull and grind-up animations; it overheats), Buried's Paralyzer (each hit slows a zombie further until it dies; the heat meter shows on the HUD), Die Rise's Sliquifier, Origins' four elemental staffs, and Black Ops I's Thunder Gun and Hacker
- 2026-10-15 Specialty weapons on any map: Mob of the Dead's Hell's Retriever, Hell's Redeemer, Blundergat, Acidgat and spoon/spork now come from the cheat menu's Special Weapons tab on every map, working as they do on Mob (charged throws, acid darts that stick and blow up). Loading them adds about 1.5 seconds.
- 2026-10-14 Specialty weapons: the Hell's Retriever and Hell's Redeemer charge like Black Ops II. Hold the throw 1 to 2 seconds or longer for a stronger axe (the Redeemer kills anything it hits), with the charge glow; the Redeemer has its own red glow, trail and hits when given from the cheat menu
- 2026-10-14 Specialty weapons: the Thunder Gun's shot bursts white before its rings, as in Black Ops I; the Hacker's progress bar fills while you hack; the Ballistic Knife has its reticle
- 2026-10-14 Just for fun, five guest weapons in the cheat menu's Special Weapons tab on every map: Half-Life 2's Gravity Gun and Super Gravity Gun (punt zombies, or grab one and throw it into the others) and Minecraft's Bow, Enchanted Bow and Enchanted Stick (arrows stick in the ground; the stick knocks zombies flying). Their pictures and sounds are read from your own Half-Life 2 and Minecraft files when you have them; without them they draw as simple stand-ins
- 2026-10-14 Maps load faster the first time: each shader is compiled once instead of once per lighting pass (about 28% fewer compiles).
- 2026-10-14 The cheat menu's Special Weapons tab: it lists every special weapon the map has by name (Ray Guns, Thunder Gun, Jet Gun, the Origins staffs, shields, craftables, Monkey Bombs and the rest) and the one you pick goes straight into your hands
- 2026-10-14 More specialty weapons: the Jet Gun pulls zombies in at its own drag speed and grinds them up; Buried's Head Chopper goes on the wall at head height and chops again and again; the Subwoofer knocks zombies down and throws them; zombies shrunk by the shrink ray can be kicked; Zombie Blood turns the screen yellow; Claymores show their red lasers; the EMP grenade stuns zombies; Trample Steam throws the player and the zombies it hits; the Matryoshka Dolls and the QED work on the Black Ops maps; the Gersh Device pulls zombies into its portal, with its event horizon and purple vision
- 2026-10-13 More specialty weapons: Winter's Howl freezes, crumples and shatters zombies as in Black Ops; the Wave Gun and the Wunderwaffe checked against Black Ops; the One Inch Punch throws zombies; the G-Strike's shells kill every zombie near where it lands; the Blundergat's pellets spread in Black Ops II's flat line and Double Tap II doubles them; Origins' upgraded staffs charge while you hold fire (three levels, each with its own sound and controller rumble) and fire the charged shot; the Galvaknuckles, the Bowie Knife and the Golden Spork swing as themselves
- 2026-10-13 Specialty weapons: the Thunder Gun throws zombies with Black Ops II's own push (the body leaves at about 6,000-12,000 units a second, hits walls and ceilings and falls limp); the Ray Gun Mark II fires as in Black Ops II; the Hell's Retriever throws from its own slot (a charged throw kills a row of zombies); the Baby Gun's shrink script runs (the small zombies still to check)
- 2026-10-12 Build zm-cloud11 played on every map in one sitting (the six Black Ops II maps, Town, the nine Black Ops maps); new pictures of the Black Ops maps, Origins, Cerberus, Afterlife and the cheat menu in Screenshots
- 2026-10-11 A cheat menu for solo games on Num 1: it pauses the game without Black Ops II's pause menu; +1000 Points, God Mode, No Clip, Zombies Ignore You, Infinite Ammo, End Round, Drop Power-Up, Power On, Open All Doors, Pack-a-Punch Gun, All Perks (never in Public Match or co-op); tested in play on 2026-10-12
- 2026-10-11 Mob of the Dead: feeding Cerberus: the wolf heads show, hide and move by the game's own world field; a zombie killed at a head is pulled into its mouth and bitten, with blood at the mouth (seen in play)
- 2026-10-11 Mob of the Dead (every map): an effect started on a model the moment it appears plays once, so the Afterlife shock after a revive strikes once (seen in play)
- 2026-10-11 In the code, to check in play: the announcer and your own lines heard at full volume; boards turn with the zombie pulling them; box guns with their magazines in place; Buried's Ray Gun Mark II blast and a fired Ballistic Knife that hurts; TranZit's bus windows turn with the bus; a smooth Die Rise lift ride; Mob of the Dead's Afterlife time bar; Black Ops II's HUD gets the part you carry, the tomahawk, the Vulture meter, Mob's collected parts and Buried's Time Bomb
- 2026-10-11 Barricades: the hold-to-rebuild prompt shows at a window zombies are tearing down (checked on TranZit, started from the game's own menus); repair speed and repair points checked against Black Ops II and already the same (points stop at the round's cap: 50 per round, at most 500)
- 2026-10-11 A dead zombie's eyes go dark, as in Black Ops II: the glow goes off and the eyes dim to Black Ops II's value (checked on TranZit; living zombies keep their map's eye colour)
- 2026-10-10 Test games cost less: a no-graphics test mode (no window, no graphics card, no sound), a lighter quality setting for hidden test windows, and a game window in the background draws 30 frames a second
- 2026-10-10 Movement: Black Ops II's rules checked line by line: aiming while walking is slower, less steering in a dive, no jumping on slick ground, a wall graze after landing keeps your speed, stairs slow you, landing sounds by fall height, aiming lying down holds you still, knifing while aiming keeps the sights
- 2026-10-10 Buried: knifing a model or switch counts as the knife, as in BO2; the Paralyzer fires on the amplifiers
- 2026-10-10 Movement: Black Ops II's dolphin dive step for step, Black Ops II's PC keys (F use, G grenade, 4 tactical, C stance, CTRL prone), Black Ops II walk speeds; Omni movement is off by default, with a switch in Controls > Move
- 2026-10-10 Buried: the Time Bomb and the Paralyzer work, a stuck Claymore arms, the turbine works
- 2026-10-10 Perk machine lights: at most four effect lights at once, the most important first, as in Black Ops II (no more bright spots where several machines meet)
- 2026-10-10 Wall chalk only on real wall buys; glowing-only surfaces no longer take the map's light
- 2026-10-10 Die Rise: the easter egg plays end to end
- 2026-10-10 Wall buys: the Claymore and every other wall item (grenades, the Bowie Knife, the Galvaknuckles, guns) slide straight out of the wall when bought, as in BO2, instead of in from the side
- 2026-10-10 Perk machines light the walls and floor around them in their own colour once the power is on (Speed Cola green, Jugger-Nog red), matched side by side with BO2 on Die Rise
- 2026-10-10 Mob of the Dead: Brutus finds you by his real route, locks perk machines and swings
- 2026-10-10 Buried: zombies keep to the floor on straight walks, the ballistic knife sets off what it passes through, step 3 of the lantern quest
- 2026-10-10 Die Rise: the easter egg's reward flies to the player's waist, as in BO2
- 2026-10-10 TranZit: every mechanic on its list passes for one player; the bus (its loop, the stop at each station, riding on the roof, the roof hatch, the plow) was the last one
- 2026-10-10 Die Rise: the Mystery Box and the elevator shafts match BO2 side by side; the cloud floor under the rooftops is the one spot still wrong
- 2026-10-10 Co-op: the host can remove a player from the Custom Games lobby as in BO2 (click the player, KICK, Yes); that player goes back to the main menu with BO2's notice. When the host leaves a Custom Games lobby, it ends for everyone else with BO2's "The party is no longer active" notice. The joining player's game clock catches up the way BO2's does (it only jumps after falling a full second behind)
- 2026-10-10 TranZit: Richtofen's side of the easter egg works end to end (overheat the Jet Gun in the tower, explosive kills around it, EMP the four safety lights, then his reward), and the Denizen portals at the lamp posts carry you to another lamp post
- 2026-10-10 Die Rise: the start spot matches BO2 side by side (desk, chairs, pillar and tower line up)
- 2026-10-10 Co-op: the player who joins a friend's Custom Games lobby sees it as a guest, as in BO2 (only the host has START MATCH, the map and the difficulty); the Friends list keeps the picked friend highlighted when it refreshes
- 2026-10-10 Die Rise: Pack-a-Punch and the dragon rooftop compared side by side with BO2 and match; an object attached to a moving one keeps its own turn, as in BO2
- 2026-10-10 TranZit: players riding the bus (inside and on the roof) stay with it; the Galvaknuckles; the Navcomputer is built from its four parts and takes the navcard; an empty gun switches to the next one as in BO2
- 2026-10-10 Co-op: Custom Games and Friends work over the relay: one player hosts from CUSTOM GAMES, the other presses F (Friends), sees that game and joins it, and START MATCH takes both into the map (two players tested together in Die Rise on one PC); a big map's first snapshot (Die Rise) no longer stops the second player joining
- 2026-10-10 Game scripts: lists walk in BO2's own order (oldest first, checked in BO2's engine code); Die Rise's and Buried's easter eggs rely on it
- 2026-10-10 Die Rise: the Sliquifier's puddles dry up on their own, as in BO2
- 2026-10-10 Die Rise: Who's Who, the Sliquifier, easter egg step 2 (the lion balls), the egg's dragon sparks and fireball, the map's moving props, Maxis's voice at the nearest speakers; the revive text names the player; zombies stay solid every tick, so the Sliquifier bolt hits them
- 2026-10-10 Game scripts: a deleted object now behaves as in BO2 (checked in the game's own engine code): a script waiting on it stops, a script that only watched it keeps going. Die Rise's easter egg ball and Buried's lantern pickup rely on this
- 2026-10-10 Public Match: ONLINE > PUBLIC MATCH looks for a game on the relay with the same map and mode and joins it, or hosts its own; the players who join fill the lobby rows and the countdown starts the match for everyone (two players tested together in Nuketown)
- 2026-10-10 Black Ops II movement: BO2's own numbers for jumps, falls, dives and climbing; steer in a dive, slides that keep their speed off a ledge, Stamin-Up and PhD Flopper work, slick ground and shellshock slow you as in BO2, hits kick the view by how much health they take
- 2026-10-10 TranZit: walking and collision checked against BO2 (open ground and real walls), Grief at the Bus Depot (Mechanics 95%)
- 2026-10-10 Co-op: a player who bleeds out watches a teammate through their eyes, with the teammate's gun, ammo and grenades on the HUD (his own points, as BO2), and is back in at the next round
- 2026-10-10 Die Rise: the Mystery Box checked against BO2 (Mechanics 100%); zombies climb between the buildings along Die Rise's own routes; the bank takes deposits and pays them back; easter egg step 1 (the car roofs and the floor dragons)
- 2026-10-09 TranZit: the perks checked against BO2 (Juggernaut takes you to 250 health, as the game's own table; Mechanics 86%); the bus: jump under the hatch, let go and jump again to climb onto the roof; riders stand on the bus floor; the Jet Gun spins up, heats and overheats, the Zombie Shield is raised and planted
- 2026-10-09 Buried: the ghosts checked against BO2 (Mechanics 94%); the Mystery Box and main street compared side by side (close); the ballistic knife comes to rest where it lands
- 2026-10-09 Die Rise checked against BO2: power-ups, going down (revive and game over), the elevators, Pack-a-Punch and falling off the buildings (Mechanics 94%)
- 2026-10-09 Mob of the Dead: the Afterlife checked: as a ghost you walk through the ghost-only doors, alive they stop you (Mechanics 97%)
- 2026-10-09 Mob of the Dead: Brutus throws his smoke grenades when his helmet comes off
- 2026-10-09 Co-op: things shown to one player only (Mob's Afterlife doors and props) are drawn and solid for that player alone
- 2026-10-09 Co-op: a second player no longer drops out of the match (two players stayed in a 4-minute game)
- 2026-10-09 Origins: the Panzer Soldat checked: he uses his flamethrower, his faceplate comes off and head shots finish him (Mechanics 94%)
- 2026-10-09 Going down: after a revive you stand back up like BO2, even with a zombie or another player on top of you; the heartbeat while you crawl keeps BO2's timing
- 2026-10-09 Origins: wall buys, going down (revive and game over) and power-ups checked (Mechanics 89%)
- 2026-10-09 Mob of the Dead: rounds, zombies spawning and finding the player, points and wall buys checked (Mechanics 91%)
- 2026-10-09 Buried checked against BO2: wall buys, the box, power, perks, rounds and points, walking, doors, power-ups, Leroy, Pack-a-Punch, going down and being revived (Mechanics 89%)
- 2026-10-09 Mob of the Dead checked against BO2: power-ups, Pack-a-Punch, doors, the box, power, perks, walking, going down, revive and game over (Mechanics 68%)
- 2026-10-09 Die Rise checked against BO2: walking and collision, doors and debris, Jumping Jacks (Mechanics 75%)
- 2026-10-09 Origins checked against BO2: walking, points and the generators (Mechanics 72%)
- 2026-10-09 Black Ops II's faster first shots, and the HAMR and B23R as BO2 has them
- 2026-10-09 Install checked from a fresh clone into an empty folder: `.env` now points at the Black Ops II folder itself (pointed at the whole Steam library, discovery searched for more than 7 minutes before the game started); first-start time noted
- 2026-10-09 Turned: as a zombie your screen matches BO2, with no points box and no blood splat where the ammo goes (survivors keep both)
- 2026-10-09 Turned at the Diner is on the menu and plays: BO2 loads it as its own small map (the Diner cut out of TranZit) with Turned's scripts; the white screen in game is gone (Turned's round scoreboard belongs on a sign in the world, not on the screen)
- 2026-10-09 Turned: as a zombie you see both zombie claws, like BO2 (Borough and the Diner)
- 2026-10-09 Nuketown's checks are done (Mechanics 100%): walking and collision, every power-up it has (Max Ammo, Insta-Kill, Double Points, Nuke, Fire Sale), going down, solo Quick Revive and game over, all against BO2; the "Max Ammo!" words now show
- 2026-10-09 Shi No Numa's swamp is dark murky water instead of a flat bright-blue sheet
- 2026-10-09 Origins, Die Rise, Mob of the Dead and Buried show their painted loading picture instead of a plain white screen
- 2026-10-08 Mob of the Dead: your body now stays visible where you left it while you are in Afterlife
- 2026-10-08 Ascension: you no longer start stuck inside the lander and unable to move
- 2026-10-08 Mob of the Dead: Afterlife looks like BO2: the blue ghost screen, see-through glowing ghost hands, and sparks at the wrists, and its centre aiming mark
- 2026-10-08 TranZit: the props from its area files are in, about 450 of them, including the Bus Depot route-map kiosk, seats, lockers, the Diner and Town
- 2026-10-08 Effects attached to a gun or a hand (muzzle flashes too) no longer stop playing after a long game
- 2026-10-08 Black Ops II movement in Zombies: its run and sprint rules, footstep and head-bob rhythm, dive to prone, a flinch when hit, hard falls that hurt and slow you, the gun swaying with the gun in hand, and dual-wield kicks
- 2026-10-08 Grief lobbies show BO2's own picture for each start (Cell Block, Town, Farm, Borough) instead of a blank white card, each Grief start loads with its own loading picture, and the lobby you return to after a game keeps its picture
- 2026-10-08 Mob of the Dead has two picks on the globe like BO2: its story mode and Cell Block Grief
- 2026-10-08 The nine Black Ops maps (Nacht der Untoten, Verrückt, Shi No Numa, Der Riese, Kino der Toten, Five, Ascension, Shangri-La, Moon) are on the globe, each at its own spot under its own name, with its own progress row
- 2026-10-08 Every Zombies map is on BO2's globe and picked like the real game: TranZit (with Bus Depot, Town and Farm), Nuketown, Die Rise, Mob of the Dead, Buried and Origins; the globe's green NEW tags show; Grief and Turned start solo; each story mode's lobby is named after its map; starts that do not play yet are kept off the menus
- 2026-10-08 Town: burning zombies singe you when they get close and burst when they die, hurting anyone next to them, as in BO2
- 2026-10-08 Survival: Town is playable: on the globe pick TranZit, then Town and Survival, and Start Match drops you in BO2's Town with its perks, wall guns, box and Pack-a-Punch; the pale squares that showed in the sky behind trees and cars on TranZit are gone
- 2026-10-08 Drinking a perk bottle no longer shows "0/0" ammo: the ammo count steps aside while the bottle (or the Pack-a-Punch knuckle crack) is in your hands, like BO2
- 2026-10-08 The black cloud that swept over you a few seconds into Nuketown is gone: a big storm smoke sheet was falling far too fast out of the sky and through the camera; it now stays up in the sky like BO2
- 2026-10-08 Zombies Public Match lobby works like BO2: it shows 4 max with three "Searching for player" rows and their turning knot icons, "Waiting for 3 more players.", then "Game starting in" counts down and the match starts with you; Solo still starts on Start Match
- 2026-10-08 Diving to prone makes BO2's whoosh and effort on take-off and a thud and grunt on landing; you grunt when a zombie hits you; the controller button cross no longer covers the ammo count; the kitchen cupboards are lit their true colour
- 2026-10-07 Nuketown lighting and fog follow the room you are in: the house interiors are lit for indoors (readable, not dark), outdoors keeps its stormy look
- 2026-10-07 Nuketown's flickering lamps flicker (fire lamps waver, electric ones buzz on and off); rooms echo like BO2 inside the houses, garage and truck; your steps sound like wood on the porches and metal in the truck bed
- 2026-10-07 Monkey Bomb works like BO2: it shows in your hands while you wind it (at its real speed), lands, claps and sings, and pulls the zombies to it; zombies close to a grenade blast burst into blood and gore (BO2's own rule), farther ones fly as bodies, and survivors lose limbs and crawl; teddy bear song and grenade refills (2 more each round, up to 4) checked
- 2026-10-07 Your Steam name shows on the scoreboard and in the lobby; Nuketown sounds: wind gusts, creaking houses, humming power lines, and bumps when you walk into the cars, truck and boxes; Fire Sale checked (box price 10, extra boxes, jingle)
- 2026-10-07 Shot mannequins throw their heads and arms, which bounce off and land on the floor beside them; the door power-up song egg checked the real way (the sign counts down to 15)
- 2026-10-07 The loading screen no longer shows the build number (real BO2 shows it on menus only); fading blurs work
- 2026-10-07 Sounds the map's scripts play (perk machines powering on, thunder) load with the map, so none arrive late and get dropped
- 2026-10-06 Nuketown comes alive: washing lines, shutters, curtains, cabinets, roaches and the roof vent move; wires spark; dust devils whirl with fire; a landing perk machine knocks you flat with a shake and wobbles; the game-over rocket ride blurs
- 2026-10-06 Nuketown eggs: knifing the backyard bunker hatch plays Marlton's voice (once a round, as in BO2); the clock, the population sign, the bus horn and the clock's door power-up checked working
- 2026-10-06 Nuketown props: grenades and knives break mannequins and other breakable props (never through walls); shots on mannequins show plastic, not blood
- 2026-10-06 Nuketown: mannequin heads shoot off and vanish (the 28-head song egg counts them), the comic loading picture, grenade kills burst zombies, blue eyes after round 25, the teddy-bear song
- 2026-10-06 Menu polish: the Theater and popup frames in BO2's silver with cracked glass, the lobby speaker icon, Theater "4 Max", the build number at the top right
- 2026-10-06 Menus checked one for one against BO2: real soft blur behind popups and Theater, menu words centred, Options list spacing, Nuketown names fixed; COD TV and Leaderboards removed
- 2026-10-06 Blood from stabs and shots paints onto zombies and the ground as in BO2 (no more black squares); no Campaign button on the main menu
- 2026-10-05 BO2's own front end: main menu, zombies lobby, the globe, Solo and Custom Games, loading screen, menu music; back to the lobby after a game
- 2026-10-05 Controller: aim assist on zombies, button pictures in menus; Pack-a-Punch camo tiled as in BO2
- 2026-10-04 README: full guide (what it is, install, run, coming soon)
- 2026-10-04 Play-test fixes: BO2's PC key layout, zombies no longer stuck, menus blur the world, bullets pass through zombies
- 2026-10-04 BO2's own pause menu and Options (key binds, FOV, video, volume, mouse); controller input in menus
- 2026-10-03 BO2's own HUD from its UI scripts (HavokScript VM and LUI widgets)
- 2026-10-03 Play-test fix lists 2 and 3: smooth zombie animation, collision walls, a solid Mystery Box
- 2026-10-02 Nuketown Zombies plays by the game's own scripts: zombies, rounds, buys, box, perks, Pack-a-Punch, game over
- 2026-10-01 Looks, sounds and shoots like BO2: guns, effects, shine, sound banks
- 2026-09-30 Nuketown loads, is walkable and lit from BO2 files alone
- 2026-09-30 Fork of IW4L at `11514ea`; BO2 zones, image packs and a byte-exact zone walk

## Credits and license

- [IW4L](https://github.com/vladtrc/iw4L) by vladtrc and its contributors: the engine this fork is built on,
  Apache License 2.0.
- IW4L's own references, read and not shipped: [OpenAssetTools](https://github.com/Laupetin/OpenAssetTools) and its
  [iw4x-x64 fork](https://github.com/iw4x-x64/oat), [IW4x](https://github.com/iw4x/iw4x-client),
  [KisakCOD](https://github.com/SwagSoftware/KisakCOD), [Ghidra](https://github.com/NationalSecurityAgency/ghidra).
- For Black Ops II, OpenAssetTools and [gsc-tool](https://github.com/xensik/gsc-tool) (both GPL-3.0) were read for
  format facts only; none of their code is copied.
- The "games inside games" goal follows chasmlol's IW4L mash-up of MW2, Skate 3 and Minecraft.
- Bundled fonts (from upstream): Oxanium (SIL Open Font License 1.1) and GNU FreeFont (GPL-3.0 or later with the
  font exception); see [`NOTICE`](NOTICE).
- Call of Duty, Black Ops, Modern Warfare and their assets, trademarks and intellectual property belong to their
  owners (Activision). No game files are included.

Licensed under the [Apache License 2.0](LICENSE), like upstream; [`NOTICE`](NOTICE) holds the copyright notices and
the licences of the bundled fonts. The license covers this source code only.
