# bo2w: Black Ops II on its own engine

bo2w is a fan rebuild of Call of Duty: Black Ops II for PC: **Multiplayer against bots** and **Zombies**, played from
your own copy of the game, on a new engine written in Rust (on [bevy](https://bevyengine.org/) and
[wgpu](https://wgpu.rs/)). The game's own scripts, menus and bots run inside it, so matches follow Black Ops II's real
rules without touching the original executable.

**No game files are included.** This repository holds source code only: no maps, models, textures, sounds, scripts,
videos or executables from Black Ops II (or any other game), and no pictures of them. You need your own copy of Black
Ops II for PC; bo2w reads its files read-only and never patches, copies or redistributes them. This is an unofficial
fan project, not affiliated with or endorsed by Activision, Treyarch or Infinity Ward.

**Last updated:** 2026-10-23

## Download

| What | Branch | Download |
|---|---|---|
| Engine + Multiplayer (newest) | `main` | [**bo2w-main.zip**](https://gitlab.com/coljax-group/bo2w/-/archive/main/bo2w-main.zip) |
| Zombies (its last build, before the merge) | `zombies` | [**bo2w-zombies.zip**](https://gitlab.com/coljax-group/bo2w/-/archive/zombies/bo2w-zombies.zip) |

Or with git:

```powershell
git clone https://gitlab.com/coljax-group/bo2w.git
cd bo2w
git checkout zombies   # only for the Zombies branch
```

There is no ready-made .exe yet: the first one comes with the finished engine (see the plan below). Until then you
build it yourself, which takes four steps.

## Build and play (Windows)

1. Install **Rust** from [rustup.rs](https://rustup.rs/) and the **Visual Studio 2022 Build Tools** with the
   "Desktop development with C++" workload.
2. Unzip the download (or clone it), open PowerShell in that folder and build:

   ```powershell
   cargo build -p launcher --profile play
   ```

   The first build takes 10-20 minutes and needs tens of GB free. The game is in `target\play\`.
3. Make a play folder anywhere, copy the game .exe from `target\play\` into it, and put a file named `.env` next to it
   that points at your Black Ops II folder (quoted, forward slashes):

   ```text
   BO2MP_GAMES="C:/Program Files (x86)/Steam/steamapps/common/Call of Duty Black Ops II"
   ```

   On the `zombies` branch the line starts with `IW4L_GAMES=` instead.
4. Run it from the play folder:

   ```powershell
   .\bo2mp.exe frontend t6mp          # Multiplayer (main)
   .\iw4l.exe frontend t6:zm_nuked    # Zombies (zombies branch)
   ```

Full details for each part: [MULTIPLAYER.md](MULTIPLAYER.md) (this branch) and `ZOMBIES.md` (on the `zombies`
branch). Needs a DirectX 12 or Vulkan GPU; built and played on Windows 11 x64 only so far.

## Where it stands (2026-10-23)

### The engine

The project started as a fork of [IW4L](https://github.com/vladtrc/iw4L), an open-source Modern Warfare 2 runtime
(Apache 2.0, credited in [NOTICE](NOTICE)). Now it is becoming fully its own engine:

- **Done (10-21):** every piece of IW4L's Modern Warfare 2 engine is out: all 20 MW2 crates deleted (about 82,000
  lines), and no IW4/IW5 or IW4L name is left in the code, build files or file names.
- **This week:** each crate that still holds code first written for IW4L is being written fresh from what it has
  to do (its tests, its callers, Black Ops II's own rules), crate by crate. At the start (10-21) about 42% of the
  lines came from IW4L. The first rewrites are merged, and most of the other crates are rewritten and in testing:
  network, front end and HUD, simulation, game scripts, rendering, tools, assets, and effects and audio.
- **Every merge must keep the game playing exactly as before:** about 2,500 tests pass, and four hidden test matches
  in different modes, a three-map check and a bots check all pass.

### Multiplayer (`main`)

- Black Ops II's own main menu, custom games lobby, Create-a-Class, scorestreak picks and Public Match map vote.
- **All 31 maps** (disc and DLC you own) load and play. Mechanics are about 96% done across all maps, and visuals
  are about 35% compared side by side with the real game.
- BO2's own match scripts and bots play Team Deathmatch, Free-for-All, Domination, Kill Confirmed, Hardpoint,
  Capture the Flag, Demolition, Headquarters and Search and Destroy.
- BO2's movement and gun handling are rebuilt from the game's own rules: sliding, diving, mantling, ladders, water,
  attachments and gun levels.
- HUD, minimap, kill feed, scoreboard, killcam and the end-of-match screen.
- **Scorestreaks:** the UAV, RC-XD, Hunter Killer and Care Package are play-tested. The rest are in, and each is
  still being checked against the real game.

Map-by-map bars and the full change list: [MULTIPLAYER.md](MULTIPLAYER.md).

### Zombies (`zombies`)

- Every Black Ops II Zombies map loads from BO2's globe: TranZit (Bus Depot, Town, Farm), Nuketown, Die Rise, Mob of
  the Dead, Buried and Origins. So do the nine Black Ops maps from the map pack.
- Nuketown and TranZit are at 100% mechanics. Die Rise is at 98%, Mob of the Dead at 97%, and Buried and Origins at 94%.
- BO2's own zombie scripts run it all: rounds, wall buys, the Mystery Box, perks, Pack-a-Punch, power-ups, going
  down and reviving. Brutus, the Panzer Soldat, Cerberus, Afterlife and the TranZit bus work too.
- **Being merged into the new engine right now**, so Multiplayer and Zombies end up as one game on one engine. The
  `zombies` branch is its last stand-alone build. It still carries the older IW4L-based engine underneath.

## The plan: next few days to a week

1. **Finish the engine (now to ~10-26):** the last crates rewritten fresh and merged. The goal is zero lines left
   from IW4L, and the game plays exactly as before.
2. **Zombies on the new engine (in parallel, ~10-26 to 10-30):** Zombies merged in crate by crate as each part of
   the engine is done. Then one combined build of Multiplayer and Zombies, switched from BO2's main menu. That
   build is the first downloadable .exe.
3. **Polish (the week after):**
   - **Scorestreaks:** every remaining one checked side by side with BO2 and play-tested one at a time:
     Counter-UAV, Care Package's drone blades, K9 Unit, Stealth Chopper, AGR, VTOL Warship, Dragonfire, Escort
     Drone, Sentry Gun, Guardian, Hellstorm (and its minimap marks), Lightning Strike's tablet, Swarm, EMP and
     Orbital VSAT. After that, two scorestreaks at once and a key of your own for each.
   - **Movement bugs:** getting up from a dive at BO2's speed and diving over low objects, the mantle pull-up
     speed, ledge and window climbs on every map, the water slowdown, aiming speed with each attachment, and
     players never spawning inside a wall.
   - **Bots:** no getting stuck on corners, props or invisible walls, never standing facing a wall, and climbing
     ledges, mantles and windows as BO2's bots do.
   - **Combat feel:** hit markers and BO2's hit and headshot sounds, a true killcam replay, ragdolls that fall
     without spinning, attachments at their exact place on the gun, and the riot shield's texture on your back.
   - **Maps:** the remaining lighting and sky fixes (Takeoff's shuttle, Carrier's helicopter, Drone's floor,
     Cargo's sky), Express's train shake, Vertigo's doors, and side-by-side visuals for every map.
   - **Zombies:** boarded windows, box guns' magazines, the TranZit bus route and climbing on, Die Rise's lift,
     Mob of the Dead's knife through windows, Origins' planes and props, and each map's quests.
   - **Performance:** no slowdown late in a match.
4. **Later:** playing together over LAN (Multiplayer, and Zombies co-op for 2-4 players).

The full "coming soon" lists are in [MULTIPLAYER.md](MULTIPLAYER.md#coming-soon) and in `ZOMBIES.md` on the
`zombies` branch.

## Credits and licence

- Licensed under the Apache License 2.0 ([LICENSE](LICENSE)). It started from
  [IW4L](https://github.com/vladtrc/iw4L) (Apache 2.0); [NOTICE](NOTICE) keeps its credit and lists every other
  project read or used.
- Bundled third-party code keeps its own licence: a patched wgpu-hal in `third_party/` (MIT / Apache 2.0), and two
  fonts compiled into the game: Oxanium (SIL Open Font License) and GNU FreeMono (GPL with the font exception).
- Call of Duty and Black Ops II are trademarks of Activision. This project uses no Activision code or files and
  needs your own copy of the game.
