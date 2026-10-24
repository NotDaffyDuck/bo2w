# Controls: Black Ops II's client input

The client turns keys, mouse and pad into one command a frame:
`console` (devices) → `net::client::input::build_usercmd` → `input_t6::create_cmd`.
There is no mode switch on this path: every movement mode gets these rules.
Written from BO2's own order and numbers (read as facts); nothing emulated or run.

| piece | where | BO2 rule kept |
|---|---|---|
| stance button | `input_t6::stance_button` | held 300 ms → prone; sprinting, held 200 ms → dive; a tap crouches on release, unless the gun in hand blocks prone (BG_WeaponBlocksProne; in MP the riot shield and the Death Machine), when the stance stays. The gun comes from the last snapshot each frame (`net::client::runtime`) |
| Use/Reload double tap | `input_t6::UseTap` | two taps of Use, Reload or Use+Reload, each let go within 500 ms of its press and pressed within 500 ms of the last, send the double-tap button on the next command (cl_dblTapMaxHoldTime, cl_dblTapMaxDelayTime); the server turns it into `doubletap_detonate` (docs/gun/OFFHAND.md) |
| toggles | `input_t6::input_cmd` | toggle crouch, toggle prone, toggle ADS |
| switch weapon | `input_t6::weapnext_hold` | a tap is the next weapon on release; held 250 ms takes the inventory weapon |
| inventory weapon asked for | `net::client::runtime` | BO2's CG_WeaponSwitchRequest: never mid-throw (states 22-29). When the inventory weapon is already up, the press goes back to the last gun he held (CG_SelectWeapon remembers a primary, or an alt mode's base gun, as he leaves it), else on to the next gun; in a vehicle it does nothing |
| attachment key | `sim::t6_gun::alt_toggle_allowed_by_state`, `net::client::runtime` | BO2's CG_AltToggleAllowedByState: no attachment switch while the gun is going down or switching modes, mid-knife or mid-throw (states 2-5, 19-29), on a charge-shot gun, or mid-climb |
| pad move | `input_t6::pad_move_stretched` | the round stick stretched onto the square (127·√(1+ratio²)), cut toward zero, added to the key move, then held to −128…127 (ClampChar). A full diagonal is 127 on both axes, so it runs at full speed (it was 71%) |
| pad look | `net::client::pad_aim` | the aim_* turn rates, acceleration, slowdown and lerp |
| pitch per frame | `input_t6::create_cmd` | the mouse and pad move the pitch at most 90° (16384 packed) in one frame |
| mouse speed | `net::client::input` | cl_mouseAccel reads the distance moved (√(dx²+dy²)) per frame ms, not the sum of both axes. Only matters when cl_mouseAccel is not 0 (default 0) |
| semi-auto jam roll | `input_t6::attack_jam_roll` | fire key down on a single-shot gun within its `iJamFireTime` of the last press rolls 60000/dt − 60000/jam percent; a hit holds the JAM bit (0x1000000) until the fire key goes up. The gun's window comes from the predicted world each frame (docs/gun/FIRE.md 3.2a) |
| hit kick in the aim | `render_anim::occupancy::view_kick::hit_aim_pitch` | `bg_useClientDamageKick`: being hit turns the shot as it turns the view. The camera's hit kick always goes into the aim; aimed through iron sights, the gun model's hit tilt too (pitch only, as BO2's server drops its yaw). The camera stops adding what the aim now carries, so the picture is unchanged |
| aim key vs change zoom | `sim::t6_gun::gun_buttons` | our aim key's wire bit (THROW, 0x80000) is BO2's change-zoom bit, so a variable-zoom scope stepped its zoom on every aim-in; the sim now drops THROW before the gun reads the buttons |
| trigger press point | `console::gamepad::bo2_trigger_thresholds` | a pad trigger counts as held once pulled past BO2's gpad_button_deadzone (0.13 of the way, 34 of 255), and let go at or under it. The pad library's own default waited for 75% |
| frozen or planting stops the turn | `net::client::runtime` | CL_MouseMove and CL_GamepadMove: a frozen player (pm_flags 0x800 or the match freeze) and a riot shield being planted (weapon state 44, DEPLOYING) get no mouse turn, no pad turn and no pad move |

## Kept different on purpose
- A stance tap whose sprint ended before the release still crouches; BO2 drops that tap.
- The key move rounds forward − back once; BO2 cuts each key and adds them (at most 1 apart).
- Keyboard look (`accumulate_look`) turns before the command is built, so it sits outside
  the 90° limit; it moves a few degrees a frame at most.
- The hit kick in the aim is timed by the client's clock and reaches the command a frame late (queued, as
  the gun kick is); BO2's server times its camera part by its own clock. The gun model's hit tilt is added to
  the pitch, which is what BO2's axis product gives with no roll in the view. It rides in the view angles, as the
  gun kick does, so others see the flinch on the body's aim; BO2 adds it to the shot only.
- The jam roll times presses with the frame clock, not the OS clock, so two fire presses in
  one frame (dt 0) never jam. BO2's own dt 0 also never jams; only clicks under one frame
  apart differ.

## Not ported (BO2 has, we don't)
- No key sends change zoom (BO2's zoom-step bit). The scope's variable zoom is stepped on the
  melee key while scoped (`view_kick.rs`), which the sim never reads as change zoom.
- BO2 also zeroes the pad move when the player state's otherFlags carry 0x400. What sets that
  flag was not found in BO2's MP code, and our server never sets it, so nothing reads it.
- The analog fire bit (0x80000000, sent when the right trigger passes cl_analog_attack_threshold):
  its readers in BO2's movement and weapon code were found, but not what they change for a
  player or the threshold's default. Our fire comes from the trigger as a button (above).
- m_filter (averaging this frame's mouse move with the last one; off by default) is not ported.

## Checks
`cargo test -p input_t6` (`bo2_stance_tests`, `bo2_weapnext_tests`,
`bo2_pad_and_look_tests`, `bo2_jam_tests`); `cargo test -p sim --lib t6_gun`
(`the_aim_key_never_reads_as_change_zoom`); `cargo test -p render_anim --lib hit_aim_tests`;
`cargo test -p console --lib gamepad` (`a_trigger_counts_once_pulled_past_bo2s_dead_zone`).
