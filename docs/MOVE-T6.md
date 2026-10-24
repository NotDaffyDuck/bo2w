# `movement_t6`: Black Ops II's own movement

Every game runs `movement_t6::pmove` (the old MW2 frame, `movement_iw4::pmove`,
was deleted 10-20). Written from BO2's own order
and numbers (the server's symbols, read as facts); nothing emulated or run.
Shared pieces live in `movement_t6::bridge` (the old `movement_iw4` crate,
folded in 10-21); they move to BO2's own modules next.

| piece | file | BO2 decisions kept |
|---|---|---|
| catch-up | `frame.rs` | a late command catches up ≤150 ms, in moves ≤66 ms; each move then clamps 1–200 ms |
| collision | `gjk/` | BO2's own brush collision when the map carries corner points, box traces otherwise |
| stance | `stance.rs` | standing box for every get-up check; dead bodies pass through players |
| timers | `frame.rs` | DropTimers in BO2's order |
| view | `view.rs` | roll never follows; dead hold still unless the link lets them look; vectoyaw 0–360 in double precision |
| footsteps | `footstep.rs` | BO2's bob cycle and stance speeds |
| slope | `slope.rs`, `footstep.rs`; sim `t6_playeranim.rs`, `t6/playeranim.rs` | PM_CalcPlayerPitch: the ground's pitch along his facing (up the slope within 35 degrees of the fall line, down it past 145, else 0; in the air 0), on BO2's own vectoangles (straight up is pitch 270; game_math has it the other way). PM_Footsteps averages it over 10 frames in a history BO2 clears for every command; `BodyAnims.average_pitch` carries the last command's average to the sim, which holds it to the next command and gives the legs' `slope` condition `up` over 30 and `down` under -30 (BG_LocalEvalSlope). A command runs at most 3 moves, so the average stays under 27 and BO2's slope legs never play at its settings; ours follow |
| foliage | `foliage.rs`; audio `entity_events.rs` | PM_FoliageSounds, ported 10-17: from 40 units/s a box 0.75 wide and 0.9 tall of his tests foliage (contents 0x2: foliage clip, moss, ivy, lily pads); inside it, EV_FOLIAGE_SOUND every 1.5 s at 40 down to 0.5 s from 180 (`foliage_sound_time`, replicated); slower than 40 for 0.5 s clears the clock. Main move only (not linked, turret or climbing). The client plays fly_movement_foliage_plr for him, _npc at the others |
| ladder | `ladder.rs` | perk bit 17 (mapped in the sim); the detonator lets go; dead bodies use the air move; `right` not renormalised; side damping sign kept; movement dir ±75 with float truncation |
| ledge climb | `mantle.rs` | see below |
| end of move | `snap.rs` | blocked → velocity = what was moved; then **cut toward zero** (MW2 rounds) |
| ADS | `frame.rs` | the flag before sprint (BO2's order, shared `update_ads_intent` with `bo2`); the sights' lerp is the gun brain's (`weapon_t6` after the move), as BO2's sits in PM_Weapon. Gun-side gaps (dive 0x31–0x33, vehicle/link, exit delay, attachment scales) wait for the gun work |
| gun states | `gun.rs` | with BO2's gun brain running, `PlayerState::weapon_state_t6` holds its own state (sent on the wire; `sim/src/t6_gun.rs` fills it) and sprint, ladder, prone and climb rules read BO2's numbers: the knife, throws and the riot shield (19–34) stop a sprint, night vision 42/43, the whole reload 11–18. Without it (−1) they read the IW4 state |
| crawl | `feel.rs`, `gun.rs` | PM_InteruptWeaponWithProneMove's table: a busy gun blocks the crawl; ready, raising, dropping, rechambering, reloading or crawling goes ahead; anything else (the sprint raise) puts the gun to rest: movement sets BO2's state to ready and the gun brain idles itself before its aim flag (`T6Env::crawl_idle`, BO2's PM_Weapon_Idle) |
| moving platforms | `mover.rs` | Pmove's mover step and BG_AdjustPositionForMover: the thing the player stands on (an entity past the 18 players), the ladder they hold or the ledge they climb carries them and turns their view by however far it moved since the last command; carried 2 s after stepping or jumping off; pitch scale 0. The sim names a brush model's mover in its traces (`clip_move_to_bmodels`, the GJK bodies) and gives its pose on its path (`mover_pose`). BO2's server never pushes a player off a mover, so nothing else carries one |
| water | `water.rs`, `stance.rs`, `friction.rs`, `sprint.rs`, `ladder.rs`, `crash.rs`, `footstep.rs` | CM_GetWaterHeight: a point from 200 over the feet to 200 under that only water (0x20) stops; the level (0–6) out of the 60 eye height (3, 24, 38, 45, 54, 60), set after the view each frame; 1 splashes footsteps, 2 grips 1.7 times, stops sprint and prone, 3 stops crouch, 4 swims (PM_SwimMove: along the view, 1.3 times walking, acceleration 2, no gravity, floats up at half of 20), 6 is under; a soft landing in water. The sim finds the surface on BO2 collision (`step.rs` `water_height`) and hands the level to the gun brain (`T6Env::waterlevel`: no fire, reload, throw, shield or melee where BO2 has none) and the body anims. The water's push (CM_GetWaterForce) comes only from `undertow_volume`, which no MP map has: not needed. A swimmer's head bobs 3 by 2 and gun 4 by 4 at any speed (`weapon_iw4::view_bob`, `weapon_t6::recoil`); the gun-angle bob still reads the head bob tables, where BO2 reads the gun's. A jump out of any water splashes (Jump_AddSurfaceEvent, `walk.rs`). Breath (Player_WaterUpdate, sim `step.rs` `drown_players`): from level 5 a player holds it 16 s, then takes 15 (MOD_DROWN, no armour, no knockback) each second until they come up |
| prone | `prone_turn.rs`, `check_prone.rs` | the gun in hand can forbid it (BG_WeaponBlocksProne, the WeaponDef's blocksProne; in MP the riot shield and the Death Machine): no prone, no dive, no sprint-to-prone (`sim/src/step.rs` sets `weapon_blocks_prone`; the script asks with `weaponblocksprone`); BO2's slot after the ledge check; the waist trace always looks down 2.5 widths (55.5), in the air too; the prone-blocked flag (0x1000 clashes with our ladder bit) not ported; knee-deep water (level 2 up) turns a prone player's lie-down into a crouch; the gun's prone lock (PM_SetProneMovementOverride) reaches the crawl: firing, meleeing, throwing or switching guns while prone holds the player still until the gun idles or the keys ease (`sim/src/t6_gun.rs` `share_prone_override`, test `the_prone_override_crosses_only_under_bo2_movement`) |

## Ledge climb (mantle)
- Surface bits translated from T6 (on 0x0400_0000 → 0x0200_0000, over 0x0800_0000 → 0x0400_0000).
- The two ledge sweeps use mask `0xfdff_7fff` (no bodies); the room check uses the full mask.
- Room to stand: a standing box (±15, 0–70) at the ledge (stuck → force crouch) and at the end (free → force stand).
- "Climb onto" uses our private bit `1<<11`; a vault with no room past the ledge becomes an onto-climb and keeps its speed.
- MP: an onto-climb stops dead at the top. Zombies keep the speed.
- Fast climb (perk) uses `from_t6_clips`; the weapon hold (SetHaveWeapon) is compiled out on BO2's server.
- Wall angle: `acos` unclamped (NaN fails) × 57.295776, ≤60°. Facing yaw from BO2's vectoyaw.

## Server player update (ClientThink_real), checked 10-17
- Player mask: BO2's is ours plus 0x8000, the AI-actor bit (SP_actor gives actors that
  contents). No Nuketown brush has it (`t6walk` counts them: 0 of 9,284), and our actors
  already block players as bodies, so the mask is left as it is.
- The serverTime clamp (level time −1000 … +200 ms): not ported on purpose. Our client
  runs ahead of the server, and the clamp would slow laggy players.
- `doubletap_detonate` is sent by the gun side before the command's events
  (`sim/src/t6_gun.rs`, docs/gun/OFFHAND.md).
- Mantle blockage (G_AddPlayerMantleBlockage), ported 10-17: a climb that starts holds the
  spot it ends on (the climb's end position, from its root track) as a player-clip box the
  climber's size; his own traces pass through it, the others' stop, and stepping onto it is
  allowed (it is not a player). It goes `g_mantleBlockTimeBuffer` (500 ms) after the climb
  ends. Scripts get `mantle_start` with the climb's end time (MP's _challenges.gsc waits for
  it). Ours: `sim/src/step.rs` (`mantle_landing`, the body list), `sim/src/world.rs`
  (`MantleBlocker`), `sim/src/t6/players.rs`. Test: `a_climbers_landing_stops_the_others_not_him`.
- A frozen player (`freezecontrols`) is stopped dead every frame, in the air too (PmoveSingle's
  PM_ClearVelocityForPmType; `sim/src/step.rs`). BO2's weapon freeze (freezeMovementWhenFiring) is set
  on none of MP's 173 guns: not needed.
- The climb's window check (inside Mantle_CheckLedge): after the fit at the top, the forward
  sweep runs again from the same start with the same box and mask, 26 units ahead
  (`mantle_check_glass_extra_range` 10 past the forward 16); a hit on an unbroken window (hit type
  4) refuses the climb. Ours: `mantle.rs` `check_ledge`. Test:
  `a_window_just_past_the_ledge_refuses_the_climb`. The windows themselves are the `maps` branch's
  (`attach_t6_glass_brushes`); on move/t6 alone there is no glass for it to find.
- Movers at each command's time (AdjustMovers/RestoreMovers): BO2 re-poses only AI actors' movers
  (Actor_Think fills the list), so in MP the list is always empty. The platform a player rides
  carries him by its pose change between command times (`mover.rs`, BG_AdjustPositionForMover).
- His box for triggers and `istouching` (ClientThink_real copies the move's box onto him), 10-17: its top
  follows his stance, 70 standing, 50 crouched or in last stand, 30 prone (`sim/src/t6/triggers.rs`
  `player_box`, the same heights as `item.rs` `stance_maxs_z`). It was always 70, so a crouched or prone
  player set off a trigger hanging above him.
- pm_flags 0x100 is BO2's knockback timer (PMF_TIME_KNOCKBACK): no ground grip, walking acceleration 1.0,
  gravity on the ground. Our walk code called it slow ground; the behaviour was already BO2's, only the
  name changed (`walk.rs`, `feel.rs` `KNOCKBACK_ACCEL`).
- Surface type: BO2 reads it as (surface flags >> 20) & 0x3f everywhere (movement, guns, sounds); its
  0x1f masks are weapon attachment masks. Our T6 footstep and dive sounds now mask 0x3f too
  (`render_anim` `t6_sounds.rs`); types 32 and up play the default sound.

## Not ported (BO2 has, we don't)
Nothing left in play (settled 10-17).
- Ported since: the legs' jump and land (ANIM_ET_JUMP / ANIM_ET_LAND, `frame.rs` `BodyAnims` to the
  sim's `t6/playeranim.rs` `note_body_anims`): a jump, a jump off a ladder, losing a ladder in the air
  and walking off an edge play the jump; a fall of over 12 units, not out of a dive, plays the land.
  The second sprint and duck pass on a climb's first frame. The extra footsteps: crouched steps
  (crouch run at speed 110 and up while striding, else crouch walk) and the ladder's hand rungs
  (foot and hand take turns in the air, metal), BO2's own events playing `fly_step_crouch_run`,
  `fly_step_crouch_walk` and `fly_step_ladder_hand`; an IW4 sound bank plays the run and walk steps
  instead. ClientThink's `doubletap_detonate` (sim `t6_gun.rs`) and `mantle_start` with the climb's
  end time (sim `t6/players.rs`), and the climb's landing blockage (sim `world.rs` `MantleBlocker`).
- The trace split is BO2's: the box trace (`PM_trace`) for foliage, ladders, the dive's friction,
  climbs and planting the shield; the body sweep (`PM_playerTrace`, GJK) for the ground, slide and
  step, the penetration checks and the all-solid fix. `BO2MP_GJK=0` puts all of it on the box. One
  difference kept on purpose: a box trace that starts inside a body is traced again without bodies
  in BO2; ours keeps the bo2zm rule (a stuck player may move away from a body, not deeper).
- Not needed: Hatch_Check (no hatch-marked geometry in the MP maps), Mount_CheckLedge and
  Mount_CheckProne (no MP weapon is mountable), the console branch (Com_IsClientConsole is true only
  on PS3, Xbox 360 and Wii U; a PC identity is "pc", the default).
- mantleEndPos and mantleDuration are not pmove fields; the sim works both out from the climb's root
  track.
A command older than the player's last runs once at 1 ms here (the sim never
sends one); BO2 skips it.
Weapon-granted perks are on the gun side (in MP only FMJ grants armor piercing and bullet
penetration); movement reads neither.

## Checks
`cargo test -p movement_t6` (unit tests per piece). `t6walk [zone.ff] [frames]`
walks every spawn on BO2 collision and box traces headless. 10-16, Nuketown 2025, 44 spawns × 4 headings × 300 moves:
every walk stays in the map; 2 of 176 end mid-drop off a ledge; 71 end within 16
units of the box-trace walk, and the biggest gaps are box walks stuck at the spawn.
