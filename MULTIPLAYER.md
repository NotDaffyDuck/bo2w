# bo2mp: Black Ops II Multiplayer

bo2mp is a fan rebuild that plays Call of Duty: Black Ops II multiplayer against bots, from your own PC copy of the
game, in its own Black Ops II engine written in Rust on [bevy](https://bevyengine.org/) and [wgpu](https://wgpu.rs/).
It grew out of [IW4L](https://github.com/vladtrc/iw4L), an open-source Call of Duty runtime (Apache 2.0, credited in
NOTICE); every piece of IW4L's MW2 engine has since been taken out (2026-10-21). The same code that runs Nuketown Zombies runs Black Ops II's own multiplayer scripts,
menus and bots, so matches follow the game's real rules without touching the original executable.

**Status:** matches against Black Ops II's own bots play on every multiplayer map, started from BO2's own menus;
first play-test build; some scorestreaks and effects are still missing · **Visibility:** private for now · **Last updated:** 2026-10-16

**No game files are included.** This repository has no Black Ops II (or Modern Warfare 2) files: no maps, models,
textures, sounds, scripts or executables. You need your own copy of Black Ops II for PC with Multiplayer installed;
bo2mp reads its files read-only and never patches, copies or redistributes them. This is an unofficial fan project,
not affiliated with or endorsed by Activision, Treyarch or Infinity Ward.

## Latest update (2026-10-15)

Each update replaces this section; everything older is in [Recent changes](#recent-changes). This covers the last
three days of work (2026-10-13 to 2026-10-15).

**New gun code, the BO2 way.** The guns were rebuilt on Black Ops II's own weapon rules instead of the older engine's:

- **Movement with a gun**: BO2's sway, bob, idle breathing, scope drift and gun angles; climbing over ledges follows
  BO2's rules (a climb starts only on a jump, and the gun and arms follow it); the dolphin dive, the dive get-up, the
  sprint and the empty-gun pose use BO2's animations.
- **Shooting and reloading**: BO2's recoil; knife and sprint cancel a reload as in BO2; the attachment key skips guns
  with no second mode.
- **Knife**: the lunge aims at the enemy you saw; a knife into a riot shield no longer hits or bleeds.
- **Dual wield**: both pistols show and fire.
- **Grenades**: the frag and semtex pin pulls, the semtex beep and light, the throw and the roll match BO2; a cooked
  grenade held too long kills you; EMP, concussion and smoke go off after they roll.
- **Riot shield**: held straight for other players, with its glass window.
- **After you die**: the camera stays on your own body and turns to your killer. A killcam fix (a true replay of how you died) is checked against BO2 and goes in with the next build.

**Scorestreaks, one at a time, each checked against real BO2:**

- **RC-XD** (passed a playtest 10-14): drives on BO2's car data, wheels roll and steer, springs, lights blink at 20 and
  26 seconds, stops against walls, and keeps going if you die while driving it.
- **Hunter Killer** (passed a playtest 10-15): unfolds its wings as it leaves your hand, flies like a jet, locks onto an
  enemy only when it can see them, and an enemy can escape behind a wall.
- **Care Package** (playtested 10-15, nearly there): the drone flies in with the crate on its hook, the crate lands on
  the ground, teammates and enemies can take it (with the steal callout) and an icon points to it. Still open: the
  drone's blades do not spin yet.
- **UAV** checked against BO2's code; **Counter-UAV** jams the enemy minimap with BO2's own static.
- One key (3) brings out your cheapest earned streak.

**Zombies wonder weapons in multiplayer** (passed a playtest 10-15): Ray Gun, Gravity Gun, Bow, Enchanted Stick and
Portal Gun (with linked portals you can see through), plus their upgraded versions, under Weapons > Zombies Wonder
Weapons in the mod menu.

**Mod menu and mods**: scripts in a `mods` folder load and run; the built-in mod menu (hold Aim, tap Knife) has God
Mode, UFO Mode, Third Person, every gun and scorestreak, bot controls, trickshot options and more.

**Faster loading**: each shader is built once, and the loading bar moves evenly to the end.

Every item is listed with its date in [Recent changes](#recent-changes). What is still to do is in
[Coming soon](#coming-soon).

## Contents

- [Latest update (2026-10-15)](#latest-update-2026-10-15)
- [Map completion](#map-completion)
- [Features](#features)
- [Screenshots](#screenshots)
- [Requirements](#requirements)
- [Install](#install)
- [Run / Play](#run--play)
- [Configuration](#configuration)
- [Mods (GSC scripts)](#mods-gsc-scripts)
- [How it works](#how-it-works)
- [Development](#development)
- [Coming soon](#coming-soon)
- [Recent changes](#recent-changes)
- [Credits and license](#credits-and-license)

## Map completion

How far along each map is, mechanically (loads, spawns, each mode, bots, scorestreaks, killcam, match end) and visually (fixed spots shot in real BO2 and here, side by side). Every number comes from [docs/maps.json](docs/maps.json), where each passing item names the test run or shot that proves it; [docs/make_bars.py](docs/make_bars.py) draws this table. A visual spot only counts once it has a same-spot real-BO2 pair, so visuals read 0% until those shots are taken.

<!-- map-bars:start -->
```text
                Mechanics         Visuals
Aftermath       ██████████  98%   ██████████ 100%
Cargo           ██████████  98%   ██████████ 100%
Carrier         ██████████  98%   █████████░  88%
Drone           ██████████  96%   █████░░░░░  50%
Express         ██████████  98%   ██████████ 100%
Hijacked        ██████████  98%   ████████░░  75%
Meltdown        ██████████  98%   not compared yet
Overflow        ██████████  96%   ██████████ 100%
Plaza           ██████████  98%   ██████████ 100%
Raid            ██████████  98%   not compared yet
Slums           ██████████  98%   █████░░░░░  50%
Standoff        ██████████  98%   ██████████ 100%
Turbine         ██████████  98%   ████████░░  75%
Yemen           ██████████  98%   █████░░░░░  50%
Nuketown 2025   ██████████  98%   █████████░  88%
Downhill        ██████████  98%   not compared yet
Mirage          ██████████  98%   not compared yet
Hydro           ██████████  96%   not compared yet
Grind           ██████████  98%   not compared yet
Encore          ██████████  98%   not compared yet
Magma           ██████████  98%   not compared yet
Vertigo         ██████████  98%   not compared yet
Studio          ██████████  98%   not compared yet
Uplink          ██████████  98%   not compared yet
Detour          ██████████  98%   not compared yet
Cove            ██████████  98%   not compared yet
Rush            ██████████  98%   not compared yet
Dig             █████████░  90%   not compared yet
Frost           ████████░░  78%   not compared yet
Pod             ████████░░  78%   not compared yet
Takeoff         █████████░  86%   not compared yet
All maps        ██████████  96%   ████░░░░░░  35%
```

Mechanics: each map's systems and modes work by Black Ops II's rules. Visuals: set spots compared side by side with Black Ops II. Scored in [`docs/maps.json`](docs/maps.json) (pass or match 1, part or close half, not yet 0), last updated 2026-10-20; drawn by [`docs/make_bars.py`](docs/make_bars.py).
<!-- map-bars:end -->

## Features

- **BO2's own front end.** `frontend t6mp` opens Black Ops II's multiplayer main menu from its UI scripts: custom
  games lobby, Setup Game, Setup Bots, Create-a-Class, Scorestreaks, then Start Match. Classes are saved next to the
  binary.
- **Every multiplayer map.** All 31 maps on the disc and in the DLC packs you own load and can be walked, each with
  its own factions, lighting, colour grading, sky, water and foliage.
- **The game's own match rules.** BO2's compiled multiplayer scripts run the match: spawns, scoring, score and time
  limits, rounds, the end of the match. The one script no PC file carries (`_globallogic`, the match flow) is
  written for the engine from what the other scripts call, and compiled by the engine's own script compiler.
- **BO2's bots.** Black Ops II's bot scripts choose goals, classes and targets; the engine walks them over the map's
  path nodes, aims, jumps, climbs ladders and fires. Bots play every mode BO2 allows them in: Team Deathmatch,
  Free-for-All, Domination, Kill Confirmed, Hardpoint, Capture the Flag, Demolition and Headquarters.
- **Other players look like BO2's.** Each faction's own soldiers, heads and gear as BO2's scripts pick them, the
  gun in their hands, and BO2's own third-person animations (idle, walk, run, sprint, crouch, jump, firing,
  reloading, switching, deaths), drawn with BO2's materials and lit by the map.
- **In the match:** BO2's class menu at spawn; its HUD with the turning minimap, compass, clock, score bars, kill
  feed and scoreboard; its pause menu and menu sounds; the killcam; guns dropped on death (walk over the same gun
  for ammo, hold use to swap) and Scavenger bags; BO2's end-of-match screen, then back to the main menu.
- **Public Match.** Combat Training plays BO2's map rotation, with BO2's vote for the next map in the lobby.
- **Scorestreaks.** Score earns streaks through BO2's own momentum code; bots call in Orbital VSAT, Counter-UAV,
  EMP, Death Machine, War Machine, Swarm, Escort Drone and K9 Unit.

## Screenshots

There are none on this page on purpose: everything the engine draws is Black Ops II's own art, read from your own
install at run time, and that art is not ours to share. What the recent fixes changed, in words:

- **Aiming down the sights** (fixed 2026-10-10): the gun used to blur whenever you aimed; now the sights and the gun
  stay sharp, as in the original.
- **Public Match map vote** (build r132): before, every Public Match was Nuketown. Now Combat Training rotates through
  the maps, and the lobby votes on the next one as in BO2: NEXT, PREVIOUS or RANDOM, most votes win.
- **Change Map** (build r131): before, the Custom Games lobby stayed on Aftermath whatever map you picked. Now it
  shows the picked map, and START MATCH loads it.

## Requirements

- **Your own Call of Duty: Black Ops II for PC** (Steam) with Multiplayer installed (Steam app 202990).
- **Windows x64** (built and played on Windows 11 only so far).
- **Rust** (stable toolchain via [rustup](https://rustup.rs/); the code uses the 2024 edition) and
  **Visual Studio 2022 Build Tools** with the C++ workload (MSVC).
- A DirectX 12 or Vulkan capable GPU. Development and testing used an RTX 4070 Ti at 1920x1080.
- Tens of GB free for the Rust build output.

## Install

There are no releases: you build it from this tree. These steps were done on the author's PC only, never on a fresh
one by someone else.

1. Install Rust and the Visual Studio 2022 Build Tools (C++ workload).
2. Clone the repository and check out the branch: `git checkout main`.
3. Build the game:

   ```powershell
   cargo build -p launcher --profile play
   ```

   The binary is `target\play\bo2mp.exe`.
4. Make a play folder anywhere, copy `bo2mp.exe` into it, and create a `.env` file next to it that points at the
   folder holding your Black Ops II install (quoted, with forward slashes), for example:

   ```text
   BO2MP_GAMES="C:/Program Files (x86)/Steam/steamapps/common/Call of Duty Black Ops II"
   ```

## Run / Play

From the play folder:

```powershell
.\bo2mp.exe frontend t6mp
```

Pick a mode, map and bots in the custom games lobby and press Start Match. A Windows shortcut with that target and
"Start in" set to the play folder works the same way. Keys follow BO2's PC defaults; Esc opens BO2's pause menu.
Settings, saved classes, caches and logs go to `bo2mp-artifacts\` next to `bo2mp.exe`.

## Configuration

- `.env` next to the binary: `BO2MP_GAMES` (required).
- `bo2mp-artifacts\settings.cfg`: saved settings; `bo2mp-artifacts\bo2mp_stats.txt`: saved classes.
- `BO2MP_HEADLESS=1`: a test match with no window, graphics card or sound device, so many can run at once (with the test switches below).
- Many game windows at once: `bo2_quality=fast` in settings.cfg (half-size textures, no sun shadow, props hidden sooner) and `BO2MP_BACKGROUND_FPS` (how often a window without focus draws, 30 by default, 0 for no limit).
- Test switches for hidden test matches (`BO2MP_SCRIPTS`, `BO2MP_BOTS`, `BO2MP_GAMETYPE`, `BO2MP_AUTOCLASS`, ...)
  are listed in `docs/BO2ZM.md`.

## Mods (GSC scripts)

BO2 script mods load from a `mods` folder next to `bo2mp.exe` (laid out like Plutonium's `storage/t6`, so most public
GSC mods drop straight in). Source `.gsc` files and PC-compiled ones both work.

- `mods/scripts/mp/<name>.gsc`: runs on its own when a match starts (its `main`, then its `init`).
- `mods/maps/mp/...gsc`: replaces BO2's own script at that path (for older mod menus that replace `_clientids.gsc`).
- A file that fails to load is named in the game log (in `bo2mp-artifacts\`) and skipped; the match still runs. Rename a
  file to `name.gsc.off` to turn it off.

**The built-in mod menu** (`mods/scripts/mp/menu.gsc`): hold **Aim** and tap **Knife** to open it. Aim / Fire move,
Use picks, Knife goes back. Main Mods (God Mode, Infinite Ammo, UFO Mode, Third Person, Speed x2, Teleport to
Crosshair, Save / Load Position, Max Ammo, Invisible, Super Jump, Low Gravity, No Recoil, Rapid Fire, Explosive
Bullets, Infinite Equipment), Weapons (every gun by class, Zombies wonder weapons and specials such as
the Ray Gun, staffs, Monkey Bomb, Hell's Retriever, the Gravity Gun, Bow, Enchanted Stick and Portal Gun, and Pack-a-Punch for the gun in your hands), Scorestreaks
(picking one uses it at once), Trickshots (a platform under your feet, Canswap, Fast Last), Game (Pause the Clock,
Unlimited Score, Slow Motion, Change Map, Change Mode), Bots (add, freeze, bring them to stand in a row in front of
you, Bots Don't Shoot, Kill All, Kick All, an enemy calling a UAV or Counter-UAV) and Fun. To add an option, open the file in Notepad and copy an `add_option(...)` line; the
comment at the top shows how. Restart the match to load your change.

## How it works

Multiplayer adds to bo2zm's crates (`fastfile_t6`, `asset_t6`, `gsc_t6`, `hks_t6`, `sim` t6 modules):

| Part | What it does |
|---|---|
| `gsc_t6::compile` | a compiler for BO2 script source, for the scripts the engine must provide (`_globallogic`) |
| `sim` `t6::mp`, `natives_mp` | the multiplayer runtime: game types, team scores, the match clock, spawn choice, objectives |
| `sim` `t6::bots` | the engine half of BO2's bot scripts: test clients, goals, paths, view, buttons, threats |
| `sim` `t6::loadout`, `t6::items` | classes from BO2's stats table; guns and Scavenger bags on the ground |
| `hks_t6::mp`, `bo2_profile` | BO2's multiplayer menus, classes and stats |

## Development

- Build: `cargo build -p launcher --profile play`; tools: `cargo build -p bo2zm_tools --profile play`.
- Tests: `cargo test -p gsc_t6` (and `cargo test -p <crate>` for the others).
- Like upstream IW4L, this code is written by an LLM (Claude), directed and play-tested by the author.

## Coming soon

> Keep this list current: when an item ships, move it to Recent changes with the date, then add what is next.

- [ ] Hellstorm's marks on the minimap as in BO2: one for the missile and one for each bomblet (sizes 50 and 32), coloured the way BO2 colours them
- [ ] Every map's own map logic from Black Ops II's map data: its windows and breakable props are on every map, and its solid models on every map (10-17); every mode plays on every map (10-17); the moving parts, sounds and end-of-match nuke BO2's client scripts run are in (10-17); whole Demolition and Search and Destroy rounds pass on the maps that came with the game (10-19; Drone's Demolition waits on the mantle links below), Nuketown 2025's and Takeoff's pass too, and Pod's, Frost's, Dig's, Rush's, Cove's, Detour's, Uplink's, Studio's, Vertigo's, Magma's, Encore's, Grind's, Hydro's and Mirage's Search and Destroy and Demolition (10-19) and Downhill's (10-20), still to come on the other DLC maps; then the 7 modes bots cannot finish yet (below), then side-by-side pictures with BO2
- [ ] Studio's brontosaurus: BO2's half-second blend between its chewing and its looking around (every map plays all of its own moving props, 10-16)
- [ ] Takeoff's shuttle lit as in BO2: it draws black in the end-of-match launch (the launch itself plays as in BO2, 10-16)
- [ ] Carrier's parked helicopter lit as in BO2: it draws solid black, though BO2's own colour map for it is light camo
- [ ] Drone's centre area floor drawn as in BO2: it draws flat pale blue (its ground there is a blended grass and dirt surface)
- [ ] Cargo's sky drawn from BO2's sunset sky cube (it draws grey and overcast), without the two small specks in its south view
- [ ] Vertigo's doors: BO2's door box, so a moving door sets off explosives in its way and a closing one crushes a sentry or a tactical insertion
- [ ] Express: BO2's camera shake and rumble when the train passes close (its dust and minimap icons are in, 10-16)
- [ ] Drone's window pieces drawn as clearly as in BO2: its windows break into pieces on screen, but the pieces draw as faintly as the unbroken pane (every other map with windows is checked on screen, 10-19)
- [ ] Bots that stop getting stuck on corners and props: BO2's own stuck rescue and its look-ahead check on the route
- [ ] Bots that never stand facing a wall, and always shoot at an enemy walking in front of them
- [ ] Bots that spawn pressed into an invisible wall walk off it, as BO2's movement lets them (on Pod a bot could stand at its spawn for a whole match)
- [ ] Bots on Nuketown's upper floor: its ledge links carry no jump or drop mark, so bots walk them; match how BO2 moves its bots over them
- [ ] Bots over BO2's ledge, mantle and window links on every map: bots face the link, press the climb, break a window in their way and, when a jump falls short, are sent on to the far side as in BO2 (10-19; Hydro: 35 of 45 climbs finish, 3 given up; Grind: 18 of 21, none given up); the other maps are still to be measured, and Dig's HQ, Drone's Kill Confirmed and Demolition, Pod's and Takeoff's CTF wait on it
- [ ] A hit marker and BO2's hit sound every time you hit someone, BO2's own headshot marker and sound, and no marker when someone else hits you
- [ ] Getting up from a dive at BO2's speed, and diving over low objects the BO2 way
- [ ] Bodies that fall without spinning, the way BO2's ragdolls throw them
- [ ] Every other scorestreak proven against BO2. Fixes from a check of every scorestreak against BO2's scripts are in the game since r130 but not yet checked side by side: Care Package capture bar and prompt, K9 Unit, Stealth Chopper, AGR, VTOL Warship, Dragonfire, Escort Drone, Sentry Gun, Guardian, Hellstorm, RC-XD, Counter-UAV, EMP, Orbital VSAT, the UAV in free-for-all, streaks taking damage, and the scorestreak icons on the HUD
- [ ] Lightning Strike's tablet looking exactly like BO2's: a mark left on each picked spot, the "Press [key] to Target (N remaining)" line, the map inside the tablet's lit screen at BO2's size and position, the world blurred behind it, the tablet itself at BO2's size and place in view, Nuketown's map turned and fitted as BO2 does, and teammates' arrows at BO2's size (the target pick, the enemies it shows and where the strikes land already match BO2)
- [ ] Scorestreaks one at a time, each checked against BO2 and passed in a playtest before the next: the UAV (r134), the RC-XD (r144) and the Hunter Killer (10-15) passed, the Counter-UAV (r135) and the Care Package (10-15) are in test (Care Package playtest 10-15: better, but the drone's blades still do not spin on the test PC; fixed next time); the Hunter Killer's last rough edges (now and then it flies past its target, or into an object on the way to one) get a second look after the last scorestreak
- [ ] Killcam: show a true replay of how you were killed (it sometimes shows a near third-person view of the other player instead, and stutters): fixed and checked side by side with BO2, goes in with the next build
- [ ] Two scorestreaks at once, and a key of your own for each scorestreak
- [ ] Public Match map vote checked against real BO2 footage next: how long the first lobby's vote runs (10 seconds for now), whether some maps come up more often than others (all equal for now), and which tile grows when the vote closes
- [ ] Attachments and gun levels (r133) checked side by side against BO2: each attachment's numbers, models, sounds and flashes, the gun level-up notice and the locked attachments in Create-a-Class
- [ ] Three attachments on one gun (Gunfighter): the third is named but its numbers and model are left out for now
- [ ] Attachment models at BO2's exact place on the gun (their position offsets are not applied yet)
- [ ] The Grip's recoil reduction, Select Fire, the attachments' perks and the Tactical Knife's own melee speed
- [ ] MMS, Dual Band thermal, Target Finder, the laser's beam and the Tri-Bolt
- [ ] Riot shield: a test that it stops shots from the front exactly as in BO2 (shots from behind and around it are checked)
- [ ] A riot shield carried on the back shows the wrong texture (a sheet of letters) instead of BO2's own
- [ ] Movement speed: walking, sprint, crouch, prone and Lightweight already match BO2's numbers; still to check the water slowdown and each attachment's aiming speed (the Stalker's own aimed walk speed is in, 10-19)
- [ ] Black Ops II's own movement and gun handling, always on in a Black Ops II game since 10-19 (no switch; the MW2 rules are for the MW2 games only): walking, jumping, the air, sliding, stances, ladders and climbing over ledges rebuilt from BO2's own rules, with moving platforms, water and drowning; every one of the 657 player-code functions is now accounted for. In a test build; next, a feel pass on the 64-bit build. Windows block a ledge climb once the maps branch's window glass joins this build
- [ ] The new guns' last missing sounds and hints: the jam click, "can't plant here" and "lock-on required" (the guns already behave right without them)
- [ ] A death camera that stays on your body until you respawn or the kill replay starts
- [ ] No slowdown late in a match: the script part is fixed (r128); the drawing time that still grows over a match is not found yet
- [ ] BO2's master compressor and limiter, the last steps of its sound mix (Super Crunch is the audio preset that needs them most): their numbers are read from BO2's sound data, but how BO2's mixer applies them is not in anything we work from, so they are not built
- [ ] Portal Gun: its own sounds
- [ ] Super Portal Gun: open portals under any player, not only enemies
- [ ] End Game counts the quit in your stats the way BO2 does (it leaves the match, but the stats step is not built yet)
- [ ] The reflex sight's red dot as bright as BO2's (needs BO2's own colour range step)
- [ ] Demolition without script errors at a kill
- [ ] Players never start a match inside a wall (the spawn check uses one more of BO2's solid kinds)
- [ ] The next map after Nuketown
- [ ] Every map's RC-XD routes checked: the holes BO2 cuts for it let it drive through, as on Nuketown
- [ ] No-graphics test matches skip more drawing-only work (bullet marks, ragdolls), so even more fit at once
- [ ] A real BO2 side-by-side check of the care package's drop and pick-up
- [ ] Every test match runs cheap by default, Multiplayer and Zombies: no graphics for checks that only read the log, the fast setting for checks that take screenshots
- [ ] Zombies draws props that share a model together, as Multiplayer now does (Nuketown Zombies draws about 1,800 props a frame)
- [ ] Playing together over LAN
- [ ] Multiplayer and Zombies in one app, switched from BO2's main menu (for now they ship as two separate apps)

## Recent changes

- 2026-10-24 (own code): the first-person viewmodel code (spawning the held weapon's arms and gun, the per-frame pose tick, placement, view kick, held-weapon sync and match reset) is rewritten from behaviour and split into small modules, each pinned by tests first. Checked: workspace build, 308 animation-crate tests and 240 simulation tests.
- 2026-10-23 (own engine): render_frontend's scene preparation has about 200 unit tests, covering view and camera maths, culling, the model and lighting caches, worker command order and world placement. Checked: 212 tests in render_frontend, 219 in sim.
- 2026-10-23 (own code): the animation crate's scene-occupancy code (other players' bodies, missiles, dropped items, dynamic entities and their physics, the killcam and the third-person and vehicle cameras) is rewritten from behaviour and split into small modules.
- 2026-10-23 (engine clean-out): the render_frame crate (the per-frame render output: packed source lists, destination-ring records, product status, GPU layout contract) is rewritten from its behaviour, with 67 tests pinning it first; the old engine's lines in it went from 1,834 to 0.
- 2026-10-21 (public-ready pass): the game screenshots are off this page (they are Black Ops II's own art); the effect random table, which was Modern Warfare 2's, is now our own generated table; code comments no longer point at reference files or personal folders; NOTICE names the patched wgpu-hal and the generated zone layouts; and `make publish-check` now also flags decompile paths and file:line pins. Checked: 900 tests in the touched crates.
- 2026-10-21 (own engine): every last piece of IW4L's MW2 engine is out: all 20 MW2 crates deleted (about 82,000 lines), and no IW4/IW5 or IW4L name is left in the code, build files, scripts or file names. The settings folder is now `bo2mp-artifacts\` (the old `iw4l-artifacts\` was copied over and kept). Checked: build, 1,033 tests, four hidden mode games and a game with only bo2mp names.
- 2026-10-21 (engine clean-out): entity_iw4 is gone: audio, bots, hud, net and sim run on the BO2 entity crate (entity_t6).
- 2026-10-21 (engine clean-out): fastfile_iw5 (Modern Warfare 3's zone loader, unused) removed
- 2026-10-21 (engine clean-out): render_anim and render_frontend no longer use the MW2 entity crate (entity_iw4); they read Black Ops II's entity states and events
- 2026-10-21 (engine clean-out): the HUD's 2D drawing (pictures and text, with the typing and fading text effect) now goes through Black Ops II's own draw commands, colours and text styles instead of MW2's; MW2's text outline, which Black Ops II does not have, is gone
- 2026-10-21 (engine clean-out): 10-21: entity step A: console, render_anim and render_frontend read BO2's entity types and name helpers from entity_t6 (console no longer uses entity_iw4).
- 2026-10-21 (engine clean-out): 10-21: anim_iw4 deleted; the sim reads anim_t6's packed legs/torso value (MW2's stance torso reset removed).
- 2026-10-20 (engine clean-out): the render crates hold BO2's own vertex rows; one conversion to MW2 rows at the GPU upload, until T2 moves the shaders to BO2 rows
- 2026-10-20 (engine clean-out): a window takes a mark only on surfaces whose material allows it, as BO2's; a shattered pane takes no more marks
- 2026-10-20 (engine clean-out): marks: a bullet mark also lands on the BO2 windows its box touches (up to 8 unbroken panes), each its own mark that the pane's shatter frees
- 2026-10-20 (engine clean-out): marks on players and vehicles remember which part of the model they hit (up to 32 parts, as in Black Ops II), and breaking a window frees the marks on that window the Black Ops II way, including its habit of clearing wall bullet holes when the map's first window breaks.
- 2026-10-20 (engine clean-out): marks on the gun in your hands are kept apart from map marks the Black Ops II way, and breaking a window no longer wipes marks off the gun by mistake.
- 2026-10-20 (engine clean-out): the game reads only Black Ops II zones now; the old MW2 and Black Ops zone paths are gone (about 290 lines).
- 2026-10-20 (engine clean-out): bullet and blood marks on players and vehicles now use BO2's entity-model mark type.
- 2026-10-20 (engine clean-out): bullet marks remember what they hit the Black Ops II way, so marks on players and vehicles no longer pick up the wrong map lighting.
- 2026-10-20 (engine clean-out): lights use Black Ops II's five light types (sun, spot, square spot, round spot, omni), so all three spot shapes are lit and cast shadows the same way.
- 2026-10-20 (engine clean-out): bullet marks follow Black Ops II's rules: they age, can expire, the mark drawn longest ago is freed first when room runs out, and new marks are only made within 1,500 units of the camera.
- 2026-10-21 (test build, passed checks): the build where you can host a LAN game from the Black Ops II menu, the menu's map list comes from the maps you have installed, Los Angeles and Raid load again, and bots and streaks check line of sight against Black Ops II's own world passed every check: all tests, hidden matches on three maps, and ten hidden Domination and Hardpoint matches on five maps.
- 2026-10-20 (test build, passed checks): the build where pistols, dual pistols and knives sprint with Black Ops II's own run, crouch or prone plus sprint stands you straight up, the Hellstorm steers and no longer blows up at launch, the Dragonfire flies up and down and marks enemies in sight, Find a Game offers the party modes with bots and the Tab board shows every name passed every check: all tests, hidden matches on three maps, and ten hidden Domination and Hardpoint matches on five maps.
- 2026-10-20 (engine clean-out): MW2's asset layout part is deleted (about 2,300 lines): models, maps, sound, guns and materials all read Black Ops II's own layouts now.
- 2026-10-20 (engine clean-out): the models, the map's surfaces, the sound code and the gun animation table no longer read anything from MW2's file layouts: vertex sizes and collision come from the Black Ops II part, the sound falloff and voice limits live in the sound code (Black Ops II's sound curves have 8 points), and a thrown weapon's bounce sounds now cover all 32 of Black Ops II's surface types (MW2 had 31, so the last one was silent).
- 2026-10-20 (test build, passed checks): the build with the slower Dexterity climb, the Dragonfire lights, the Hellstorm minimap marks, the fixed Carrier rotors and the four Uplink party modes passed every check: all tests, hidden matches on three maps, and ten hidden Domination and Hardpoint matches on five maps. Next build: sprinting with the pistols, dual pistols and knives plays Black Ops II's own run; only the knives, whose loop jumps far more than its own swing, are eased.
- 2026-10-20 (map logic, game modes): Downhill's Demolition rounds pass in a hidden test match: bombs planted and defused, the match ends by itself with no script error, and every player is back for each round. So Downhill's Search and Destroy and Demolition both pass now.
- 2026-10-20 (engine clean-out): MW2's fast-file reader is deleted (about 9,000 lines); nothing in the engine opens an MW2 file any more.
- 2026-10-20 (engine clean-out): the MW2 game path is out of the launcher: `menu` opens Black Ops II's own front end, the MW2 map loader refuses to load, and MW2's player animation scripts and animation trees are gone (about 5,900 lines). The LAN and master server lists now look for Black Ops II's files.
- 2026-10-20 (engine clean-out): the drawing code no longer reads anything from MW2's file layouts: Black Ops II's own vertex sizes come from the Black Ops II part, and the drawing settings MW2 still supplies (the vertex table, depth and polygon offset) now sit with the renderer, marked as MW2's until Black Ops II's own are known. Nothing on screen changes. Checked in tests, not yet in a match
- 2026-10-20 (engine clean-out): Black Ops II's lighting (the light grid, what lights a moving thing, the lightmaps used for marks and glass, reflection probes) and its impact marks (bullet holes, scorch marks) now live in their own Black Ops II parts of the engine, ready to reuse in other projects. The marks part needs nothing else from the engine, and the lighting part now does all of its own work too: the light grid, scene and dynamic lights, spot-light shadows and model light caches; the old MW2 lighting part is deleted (marks use BO2's own limits: 256 marks, 128 in split-screen, 1024 triangle groups, 1536 point groups, and at most 8 marks in any 20-unit spot, so a sprayed wall stops piling up holes the way BO2 does). The MW2-only ways of placing marks are gone, since a Black Ops II map never used them. Checked in tests, not yet compared side by side in a match
- 2026-10-20 (test build, newest): sprinting with the ballistic knife or a pistol, the arms now swing in turn like a real run: each arm keeps Black Ops II's own swing and eases back while BO2 would hold it still, so the right arm goes back while the left comes forward and the other way round, once per stride each. Before, both arms went back at the same moments and the view rocked. The knife stays in the left fist and the silver piece in the right hand, each locked to its hand every frame.
- 2026-10-20 (test build): the ballistic knife sits in the left fist again while sprinting, as in Black Ops II. Each arm swings out and back on BO2's own swing (played back the other way where BO2 holds it still); the green knife follows the left arm's swing and the small silver piece in the right hand follows the right arm's. Checked in a hidden, muted test: the knife is in the left fist in every frame the fist is on screen.
- 2026-10-20 (test build, earlier): each arm swings out and back on Black Ops II's own swing. This build moved the knife onto the right arm's timing, which took it out of the left fist (fixed in the build above).
- 2026-10-20 (test build, later): the ballistic knife's sprint now keeps the left hand moving too: Black Ops II's own run animation holds the left arm still for half the loop, swings it, then jumps it back in one frame; that jump is now spread over the half where the arm would hold still. The scroll wheel no longer reaches the knife the melee key swings (it showed as a two-hand knife); Black Ops II's held Combat Knife is a real pick again and is given as BO2 gives it.
- 2026-10-20: climbing onto ledges was timed against Black Ops II's own rules in a hidden match on Nuketown: without Dexterity a climb takes 0.5 to 0.9 seconds, with Dexterity the pull-up takes 40% of that (about 0.2 s), exactly as BO2 does. A climb that feels very quick means the class has Dexterity.
- 2026-10-19 (test build, later): the ballistic knife's sprint now really keeps the right hand moving. Black Ops II's own run animation swings the arm forward, holds it, then jumps it back in a single frame every time the run loops (the earlier fix only restarted the loop, so the jump stayed). That jump is now spread over the second half of the run, for the one-handed runs that have it (the ballistic knife, the pistols); only the bones that jump change. Rifle runs already loop cleanly and stay exactly as BO2 has them. This is a deliberate change from BO2's own animation data. Checked in a hidden, muted test: across seven run loops the knife never jumps at the loop point.
- 2026-10-19 (test build): no class gets the two-hand held knife you could scroll to any more; the combat knife you melee with stays. The ballistic knife's sprint no longer stops the right hand mid-run. New: ENGINE-PROGRESS.md lists what the new Black Ops II engine has replaced and what is still the original MW2 engine.
- 2026-10-19 (test build): the MW2 rules are out of Black Ops II's movement and gunplay. A Black Ops II game always moves the player with BO2's own movement and gun handling; the gun sits, sways, bobs and drifts by BO2's own rules standing, walking, sprinting, diving, sliding and climbing; aiming down the sights asks BO2's own rule for the gun in hand; sprint ends in waist-deep water and when the gun is lowered; no going prone in mid-air. New: BO2's grenade warning (the grenade icon with its arrow around the crosshair when a frag or Semtex lands near you, hidden while scoped and in hardcore, as in BO2). A sniper's bolt is pulled only when BO2 pulls it, and the combat knife is held in the right hand only.
- 2026-10-19 (merged test build): the day's work of all three chats (sliding, the audio presets, bots through windows and up ladders and ledges, Search and Destroy and Demolition on nearly every map, the map look fixes) merged into one test build, and the second test PC's launcher now starts it with Black Ops II's own movement. All 1,229 unit tests pass, and a hidden, muted test match on Nuketown runs to its end with no errors.
- 2026-10-19 (BO2 movement test build): Sliding works the way BO2's does: the controller rumbles once as a slide starts and stops the moment it ends, and a puff of dust rises from between the feet every few frames of the slide, for every player. Your own dive now plays its sounds and rumble in first person too. Checked against BO2's own client code and in tests, not yet felt side by side with BO2
- 2026-10-19 (BO2 movement test build): Options > Audio > Audio Presets changes the sound the way BO2's does: each preset plays every sound through BO2's own four-band equaliser settings for it (Treyarch Mix: fuller lows and a narrow cut around 2.8 kHz; Bass Boost: more low end; High Boost: brighter highs; Headphones: less low end, brighter highs). Super Crunch sounds the same as Treyarch Mix for now: BO2 changes only its compressor for that preset, and the compressor is not built yet (Coming soon). While the announcer speaks, BO2's own announcer duck turns the other voices down about 13 dB, and impacts, explosions and the ambience a little, fading in over 0.2 s and back over 0.5 s; gunfire is not turned down, as in BO2. Checked against BO2's own sound data and in tests, not yet heard side by side with BO2
- 2026-10-19 (bots, in testing): once a bot has climbed over a wall or ledge on its route, it plans on from the far side, as in BO2, instead of from the nearest spot on the map, which on top of a thin wall sent it back over the same wall (Dig).
- 2026-10-19 (bots, next test build): a bot whose jump at a ledge falls short is sent on to the ledge's far side for up to a second, as in BO2, and keeps its crouch, fire, aim and knife buttons while it is; on Grind's skate ramp every jump made it in a hidden test match (2 of 2, before 1 of 8), and in a hidden Grind Headquarters match 18 of 21 climbs finished with none given up (before 17 of 42, 9 given up). A script's check for a Wildcard in a class (Overkill, Primary Gunfighter and the rest) reads the class the way BO2 does; bots have none, as in BO2.
- 2026-10-19 (bots, BO2 movement test build): a bot whose jump across a gap is blocked by a window looks for the glass and breaks it with its knife, as in BO2, instead of standing against the pane (in a hidden test match on Hydro all 16 of its jumps through the windows went across).
- 2026-10-19 (bots, BO2 movement test build): bots go up ladders (on Hydro a bot could stand at a ladder's foot for a whole match), and at a low wall, railing or ledge on their route they face it and press the climb, so they get over it: in a hidden test match on Hydro they were stuck 78 times instead of 239 and made 44 kills instead of 7. At most four bodies lie around at once, as in BO2 (a fifth takes the place of the one farthest from you). A ranked match counts as an online one.
- 2026-10-19 (map look): surfaces that use Black Ops II's small built-in images (the flat normal map, the plain shadow and black ones), shine maps kept in another map's files and single-colour reflection probes now draw from BO2's own data instead of stand-ins. Aftermath's four set views match BO2 on every look check. Cargo's, Carrier's and Drone's set views are graded: Cargo's south overview shows two small sky specks, Carrier's parked helicopter draws black and Drone's centre floor draws flat pale blue (all three are listed in Coming soon)
- 2026-10-19 (map logic, Uplink): the cable car proven in hidden test matches: a player standing in the cab rides its swing with BO2's movement (stays within about a unit of the same spot in the cab, at most a quarter second behind), its floor is solid where BO2's own cab collision puts it, and bodies next to the cab are cleared as it swings back, as in BO2
- 2026-10-19 (map logic, game modes): whole Search and Destroy and Demolition rounds proven in hidden test matches on 13 of the 14 maps that came with the game: a round is won, the match ends by itself with no script error, and every player is back for the next round. Drone passes Search and Destroy (its Demolition waits on bots climbing over mantle links). Standoff's party modes pass too: Gun Game, One in the Chamber, Sharpshooter, Sticks and Stones and One Flag (Sharpshooter in a match long enough to hand out 8 guns). Nuketown 2025's Search and Destroy and Demolition rounds pass the same way. Grind's party modes pass: Gun Game, One in the Chamber, Sharpshooter, Sticks and Stones and One Flag (in its first two matches every flag carrier was shot down on the way; in the third a carrier brought the flag home and the match ended on it), Takeoff's Search and Destroy and Demolition rounds pass, Pod's, Frost's, Dig's, Rush's, Cove's, Detour's, Uplink's, Studio's, Vertigo's, Magma's, Encore's, Grind's, Hydro's and Mirage's Search and Destroy and Demolition rounds pass (Dig's, Detour's, Uplink's, Studio's, Magma's, Encore's, Hydro's and Mirage's Search and Destroy rounds were won by wiping out the other team), and Encore's party modes pass: Gun Game, One in the Chamber, Sharpshooter, Sticks and Stones and One Flag; Magma's, Vertigo's and Studio's party modes pass too; Downhill's Search and Destroy rounds pass, won by wiping out the other team (its Demolition is still to run)
- 2026-10-19 (map logic, windows, DLC maps): windows proven breaking on screen in hidden test matches on the nine DLC maps that have them: Encore, Frost, Grind, Hydro, Rush, Studio, Takeoff, Uplink and Vertigo. Every map with breakable windows now shows them breaking into pieces, except Drone, whose pieces draw only faintly. On Encore the drinks cooler keeps jagged glass at its edges; on Takeoff the pieces show BO2's cracked-glass look.
- 2026-10-19 (map logic, windows, later): windows proven breaking on screen in hidden test matches on six more maps: Express, Hijacked, Nuketown 2025, Raid, Slums and Village. On Village the cooler doors keep jagged glass in their frames; on Hijacked the shards burst toward the camera.
- 2026-10-19 (map logic, windows): a window makes only as many pieces as its map's glass piece budget has room for, the way BO2 does (Meltdown's budget is 80), so a second window broken right after the first still breaks into pieces instead of none. Windows proven on screen in hidden test matches on Dockside, Meltdown, Nightclub and Turbine; the scorecard adds Downhill's and Mirage's party modes from the hidden sweep.
- 2026-10-19 (map logic, windows): a window breaks the way BO2's own glass code breaks it: the pieces along its edge stay in the frame and the rest drop out and fall, the pieces left hanging are cut again by every bullet that crosses them (one that flies on into the sky too), and a grenade's blast throws the loose pieces away. Windows crack, shatter and burst with BO2's own sounds and effects. Checked in hidden test matches on Express
- 2026-10-19 (BO2 movement test build): the sound options work the BO2 way: the music, effects, voice and cinematic sliders each turn their own sounds up or down (menu sounds follow the master volume alone), and every sound plays at its BO2 sound group's level, so the whole game sits about 10 dB lower than before, as BO2 does. Sounds BO2 gives a fixed speaker pan (shell casings on the right, music across both speakers) keep it instead of turning with the camera, and sounds BO2 sends only to a subwoofer or centre speaker stay silent in stereo, as in BO2
- 2026-10-18 (BO2 movement test build): the map work and the engine work are one build again: map props lit by their own light sets and plants that sway in each map's wind now sit with bots and script traces that see fences and cars, Uplink's player bodies, and every player coming back for the next Search and Destroy round
- 2026-10-18 (BO2 movement test build): minimap objective icons follow BO2's own row data: each icon's size, icons that turn with the thing they mark (Express's train cars), and icons that stay inside the minimap's edge. Map scripts can shake the screen the way BO2's earthquake does, for everyone nearby (Express's passing train uses it once the maps work lands)
- 2026-10-18 (BO2 movement test build): the Combat Training lobby holds BO2's number of players: 12 for a 6 v 6 playlist, 8 for Free-for-All (the match's seats, bots included), and every other kind of lobby starts from BO2's own limit (Public Match 12, Theater 6, League 4, the party lobby 18). Its "Searching for player..." rows are BO2's own: BO2's public lobby shows one for each open seat while the map vote runs and clears them, showing the teams, once the start timer begins
- 2026-10-18 (BO2 movement test build): in a match, the change-class menu's default classes show their gun and remember your pick, and the HUD uses your own team's icon and colours (it showed the Axis ones to everyone)
- 2026-10-18 (BO2 movement test build): Create-a-Class shows which attachments clash with the one you are looking at, as BO2 does, instead of marking every other attachment once one is on
- 2026-10-18 (BO2 movement test build): the VTOL Warship's minigun fires from both barrels and its rockets take turns from each side, the AGR's gun goes round its four barrels, and a scorestreak gunner's shots give the small screen shake BO2 adds near the vehicle
- 2026-10-18 (BO2 movement test build): the RC-XD drives through the holes cut for it in Nuketown's fences, and its wheels stand on what its body does, as in BO2 (from the maps window)
- 2026-10-18 (BO2 movement test build): a knife swing breaks the loose glass hanging in a broken window, as in BO2
- 2026-10-18 (BO2 movement test build): the screen shakes when a bullet or a blast strikes the riot shield you carry, as in BO2
- 2026-10-18 (BO2 movement test build): Hardpoint on Hijacked plays a full match: the game's hills sit on trigger boxes it spawns itself, and those were never counted as touched, so the match stopped with a script error. In the hidden match sweep so far, 143 of 144 map-and-mode matches pass (Drone Headquarters plays, with no kills yet); the sweep is still working through the rest of the maps
- 2026-10-18 (BO2 movement test build): a player stuck in the floor still stands on it, as in BO2, so he can walk, jump and pick things up there (from the maps window)
- 2026-10-18 (BO2 movement test build): a tomahawk that kills drops to the floor by the body instead of hanging in the air, and a tomahawk from a scavenger bag lands in your hand, so throwing and picking it up keeps working after one
- 2026-10-18 (BO2 movement test build): scorestreak vehicles hit directly (bullets, knives, script damage) now take their damage; an unset vehicle damage scale stopped each hit with a script error
- 2026-10-18 (BO2 movement test build): the screen shakes near grenades, rockets and other explosives going off, and near scorestreak guns firing, by each weapon's own BO2 values
- 2026-10-18 (BO2 movement test build): the screen shakes near the Hellstorm's launch, the RC-XD, scorestreak fire, and exploding cars and barrels, as in BO2
- 2026-10-18 (BO2 movement test build): the Hellstorm's view shakes when its rider boosts it, as in BO2
- 2026-10-18 (BO2 movement test build): crossbow bolt and thrown knife impacts, and the sensor dart's laser, now reach the other players' games (the update that carried one was being thrown away whole, so the impact sound and effect never played)
- 2026-10-18 (BO2 movement test build): a weapon in a player's controls that he does not hold reads as the one in his hand, as BO2's server does, so a stray one never puts his gun away
- 2026-10-18 (BO2 movement test build): once the host ends the game, Esc no longer opens the in-game menu, as in BO2
- 2026-10-18 (BO2 movement test build): the crossbow works as in BO2: a bolt that sticks in a wall or a player blinks and beeps, then blows up a second later (the game had never read BO2's explosion type for its weapons, so a bolt could not tell it was a bolt); a scorestreak vehicle now takes or ignores a heavy explosive's damage by its own BO2 settings
- 2026-10-18 (BO2 movement test build): crossbow bolts and thrown knives make BO2's impact sound and effect for what they strike (wood, metal, water, a player, a riot shield...)
- 2026-10-18 (BO2 movement test build): flash grenades blind and deafen the players who see them go off (more the nearer they are and the more squarely they face them), and an RC-XD's driver, the way BO2's do
- 2026-10-18 (BO2 movement test build): a tomahawk, crossbow bolt, thrown knife or grenade that hits a player counts as an impact, as in BO2 (and so does a rocket that hits him before it arms, or a Semtex that sticks to him), so a tomahawk hit no longer shell-shocks like a frag and the challenges that need an impact hit can count; the game's scripts now hear when a player takes damage, as BO2's do
- 2026-10-18 (BO2 movement test build): BO2's own spawn system picks where players spawn (influencers, enemy sight, telefrag check); thrown grenades spin and a bounced tomahawk comes to rest so it can be picked up
- 2026-10-18 (BO2 movement test build): the Dragonfire flies the way BO2's does: it speeds up and slows down as BO2's helicopter rules say (slowing faster than it speeds up), turns toward where you look no faster than its own turn rate, leans into what it does (nose down flying forward, banking into a turn), and with neither up nor down held it keeps its height over the ground, follows steps in the ground, climbs when too low and keeps clear of ceilings. It bumps through the map by its own shape instead of a plain box, and players, doors, lifts, moving objects and the other scorestreak vehicles (by their own collision shapes) stop it as in BO2
- 2026-10-18 (BO2 movement test build): a vehicle gunner's rockets now leave from where he looks and fly out through his own vehicle, as in BO2, instead of leaving from the gun's muzzle
- 2026-10-18 (BO2 movement test build): the Sentry Gun's and the scorestreak vehicles' bullets now glance off a riot shield the way a player's do (about half of them fly on to the side), and the AGR's driver and a vehicle's gunner fire from where they look, as in BO2, so their bullets land exactly on the crosshair
- 2026-10-18 (BO2 movement test build): One in the Chamber's HUD shows how many lives you have left, counted down each death the way BO2's is, instead of showing nothing and stopping its script each spawn
- 2026-10-18 (BO2 movement test build): the K9 Unit's dogs move as BO2's do: a dog turns to face its way before it sets off running, flinches toward a hit that does not kill it (a running dog keeps running), and knifing a dog counts as the knife you swung, the same as knifing a player
- 2026-10-18 (BO2 movement test build): a smoke grenade's cloud now hides you from a Sentry Gun as in BO2: it cannot pick you out through the smoke, and stops shooting once the cloud comes between you
- 2026-10-18 (BO2 movement test build): the Sentry Gun's gun now turns and tilts on its own stand as BO2's does: its legs stay where it was set down while the gun swings round, it tips up and down to aim at whoever it shoots (or wherever its driver looks), and its bullets leave the tipped barrel
- 2026-10-18 (BO2 movement test build): locked-on rockets (the SMAW, the FHJ-18) and the strafe run's rockets now steer onto their target the way BO2's do, never turning harder than each launcher allows, and the Hellstorm's bomblets climb gently, then dive onto their target. The Hellstorm speeds up once to full speed as in BO2, turns faster steered with a mouse than with a controller, and drifts on slowly once its bomblets are out. A gun or rocket aimed straight down no longer faces straight up. With no one to shoot, a placed Sentry Gun now sweeps side to side across its turning range as BO2's does, with its scanning sound, and stops when it finds someone. Every one of the server's gun functions, scorestreaks included, is now rebuilt from BO2's rules, done our own way or not needed in multiplayer
- 2026-10-18 (BO2 movement test build): rockets weave in flight the way BO2's do (the SMAW, the RPG, the FHJ-18, the strafe run's rockets: straight for their first stretch, then turning a little every moment), and the Escort Drone draws rockets fired near it onto itself, as BO2's does
- 2026-10-18 (BO2 movement test build): a thrown grenade passing through a trigger that a map's scripts set to feel grenades now sets it off, as in BO2 (BO2's own maps never turn this on; mods and custom maps can). Triggers that shut out some kinds of damage now tell an explosion from a grenade's direct hit as BO2 does (they were mixed up, so Encore's instruments shrugged off a grenade's direct hit). Every server gun function outside the scorestreaks is now rebuilt from BO2's rules, done our own way or not needed in multiplayer
- 2026-10-18 (BO2 movement test build): the Sensor Grenade works as in BO2: every 0.8 s it sends out its scan, and for 0.4 s of each its laser sweeps the enemies within range one after another, with the alert sound and a ping on the thrower's minimap for each it reaches; when it goes off, the enemies it can see stay on his minimap for 6 seconds. A perk with specialty_nomotionsensor hides you from it
- 2026-10-18 (BO2 movement test build): the Sentry Gun's and the vehicles' guns (Stealth Chopper, VTOL Warship, AGR, drones) fire real bullets traced the way a player's are: through glass, into breakables, dogs and other scorestreaks, with BO2's own spread for each gun (wider when no one is aiming it), and a gunner's gun holds fire while his own vehicle is in the way. Still missing: turret bullets glancing off a riot shield, and a vehicle gun firing from where its gunner looks
- 2026-10-18 (props lit like BO2): every placed prop now takes its light from its own light set, the way Black Ops II stores it for each one (its colour from 56 directions plus a soft fill), read with BO2's own decode and shading instead of a guessed brightness scale. Checked in a hidden test match on Nuketown.
- 2026-10-18 (trees and plants in the wind): palms, ivy, hedgerows, flowers and the other plants BO2 sways now move in each map's own wind, the way BO2's shaders move them; the trees and grass BO2 keeps still stay still. Checked in hidden test matches on Castaway (palms) and Village (ivy, flowers, lawn).
- 2026-10-18 (map logic, trees and Nuketown): checked in hidden test matches on all 31 maps. Tree leaves and props are lit by the sun the way BO2 stores it for each one, so far and out-of-map trees are no longer dark and indoor props no longer glow; leaves use BO2's canopy shading and draw solid first, then their soft edges. On Nuketown, loose things can be pushed again and again, the RC-XD drives through the fence gaps cut for it, and a player stuck in the floor stays standing on it, as in BO2.
- 2026-10-17 (BO2 movement test build): a grenade launcher round that has not armed yet now bounces off a wall it strikes at a glance, as in BO2, and one slower than 500 a second bounces too; it still duds when it hits a player or strikes a wall head-on at speed. Every grenade bounce now tells the game's scripts where it bounced and off what, so a Shock Charge that bounces off an enemy goes off and a decoy turns away from a wall, as BO2's do; a breakable (a car, a barrel) hears its first five bounces. A grenade off a riot shield counts as hitting the shield
- 2026-10-17 (BO2 movement test build): a bullet hitting a riot shield now tells the game where on the shield it struck, as BO2 does: everyone else sees the shield's own hit effect there and the holder hears his shield take the hit (BO2's view shake and dent on the held shield are not drawn yet). Armour-piercing guns (SCAR-H, LSAT, Ballista, DSR 50 and others, 27 in all) no longer play a riot shield hit on everything they strike. A grenade bouncing off a riot shield plays BO2's bounce effect for a shield
- 2026-10-17 (BO2 movement test build): the legs' slope animations follow BO2's own rule: how steeply the ground rises the way you face, averaged the way BO2 does it. As in BO2, at its settings they never play, so nothing looks different. The function list now also covers 267 of the server's own gun functions (bullets, rockets, grenades, damage, melee, the riot shield and breakables): 91 rebuilt from BO2's rules, 90 done our own way, 38 not used in multiplayer and 48 still missing, 33 of them scorestreak pieces
- 2026-10-17 (BO2 movement test build): your character's legs now jump and land with you as in BO2: a jump, a jump off a ladder, letting go of a ladder in midair and walking off an edge play the jump, and a fall of more than 12 units plays the landing. Crouched footsteps use BO2's own crouched run and crouched walk sounds, and climbing a ladder plays a hand on each rung between the foot steps. The first frame of a ledge climb now sorts out sprint and crouch the way BO2 does. Every one of the 657 player-code functions is accounted for (439 rebuilt from BO2's rules, 150 done our own way, 68 not used in multiplayer, none missing; [docs/MOVE-T6.md](docs/MOVE-T6.md))
- 2026-10-17 (BO2 guns and movement, in the movement test build): dropped guns now follow BO2: walking over one takes the ammo your guns can hold and leaves the gun there if some is left; the one who dropped it waits a second before he can touch it; holding use swaps it, and your old gun lands where it lay, on its side along the floor; a gun dropped while crouched or prone starts lower; a falling gun slides off steep slopes. No use prompt while you climb, sprint, dive or are in last stand. Through iron sights the gun's own sway and bob go where your bullets go; a riot shield turns about half the bullets that hit it aside. Triggers now see how tall you are: crouched or prone you no longer set off one hanging above you ([docs/MOVE-T6.md](docs/MOVE-T6.md))
- 2026-10-17 (BO2 movement test build): Black Ops II's whole player code is now accounted for, function by function (657: 440 rebuilt from BO2's rules, 150 done our own way, 67 not used in multiplayer, none missing; docs/T6-LEDGER.md). New since the last build: a double tap of use or reload blows up C4; a gun in your hands that forbids prone stops you going prone; a frozen player stops dead; the spot a climber lands on is held for him; feet rustle through bushes; the legs turn in place and flinch when hit, as BO2's; slow motion (end-of-match killcam) as BO2 eases it; the Escort Drone and Dragonfire find each map's helicopter ceiling; a window just past a ledge stops you climbing it; a controller's triggers fire and aim once pulled 13% of the way, as BO2's do (they waited for 75%); and planting a riot shield holds your view still, as in BO2 ([docs/INPUT-T6.md](docs/INPUT-T6.md))
- 2026-10-17 (BO2 guns, in the movement test build): getting shot now moves where your bullets go as well as the camera, as BO2 does: the flinch is in the aim, and when you aim through iron sights the gun's own jolt is too ([docs/INPUT-T6.md](docs/INPUT-T6.md))
- 2026-10-17 (BO2 guns, in the movement test build): attachments change the gun the way BO2's files say: Fast Mag reloads faster and swaps mags, Extended Mag holds more and reloads slower, the Tri-Bolt uses its own magazine, a KSG with Fast Mag loads two shells at a time, Quickdraw and the other quick-aim attachments speed up aiming in, and scope attachments use their own zoom. Also: firing or switching guns while prone keeps you crawling the BO2 way; a claymore or Betty can't be planted in midair or in deep water, and a tactical insertion can't go down under a low ledge; a burst gun ignores a mouse click made mid-burst, as BO2 does; a variable-zoom scope no longer steps its zoom every time you aim in; the FHJ-18 only fires with a lock; and a single-shot gun clicked faster than 20 times a second jams for 2 seconds, as in BO2
- 2026-10-17 (BO2 controls, in the movement test build): a controller pushed fully diagonal now runs at full speed (it was 71%), as BO2 stretches the round stick to the square; the mouse and pad turn the view up or down at most 90 degrees in one frame, as BO2 does; mouse acceleration reads how far the mouse moved, not the sum of both directions ([docs/INPUT-T6.md](docs/INPUT-T6.md))
- 2026-10-17 (map logic, match end): checked in hidden test matches on all 31 maps. Every match ends by BO2's own rules (the score or the time limit), the end-of-match board opens and the final killcam plays; Takeoff's own end sequence runs too, its closing camera fading to black.
- 2026-10-17 (menus): all 31 maps start from BO2's own Custom Games map list, picked with real key presses and checked in hidden test matches: Dig, the last one, now loads from the menu and its match begins.
- 2026-10-17 (map logic, Drone): the scanner sets off its alarm when a player walks in (detect sound at the trigger, alarm a quarter second later), as the original's client script plays them.
- 2026-10-17 (map logic, client scripts): checked in hidden test matches on 7 maps. The moving parts BO2's client scripts run now move here too: the spinning fans, turbines and windmills on Uplink, Frost, Turbine, Downhill, Vertigo and Hydro; Frost's and Nuketown's clocks show the time; Hydro's and Vertigo's security cameras follow the player; Uplink's light post sways, its wind vanes swing when the wind turns, and its lightning strikes with thunder; Turbine's windmills whoosh; Nuketown's sign topper spins and its end-game cars start hidden, as BO2 shows them only at the end.
- 2026-10-17 (map logic, Cargo's containers and Frost's kill zone): checked in hidden test matches. Cargo's crane containers cut the bot paths they stand on, as BO2's script does, so bots walk around them; on Frost, the kill zone BO2's script places under the map kills a player who reaches it, with BO2's own death.
- 2026-10-17 (map logic, bot paths): checked in hidden test matches. The hidden blockers that BO2's maps stand on bot path nodes now cut those paths by the blocker's own size, so bots keep off Frost's and Rush's unbroken glass and Detour's two bad links; once the glass breaks, its blocker is removed and the paths join again, as BO2 does.
- 2026-10-17 (map logic, Vertigo's windows and edge): checked against BO2's map scripts in a test match. Breaking the windows by the drop plays BO2's wind loop at each broken pane and its window effects, as BO2's script runs them; falling off the edge there kills with BO2's falling death, and the body goes limp once it lands.
- 2026-10-17 (map logic, props and menus): checked in hidden test matches in both games. A grenade breaks a parked car's doors, windows and wheels, and the car's own last explosion reaches every piece of it, as BO2's scripts deal it; a blast reaches each breakable once, credited to the thrower's weapon. Cargo starts when picked from the Custom Games map list, and 29 of the 31 maps now start from BO2's own menus with real key presses (Dig and Takeoff are next).
- 2026-10-17 (map logic, game modes): all 9 modes on all 31 maps, checked in hidden test matches with 2 friendly and 3 enemy bots, with no new script error in any. Team Deathmatch, Free-for-all, Domination and Hardpoint pass on every map, Kill Confirmed on 30, HQ on 29 and CTF on 28 (kills plus the mode's own goal: tags confirmed, zones captured, flags taken). In Demolition a bomb is planted on 29 maps; Search and Destroy starts on all 31 (whole rounds of both are not checked yet). A car the mode removes at the start (Raid's sports car, Hydro's two red vans) no longer blows up, as in BO2
- 2026-10-17 (map logic, solid models): checked against BO2's collision maps in test matches on all 31 maps. A model stops you only where BO2's own collision map makes it solid: parked cars stop you (Bridge's police car, Skate's green sports car and the rest), you step up onto low solid props (Pod's bowl handles), and small props such as bins, plants and luggage are walked through, as in BO2. Where the map's own walls already wrap a car, they stop you first, as in BO2. A model the map's script moves takes its collision along: Studio's moving targets stop you where they are now (you walked through them before), and Vertigo's sliding glass doors let you through once they open (they kept blocking the doorway).
- 2026-10-17 (map logic, every map): checked against BO2's map data in test matches on all 31 maps. Every map's windows are read from its own map file and break by BO2's damage rules (Skate reads 36 and breaks 35: the 36th has no outline in BO2's own map file, so BO2 skips it too). Every map's breakable props are hit and break. Falling into Encore's water plays BO2's death splash sound with the splash just above you, and kills.
- 2026-10-16 (BO2 movement test build): a new "BO2 MP movement" launcher runs the new guns with Black Ops II's own movement, written from BO2's own rules: walking, sprint, jumps and landings, the air, friction, sliding, crouch, prone and dive, ladders, climbing and vaulting ledges, a late move caught up the BO2 way, and speeds cut to whole numbers as BO2 does ([docs/MOVE-T6.md](docs/MOVE-T6.md))
- 2026-10-16 (map logic, Raid, Frost water): checked against BO2's map scripts in test matches. Stepping into Raid's pools plays BO2's small splash where you step in, once each time; dying in Frost's canal plays BO2's splash sound with the splash at the water's surface, as BO2 measures the water's depth.
- 2026-10-16 (map logic, Overflow, Takeoff): checked against BO2's map scripts in test matches. Falling into either of Overflow's canals plays BO2's splash sound and water splash and kills. When a Takeoff match ends, the shuttle launches as in BO2: its thrusters fire, every player watches from the launch camera, which starts shaking half a second later, the screen fades to black 4 seconds in and the level ends 5 seconds after the match. Not yet: the shuttle itself draws black (its lighting, still to fix).
- 2026-10-16 (map logic, Hydro, Studio, Magma): checked against BO2's map scripts in test matches. Hydro's water rush comes down the spillway a quarter and three quarters of the way through the time limit and kills whoever it catches. Studio's targets run their rails, the alley target stays at its far end while someone stands in the alley, and a shot target spins round. Magma's lava and its spawn-kill trigger kill as in BO2, and its police cars now flash Magma's own light bars (they used Aftermath's).
- 2026-10-16 (map logic, Downhill): Downhill's flags, windsock, Christmas lights, standing skis and its two lift chairs move, as BO2's client script runs them (read from the map's own compiled script, which the reference scripts lack).
- 2026-10-16 (map logic, Vertigo's blinds): Vertigo's blinds swing when a window within reach of them breaks, then settle into their idle, each only once, as BO2's client script runs them.
- 2026-10-16 (map logic, Cove, Uplink, Frost, Takeoff): the last four DLC maps play their moving props as BO2's client scripts run them: Cove's crabs take turns between their two moves, the radars on Uplink and Takeoff and Frost's crane go through their five moves (Uplink's with its red beacons), and Takeoff's decontamination blasters steam from the nozzles their animation turns on, its planets glow and its blue lights come on.
- 2026-10-16 (map logic, Studio): Studio's pirates ride their boats, the T-rex plays one of its three moves every 10 to 15 seconds and the brontosaurus chews and looks around, as BO2's client scripts run them (not yet: the brontosaurus's half-second blend between the two).
- 2026-10-16 (map logic, Detour): Detour plays its moving props as BO2's client scripts run them, its map pack's own animations included, and its hanging wires spark as they swing.
- 2026-10-16 (map logic, 9 DLC maps): Grind, Hydro, Mirage, Encore, Dig, Pod, Rush, Magma and Vertigo play their moving props as BO2's client scripts run them, each map pack's own animations included: Encore's river boats carry their sails, and Magma's sparking wires spark.
- 2026-10-16 (map logic, Express): the train raises BO2's dust along the track as each engine passes, and the cars show on the minimap the way BO2's client script marks them: the engine and the carriages each with their own icon, turning with the track and cut off at the map's edge.
- 2026-10-16 (map logic, all 15 base maps): the maps' moving props play as BO2's client scripts run them: flags, cloth, signs and wires sway, sparking wires spark, Drone's welders weld, Nuketown's dome and its end-game car flip and glass play, and props that wait for a map event start on it
- 2026-10-16 (map logic, Nuketown 2025, Hijacked, Turbine, Overflow): Nuketown's town sign turns as in BO2 (the top spins, the needle and the number wheels move), and at the match's end the nuke falls the BO2 way: the closing camera faces the blast, the two display cars come up on their stands behind their glass, every flag turns in the blast and the mannequin by the gate loses her head and tips over. Hijacked's water kills with a splash; Turbine's rock pile is solid where BO2 puts it; Overflow sets its helicopter height lock off as BO2's map script does.
- 2026-10-16 (map logic, Express): the train runs its whole loop the way BO2 drives it, on its own track, easing round the bends, and it kills whoever stands on the line; the barriers at the platforms rise and drop on its schedule. The Warthog scorestreak now flies BO2's own flight path too: it makes its strafing runs and then leaves.
- 2026-10-16: Drone's sliding doors work as in the original: step up to one and it opens, walk away and it shuts, and
  shooting a door holds it open for a moment.
- 2026-10-16 (map logic, Carrier): the warships out at sea rock slowly on the waves, as BO2's map scripts float them, and falling into the sea kills with a splash; map traces now report the surface they hit the BO2 way, which Carrier's water test reads.
- 2026-10-16 (map logic, Cargo): Cargo's crane runs its cycle and its crate crushes whoever stands under it; falling into the sea kills with a splash, as BO2's map scripts do; and a death with no player behind it (the crane, the sea) shows only the victim in the kill feed, with no "Killed By" card.
- 2026-10-16 (map logic, Aftermath): the police cars' and the ambulance's light bars flash, as BO2's map scripts switch them on, and go dark when the car is blown up.
- 2026-10-16 (map logic, Raid windows): Raid's windows take damage the BO2 way: a grenade, rocket or knife breaks one at once, a bullet cracks it and a cracked window weakens until it falls apart; blasts reach windows too, weaker toward their edge, and each broken window tells the map's own scripts, as BO2's server does.
- 2026-10-16 (map logic, Raid first): Raid's cars are solid, its props break and fly when shot or blown up and stay on the map, and its windows stand where BO2 puts them: they stop you and your bullets until they break, and they are drawn with BO2's own glass (clear while whole, gone once shattered). Every map now has these three boxes on the scorecard.
- 2026-10-15: Black Ops II Multiplayer has its own private repository now (bo2mp, this branch is the front page); Zombies stays in bo2zm, Call of Duty Multiplayer in codmp, Black Ops Minecraft in bo2mc.
- 2026-10-15 (new guns test build, evening): no more freezes while you play with the gun log on (it is written on its own thread now); after you die, the camera stays on your own body and turns to your killer instead of flying to where you died last time and back; the Ray Gun fires one shot per trigger pull; the game reads BO2 from the fast SSD copy, so maps load in about 25 seconds instead of 40. Also in this build: point-blank hits use BO2's hit boxes, BO2's damage ranges, red ammo numbers when you are low, and riot shield fixes.
- 2026-10-15 (r149): the Ray Gun fires one shot per trigger pull again (since r147 it kept firing while the trigger was held and burned its ammo); linked portals show what is on the other side; the Portal Gun is the blocky model; the test laptop now always copies the menu and the Gravity Gun art with the game. Waiting on a playtest.
- 2026-10-15 (new guns test build): the "BO2 MP new guns" launcher now has knife lunges that aim at the enemy you saw, the right sprint, dive and empty-gun animations, the riot shield held straight for other players, its glass window drawn, a cooked grenade held too long killing you and letting you respawn, and the Hunter Killer held until you let go. Waiting on a playtest.
- 2026-10-15 (new guns test build, update): dual-wield pistols now show and fire both guns; the frag pin pull, semtex pull, semtex beep + light and the grenade throw and roll match BO2; EMP, concussion and smoke grenades go off after they roll. Waiting on a playtest.
- 2026-10-15 (new guns test build, afternoon): a knife into a riot shield no longer hits or bleeds; the attachment key skips guns that have no second mode; climbing over ledges follows BO2's rules for the gun and the arms; other players see a cooked grenade as one pin pull and one throw; the Bow and Portal Gun fixes from the zombie weapons build are in. Waiting on a playtest.
- 2026-10-15: Care Package playtested: better, not perfect yet. Still open: the drone's blades do not spin in the play-test (they turned in a hidden test, so the cause is still to find). Next time.
- 2026-10-15: Care Package fixes, waiting on a playtest. The drone's main and tail rotors spin, with the crate hanging from its hook under it. The crate lands on top of the ground (before, half of it sat in the ground) and only kills someone it falls on: a crate lying still no longer kills whoever walks up to it, so bots can now take it. Enemy and teammate bots next to your crate take it, and you hear BO2's "enemy stole your package" or "teammate took your package" callout. Your crate's icon stays on the edge of the minimap when it is off the map's edge, pointing to it.
- 2026-10-15: the Hunter Killer passed a playtest ("a lot better now"). Next: the Care Package.
- 2026-10-15: the Hunter Killer only locks onto an enemy it can see, then flies straight at them, fast. It no longer locks on through walls or flies into the wall in between: an enemy can get away by putting a wall between themselves and the drone, and it goes back to hunting. With nobody in sight it hunts and circles back round the map until it finds someone. Like BO2, it can't target enemies using Blind Eye. In the next build.
- 2026-10-15 (r148): the Bow draws on either mouse button and its 20 arrows refill by themselves 1 second after the last one, with no Ray Gun reload in between. The Portal Gun is the blocky Minecraft Portal Gun, its portals are Minecraft's pixel ovals that grow open (the Super one has a purple halo), they stick to hedges, fences, props and bumpy ground, and you walk through once both are open. Arrows, the stick and portal shots stop at hedges and fences instead of passing through.
- 2026-10-15 (r147): the Hunter Killer flies like a jet. Thrown with nobody in sight, it flies straight and fast along your throw, only drifting up a little, and blows up when it hits something or has flown too far; it no longer climbs high into the sky. When it sees an enemy it locks on and dives at them as before. You now hold the button on top of it before the throw (the new BO2 gun code), and it launches when you let go. Waiting on a playtest.
- 2026-10-14 (r146g): Weapons > Zombies Wonder Weapons gains the Gravity Gun, Super Gravity Gun, Bow, Enchanted Bow, Enchanted Stick, Portal Gun and Super Portal Gun, each working against players and bots (Pack-a-Punch turns the Bow, Gravity Gun and Portal Gun into their upgraded versions).
- 2026-10-14 (r146): the thrown Hunter Killer opens its wings and tail fins as it leaves your hand, as in BO2 (before, they stayed folded). Waiting on a playtest.
- 2026-10-14 (r145): faster map loading: each shader is now built once instead of once per copy, and the loading bar moves evenly to the end instead of sitting at 96%.
- 2026-10-14 (r144): the RC-XD stops exactly where it blows up (the view no longer slides on), and if you are killed while driving it you keep driving with the normal car view until it blows up, then respawn, like BO2.
- 2026-10-14 (r143): the RC-XD checked side by side against real BO2: its wheels roll with its speed and the front pair turns as you steer, each wheel springs up and down over bumps, it stops against walls and objects instead of driving through, backing up goes straight back, its lights blink at 20 and 26 seconds and go out the moment it blows up, enemies and bots shoot at it and their bullets blow it up, and the blast shows BO2's flash, fire and scorch for one second with no black frame before the laptop comes down. Waiting on a playtest.
- 2026-10-14 (r142): more mod menu options. Player: Invisible, Super Jump, Low Gravity, No Recoil, Rapid Fire, Explosive Bullets, Infinite Equipment. Trickshots: a Care Package platform under your feet (and Remove Platforms), Canswap (a random gun dropped at your feet), Fast Last (one kill from winning). Game: Pause the Clock, Unlimited Score, Slow Motion, Change Map, Change Mode. Bots: Bots Don't Shoot, Kill All Bots, Kick All Bots. Closing the menu with Knife no longer knifes whoever is in front of you.
- 2026-10-14 (r141): the RC-XD keeps going when you die while driving it: you keep steering, there is no killcam, and you respawn 2 seconds after the car is gone, as in BO2. Like BO2, it has no on-screen timer: it beeps and blinks at 20 and 26 seconds and blows up at 30. The Hunter Killer now sees enemies standing on the floor the way BO2 does, so it locks on right after the throw and dives at them (r139 only found them late, or never, and climbed slowly instead). Waiting on a playtest.
- 2026-10-14 (r140): mod menu fixes: UFO Mode flies, Teleport to Crosshair keeps your crosshair, the Death Machine has its sound and its HUD, Bring Bots stands them in a row in front of you, and picking a scorestreak uses it at once. New Weapons pages: every Zombies wonder weapon (Ray Gun, Ray Gun Mark II, Jet Gun, Sliquifier, Paralyzer, Blundergat, Acidgat, the four staffs) and special (Ballistic Knife, Monkey Bomb, EMP, Time Bomb, G-Strike, Hell's Retriever, Hell's Redeemer, Zombie Shield, Bowie Knife, Galvaknuckles), plus Pack-a-Punch for the gun in your hands. Passed the playtest.
- 2026-10-14 (r139): the Hunter Killer as BO2 has it: held, its nose light blinks and its raise sound plays once when it has a target; thrown, it unfolds, blinks red and steers onto an enemy BO2's way. While you hold a scorestreak item (Hunter Killer, RC-XD, care package marker), the ammo box keeps showing the gun you had before, as BO2's does. Waiting on a playtest.
- 2026-10-13 (r138): scorestreaks use one key (3): it brings out your cheapest earned streak first, and does nothing when none is earned (keys 1 and 2 no longer pull out streaks). The RC-XD's wheels spin with its speed and the front pair steers, its body leans with the ground under its four wheels, and its engine revs and fades like BO2's (its four engine loops, BO2's own engine maths). The mod menu can make an enemy call a UAV or Counter-UAV for testing. Waiting on a playtest.
- 2026-10-13 (r137): GSC mod support: scripts in a `mods` folder next to the game load and run (source or compiled), and a built-in mod menu (hold Aim, tap Knife) with God Mode, Infinite Ammo, UFO Mode, Third Person, every gun, every scorestreak, bot controls and more; God Mode and Third Person work in the engine for any mod that uses them. See [Mods](#mods-gsc-scripts).
- 2026-10-13 (r136): the RC-XD drives like BO2's: its own boost, steering, grip, hand brake and wheel spin from the car's BO2 data (steering and boost used to reset every moment); the camera sits behind it and falls back with speed; green lights for your team, red blinking for enemies, a beep and flash with each countdown tick; BO2's Boost / Detonate / Handbrake hints; dust or concrete spray off the back wheels by surface; dirt (sand, grass, mud) or dust (everything else) thrown on the driver's screen; and the boost sound. Waiting on a playtest.
- 2026-10-13 (r135): the Counter-UAV jams the enemy minimap the way BO2 does: the whole map breaks up into BO2's own moving, blocky, colour-split static (its shader, ported), your arrow, teammates and enemies vanish while it lasts, and Hardwired keeps a clear map. Waiting on a playtest.
- 2026-10-13 (r134): the UAV checked against BO2's own code: the tablet comes up and goes away in 0.6 seconds as BO2's does (it used to take a full weapon switch); the sweep's speed, direction, width and look, the red dots and their 4-second fade, and the voice lines all match BO2. BO2 itself plays no sound on each sweep, so neither do we.
- 2026-10-13: this repository stays exact Black Ops II (tag `exact-bo2-before-pr2`). The shared public repo now carries a contributor's merged work (other games' maps and guns, online play) as a separate build; Black Ops II fixes from it come here only after a side-by-side check against the original.
- 2026-10-11 (r133): attachments and gun levels from BO2's own weapon data: every attachment's numbers, two attachments per gun, attachment models in hand, on other players' guns and on the floor (iron sights hidden under a sight, a dropped gun keeps its attachments), the Suppressor's sound, flash and no minimap dot, FMJ twice as deep with its through-wall stats, the Grenade Launcher and Masterkey switch, Dual Wield, Tactical Knife kills, default classes' attachments and bots' picks; gun XP from use in Combat Training on BO2's level table, the level-up notice, locked attachments showing the gun level they need, everything unlocked in Custom Games. From a check of BO2's scripts and data; not yet checked side by side.
- 2026-10-11 (r132): Public Match > Combat Training plays BO2's map rotation: every lobby offers a NEXT map, the PREVIOUS one and RANDOM, with BO2's own vote tiles, counts and "Time remaining" line; your vote picks the map that loads (most votes win, ties go to NEXT), then the lobby shows "The match is about to begin..." and "Game starting in N" as BO2 does. Maps come from a shuffled bag that never repeats the last two.
- 2026-10-11 (r131): Custom Games > Setup Game > Change Map works: the lobby shows the picked map's picture, name and mode, as BO2 does, and START MATCH loads that map (it used to stay on Aftermath).
- 2026-10-11 (r130): the Lightning Strike can be called in: its tablet shows where the enemies are as it opens, you pick three spots on the map, and the strikes land on those spots in BO2's order and timing; the UAV says "UAV inbound" and its sweep crosses the minimap every 4 seconds, showing enemies where it finds them and fading them over 4 seconds, by BO2's own values.
- 2026-10-11 (r129): bullets pass through players and thin surfaces the way BO2 works it out, using BO2's own penetration table (how deep a bullet gets into each surface type), with the damage dropping after each target it goes through; a sniper shot can now kill the player behind the one it hits (a collateral), checked for the DSR 50, Ballista, SVU-AS and XPR-50 against BO2's numbers; Create a Class: the weapon category list under the guns no longer draws names on top of each other or repeats a row, and every category screen lists the others in BO2's order.
- 2026-10-11 (r128): the final killcam plays for everyone at the end of a match, including a player who is still alive (BO2 seats every player on it); End Game in the pause menu leaves the match, and from Combat Training it goes back to the Public Match menu, as BO2 does when the public lobby breaks up; the scripts that run for each life now end when a player dies, so they no longer pile up over a match (one part of the late-match slowdown); with vsync on, frames are queued the way BO2 leaves them (the graphics default of 3).
- 2026-10-10 (r127): riot-shield hits use the shield's own BO2 model instead of a guessed angle, so shots that reach the body or legs before the shield now count, and a shield on the back still stops them; the kill replay draws players between the right two snapshots, the way BO2 does, so they move smoothly instead of jumping around.
- 2026-10-10 (r126): scorestreaks are earned again with Hardline (the perks' own numbers were never set, so a Hardline kill counted for nothing; every perk now has BO2's values), the announcer speaks (match start, lead taken and lost, one minute left, time's up, win or loss) along with the match-end countdown, enemy names are red and teammates' green in BO2's own colours, the class menu inside a match shows the same classes as the lobby, and a body no longer pops back up after the kill replay. Bots, hit markers, the dive, the riot shield, the death camera, the kill replay and body spin are still on the list above.
- 2026-10-10: aiming down the sights no longer blurs the whole gun (every gun, with or without an optic): the blur now works out each gun pixel's distance with the same near plane the gun is drawn with, so only the far end of the gun softens, by each weapon's own BO2 values (checked side by side against real BO2 with the M27 + reflex sight and the MTAR's iron sights).
- 2026-10-10: a game no longer quits with "lost the graphics card" when the PC is busy for over a second: the graphics library counted a slow hand-over of a finished frame as a lost card, so it now waits again instead (a fixed copy of the library is in `third_party/wgpu-hal`).
- 2026-10-10: the render thread takes about 30% less time a frame (12.8 → 9.2 ms in a Nuketown match on the fast setting): props that share a model are drawn together (about 1,500 props in about 540 draws), posed bodies' index lists are sent again only when they change, bullet marks lit alike share one draw, and the body-posing pass goes out with the rest of the frame.
- 2026-10-10: bodies are posed on the graphics card instead of the processor, so each game window needs about half the processor time (4 windows at once: 28 → 30 frames a second each), and bullet marks' lighting is worked out once instead of every frame. 8 game windows now run at once on one PC at 27–29 frames a second each (`bo2_quality=fast`), each with about 2.1 GB of RAM and 0.9 GB of graphics memory.
- 2026-10-10: bots move and aim by BO2's own rules (how fast they turn and settle their aim, where they look while walking, when they sprint, how fast semi-automatic guns fire, when they knife) and step around teammates the way BO2's bots do; they judge what they can see and reach from the map's own baked node tables. The game thread no longer stalls: the HUD and menus only redraw values that change, settings save only when a menu changes them, and settled bodies stop being re-posed. Objective markers show in the objective modes.
- 2026-10-10: bodies, dropped guns and props out of view are no longer drawn, so each game window spends about a sixth less time preparing a frame (4 windows at once: 22 → 24 frames a second each).
- 2026-10-10: each game window needs much less memory (Nuketown at full quality: 3.1 → 2.2 GB of RAM, 1.6 → 1.4 GB on the graphics card; under 1 GB on the card with `bo2_quality=fast`), a window without focus draws 30 times a second, and a match no longer closes itself when several dropped guns or bodies come to rest in the same moment.
- 2026-10-10: a scorestreak from a care package waits in your inventory slot and comes out the BO2 way: hold the switch-weapon key (controller Y) for a quarter second; a tap still swaps guns, now on release as in BO2. The console's `give killstreak` hands it over through BO2's own scripts. Sun shadows sit around you the way BO2 places them, reach further ahead of where you look and no longer crawl as you turn.
- 2026-10-10: Nuketown's mannequins break the way BO2 breaks them, the match sets up spawns the way BO2 does, names show over teammates' heads, being shot no longer hangs a blood cloud at your own camera (only BO2's red screen edges), and the sun casts live shadows sized for each map.
- 2026-10-10: movement closer to BO2: a hit jolts your view by how hard it hit and the damage arrow points back at the shooter, Lightweight's speed and BO2's sprint recovery, a dive that stops sliding ends at once, a controller dive needs the stance button held, going prone takes BO2's 600 ms, BO2's scripts can switch sprint, stances, knifing and aiming off and on, and the second PC predicts its moves with the match's own movement rules.
- 2026-10-10: a no-graphics test mode (`BO2MP_HEADLESS=1`): a test match with no window, graphics card or sound device. A Nuketown match with bots takes about a third of the CPU, under half the memory and a seventh of the disk reads of a drawn one and loads in seconds, so 8 test matches run at once on one PC.
- 2026-10-09 (r102): movement from BO2's own numbers: a dive needs a short sprint first, waits about 1.5 s after the last one, pops higher and can be steered a little; falls hurt from BO2's heights and the fall-damage perk stops it; a shellshock slows your walk by the shock's own amount. The Lodestar's view snaps to its zoom when you aim, and your gun stays quiet while a scorestreak's view is up.
- 2026-10-09 (r101): the Lodestar puts your view on the drone with BO2's own Lodestar screen (thermal at the start, mouse wheel up switches thermal and normal, left mouse fires missiles from the sky onto your laser, hold F to leave; your own gun stays quiet meanwhile), BO2's Killed By card shows and stays up while you are dead, bodies fall as ragdolls (the killcam replays them), the world blurs behind your sights and while you are dead, names over heads use BO2's font, and Nuketown's cars break when shot.
- 2026-10-09 (r101): movement and guns closer to BO2: sprint and dive in any direction, aim and fire through a dive, a quicker get-up, BO2's own gun animations for diving, crawling, sprinting on an empty magazine and climbing, the fast-climb perk, low climbs keep the gun up, an empty magazine knifes with BO2's empty animation, and the HAMR, AN-94 and B23R fire their first shots faster (set BO2_FEEL=mw2 for the old feel).
- 2026-10-09 (r89): a launcher on a player's back hangs slung the way BO2 carries it, SMG and pistol kills count as the right kind of bullet (a rocket's direct hit is no longer cut by Flak Jacket), and the HUD reads BO2's own weapon names.
- 2026-10-08 (r88): the Hellstorm shows BO2's own missile screen and closes it when the missile hits, the VTOL Warship's thermal view switches on and off (mouse wheel up), sprinting bobs the gun the way each gun does in BO2 (rifles stay steady, pistols bob), the dive has its own rhythm, dual-wielded guns both kick, and stuns make your gun sway as much as BO2's own gun files say.
- 2026-10-08 (r87): BO2 Prestige at level 55 (Barracks > Prestige Mode resets rank, unlocks and classes; the prestige icon shows in the lobby, on the scoreboard and on your card), flashbangs and concussions last as long and slow you as much as BO2's own shock files say, the AGR drives in first person, and the drone prompts show BO2's default keys.
- 2026-10-08 (r86): movement, footsteps and head bob follow Black Ops II's own rules (set BO2_FEEL=mw2 for the old feel), the gun no longer swings when the scope comes off, hard landings slow you down and take health (the fall perk stops that, Quickdraw aims faster), the VTOL Warship, Dragonfire and AGR show BO2's own screens, and helicopters and drones play their engine sounds.
- 2026-10-08 (r85): other players go prone, dive and climb ladders at BO2's own animation speeds, and the VICTORY / DEFEAT banner at match end sits where BO2 puts it.
- 2026-10-08 (r84): Frostbite's open sea no longer shows a dark wedge beside the rooftops, and cliffs, rocks and mountains on 13+ maps (Hydro, Cove, Drone, Castaway...) draw BO2's own tiled rock detail instead of one flat colour.
- 2026-10-08: release build (r83). Multiplayer ships as its own app; Zombies ships as a separate app.
- 2026-10-08 (r83): small HUD and menu text sits on BO2's own line height, so footers and fire-mode text line up with BO2.
- 2026-10-08 (r82): the Weapon Emblem tile shows its default chevron, as in BO2.
- 2026-10-08 (r81): Personalize Weapon's Emblem and Tag tab shows the clan tag tile; the DLC camo tab has no purchase arrows.
- 2026-10-08 (r80): the minimap's dark backing is drawn at the game's own strength (darker at the top, like real BO2).
- 2026-10-08 (r79): kill feed uses BO2's team colours, shows how a toon died with an icon (no more NONE), keeps 4 lines and fades after 5 s; Controls list SHIFT/CTRL once; the version text uses the condensed HUD font in white.
- 2026-10-08 (r78): Controls screen shows mouse buttons with BO2's own mouse pictures (key OR mouse); the Look tab shows invert mouse, free look and sensitivity from your settings.
- 2026-10-08 (r77): parked map vehicles (police cars, ambulances, SUVs) are back; destructible props now load their real model on every map.
- 2026-10-08 (r76): DEFEAT/VICTORY screen team flags glow at full brightness, as in BO2.
- 2026-10-08 (r75): killcam shows the killer's view clean (no blood or hit marks); End Game? over a dead player no longer shows the respawn prompt or the YOU marker.
- 2026-10-08 (r74): death camera follows your falling body (the drop and low view before the killcam), as the original engine does.
- 2026-10-08 (r73): the Killed By card and the dead-player HUD now clear when you respawn (ammo, score and minimap come back); the emblem icon selector opens with its tabs and unlock line (r72).
- 2026-10-08 (r71): Emblem Editor opens to its canvas (layers, Layers Used count, colour) instead of erroring; matches the original screen.
- 2026-10-08 (r70): Create-a-Class pickers no longer list items no class can hold (the stray War Machine tile is gone), as the game's item table marks them.
- 2026-10-08 (r69): Create-a-Class attachments unlock by weapon level from the game's own table, so the grid opens and shows the green and red stat changes like real.
- 2026-10-08 (r68): option sliders and Create-a-Class stat bars draw the game's own pip pictures (grey, hollow, green, red) at the script's spacing.
- 2026-10-08 (r67): menu descriptions break lines exactly where real does (each word counted with its trailing space).
- 2026-10-08 (r66): menu blur behind popups follows the game's own blur shader (darker toward the corners), and covered text blurs like real; Quit popup now matches real.
- 2026-10-08 (r65): scoped in, the score panel, ammo, scorestreaks and killfeed stay visible over the black scope surround, like real.
- 2026-10-08 (r64): loading screen draws the game's own loading bar, the spinner waits for the script's timer, and the version text no longer covers it.
- 2026-10-08 (r63): sniper hold-breath and scoped sway follow the engine's rule; loading screens use the game's patched pictures and match real.
- 2026-10-08 (r62): menus use the game's patched pictures (main menu soldiers, map thumbnails, medals) like the real game; the main menu matches real and the logo behind the Quit popup no longer shows as a grey block.
- 2026-10-08 (r61): the reflex sight's red dot glows onto the scene like the real sight instead of being painted over it.
- 2026-10-08 (r60): the minimap box is backed by BO2's own minimap backing pictures instead of a flat grey, so it is no longer too bright.
- 2026-10-08 (r59): the match HUD blends like the real game (soft parts no longer faint or blown out; minimap backing dark like real).
- 2026-10-08 (r58): menu blur strength and darkening follow the engine's own blur settings (Black Ops 1, the parent engine) instead of tuned values.
- 2026-10-08 (r57): grenade and stun icons in a match are no longer washed out; their brightness matches the real game.
- 2026-10-08 (r56): every menu that opens over another darkens what is behind it the same way (Quit and Options popups included), closer to the real game.
- 2026-10-08 (r55): the world behind the in-match class select is no longer dimmed twice; it is as bright as the real game.
- 2026-10-07 (r54): Setup Bots values line up where the real game puts them (they sat about 40 pixels left).
- 2026-10-07 (r53): a menu covered by another menu stays visible underneath, blurred like the real game, map preview included.
- 2026-10-07 (r52): the class select camera stands where the real game puts it (the map's intermission point), not over the middle of the map.
- 2026-10-07 (r51): menus that cover the screen show a flat grey background at the real game's brightness; the right-hand panel is no longer too bright.
- 2026-10-07 (r50): the world shows sharp (greyed, dimmed) behind the in-match class select like the real game.
- 2026-10-07 (r49): in-match class select text lines up like the real game and weapon pictures are no longer washed out; menu backgrounds get the real dot grid.
- 2026-10-07 (r48): end scoreboard stays up 5 seconds like the real game; Barracks League Teams name block left-aligned like real; Emblem list screen.
- 2026-10-07 (r47): default keys are now Black Ops II's own PC keys (F use, G grenade, C stance, Ctrl prone); older saved key layouts move over, controller buttons stay. End scoreboard rows now show after the final killcam. Barracks League card, Combat Record rank hint, Create-a-Class NEW badge.
- 2026-10-07 (r45): killcam bands tinted like BO2, "Press F to respawn" prompt, default Use key is F (grenade moves to G)
- 2026-10-07 (r44): end-of-match scoreboard waits for the final killcam to finish, like BO2
- 2026-10-07 (r43): Barracks like BO2 (card brightness, NEW badges, name card with hex backing and rank).
- 2026-10-07 (r42): Tab scoreboard colours like BO2 (orange outline, darker score cells); Scorestreaks arrows point right; gun name on the ammo panel readable again.
- 2026-10-07 (r39): lobby globe shaded like BO2 (orange-lit seas, dark land, glowing edge).
- 2026-10-07 (r38): main menu dark on the left like BO2 (globe hidden there, soldier art contrast); lobby globe placed like BO2.
- 2026-10-07 (r37): lobby star floor and globe spin match BO2 (ported from the real menu effects).
- 2026-10-07 (r36): minimap compass letters evenly spaced like BO2; faint hex pattern in the empty minimap.
- 2026-10-07 End of match: the far world goes soft behind VICTORY/DEFEAT, as in BO2
- 2026-10-07 Hybrid Optic: the scope and the red dot share one magazine (the scope showed 0/0)
- 2026-10-07 End-of-match team emblems glow as in BO2
- 2026-10-07 Hybrid Optic swap takes as long as BO2's own swap animation (a third of a second)
- 2026-10-07 Minimap as in BO2: the real zoom, see-through backdrop, arrow size, compass marks fading at the ends
- 2026-10-07 Frag grenade shows BO2's camo pineapple; no skull or bright map arrows over the End Game? popup
- 2026-10-07 In-match ammo panel and scorestreak boxes as in BO2 (outline on the mirrored side, faint hex fill)
- 2026-10-07 End Game? popup: the crosshair no longer shows through it (BO2 darkens the HUD under its menus)
- 2026-10-07 In-match score panel as in BO2 (its border, hex pattern and glow at the real brightness); End Game? popup solid, no stray "Killed By"; Options list where BO2 puts it
- 2026-10-06 Options, Quit and End Game? as in BO2: the menu or match stays visible behind them, blurred and dimmed (they sat on black)
- 2026-10-06 Esc-menu map as in BO2: teammates are green arrows, enemies show as red dots only after they fire or under UAV; the Hybrid Optic swap keeps you aimed
- 2026-10-06 End of match as in BO2: VICTORY/DEFEAT title, team icons and scores at the real size and place; scoreboard Ratio prints two decimals (1.20) and Ping reads like the real game
- 2026-10-06 Pause menu as in BO2: the world behind keeps its colour (it went grey); the perk list on spawn sits where BO2 puts it
- 2026-10-06 In-match Choose Class as in BO2: your current class dims and pulses, text and pictures line up with the real screen
- 2026-10-06 End scoreboard as in BO2: back in full colour, skull, rank and Ping on every row, winning team on top; the match-end screen stays at your own view
- 2026-10-06 Hybrid Optic: tap Shift while aiming to swap between the red-dot window and the magnified scope
- 2026-10-06 The end of a match always reaches the end scoreboard (it could hang with no kills); grey end-of-match world; the scoreboard opens by itself at the end
- 2026-10-06 Bots wear 1000 PS3-era lobby gamertags (some with clan tags), a fresh mix each time the game opens
- 2026-10-06 BO2's final killcam when a match ends on a kill (title, bands, YOU marker, killer card)
- 2026-10-06 BO2's End Game and quit popups (panel shade and menu colours as in BO2); the pause menu's orange text
- 2026-10-06 BO2's Killed-By card when you die, the YOU marker in the killcam, the default chevron on player cards
- 2026-10-06 BO2's lobby countdown on one line, the loading screen's "Awaiting challenge", the mode name in the pause menu
- 2026-10-06 BO2's HUD corners (score panel, streak bar, weapon strip, mirrored gun panels), the killcam, ACOG and Dual Band sights
- 2026-10-06 Create-a-Class polish: locked camos stay locked in custom games (Gold and Diamond hidden as in BO2), mouse icons on the class tiles, the world behind in-match menus is blurred, grey and dim
- 2026-10-06 Create-a-Class as in BO2: weapon, attachment, perk, equipment and wildcard pickers with their pictures, Personalize Weapon and Knife, rank-gated unlocks; Barracks opens; lobby footer and STORE; BO2's game-mode order and defaults (Aftermath, 10 minutes, 75 kills); two quick key presses no longer count as one
- 2026-10-06 BO2's sniper scope (overlay, hold breath on Shift) and sights on every gun; Create-a-Class class sets; BO2's lobby backdrop and loading screen; match rules and bots (Regular) start at BO2's defaults
- 2026-10-05 The keyboard and mouse work BO2's main menu and lobbies (before, every key was ignored there); bots default to Regular
- 2026-10-05 The knife kills in one hit, as in BO2; ranks, XP and rank-ups; Combat Training and the After Action Report; K9 Unit and AGR fixes; impact, jump, dive and body-fall sounds
- 2026-10-05 Your own Create-a-Class classes in matches; back to the Custom Games lobby after a match; ridden vehicle streaks (RC-XD, VTOL gunner)
- 2026-10-05 Kill feed icons, score popups, BO2's damage feedback; Care Package and more streaks; 6 renderer fixes across all 31 maps; ZOMBIES on the main menu starts Nuketown Zombies
- 2026-10-05 Other players drawn as BO2's soldiers with BO2's animations; scoreboard, kill feed, minimap; scorestreaks
- 2026-10-04 Guns on the ground and Scavenger bags; BO2's water, foliage, lava and seas; the killcam
- 2026-10-04 Every mode BO2 lets bots into plays against bots; BO2's class menu, HUD and pause menu in the match
- 2026-10-04 Team Deathmatch against BO2's own bots on Nuketown 2025
- 2026-10-04 All 31 multiplayer maps load with their own factions; BO2's colour grading from each map's vision file
- 2026-10-04 BO2's multiplayer front end: lobbies, Create-a-Class, Setup Bots, Start Match
- 2026-10-04 A multiplayer map loads from BO2's multiplayer zones; a compiler for BO2 script source

## Credits and license

- [IW4L](https://github.com/vladtrc/iw4L) by vladtrc and its contributors: the engine this fork is built on,
  Apache License 2.0.
- For Black Ops II, OpenAssetTools and [gsc-tool](https://github.com/xensik/gsc-tool) (both GPL-3.0) were read for
  format facts only; none of their code is copied.
- The Portal Gun's blocky model and texture are copied from the Portal Gun mod by MeowMC
  ([iPortalTeam/PortalGun](https://github.com/iPortalTeam/PortalGun)), MIT License; see [`NOTICE`](NOTICE).
- Call of Duty, Black Ops and their assets, trademarks and intellectual property belong to their owners
  (Activision). No game files are included.

Licensed under the [Apache License 2.0](LICENSE), like upstream; [`NOTICE`](NOTICE) holds the copyright notices.
The license covers this source code only.
