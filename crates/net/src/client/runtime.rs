use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use bevy::prelude::*;
use frame::{HasWorld, MatchTornDown, RuntimeRole};
use playerstate_t6::game::UserCmd;

use crate::ServerTime;
use crate::authority::inbox::{
    AUTHORITY_MS, ClientActionInbox, ClientCommandInbox, MAX_REDUNDANT_CMDS,
};
use crate::authority::runtime::{AuthorityInputGate, AuthorityWorld, ClientShotSamples};
use crate::client::frame_clock::{FrameClock, GameActive, GameJoinCensus};
use crate::client::input::{
    ClientActionInput, LookState, accumulate_look, build_usercmd, frame_time_msec, key_frame_msec,
    remote_control_axes,
};
use crate::client::predict::{ClientPrediction, CmdSeq};
use crate::client::presentation::entities::CEntityBirthCensus;
use crate::client::presentation::presented::{
    LocalPresentClient, PresentLocalCensus, PresentedSnapshot, interpolate_player_state,
};
use crate::client::presentation::projectiles::merge_presented_projectiles;
use crate::client::proxy::{ProxySample, RemoteProxy};
use crate::client::realtime::ClientRealtime;
use crate::schedule::ClientSet;
use crate::transport::loopback_live::{ListenLoopback, ReceivedTick};
use sim::Snapshot;

#[derive(Resource)]
pub struct ClientPredictionState(pub ClientPrediction);

#[derive(Resource, Default)]
pub struct PendingPresentedEntityEvents {
    pub live: Vec<sim::EntityEventRecord>,

    pub archived: Vec<sim::EntityEventRecord>,

    pub in_killcam: bool,
}

#[derive(Resource, Default)]
pub struct PendingPelletFx(pub Vec<sim::PelletFxRecord>);

/// bo2mp: broken-off pieces the server sent that this PC has not spawned
/// yet.
#[derive(Resource, Default)]
pub struct PendingDebris(pub Vec<sim::DebrisRecord>);

fn collect_received_entity_events(
    last_tick: Option<sim::Tick>,
    local: sim::ClientId,
    snapshot: &mut Snapshot,
    pending: &mut PendingPresentedEntityEvents,
) -> bool {
    if crate::classify_snapshot(last_tick, snapshot.tick) != crate::SnapshotOrder::New {
        return false;
    }
    pending.in_killcam = snapshot
        .meta
        .for_client(local)
        .is_some_and(|m| m.killcam_hud.is_some());
    if pending.in_killcam {
        pending.archived.append(&mut snapshot.meta.entity_events);
    } else {
        pending.live.append(&mut snapshot.meta.entity_events);
    }
    true
}

impl Default for ClientPredictionState {
    fn default() -> Self {
        Self(ClientPrediction::new(sim::ClientId(0)))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClockTick {
    pub released: u32,

    pub dropped: u32,
}

#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct ClientClock {
    pub accumulator_ms: f32,

    pub dropped_slots: u32,
}

impl ClientClock {
    pub fn tick(&mut self, delta_ms: f32) -> ClockTick {
        let before = self.accumulator_ms;
        self.accumulator_ms += delta_ms;
        if self.accumulator_ms < AUTHORITY_MS as f32 {
            return ClockTick {
                released: 0,
                dropped: 0,
            };
        }
        let slots = (self.accumulator_ms / AUTHORITY_MS as f32).floor() as u32;
        let dropped = slots.saturating_sub(1);
        self.dropped_slots = self.dropped_slots.wrapping_add(dropped);
        self.accumulator_ms %= AUTHORITY_MS as f32;

        if self.accumulator_ms >= AUTHORITY_MS as f32 - 0.0005 {
            self.accumulator_ms = 0.0;
        }
        debug_assert!(
            self.accumulator_ms >= 0.0 && self.accumulator_ms < AUTHORITY_MS as f32,
            "clock debt after tick: before={before} dt={delta_ms} after={}",
            self.accumulator_ms
        );
        ClockTick {
            released: 1,
            dropped,
        }
    }

    pub fn debt_ms(&self) -> f32 {
        self.accumulator_ms
    }
}

#[derive(Resource, Default)]
pub struct ReceivedTicks(pub VecDeque<ReceivedTick>);

#[derive(Resource, Default, Debug)]
pub struct LastAdoptedSnapshot {
    pub snap: Option<Arc<Snapshot>>,
    pub next_snap: Option<Arc<Snapshot>>,

    pub applied_this_frame: bool,
}

impl LastAdoptedSnapshot {
    pub fn next(&self) -> Option<&Snapshot> {
        self.next_snap.as_deref()
    }

    pub fn sound_alias_name(&self, index: u8) -> Option<&str> {
        for snap in [self.next_snap.as_ref(), self.snap.as_ref()]
            .into_iter()
            .flatten()
        {
            if let Some((_, name)) = snap.meta.sound_aliases.iter().find(|(i, _)| *i == index) {
                return Some(name.as_str());
            }
        }
        None
    }

    pub fn effect_name(&self, index: u8) -> Option<&str> {
        self.next().and_then(|snap| {
            snap.meta
                .effect_names
                .iter()
                .find(|(i, _)| *i == index)
                .map(|(_, name)| name.as_str())
        })
    }
}

#[derive(Resource, Default, Debug, Clone)]
pub struct PendingClientSends {
    cmds: VecDeque<(CmdSeq, UserCmd, sim::ShotSampleProvenance)>,

    evicted_unacked: u32,

    acknowledged: Option<CmdSeq>,
}

impl PendingClientSends {
    pub fn len(&self) -> usize {
        self.cmds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cmds.is_empty()
    }

    pub fn push(&mut self, seq: CmdSeq, cmd: UserCmd, sample: sim::ShotSampleProvenance) {
        if self.cmds.len() >= MAX_REDUNDANT_CMDS {
            self.evicted_unacked = self.evicted_unacked.saturating_add(1);
            return;
        }
        self.cmds.push_back((seq, cmd, sample));
    }

    pub fn evicted_unacked(&self) -> u32 {
        self.evicted_unacked
    }

    pub fn clear_acknowledged(&mut self, acked: CmdSeq) {
        self.acknowledged = Some(self.acknowledged.map_or(acked, |last| last.max(acked)));
        while self
            .cmds
            .front()
            .is_some_and(|(seq, _, _)| seq.0 <= acked.0)
        {
            self.cmds.pop_front();
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &(CmdSeq, UserCmd, sim::ShotSampleProvenance)> {
        self.cmds.iter()
    }

    pub fn has_command_capacity(&self, server_time: i32) -> bool {
        if self.len() >= MAX_REDUNDANT_CMDS {
            return false;
        }
        // Reserve one full step for the gap from the authority's last consumed
        // command. Bound unacknowledged work before allocating a sequence: after
        // a delayed ACK we can resume without dropping commands or leaving holes.
        let step = crate::authority::inbox::COMMAND_TIME_STEP_MS;
        let mut work = step;
        let mut previous: Option<i32> = None;
        for time in self
            .cmds
            .iter()
            .map(|(_, cmd, _)| cmd.server_time)
            .chain(std::iter::once(server_time))
        {
            if let Some(previous) = previous {
                work = work.saturating_add(time.saturating_sub(previous).clamp(0, step));
            }
            previous = Some(time);
        }
        work <= crate::authority::inbox::MAX_QUEUED_COMMAND_MS
    }

    pub fn oldest_seq(&self) -> Option<CmdSeq> {
        self.cmds.front().map(|(seq, _, _)| *seq)
    }

    pub fn newest_seq(&self) -> Option<CmdSeq> {
        self.cmds.back().map(|(seq, _, _)| *seq)
    }
}

#[derive(Resource, Default)]
pub struct ClientCmdTemplate {
    pub cmd: UserCmd,
    pub ready: bool,
}

#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeaponSelect {
    pub index: u32,

    pub time: i32,

    pub life_sequence: u32,

    pub held_index: u32,

    pub last_primary: u32,
    pub mapped_index: u32,
    /// bo2mp: Black Ops II's switchingToInventory: the inventory weapon was
    /// asked for and the player does not hold it yet.
    pub switching_to_inventory: bool,
    /// bo2mp: Black Ops II's lastWeaponForAlt: per `ps.weapons` slot, the alt
    /// mode (select fire, Hybrid's other optic, launcher) that gun was last
    /// left in, so the cycle brings it back up in that mode.
    pub last_mode: [u32; 15],
}

pub fn follow_held_weapon_select(
    select: &mut WeaponSelect,
    ps_weapon: u32,
    life_sequence: u32,
    cg_time: i32,
) {
    if ps_weapon == 0 {
        return;
    }
    if select.index == 0
        || select.life_sequence != life_sequence
        || (select.index == select.held_index && ps_weapon != select.held_index)
    {
        select.index = ps_weapon;
        select.mapped_index = ps_weapon;
        select.last_primary = 0;
        select.life_sequence = life_sequence;
        select.time = cg_time;
    }
    select.held_index = ps_weapon;
}

/// The player-state fields the weapon-select gate reads.
fn select_flags(ps: &playerstate_t6::game::PlayerState) -> input_t6::weapon_select::SelectFlags {
    input_t6::weapon_select::SelectFlags {
        pm_flags: ps.pm_flags,
        weap_flags: ps.weap_flags,
        other_flags: ps.other_flags,
        e_flags: ps.e_flags,
        pm_type: ps.pm_type,
    }
}

pub fn cycle_weapon_select(
    select: &mut WeaponSelect,
    input: &mut input_t6::ClientInput,
    ps: &playerstate_t6::game::PlayerState,
    world: &sim::SimWorld,
    time: i32,
    next: bool,
    t6_offhand: Option<(i32, u32)>,
) {
    use input_t6::weapon_select::{cycle_weapon, weapon_cycle_allowed};
    if !weapon_cycle_allowed(select_flags(ps), time, select.time, 0, 0) {
        diag::info!(
            Net,
            "bo2mp cycle: refused (pm {:#x} weap {:#x} other {:#x} e {:#x} type {}, {} ms since the last pick)",
            ps.pm_flags,
            ps.weap_flags,
            ps.other_flags,
            ps.e_flags,
            ps.pm_type,
            time.wrapping_sub(select.time)
        );
        return;
    }

    // bo2mp gun brain: BO2 asks before every cycle whether a cancelable grenade is in hand
    // (offhand init, prepare, hold, detonate, start or throw: raw states 22-27). If so the
    // press becomes the cancel button on the next command and the cycle is dropped. Both the
    // state and the grenade are this client's own predicted T6 ones.
    if let Some((state, offhand)) = t6_offhand {
        if (22..=27).contains(&state)
            && world
                .weapon_combat_row(offhand)
                .and_then(|f| f.offhand_hold_is_cancelable)
                .unwrap_or(false)
        {
            diag::info!(Net, "bo2mp cycle: grenade cancel (state {state}, grenade {offhand})");
            input.offhand_switch_t6 = true;
            return;
        }
    } else if matches!(ps.weapon_state_t6, 22..=24 | 28) {
        // No predicted T6 grenade: the player's own BO2 state (offhand init, prepare, hold).
        let offhand = ps.off_hand_index.max(0) as u32;
        let cancelable = world
            .weapon_combat_row(offhand)
            .unwrap_or_else(weapon_t6::combat_facts::WeaponCombatFacts::none)
            .offhand_hold_is_cancelable
            .unwrap_or(false);
        if cancelable {
            input.offhand_hold_cancel = true;
            return;
        }
    }
    select.time = time;
    if world.weapon_combat_len() < 2 {
        return;
    }
    let facts = |weapon| {
        world
            .weapon_combat_row(weapon)
            .expect("owned weapon must have a captured catalog row")
    };
    // bo2mp: Black Ops II's cycle from an alt mode goes back to its base gun,
    // unless the mode disables switching (then the walk moves on).
    let back = sim::alt_mode_cycle_base(world, select.index)
        .filter(|&base| ps.weapons.contains(&(base as i32)));
    // His scorestreaks are never in the weapon cycle (they come out on the
    // streak key alone), and nor is the knife (the melee key swings it).
    let walked = || {
        cycle_weapon(
            &ps.weapons,
            select.index,
            select.mapped_index,
            select.last_primary,
            next,
            |weapon| {
                if streak_slot_weapons(ps).any(|w| w == weapon) || sim::is_melee_knife(world, weapon) {
                    -1
                } else {
                    facts(weapon).inventory_type
                }
            },
        )
        // The walked-to gun comes up in the mode it was last left in.
        .map(|w| sim::remembered_mode(world, &ps.weapons, &select.last_mode, w))
    };
    let Some(target) = back.or_else(walked) else {
        diag::info!(Net, "bo2mp cycle: nothing to cycle to from {}", select.index);
        return;
    };
    diag::info!(Net, "bo2mp cycle: {} -> {target} (holding {})", select.index, ps.weapon);
    let target_facts = facts(target);
    let requires_ammo = target_facts.select_requires_ammo.unwrap_or(false);
    if requires_ammo {
        let ammo_key = weapon_t6::ammo_table::ammo_table_key(target_facts.ammo_index, target);
        let clip_key = weapon_t6::ammo_table::clip_table_key(target_facts.clip_index, target);
        if weapon_t6::ammo_table::get_ammo_player_both_clips(&ps.weapons, &ps.weapon_data, &ps.ammo, &ps.ammoclip, target, ammo_key, clip_key) == 0 {
            return;
        }
    }
    if select.index != target {
        if select.index != 0 && facts(select.index).inventory_type == 0 {
            select.last_primary = select.index;
        }
        // An alt mode's base gun is its mapped gun.
        select.mapped_index = if target_facts.inventory_type == 3 {
            target_facts.alternate_weapon
        } else {
            target
        };
        select.index = target;
        sim::remember_mode(world, &ps.weapons, &mut select.last_mode, target);
        input_t6::set_ads(input, false);
    }
}

/// His scorestreak weapons on the d-pad slots, cheapest first (BO2's
/// `_class` puts them on slots 4, 2, 1 by cost).
fn streak_slot_weapons(ps: &playerstate_t6::game::PlayerState) -> impl Iterator<Item = u32> + '_ {
    [3usize, 1, 0]
        .into_iter()
        .filter(|&i| ps.action_slot_type[i] == 1)
        .map(|i| ps.action_slot_param[i].max(0) as u32)
        .filter(|&w| w != 0)
}

/// bo2mp, his 10-13 ask ("It should only be one key for all the skill
/// streaks. You use the lowest one first"): a streak key brings out the
/// cheapest streak he has earned; pressed again with one up, the next earned
/// one. BO2 gives every streak weapon at spawn with no ammo: one he has not
/// earned (empty) never comes out.
fn next_earned_streak(ps: &playerstate_t6::game::PlayerState, world: &sim::SimWorld, held: u32) -> Option<u32> {
    let earned: Vec<u32> = streak_slot_weapons(ps)
        .filter(|&w| ps.weapons.contains(&(w as i32)))
        .filter(|&w| {
            world.weapon_combat_row(w).is_some_and(|f| {
                let ammo_key = weapon_t6::ammo_table::ammo_table_key(f.ammo_index, w);
                let clip_key = weapon_t6::ammo_table::clip_table_key(f.clip_index, w);
                weapon_t6::ammo_table::get_ammo_player_both_clips(&ps.weapons, &ps.weapon_data, &ps.ammo, &ps.ammoclip, w, ammo_key, clip_key) > 0
            })
        })
        .collect();
    match earned.iter().position(|&w| w == held) {
        Some(k) => earned.get(k + 1).or(earned.first()).copied(),
        None => earned.first().copied(),
    }
}

#[derive(Resource, Default, Debug, Clone)]
pub struct ClientPhaseTrace(pub Vec<&'static str>);

fn push_phase(trace: Option<ResMut<ClientPhaseTrace>>, name: &'static str) {
    if let Some(mut t) = trace {
        t.0.push(name);
    }
}

pub fn listen_prediction_needs_content(pred: &ClientPrediction, authority: &sim::SimWorld) -> bool {
    if !authority.has_world_clip() {
        return false;
    }
    if !pred.is_armed() || !pred.world().has_world_clip() {
        return true;
    }
    if pred.world().content_digest() != authority.content_digest() {
        return true;
    }
    authority.pen_table_loaded() && !pred.world().pen_table_loaded()
}

pub fn arm_listen_prediction(
    role: Res<RuntimeRole>,
    authority: Option<Res<AuthorityWorld>>,
    mut prediction: ResMut<ClientPredictionState>,
) {
    if !matches!(*role, RuntimeRole::Listen | RuntimeRole::Client) {
        return;
    }
    let Some(authority) = authority else {
        return;
    };
    if !listen_prediction_needs_content(&prediction.0, &authority.0) {
        return;
    }
    prediction.0.arm_from_content(&authority.0);
}

pub fn advance_cls_realtime(mut cls: ResMut<ClientRealtime>, time: Res<Time>) {
    cls.advance_listen(time.delta_secs());
}

pub fn advance_cg_frame_clock(
    mut clock: ResMut<FrameClock>,
    mut prediction: ResMut<ClientPredictionState>,
    mut join: ResMut<GameJoinCensus>,
    time: Res<Time>,
    cgame_active: Res<GameActive>,
    authority: Option<Res<crate::AuthorityClock>>,
    role: Res<RuntimeRole>,
    fixed: Res<Time<Fixed>>,
    adopted: Option<Res<LastAdoptedSnapshot>>,
) {
    let was_started = clock.started();
    let adopted_tick = adopted.as_ref().and_then(|a| a.next().map(|s| s.tick));

    let local_authority = authority.as_ref().filter(|_| role.runs_authority());
    let server_time = local_authority
        .map(|clock| ServerTime::from_ms(clock.time_ms))
        .or_else(|| adopted_tick.map(ServerTime::from_tick));
    if *role == RuntimeRole::Listen && cgame_active.0 && was_started {
        let server = authority.as_ref().expect("listen authority clock").time_ms;
        clock.assign_server_time(server.wrapping_add(fixed.overstep().as_millis() as i32));
    } else if *role == RuntimeRole::Client {
        clock.advance_remote(time.delta_secs(), cgame_active.0, server_time);
    } else {
        clock.advance_listen(time.delta_secs(), cgame_active.0, server_time);
    }
    if clock.started() && !was_started {
        join.latch(
            server_time.map(ServerTime::ms).unwrap_or(0),
            local_authority
                .map(|c| c.tick)
                .or_else(|| adopted_tick.map(|t| t.0))
                .unwrap_or(0),
            (time.elapsed_secs() * 1000.0) as i32,
        );
    }
    prediction.0.predicted_error_mut().sync_frame(&clock);
}

#[derive(Default)]
pub struct JoinLinkWatch {
    connected: bool,
}

pub fn receive_ticks(
    mut link: Option<ResMut<crate::transport::udp_session::UdpClientLink>>,
    loopback: Option<ResMut<ListenLoopback>>,
    mut received: ResMut<ReceivedTicks>,
    mut local: ResMut<LocalPresentClient>,
    mut prediction: ResMut<ClientPredictionState>,
    trace: Option<ResMut<ClientPhaseTrace>>,
    mut watch: Local<JoinLinkWatch>,
    mut reliable: ReliableInbound,
) {
    push_phase(trace, "Receive");
    if let Some(link) = link.as_mut() {
        match link.recv_ticks() {
            Ok(ticks) => {
                for tick in ticks {
                    received.0.push_back(tick);
                }
            }
            Err(e) => {
                if link.handshake_reject().is_some() {
                    diag::error!(Net, "udp recv: {e}");
                } else {
                    diag::warn!(Net, "udp recv: {e}");
                }
            }
        }
        for payload in link.take_controls() {
            reliable.apply(link.assigned_client.unwrap_or(local.0), &payload);
        }
        note_join_link(&mut watch, link);

        if let Some(id) = link.assigned_client {
            if local.0 != id {
                local.0 = id;
                prediction.0.set_local(id);
            }
        }
        return;
    }
    if let Some(mut loopback) = loopback {
        let mut drained = Vec::new();
        let outcome = loopback.recv_all(&mut drained);
        for tick in drained {
            received.0.push_back(tick);
        }
        if let Err(e) = outcome {
            diag::warn!(Net, "client_runtime: recv_all failed: {e}");
        }
    }
}

fn note_join_link(watch: &mut JoinLinkWatch, link: &crate::transport::udp_session::UdpClientLink) {
    if link.connection.is_some() && !watch.connected {
        watch.connected = true;
        diag::info!(
            Net,
            "udp client accepted as client {}",
            link.assigned_client.map(|c| c.0).unwrap_or(0)
        );
    }
}

#[derive(Resource, Default)]
pub struct RemoteProxyState(pub RemoteProxy);

pub fn reconcile_prediction(
    mut received: ResMut<ReceivedTicks>,
    mut prediction: ResMut<ClientPredictionState>,
    local: Res<LocalPresentClient>,
    mut reliable: ReliableInbound,
    mut last_adopted: ResMut<LastAdoptedSnapshot>,
    mut proxy: ResMut<RemoteProxyState>,
    mut pending: ResMut<PendingClientSends>,
    mut entity_events: ResMut<PendingPresentedEntityEvents>,
    mut pellet_fx: ResMut<PendingPelletFx>,
    mut debris: ResMut<PendingDebris>,
    trace: Option<ResMut<ClientPhaseTrace>>,
) {
    push_phase(trace, "Reconcile");
    last_adopted.applied_this_frame = false;
    while let Some(mut tick) = received.0.pop_front() {
        if !collect_received_entity_events(
            last_adopted.next().map(|last| last.tick),
            local.0,
            &mut tick.snapshot,
            &mut entity_events,
        ) {
            continue;
        }
        pellet_fx.0.append(&mut tick.snapshot.meta.pellet_fx);
        debris.0.append(&mut tick.snapshot.meta.debris);
        reliable.apply(local.0, &tick.frame.reliable);
        let ack = tick.ack_for(local.0);
        if let Some(acked) = ack {
            pending.clear_acknowledged(acked);
        }
        last_adopted.applied_this_frame = true;
        if let Some(old_next) = last_adopted.next_snap.take() {
            last_adopted.snap = Some(old_next);
        }
        let snap = Arc::new(tick.snapshot);
        proxy.0.push_arc(Arc::clone(&snap));
        last_adopted.next_snap = Some(snap);
        let newest = !received.0.iter().any(|pending| {
            pending.snapshot.tick.0 > last_adopted.next().expect("next snapshot").tick.0
        });
        if newest {
            prediction
                .0
                .on_authority(last_adopted.next().expect("next_snap just stored"), ack);
        } else {
            prediction.0.retire_acks(ack);
        }
        for cmd in tick.frame.svc_sounds {
            reliable.svc.sound.write(crate::SvcLocalSound {
                stop: cmd.stop,
                index: cmd.index,
            });
        }
        for cmd in tick.frame.svc_card_slots {
            reliable.svc.card.write(crate::SvcCardSlotCmd {
                client: cmd.client,
                slot: cmd.slot,
            });
        }
        for cmd in tick.frame.svc_hud_splashes {
            reliable.svc.splash.write(cmd);
        }
        for cmd in tick.frame.svc_game_notifies {
            reliable.svc.notify.write(cmd);
        }
        for cmd in tick.frame.svc_open_menus {
            reliable.svc.menu.write(crate::SvcOpenMenuCmd {
                cs_index: cmd.cs_index,
            });
        }
        if let Some(cmd) = tick.frame.svc_scores {
            reliable.scores.parsed = crate::parse_scoreboard_cmd(&cmd);
            reliable.scores.cmd = Some(cmd);
        }
    }
}

#[derive(bevy::prelude::Message, Clone, Copy, Debug)]
pub struct ReliableControlEvent(pub sim::SimEvent);

#[derive(bevy::ecs::system::SystemParam)]
pub struct SvcFrameWriters<'w> {
    sound: MessageWriter<'w, crate::SvcLocalSound>,
    card: MessageWriter<'w, crate::SvcCardSlotCmd>,
    menu: MessageWriter<'w, crate::SvcOpenMenuCmd>,
    splash: MessageWriter<'w, crate::SvcHudSplash>,
    notify: MessageWriter<'w, crate::SvcGameNotify>,
}

#[derive(bevy::ecs::system::SystemParam)]
pub struct ReliableInbound<'w> {
    actions: ResMut<'w, ClientActionInbox>,
    ack: ResMut<'w, ClientReliableAck>,
    events: MessageWriter<'w, ReliableControlEvent>,
    svc: SvcFrameWriters<'w>,
    scores: ResMut<'w, crate::Scoreboard>,
    signon: ResMut<'w, crate::SignonState>,
    bridge: Option<Res<'w, crate::MasterBridge>>,
}

impl ReliableInbound<'_> {
    fn fail(&mut self, reason: &str) {
        self.actions.clear();
        let match_key = crate::signon::live_match_key(self.bridge.as_deref());
        if let Some(bridge) = &self.bridge {
            bridge.fail(reason);
        }
        self.signon.set_phase(crate::SignonPhase::Failed(
            crate::SignonFailReason::Transport {
                source: reason.to_owned(),
                stage: crate::FailStage::Transport,
                match_key,
            },
        ));
    }

    fn apply(&mut self, local: sim::ClientId, payload: &crate::ReliablePayload) {
        if payload.dropped_oldest != 0 {
            self.fail("ReliableControlOverflow: outcome unknown");
            return;
        }
        if let Some(reason) = payload.rows.iter().find_map(|(_, row)| {
            if let crate::ReliableRow::Failure(reason) = row {
                Some(reason)
            } else {
                None
            }
        }) {
            self.fail(reason);
            return;
        }
        let mut rows: Vec<&(u16, crate::ReliableRow)> = payload.rows.iter().collect();
        rows.sort_by_key(|(seq, _)| seq.wrapping_sub(self.ack.0));
        for (seq, row) in rows {
            if !reliable_seq_after(*seq, self.ack.0) {
                continue;
            }
            match row {
                crate::ReliableRow::Failure(_) => unreachable!("terminal handled before sequence"),
                crate::ReliableRow::Sound(cmd) => {
                    self.svc.sound.write(crate::SvcLocalSound {
                        stop: cmd.stop,
                        index: cmd.index,
                    });
                }
                crate::ReliableRow::Card(cmd) => {
                    self.svc.card.write(crate::SvcCardSlotCmd {
                        client: cmd.client,
                        slot: cmd.slot,
                    });
                }
                crate::ReliableRow::Menu(cmd) => {
                    self.svc.menu.write(crate::SvcOpenMenuCmd {
                        cs_index: cmd.cs_index,
                    });
                }
                crate::ReliableRow::Splash(cmd) => {
                    self.svc.splash.write(cmd.clone());
                }
                crate::ReliableRow::Notify(cmd) => {
                    self.svc.notify.write(cmd.clone());
                }
                crate::ReliableRow::Scores(cmd) => {
                    self.scores.parsed = crate::parse_scoreboard_cmd(cmd);
                    self.scores.cmd = Some(cmd.clone());
                }
                crate::ReliableRow::ActionOutcome {
                    request_id,
                    verdict,
                } => {
                    match verdict {
                        crate::ActionVerdict::PayloadMismatch => {
                            self.fail("ActionPayloadMismatch: connection retired");
                            return;
                        }
                        crate::ActionVerdict::Expired => {
                            self.fail("ActionOutcomeUnknown: request expired");
                            return;
                        }
                        crate::ActionVerdict::Applied | crate::ActionVerdict::Refused => {}
                    }
                    self.actions.retire(local, *request_id);
                }
                crate::ReliableRow::Event(event) => {
                    self.events.write(ReliableControlEvent(*event));
                }
            }
            self.ack.0 = *seq;
        }
    }
}

fn reliable_seq_after(a: u16, b: u16) -> bool {
    a != b && a.wrapping_sub(b) < 0x8000
}

fn apply_weapon_switch_requests(
    mut events: MessageReader<ReliableControlEvent>,
    mut select: ResMut<WeaponSelect>,
    clock: Res<FrameClock>,
) {
    for event in events.read() {
        if let sim::SimEvent::WeaponSwitchRequested { weapon } = event.0 {
            select.index = weapon;
            select.mapped_index = weapon;
            select.time = clock.time();
        }
    }
}

pub fn sample_client_input(
    time: Res<Time>,
    mut actions: ResMut<ClientActionInput>,
    mut look: ResMut<LookState>,
    mut template: ResMut<ClientCmdTemplate>,
    mut select: ResMut<WeaponSelect>,
    prediction: Res<ClientPredictionState>,
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    gate: Res<AuthorityInputGate>,
    cls: Res<ClientRealtime>,
    clock: Res<FrameClock>,
    mut action_inbox: Option<ResMut<ClientActionInbox>>,
    mut request_ids: Option<ResMut<crate::ActionRequestIds>>,
    view: Option<Res<frame::ViewSubject>>,
    trace: Option<ResMut<ClientPhaseTrace>>,
    mut cursor: ResMut<LocationCursor>,
) {
    push_phase(trace, "Input");
    if !gate.local_cmds_enabled {
        if !actions.client.weapon_cycles.is_empty() {
            diag::info!(Net, "bo2mp cycle: dropped, local commands are off");
        }
        actions.client.weapon_cycles.clear();
        actions.client.action_slots.clear();
        return;
    }
    actions.frame_msec = key_frame_msec(time.delta_secs());
    actions.now_msec = frame_time_msec(time.elapsed_secs());
    let ps = prediction
        .0
        .predicted_local()
        .or_else(|| presented.player(local.0));
    // bo2zm: the stance button dives when held while sprinting at full height.
    actions.client.sprinting = ps.is_some_and(|ps| {
        ps.pm_flags & playerstate_t6::game::pm_flags::SPRINTING != 0
            && ps.view_height_current >= movement_t6::bridge::view_height::STAND as f32
    });
    // bo2mp gun brain: the fire key's semi-auto jam roll reads the last snapshot's gun (BO2's
    // IN_Attack_Down); no snapshot, no roll.
    actions.client.jam_fire_time = presented
        .player(local.0)
        .map(|snap| prediction.0.world().gun_facts().get(snap.weapon as u32).jam_window());
    // bo2mp: the stance key reads the last snapshot's gun too (BO2's IN_Stance_Up,
    // BG_WeaponBlocksProne).
    actions.client.weapon_blocks_prone = presented.player(local.0).is_some_and(|snap| {
        prediction.0.world().gun_facts().get(snap.weapon as u32).moves.blocks_prone
    });

    let frozen = ps.is_some_and(|ps| (ps.pm_flags & 0x800) != 0)
        || presented
            .snapshot()
            .and_then(|snapshot| snapshot.meta.for_client(local.0))
            .is_none_or(|meta| meta.controls.frozen);
    // bo2mp: BO2's CL_MouseMove and CL_GamepadMove also skip the turn and the pad
    // move while a riot shield is being planted (raw weapon state 44, DEPLOYING).
    const T6_DEPLOYING: i32 = 44;
    let planting = prediction
        .0
        .world()
        .client_meta(local.0)
        .and_then(|meta| meta.t6_offhand())
        .or_else(|| {
            presented
                .snapshot()
                .and_then(|snapshot| snapshot.meta.for_client(local.0))
                .and_then(|meta| meta.t6_view)
                .map(|view| (view.weaponstate, 0))
        })
        .is_some_and(|(state, _)| state == T6_DEPLOYING);

    // bo2mp: riding a Hellstorm his mouse and right stick steer the missile,
    // frozen or not (they are taken for it below).
    let riding = presented
        .snapshot()
        .and_then(|snapshot| snapshot.meta.for_client(local.0))
        .and_then(|meta| meta.remote_missile)
        .is_some_and(|link| link.unlink_at_ms.is_none());
    if riding && !planting {
        actions.pad_move = [0.0; 2];
    } else if frozen || planting {
        actions.mouse_x = 0.0;
        actions.mouse_y = 0.0;
        actions.pad_move = [0.0; 2];
        actions.pad_look = [0.0; 2];
    }
    if let Some(ps) = ps {
        let world = prediction.0.world();
        let eye = [
            ps.origin[0],
            ps.origin[1],
            ps.origin[2] + ps.view_height_current,
        ];
        let visible = |point: [f32; 3]| {
            let mask = hud_t6::overhead_names::OVERHEAD_TRACE_MASK;
            let hit = world.trace_world(eye, point, [0.0; 3], [0.0; 3], mask);
            hit.fraction >= 1.0 && !hit.startsolid
        };
        let angles = [
            look.angles[0] as f32 / input_t6::ANGLE2SHORT,
            look.angles[1] as f32 / input_t6::ANGLE2SHORT,
        ];
        let (sp, cp) = angles[0].to_radians().sin_cos();
        let (sy, cy) = angles[1].to_radians().sin_cos();
        let forward = [cp * cy, cp * sy, -sp];
        let in_front = |origin: [f32; 3], radius: f32| {
            let d = [
                origin[0] - ps.origin[0],
                origin[1] - ps.origin[1],
                origin[2] - ps.origin[2],
            ];
            d[0] * forward[0] + d[1] * forward[1] + d[2] * forward[2] + radius >= 0.0
        };
        let mut targets: Vec<crate::client::pad_aim::AimTarget> = Vec::new();
        if actions.pad_aim_assist > 0 {
            const RADIUS: f32 = 10.0;
            let snapshot = presented.snapshot();
            let team = |id: sim::ClientId| {
                snapshot
                    .and_then(|s| s.meta.for_client(id))
                    .map(|m| m.client_state_team)
            };
            let teams = snapshot.is_some_and(|s| s.meta.kind.is_team());
            let mine = team(local.0);
            for id in snapshot
                .into_iter()
                .flat_map(|s| s.players.iter().map(|(id, _)| *id))
            {
                if id == local.0 || (teams && team(id) == mine) {
                    continue;
                }
                let Some(other) = presented.player(id).filter(|o| o.pm_type == 0) else {
                    continue;
                };
                let o = other.origin;
                let head = [o[0], o[1], o[2] + other.view_height_current];
                if !in_front(o, RADIUS) || !visible(head) {
                    continue;
                }
                let top = other.view_height_current + 8.0;
                targets.push(crate::client::pad_aim::AimTarget {
                    key: u64::from(id.0),
                    mins: [o[0] - RADIUS, o[1] - RADIUS, o[2]],
                    maxs: [o[0] + RADIUS, o[1] + RADIUS, o[2] + top],
                    aim: [o[0], o[1], o[2] + top * 0.75],
                    velocity: other.velocity,
                });
            }
        }
        let ranges = world
            .weapon_combat_row(playerstate_t6::game::get_viewmodel_weapon_index(ps))
            .map_or(weapon_t6::combat_facts::AimAssistRanges::NONE, |facts| facts.aim_assist);
        let view = crate::client::pad_aim::AimView {
            eye,
            angles,
            velocity: ps.velocity,
            ads_lerp: ps.f_weapon_pos_frac,
            fov_scale: actions.fov_scale.max(0.01),
            ranges,
            dt: cls.frametime_secs(),
        };
        let ads = actions.client.using_ads || actions.client.kb.speed.active;
        crate::client::pad_aim::pad_look_frame(&mut actions, &view, &targets, ads);
    } else {
        actions.pad_look_delta = [0.0; 2];
    }
    if frozen {
        actions.pad_look_delta = [0.0; 2];
    }
    let choose_direction = presented
        .snapshot()
        .and_then(|snapshot| snapshot.meta.for_client(local.0))
        .and_then(|meta| meta.location_selection.as_ref())
        .map(|selection| selection.choose_direction);
    let location_mouse = choose_direction.is_some().then(|| {
        let mouse = (actions.mouse_x, actions.mouse_y);
        actions.mouse_x = 0.0;
        actions.mouse_y = 0.0;
        mouse
    });
    let remote_mouse = presented
        .snapshot()
        .and_then(|snapshot| snapshot.meta.for_client(local.0))
        .and_then(|meta| meta.remote_missile)
        .filter(|link| link.unlink_at_ms.is_none())
        .map(|_| {
            let mouse = (actions.mouse_x, actions.mouse_y, actions.pad_look);
            actions.mouse_x = 0.0;
            actions.mouse_y = 0.0;
            actions.pad_look = [0.0; 2];
            actions.pad_look_delta = [0.0; 2];
            mouse
        });
    let look_state = ps
        .zip(presented.shellshock(local.0))
        .map(|(ps, shock)| {
            hud_t6::flashbang::shellshock_look_control(
                hud_t6::flashbang::shellshock_remaining_ms(
                    clock.time(),
                    ps.shellshock_time,
                    ps.shellshock_duration,
                ),
                &shock.look_control,
            )
        })
        .unwrap_or(hud_t6::flashbang::SHELLSHOCK_LOOK_ENDED);
    actions.shellshock_look_scale = look_state.sensitivity;
    actions.cgame_max_pitch_speed = look_state.max_pitch_speed;
    actions.cgame_max_yaw_speed = look_state.max_yaw_speed;
    let now_msec = actions.now_msec;
    let frame_msec = actions.frame_msec;
    let cl_yawspeed = actions.cl_yawspeed;
    let max_pitch = actions.cgame_max_pitch_speed;
    let max_yaw = actions.cgame_max_yaw_speed;
    accumulate_look(
        cls.frametime_secs(),
        now_msec,
        frame_msec,
        &mut actions.client,
        &mut look,
        cl_yawspeed,
        frozen,
        max_pitch,
        max_yaw,
    );
    if std::mem::take(&mut actions.client.center_view) {
        if let Some(ps) = ps {
            look.angles[0] = (-ps.delta_angles[0] * input_t6::ANGLE2SHORT) as i32;
        }
    }
    let ps_weapon = ps.map(|ps| ps.weapon).unwrap_or(0);
    let life_sequence = presented
        .snapshot()
        .and_then(|snapshot| snapshot.meta.for_client(local.0))
        .map(|meta| meta.life_sequence.0)
        .unwrap_or(0);
    if actions.stance_life != Some(life_sequence) {
        actions.client.stance_latch = 0;
        actions.stance_life = Some(life_sequence);
        actions.stance_event_sequence = ps.map_or(0, |ps| i32::from(ps.predictable_event_sequence));
        actions.stance_events = ps.map_or([0; 4], |ps| ps.predictable_events);
    } else if let Some(ps) = ps {
        actions.read_stance_events(i32::from(ps.predictable_event_sequence), ps.predictable_events);
    }
    follow_held_weapon_select(&mut select, ps_weapon, life_sequence, clock.time());
    if let Some(ps) = ps {
        if select.index == ps.weapon {
            select.mapped_index = if prediction
                .0
                .world()
                .weapon_combat_row(ps.weapon)
                .is_some_and(|facts| facts.inventory_type == 3)
            {
                ps.weapon_primary
            } else {
                ps.weapon
            };
        }
    }
    let slots = std::mem::take(&mut actions.client.action_slots);
    if let (Some(inbox), Some(ids)) = (action_inbox.as_mut(), request_ids.as_mut()) {
        for &slot in &slots {
            let request_id = ids.allocate();
            let action = sim::ClientAction::ActionSlot {
                request_id,
                slot: slot as u8,
            };
            if let Err(error) = inbox.push(local.0, action) {
                diag::warn!(Net, "action_slot: not queued — {error}");
            }
        }
    }
    if let Some(ps) = ps.filter(|_| !frozen) {
        for slot in slots {
            if !input_t6::weapon_select::weapon_cycle_allowed(select_flags(ps), clock.time(), select.time, 0, 0)
            {
                continue;
            }
            let world = prediction.0.world();
            let mut walked = 0;
            let target = match ps.action_slot_type.get(slot).copied() {
                Some(1) => match next_earned_streak(ps, world, select.index) {
                    Some(w) => w,
                    None => continue,
                },
                // bo2mp: Black Ops II's CG_AltToggleAllowedByState: no attachment key
                // mid-switch, mid-knife, mid-throw, on a charge-shot gun or mid-climb.
                Some(2) if !sim::alt_toggle_allowed_by_state(world, ps) => continue,
                Some(2) => match world.weapon_combat_row(select.index) {
                    Some(facts) if facts.inventory_type == 3 => select.mapped_index,
                    Some(facts) if facts.alternate_weapon != 0 => facts.alternate_weapon,
                    // bo2mp: Black Ops II walks on to the next gun that has
                    // a mode and brings that mode out (the launcher can swap).
                    Some(_) => {
                        let has_ammo = |w: u32| {
                            world.weapon_combat_row(w).is_some_and(|f| {
                                let ammo_key = weapon_t6::ammo_table::ammo_table_key(f.ammo_index, w);
                                let clip_key = weapon_t6::ammo_table::clip_table_key(f.clip_index, w);
                                weapon_t6::ammo_table::get_ammo_player_both_clips(&ps.weapons, &ps.weapon_data, &ps.ammo, &ps.ammoclip, w, ammo_key, clip_key)
                                    != 0
                            })
                        };
                        let streak = |w: u32| streak_slot_weapons(ps).any(|s| s == w);
                        match sim::alt_toggle_walk(world, &ps.weapons, select.index, streak, has_ammo)
                        {
                            Some(base) => {
                                walked = base;
                                world
                                    .weapon_combat_row(base)
                                    .map_or(0, |f| f.alternate_weapon)
                            }
                            None => 0,
                        }
                    }
                    None => 0,
                },
                _ => 0,
            };
            if target == 0 {
                continue;
            }
            let parent = if walked != 0 {
                walked
            } else if ps.weapons.contains(&(target as i32)) {
                target
            } else {
                select.mapped_index
            };
            let owned = ps.weapons.contains(&(parent as i32))
                && (parent == target
                    || prediction
                        .0
                        .world()
                        .weapon_combat_row(parent)
                        .is_some_and(|f| f.alternate_weapon == target));
            if !owned {
                continue;
            }
            select.index = target;
            select.mapped_index = parent;
            select.time = clock.time();
            sim::remember_mode(
                prediction.0.world(),
                &ps.weapons,
                &mut select.last_mode,
                target,
            );
            input_t6::set_ads(&mut actions.client, false);
        }
    }
    // bo2mp: Hybrid Optic: Hold Breath while aiming switches to the gun's
    // other optic (its alternate weapon, which stays aimed).
    // One tap, one switch: the press flag outlives a frame, so the switch
    // waits until the last one has landed (the player holds the weapon it
    // selected) and a beat after it.
    if look.breath_is_switch && !frozen && actions.client.kb.holdbreath.was_pressed {
        if let Some(ps) = ps.filter(|ps| {
            select.index == ps.weapon && clock.time().wrapping_sub(select.time) >= 350
        }) {
            let target = prediction
                .0
                .world()
                .weapon_combat_row(select.index)
                .map_or(0, |facts| facts.alternate_weapon);
            if target != 0
                && input_t6::weapon_select::weapon_cycle_allowed(select_flags(ps), clock.time(), select.time, 0, 0)
            {
                let parent = if ps.weapons.contains(&(target as i32)) {
                    target
                } else {
                    select.mapped_index
                };
                let owned = ps.weapons.contains(&(parent as i32))
                    && (parent == target
                        || prediction
                            .0
                            .world()
                            .weapon_combat_row(parent)
                            .is_some_and(|f| f.alternate_weapon == target));
                if owned {
                    select.index = target;
                    select.mapped_index = parent;
                    select.time = clock.time();
                    sim::remember_mode(
                        prediction.0.world(),
                        &ps.weapons,
                        &mut select.last_mode,
                        target,
                    );
                }
            }
        }
    }
    let cycles = std::mem::take(&mut actions.client.weapon_cycles);
    // bo2mp: the switch-weapon key (`+weapnext_inventory`, wheel up) is also
    // BO2's change seat button, on the press.
    let change_seat = std::mem::take(&mut actions.client.change_seat_press);
    let in_killcam = view.is_some_and(|v| v.in_killcam());
    // bo2mp gun brain: Some when the T6 brain runs: the predicted raw state and grenade in
    // hand (the last snapshot's state, no grenade, before prediction has started the brain).
    let t6_offhand = prediction
        .0
        .world()
        .client_meta(local.0)
        .and_then(|meta| meta.t6_offhand())
        .or_else(|| {
            presented
                .snapshot()
                .and_then(|snapshot| snapshot.meta.for_client(local.0))
                .and_then(|meta| meta.t6_view)
                .map(|view| (view.weaponstate, 0))
        });
    if ps.is_none() && !cycles.is_empty() {
        diag::info!(Net, "bo2mp cycle: dropped, no player state yet");
    }
    if let Some(ps) = ps {
        for next in cycles {
            if next && copycat_weapnext_bind_active(ps.pm_type, in_killcam) {
                if let (Some(inbox), Some(ids)) = (action_inbox.as_mut(), request_ids.as_mut()) {
                    let request_id = ids.allocate();
                    if let Err(error) =
                        inbox.push(local.0, sim::ClientAction::UseCopycat { request_id })
                    {
                        diag::warn!(Net, "use_copycat: not queued — {error}");
                    }
                }
            }
            // BO2's cycle check refuses while frozen (the match-begins
            // countdown): the pick stays, so no swap fires when it ends.
            if frozen {
                diag::info!(Net, "bo2mp cycle: refused while frozen");
                continue;
            }
            cycle_weapon_select(
                &mut select,
                &mut actions.client,
                ps,
                prediction.0.world(),
                clock.time(),
                next,
                t6_offhand,
            );
        }
    }
    // bo2zm: a Black Ops II gun's kick moves the aim (whole short-angle
    // steps; the remainder waits for the next command).
    for axis in 0..2 {
        if frozen {
            look.kick_pending[axis] = 0.0;
            continue;
        }
        let step = (look.kick_pending[axis] * input_t6::ANGLE2SHORT).round() as i32;
        if step != 0 {
            look.angles[axis] = look.angles[axis].wrapping_add(step);
            look.kick_pending[axis] -= step as f32 / input_t6::ANGLE2SHORT;
        }
    }
    // bo2mp: melee on a Variable Zoom scope changes the zoom, no knife.
    if look.melee_is_zoom {
        let melee = &mut actions.client.kb.melee;
        if melee.was_pressed || melee.active {
            if melee.was_pressed {
                look.zoom_presses = look.zoom_presses.wrapping_add(1);
            }
            *melee = input_t6::Kbutton::default();
        }
    }
    // bo2mp: flying a Dragonfire, Space takes it up and Ctrl down while held;
    // Ctrl is BO2's toggleprone, so its toggle is dropped and the held key
    // goes as crouch, which the drone reads as down.
    let drone = presented
        .snapshot()
        .and_then(|snapshot| snapshot.meta.script_dvars(local.0).string("bo2mp_vehicle"))
        .is_some_and(|v| v.split(' ').next().is_some_and(|k| k.to_ascii_lowercase().starts_with("qrdrone")));
    if drone {
        actions.client.stance_latch = 0;
    }
    let mut cmd = build_usercmd(&mut actions, &look, 0);
    if drone && input_t6::lower_key_held(&actions.client) {
        cmd.buttons |= playerstate_t6::game::buttons::CROUCH;
    }
    cmd.last_input = if actions.last_input_pad {
        playerstate_t6::game::last_input::GAMEPAD
    } else {
        playerstate_t6::game::last_input::MOUSECLICK
    };
    if cmd.buttons & playerstate_t6::game::buttons::USE_RELOAD != 0
        && ps.is_some_and(|ps| ps.cursor_hint == 0)
    {
        cmd.buttons |= playerstate_t6::game::buttons::RELOAD;
    }
    look.angles = cmd.angles;
    if let Some((mouse_x, mouse_y, pad_look)) = remote_mouse {
        cmd.remote_control = remote_control_axes(&actions, mouse_x, mouse_y, pad_look);
        cmd.buttons |= playerstate_t6::game::buttons::REMOTE_CONTROL;
    }

    if change_seat {
        cmd.buttons |= playerstate_t6::game::buttons::CHANGE_SEAT;
    }
    // bo2mp: Black Ops II's CG_SelectInventoryWeapon (switch-weapon held, or
    // `inventory`) takes the inventory weapon (the streak from a care
    // package); each command then carries the inventory button until the
    // server's snapshot has him holding it (CL_WeapNextButtonUpdate reads the
    // snapshot, not the prediction), so the streak script, which reads the
    // button when the weapon changes on the server, knows it came from the
    // inventory.
    let inventory_request = std::mem::take(&mut actions.client.inventory_request);
    if let Some(ps) = ps {
        let target = ps.inventory_weapon.max(0) as u32;
        if inventory_request {
            diag::info!(
                Net,
                "bo2mp inventory: switch asked; inventory weapon {target}, holding {}, selected {}, carried {}, frozen {frozen}, can switch {}",
                ps.weapon,
                select.index,
                ps.weapons.contains(&(target as i32)),
                input_t6::weapon_select::weapon_cycle_allowed(select_flags(ps), clock.time(), select.time, 0, 0)
            );
        }
        // bo2mp: Black Ops II's CG_WeaponSwitchRequest: no pick mid-throw
        // (offhand states 22-29).
        let throwing = (22..=29).contains(&ps.weapon_state_t6);
        let can_switch =
            input_t6::weapon_select::weapon_cycle_allowed(select_flags(ps), clock.time(), select.time, 0, 0);
        if inventory_request
            && !frozen
            && !throwing
            && target != 0
            && target != ps.weapon
            && target != select.index
            && ps.weapons.contains(&(target as i32))
            && can_switch
        {
            // CG_SelectWeapon remembers the gun he leaves (an alt mode's base
            // gun) so a second press goes back to it.
            let world = prediction.0.world();
            if let Some(row) = world.weapon_combat_row(select.index) {
                match row.inventory_type {
                    0 => select.last_primary = select.index,
                    3 if row.alternate_weapon != 0 => select.last_primary = row.alternate_weapon,
                    _ => {}
                }
            }
            select.index = target;
            select.mapped_index = target;
            select.time = clock.time();
            select.switching_to_inventory = true;
            input_t6::set_ads(&mut actions.client, false);
        } else if inventory_request
            && !frozen
            && !throwing
            && target != 0
            && (target == ps.weapon || target == select.index)
        {
            // The inventory weapon is already up: the press goes back to the
            // last gun he held if he still carries it, else on to the next gun
            // (not in a vehicle).
            let last = select.last_primary;
            if last != 0 && last != target && ps.weapons.contains(&(last as i32)) {
                if can_switch {
                    diag::info!(Net, "bo2mp inventory: already up, back to {last}");
                    let world = prediction.0.world();
                    let last = sim::remembered_mode(world, &ps.weapons, &select.last_mode, last);
                    select.mapped_index = world
                        .weapon_combat_row(last)
                        .filter(|row| row.inventory_type == 3)
                        .map_or(last, |row| row.alternate_weapon);
                    select.index = last;
                    select.time = clock.time();
                    select.switching_to_inventory = false;
                    input_t6::set_ads(&mut actions.client, false);
                }
            } else if ps.e_flags & 0x4000 == 0 {
                diag::info!(Net, "bo2mp inventory: already up, cycling on");
                cycle_weapon_select(
                    &mut select,
                    &mut actions.client,
                    ps,
                    prediction.0.world(),
                    clock.time(),
                    true,
                    t6_offhand,
                );
            }
        }
        // `presented.player` is the prediction for him; the snapshot's own.
        let snap = presented
            .snapshot()
            .and_then(|snapshot| snapshot.players.iter().find(|(c, _)| *c == local.0))
            .map_or(ps, |(_, snap)| snap);
        if select.switching_to_inventory
            && target != 0
            && select.index == snap.inventory_weapon.max(0) as u32
            && snap.weapon != select.index
        {
            cmd.buttons |= playerstate_t6::game::buttons::INVENTORY;
        } else {
            select.switching_to_inventory = false;
        }
    }
    cmd.weapon = select.index as u16;
    cmd.weapon_mapped = select.mapped_index as u16;
    if let Some(loadout) = presented
        .snapshot()
        .and_then(|snapshot| snapshot.meta.for_client(local.0))
        .and_then(|meta| meta.loadout.as_ref())
    {
        if actions.client.kb.frag.active || actions.client.kb.frag.was_pressed {
            cmd.off_hand_index = loadout.lethal as u16;
        } else if actions.client.kb.smoke.active || actions.client.kb.smoke.was_pressed {
            cmd.off_hand_index = loadout.tactical as u16;
        }
    }
    match location_mouse {
        Some((mouse_x, mouse_y)) => {
            let held = cmd.buttons;
            let directing =
                choose_direction == Some(true) && held & playerstate_t6::game::buttons::ADS != 0;
            let cancel = if choose_direction == Some(true) {
                playerstate_t6::game::buttons::MELEE_CHARGE
            } else {
                playerstate_t6::game::buttons::ADS | playerstate_t6::game::buttons::MELEE_CHARGE
            };
            cmd.selected_location = cursor.step(&actions, mouse_x, mouse_y, directing);
            if choose_direction != Some(true) {
                cmd.selected_location[2] = 0;
            }
            cmd.buttons &= playerstate_t6::game::buttons::CROUCH | playerstate_t6::game::buttons::PRONE;
            if held & playerstate_t6::game::buttons::ATTACK != 0 {
                if template.cmd.buttons & playerstate_t6::game::buttons::LOCATION_SELECT == 0 {
                    diag::info!(Net, "bo2mp location: pick at {:?}", cmd.selected_location);
                }
                cmd.buttons |= playerstate_t6::game::buttons::LOCATION_SELECT;
            } else if held & cancel != 0 {
                cmd.buttons |= playerstate_t6::game::buttons::LOCATION_CANCEL;
            }
        }
        None => *cursor = LocationCursor::default(),
    }
    if cmd.buttons & playerstate_t6::game::buttons::MELEE_CHARGE != 0
        && let (Some(ps), Some(snapshot)) = (ps, presented.snapshot())
    {
        let world = prediction.0.world();
        if sim::t6_lunge::brain_on(world.gun_facts()) {
            // bo2mp: Black Ops II's pick; the target goes with the press.
            if let Some(pick) = bo2_lunge_target(ps, local.0, snapshot, &presented, world) {
                cmd.melee_charge_ent = Some(pick.ent);
                cmd.melee_charge_yaw = pick.yaw;
                cmd.melee_charge_dist = pick.dist;
            }
        } else if let Some((yaw, dist)) = melee_charge_target(ps, local.0, snapshot, world) {
            cmd.melee_charge_yaw = yaw;
            cmd.melee_charge_dist = dist;
        }
    }
    template.cmd = cmd;
    template.ready = true;
}

/// bo2mp: Black Ops II's lunge target among the live enemies in the snapshot.
fn bo2_lunge_target(
    ps: &playerstate_t6::game::PlayerState,
    local: sim::ClientId,
    snapshot: &Snapshot,
    presented: &PresentedSnapshot,
    world: &sim::SimWorld,
) -> Option<sim::t6_lunge::LungePick> {
    let local_team = snapshot.meta.for_client(local)?.client_state_team;
    let candidates = snapshot.players.iter().filter_map(|(id, other)| {
        let meta = snapshot.meta.for_client(*id)?;
        let enemy = meta.lifecycle == sim::ClientLifecycle::Alive
            && meta.client_state_team != entity_t6::team::SPECTATOR
            && (local_team == 0 || meta.client_state_team != local_team);
        (*id != local && other.pm_type <= 1 && enemy).then_some(())?;
        // Where he sees the enemy (BO2 picks among the drawn players).
        Some(sim::t6_lunge::LungeCandidate {
            ent: u16::try_from(id.0).ok()?,
            origin: presented.player(*id).map_or(other.origin, |p| p.origin),
        })
    });
    let range = sim::t6_lunge::reach(world.gun_facts(), ps.weapon, false);
    sim::t6_lunge::pick(sim::t6_lunge::picker(ps, range), candidates, |eye, center| {
        world.trace_static_world(eye, center, [0.0; 3], [0.0; 3], sim::MASK_SHOT).fraction >= 1.0
    })
}

const AIM_AUTOMELEE_RANGE: f32 = 128.0;
const MELEE_REGION_TAN_X: f32 = 0.849 * 0.5;
const MELEE_REGION_TAN_Y: f32 = 0.478 * 0.5;
fn melee_charge_target(
    ps: &playerstate_t6::game::PlayerState,
    local: sim::ClientId,
    snapshot: &Snapshot,
    world: &sim::SimWorld,
) -> Option<(f32, u8)> {
    if ps.pm_flags & playerstate_t6::game::pm_flags::PRONE != 0 {
        return None;
    }
    let local_team = snapshot.meta.for_client(local)?.client_state_team;
    let eye = [
        ps.origin[0],
        ps.origin[1],
        ps.origin[2] + ps.view_height_current,
    ];
    let (forward, right, up) = game_math::angle_vectors(ps.viewangles);
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let mut best: Option<(f32, [f32; 3])> = None;
    for (id, other) in &snapshot.players {
        if *id == local || other.pm_type > 1 {
            continue;
        }
        let Some(meta) = snapshot.meta.for_client(*id) else {
            continue;
        };
        if meta.lifecycle != sim::ClientLifecycle::Alive
            || meta.client_state_team == entity_t6::team::SPECTATOR
            || (local_team != 0 && meta.client_state_team == local_team)
        {
            continue;
        }
        let delta = [
            other.origin[0] - ps.origin[0],
            other.origin[1] - ps.origin[1],
            other.origin[2] - ps.origin[2],
        ];
        if dot(delta, delta) > AIM_AUTOMELEE_RANGE * AIM_AUTOMELEE_RANGE {
            continue;
        }
        let center = [other.origin[0], other.origin[1], other.origin[2] + 36.0];
        let to = [center[0] - eye[0], center[1] - eye[1], center[2] - eye[2]];
        let depth = dot(to, forward);
        if depth <= 1.0 {
            continue;
        }
        let x = dot(to, right) / depth;
        let y = dot(to, up) / depth;
        if x.abs() - 15.0 / depth > MELEE_REGION_TAN_X
            || y.abs() - 36.0 / depth > MELEE_REGION_TAN_Y
        {
            continue;
        }
        let crosshair = x * x + y * y;
        if best.is_some_and(|(b, _)| b <= crosshair) {
            continue;
        }
        let hit = world.trace_static_world(eye, center, [0.0; 3], [0.0; 3], sim::MASK_SHOT);
        if hit.fraction < 1.0 {
            continue;
        }
        best = Some((crosshair, delta));
    }
    let (_, delta) = best?;
    let dist = dot(delta, delta).sqrt();
    if dist <= movement_t6::bridge::MELEE_CHARGE_PLAYER_MELEE_RANGE_DEFAULT {
        return None;
    }
    Some((game_math::vec_to_yaw(delta[0], delta[1]), dist as u8))
}

/// `yaw` is counterclockwise from map up.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct LocationCursor {
    pub at: [f32; 2],
    pub aim: [f32; 2],
    pub yaw: f32,
    pub directing: bool,
}

impl Default for LocationCursor {
    fn default() -> Self {
        Self {
            at: [0.5, 0.5],
            aim: [0.5, 0.5],
            yaw: 0.0,
            directing: false,
        }
    }
}

const LOCATION_CURSOR_SPEED: f32 = 0.6;

impl LocationCursor {
    fn step(
        &mut self,
        input: &ClientActionInput,
        mouse_x: f32,
        mouse_y: f32,
        directing: bool,
    ) -> [u8; 3] {
        let scale = LOCATION_CURSOR_SPEED * input.sensitivity * 0.002;
        let moved = [mouse_x * scale, mouse_y * scale];
        self.directing = directing;
        if directing {
            self.aim = [
                (self.aim[0] + moved[0]).clamp(0.0, 1.0),
                (self.aim[1] + moved[1]).clamp(0.0, 1.0),
            ];
            let d = [self.aim[0] - self.at[0], self.aim[1] - self.at[1]];
            if d[0] != 0.0 || d[1] != 0.0 {
                self.yaw = (-d[0]).atan2(-d[1]).to_degrees().rem_euclid(360.0);
            }
        } else {
            self.at = [
                (self.at[0] + moved[0]).clamp(0.0, 1.0),
                (self.at[1] + moved[1]).clamp(0.0, 1.0),
            ];
            self.aim = self.at;
        }
        let byte = |v: f32| ((v * 255.0 - 128.0).round() as i32).clamp(-128, 127) as i8 as u8;
        let yaw = (self.yaw * (256.0 / 360.0)).round() as i32 as u8;
        [byte(self.at[0]), byte(self.at[1]), yaw]
    }
}

/// A stall is a stretch in which the authority acked none of the local client's
/// commands. Both measures of one — the age of the oldest command still waiting
/// to be acked, and the command time spanned by the unacknowledged history — are
/// wall-clock spans, so a single long frame fills either on its own — which is
/// why one stall is not a failure.
pub const BACKLOG_STALL_MS: i32 = 1000;

/// How many stalls the link may take before the session is failed for real. A
/// hitch costs one and is recovered from; an authority that has genuinely gone
/// away keeps earning them and fails once they add up.
pub const BACKLOG_STALLS_BEFORE_FAIL: u32 =
    (master_protocol::SESSION_IDLE.as_millis() / BACKLOG_STALL_MS as u128) as u32 - 1;

#[derive(Debug, Default)]
pub struct BacklogStalls {
    counted: u32,

    acks_at_last: u64,

    last_ms: Option<i32>,
}

impl BacklogStalls {
    /// Count a stall, at most one per `BACKLOG_STALL_MS` and only while the
    /// authority has acked nothing since the last one. `None` means the caller
    /// has already acted on the stall still in progress.
    fn note(&mut self, now_ms: i32, acks: u64) -> Option<u32> {
        if acks != self.acks_at_last {
            self.acks_at_last = acks;
            self.counted = 0;
            self.last_ms = Some(now_ms);
            return None;
        }
        if self
            .last_ms
            .is_some_and(|last| now_ms.saturating_sub(last) < BACKLOG_STALL_MS)
        {
            return None;
        }
        self.last_ms = Some(now_ms);
        self.counted = self.counted.saturating_add(1);
        Some(self.counted)
    }

    fn clear(&mut self) {
        *self = Self::default();
    }
}

pub fn enforce_client_work_limits(
    pending: Res<PendingClientSends>,
    mut prediction: ResMut<ClientPredictionState>,
    cg: Res<FrameClock>,
    cls: Res<ClientRealtime>,
    mut signon: ResMut<crate::SignonState>,
    bridge: Option<Res<crate::MasterBridge>>,
    mut gate: ResMut<AuthorityInputGate>,
    actions: Res<ClientActionInbox>,
    mut stalls: Local<BacklogStalls>,
) {
    if signon.phase.is_failed() {
        gate.local_cmds_enabled = false;
        return;
    }
    let oldest = pending.iter().next().map(|(_, cmd, _)| cmd.server_time);
    let duration =
        queued_command_duration(prediction.0.history().iter().map(|row| row.cmd.server_time));
    let stalled = oldest.is_some_and(|oldest| cg.time().saturating_sub(oldest) > BACKLOG_STALL_MS)
        || duration > BACKLOG_STALL_MS;
    let stall = if stalled {
        stalls.note(
            cls.realtime(),
            pending.acknowledged.map_or(0, |seq| u64::from(seq.0)),
        )
    } else {
        stalls.clear();
        None
    };
    let reason = if actions.timed_out() {
        Some("ActionOutcomeUnknown: authority outcome deadline exceeded")
    } else if stall.is_some_and(|counted| counted >= BACKLOG_STALLS_BEFORE_FAIL) {
        Some("InputBacklogExceeded")
    } else {
        None
    };
    let Some(reason) = reason else {
        if let Some(counted) = stall {
            diag::warn!(
                Net,
                "input backlog: no authority ack in {BACKLOG_STALL_MS} ms \
                 (stall {counted}/{BACKLOG_STALLS_BEFORE_FAIL}) — re-adopting"
            );
            prediction.0.force_resync();
        }
        return;
    };
    gate.local_cmds_enabled = false;
    prediction.0.disarm();
    let match_key = crate::signon::live_match_key(bridge.as_deref());
    if let Some(bridge) = bridge {
        bridge.fail(reason);
    }
    signon.set_phase(crate::SignonPhase::Failed(
        crate::SignonFailReason::Transport {
            source: reason.to_owned(),
            stage: crate::FailStage::Transport,
            match_key,
        },
    ));
}

fn queued_command_duration(times: impl IntoIterator<Item = i32>) -> i32 {
    let mut previous = None;
    let mut duration = 0i32;
    for time in times {
        if let Some(previous) = previous {
            duration = duration.saturating_add(time.saturating_sub(previous).max(0));
        }
        previous = Some(time);
    }
    duration
}

pub fn predict_local_move(
    time: Res<Time>,
    mut clock: ResMut<ClientClock>,
    role: Res<RuntimeRole>,
    mut prediction: ResMut<ClientPredictionState>,
    template: Res<ClientCmdTemplate>,
    mut actions: ResMut<ClientActionInput>,
    gate: Res<AuthorityInputGate>,
    mut pending: ResMut<PendingClientSends>,
    proxy: Res<RemoteProxyState>,
    cls: Res<ClientRealtime>,
    cg_clock: Res<FrameClock>,
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    trace: Option<ResMut<ClientPhaseTrace>>,
) {
    push_phase(trace, "Predict");

    if *role != RuntimeRole::Replay {
        let _tick = clock.tick(time.delta_secs() * 1000.0);
    }
    if !gate.local_cmds_enabled
        || !prediction.0.is_armed()
        || !template.ready
        || !cg_clock.started()
        || !pending.has_command_capacity(cg_clock.time())
    {
        return;
    }

    let msec = cls.frametime();
    if msec <= 0 {
        return;
    }

    // bo2mp: a predicted knife lunge goes at the enemy where he is drawn
    // this frame (the same moment the command's shot sample names, so the server's rewind
    // finds him at the same spot).
    if let Some(snapshot) = presented.snapshot() {
        let alive = |id: &sim::ClientId| {
            let meta = snapshot.meta.for_client(*id);
            meta.is_some_and(|m| m.lifecycle == sim::ClientLifecycle::Alive)
        };
        let seen = snapshot
            .players
            .iter()
            .filter(|(id, _)| *id != local.0 && alive(id))
            .map(|(id, other)| {
                let p = match proxy.0.interpolate_at(*id, cg_clock.time()) {
                    ProxySample::Pose { ps, .. } | ProxySample::Starved { held: Some(ps), .. } => {
                        ps
                    }
                    ProxySample::Starved { held: None, .. } => *other,
                };
                (*id, p.origin, p.velocity)
            });
        prediction.0.set_lunge_seen(seen);
    }
    let sample = proxy.0.shot_sample(cg_clock.time());
    if let Some((seq, cmd)) = prediction
        .0
        .predict(template.cmd, ServerTime::from_ms(cg_clock.time()))
    {
        actions.consume_edges();
        pending.push(seq, cmd, sample);
    }
}

pub fn flush_bootstrap_applied(
    mut link: Option<ResMut<crate::transport::udp_session::UdpClientLink>>,
    adopted: Res<LastAdoptedSnapshot>,
    defer: Option<Res<crate::DeferBootstrapApplied>>,
) {
    if defer.is_some_and(|defer| defer.0) {
        return;
    }
    if !adopted.applied_this_frame {
        return;
    }
    if let Some(link) = link.as_mut() {
        link.flush_applied_after_adopt();
    }
}

pub fn send_pending_commands(
    link: Option<ResMut<crate::transport::udp_session::UdpClientLink>>,
    mut inbox: ResMut<ClientCommandInbox>,
    mut action_inbox: Option<ResMut<ClientActionInbox>>,
    reliable_ack: Res<ClientReliableAck>,
    mut pacer: ResMut<GameplaySendPacer>,
    cls: Res<ClientRealtime>,
    pending: Res<PendingClientSends>,
    local: Res<LocalPresentClient>,
    trace: Option<ResMut<ClientPhaseTrace>>,
) {
    push_phase(trace, "Send");
    if let Some(mut link) = link {
        if link.connection.is_none() || !link.has_entered_match() {
            return;
        }

        let actions: Vec<_> = action_inbox
            .as_deref()
            .map(|inbox| inbox.pending_for(local.0).collect())
            .unwrap_or_default();
        let now_ms = cls.realtime();
        let newest = pending.newest_seq();
        let gameplay_due = pacer.gameplay_due(now_ms, newest);

        let ack_due = reliable_ack.0 != pacer.last_sent_ack;
        let control_due = ack_due || link.has_unsent_actions(&actions);
        if !gameplay_due && !control_due {
            return;
        }

        let (cmds, retry_cursor) = if gameplay_due {
            pacer.command_window(&pending)
        } else {
            (Vec::new(), None)
        };
        let window = &cmds;
        if let Err(e) = link.send_commands(window, &actions, reliable_ack.0) {
            diag::warn!(Net, "udp send cmds: {e}");
            return;
        }
        if let Some(inbox) = action_inbox.as_deref_mut() {
            for action in &actions {
                inbox.mark_dispatched(local.0, sim::action_request_id(action));
            }
        }
        if gameplay_due {
            pacer.note_sent(now_ms, newest, reliable_ack.0);
            pacer.retry_cursor = retry_cursor;
        } else {
            pacer.last_sent_ack = reliable_ack.0;
        }
        if control_due {
            pacer.last_control_ms = Some(now_ms);
        }
        return;
    }

    if !pacer.gameplay_due(cls.realtime(), pending.newest_seq()) {
        return;
    }
    pacer.note_sent(cls.realtime(), pending.newest_seq(), reliable_ack.0);
    for (seq, cmd, sample) in pending.iter().copied() {
        inbox.push(local.0, Some(seq), cmd, Some(sample));
    }
}

pub const MAX_GAMEPLAY_SENDS_PER_SEC: i32 = 60;

pub const GAMEPLAY_SEND_INTERVAL_MS: i32 =
    (1000 + MAX_GAMEPLAY_SENDS_PER_SEC - 1) / MAX_GAMEPLAY_SENDS_PER_SEC;

#[derive(Resource, Debug, Default)]
pub struct GameplaySendPacer {
    last_send_ms: Option<i32>,

    last_sent_seq: Option<CmdSeq>,

    retry_cursor: Option<CmdSeq>,

    last_sent_ack: u16,

    last_control_ms: Option<i32>,
}

impl GameplaySendPacer {
    fn command_window(
        &self,
        pending: &PendingClientSends,
    ) -> (
        Vec<(CmdSeq, UserCmd, sim::ShotSampleProvenance)>,
        Option<CmdSeq>,
    ) {
        let (mut fresh, retries): (Vec<_>, Vec<_>) = pending
            .iter()
            .copied()
            .partition(|(seq, _, _)| self.last_sent_seq.is_none_or(|last| *seq > last));
        let mut cursor = self.retry_cursor;
        // Repeat the oldest command on every send to unblock the ordered stream,
        // plus a small rotating window. A full 16-command repeat at 60 Hz floods
        // high-latency links with commands that are already in flight.
        if let Some(oldest) = retries.first() {
            fresh.push(*oldest);
            let rotating = &retries[1..];
            if !rotating.is_empty() {
                let start = rotating
                    .iter()
                    .position(|(seq, _, _)| cursor.is_none_or(|last| *seq > last))
                    .unwrap_or(0);
                for row in rotating
                    .iter()
                    .cycle()
                    .skip(start)
                    .take(rotating.len().min(3))
                {
                    cursor = Some(row.0);
                    fresh.push(*row);
                }
            }
        }
        fresh.sort_by_key(|(seq, _, _)| *seq);
        (fresh, cursor)
    }

    fn gameplay_due(&self, now_ms: i32, newest: Option<CmdSeq>) -> bool {
        if newest.is_none() {
            return false;
        }
        match self.last_send_ms {
            None => true,
            Some(last) => now_ms.saturating_sub(last) >= GAMEPLAY_SEND_INTERVAL_MS,
        }
    }

    fn note_sent(&mut self, now_ms: i32, newest: Option<CmdSeq>, ack: u16) {
        self.last_send_ms = Some(now_ms);
        self.last_sent_seq = newest;
        self.last_sent_ack = ack;
    }
}

#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct ClientReliableAck(pub u16);

/// BO2MP_KILLCAMLOG=1 prints one line per presented frame while the kill
/// replay plays, plus a line for every body that moved over 40 units between
/// two frames. Grep the log for "killcamlog:".
/// The MW2 copycat bind check, moved here from the old MW2 game-mode crate.
fn copycat_weapnext_bind_active(pm_type: i32, in_killcam: bool) -> bool {
    in_killcam || pm_type >= 10
}

fn killcam_log_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BO2MP_KILLCAMLOG").is_some_and(|v| v != "0"))
}

const KILLCAM_JUMP_UNITS: f32 = 40.0;

#[derive(Default)]
pub struct KillcamLogState {
    active: bool,
    check_jumps: bool,
    frame: u32,
    last_origin: HashMap<i32, [f32; 3]>,
}

impl KillcamLogState {
    fn note_frame(
        &mut self,
        archived: bool,
        clock_ms: i32,
        body_ms: i32,
        proxy: &crate::RemoteProxy,
    ) {
        if archived != self.active {
            diag::info!(
                Net,
                "killcamlog: {} clock={clock_ms}",
                if archived { "replay begins" } else { "replay ends" }
            );
            self.frame = 0;
        }
        // The frame that ends the replay is still checked for jumps.
        if archived {
            self.frame += 1;
            let left = proxy.snapshot_at(clock_ms).map(|s| s.tick.0 as i32 * AUTHORITY_MS);
            let right = proxy
                .snapshot_after(clock_ms)
                .map(|s| s.tick.0 as i32 * AUTHORITY_MS);
            let effective = clock_ms.saturating_sub(crate::PROXY_DELAY_MS);
            let alpha = match (left, right) {
                (Some(l), Some(r)) if r > l => {
                    Some(((effective - l) as f32 / (r - l) as f32).clamp(0.0, 1.0))
                }
                _ => None,
            };
            diag::info!(
                Net,
                "killcamlog: frame={} replay_t={clock_ms} effective={effective} body_t={body_ms} left={left:?} right={right:?} alpha={alpha:?}",
                self.frame
            );
        }
        self.check_jumps = archived || self.active;
        self.active = archived;
    }

    fn note_entity(
        &mut self,
        number: i32,
        client_num: i32,
        e_type: u8,
        origin: [f32; 3],
        via_proxy: bool,
    ) {
        let was = self.last_origin.insert(number, origin);
        let Some(was) = was else {
            return;
        };
        if !self.check_jumps {
            return;
        }
        let d = ((origin[0] - was[0]).powi(2)
            + (origin[1] - was[1]).powi(2)
            + (origin[2] - was[2]).powi(2))
        .sqrt();
        if d > KILLCAM_JUMP_UNITS {
            diag::info!(
                Net,
                "killcamlog: jump frame={} ent={number} client={client_num} e_type={e_type} d={d:.1} from=({:.0} {:.0} {:.0}) to=({:.0} {:.0} {:.0}) via={}",
                self.frame,
                was[0],
                was[1],
                was[2],
                origin[0],
                origin[1],
                origin[2],
                if via_proxy { "proxy_pose" } else { "present_pose" }
            );
        }
    }
}

pub fn publish_presented(
    clock: Res<ClientClock>,
    cg_clock: Res<FrameClock>,
    mut entities: Query<&mut crate::CEntityRuntime>,
    role: Res<RuntimeRole>,
    prediction: Res<ClientPredictionState>,
    last_adopted: Res<LastAdoptedSnapshot>,
    proxy: Res<RemoteProxyState>,
    local: Res<LocalPresentClient>,
    has_world: Option<Res<HasWorld>>,
    mut presented: ResMut<PresentedSnapshot>,
    mut present_census: ResMut<PresentLocalCensus>,
    phase: Option<ResMut<crate::UpdatePhaseCensus>>,
    trace: Option<ResMut<ClientPhaseTrace>>,
    mut killcam_log: Local<KillcamLogState>,
) {
    let publish_started = std::time::Instant::now();
    struct PublishStamp<'a> {
        started: std::time::Instant,
        phase: Option<ResMut<'a, crate::UpdatePhaseCensus>>,
    }
    impl Drop for PublishStamp<'_> {
        fn drop(&mut self) {
            let ms = self.started.elapsed().as_secs_f32() * 1000.0;
            if let Some(phase) = self.phase.as_mut() {
                phase.publish_presented_ms = Some(ms);
            }
        }
    }
    let _publish_stamp = PublishStamp {
        started: publish_started,
        phase,
    };
    push_phase(trace, "Present");

    if has_world.is_some_and(|world| !world.0) {
        presented.clear();
        *present_census = PresentLocalCensus {
            choice: Some("no_next"),
            snapshot_delta_time: None,
            predicted_delta_time: prediction.0.predicted_local().map(|ps| ps.delta_time),
        };
        return;
    }
    let Some(snap_arc) = last_adopted.next_snap.clone() else {
        *present_census = PresentLocalCensus {
            choice: Some("no_next"),
            snapshot_delta_time: None,
            predicted_delta_time: prediction.0.predicted_local().map(|ps| ps.delta_time),
        };
        return;
    };
    let archived = snap_arc
        .players
        .iter()
        .any(|(id, ps)| *id == local.0 && !ps.is_live_frame());
    // A replay draws the world from the delayed snapshot, but the viewer's
    // own screen (killcam bands, killer card) comes from the newest one:
    // real BO2 sends the archived state and the viewer's HUD in one
    // snapshot, so the bands appear on the replay's first frame.
    let snap_arc = if archived {
        match proxy.0.snapshot_at(cg_clock.time()) {
            Some(delayed) if delayed.tick != snap_arc.tick => {
                let newest = snap_arc.meta.for_client(local.0);
                let mut snap = (*delayed).clone();
                if let (Some(newest), Some((_, meta))) = (
                    newest,
                    snap.meta.clients.iter_mut().find(|(c, _)| *c == local.0),
                ) {
                    meta.client_dvars = newest.client_dvars.clone();
                    meta.killcam_hud = newest.killcam_hud.clone();
                }
                Arc::new(snap)
            }
            Some(delayed) => delayed,
            None => snap_arc,
        }
    } else {
        snap_arc
    };
    let snapshot = snap_arc.as_ref();
    let snapshot_local = snapshot
        .players
        .iter()
        .find(|(id, _)| *id == local.0)
        .map(|(_, ps)| ps);
    let predicted_ps = prediction.0.predicted_local().copied();
    let previous = presented.player(local.0).copied();
    let armed = prediction.0.is_armed();
    let frame_interpolation = clock.accumulator_ms / AUTHORITY_MS as f32;
    let interpolation_previous = if *role == RuntimeRole::Replay {
        last_adopted
            .snap
            .clone()
            .filter(|previous| previous.tick.0 < snapshot.tick.0)
    } else {
        None
    };
    let interpolated = interpolation_previous.as_deref().and_then(|previous| {
        let previous_local = previous
            .players
            .iter()
            .find(|(id, _)| *id == local.0)
            .map(|(_, ps)| ps)?;
        let next_local = snapshot_local?;
        (previous_local.is_live_frame() && next_local.is_live_frame())
            .then(|| interpolate_player_state(previous_local, next_local, frame_interpolation))
    });
    *present_census = PresentLocalCensus {
        choice: Some(if interpolated.is_some() {
            "interpolated"
        } else if armed {
            PresentedSnapshot::presentation_local_choice(
                snapshot_local,
                predicted_ps.as_ref(),
                previous.as_ref(),
            )
        } else {
            PresentedSnapshot::presentation_local_choice_unarmed(snapshot_local)
        }),
        snapshot_delta_time: snapshot_local.map(|ps| ps.delta_time),
        predicted_delta_time: predicted_ps.as_ref().map(|ps| ps.delta_time),
    };
    let mut predicted = match interpolated {
        Some(interpolated) => interpolated,
        None if armed => PresentedSnapshot::presentation_local_player_state(
            snapshot_local,
            predicted_ps,
            previous,
        ),
        None => PresentedSnapshot::presentation_local_player_state_unarmed(snapshot_local),
    };
    let render_time_ms = if *role == RuntimeRole::Replay {
        (snapshot.tick.0 as i32 + 1) * AUTHORITY_MS + clock.accumulator_ms as i32
    } else {
        cg_clock.time()
    };

    let body_time_ms = if *role == RuntimeRole::Replay {
        snapshot.tick.0 as i32 * AUTHORITY_MS - AUTHORITY_MS + clock.accumulator_ms as i32
    } else if archived {
        cg_clock.time().saturating_sub(crate::PROXY_DELAY_MS)
    } else {
        cg_clock.time().saturating_sub(AUTHORITY_MS)
    };

    let killcam_log_on = killcam_log_enabled();
    if killcam_log_on {
        killcam_log.note_frame(archived, cg_clock.time(), body_time_ms, &proxy.0);
    }

    let mut remote_poses = HashMap::new();
    let mut remote_provenance = HashMap::new();
    for (id, _) in &snapshot.players {
        if *id == local.0 && !archived {
            continue;
        }
        match proxy.0.interpolate_at(*id, render_time_ms) {
            ProxySample::Pose { ps, provenance } => {
                remote_poses.insert(*id, ps);
                remote_provenance.insert(*id, provenance);
            }
            ProxySample::Starved { held, provenance } => {
                if let Some(held_ps) = held {
                    remote_poses.insert(*id, held_ps);
                }
                remote_provenance.insert(*id, provenance);
            }
        }
    }

    if archived {
        if let Some(seat) = remote_poses.remove(&local.0) {
            predicted = seat;
        }
    }
    for mut runtime in &mut entities {
        runtime.presented_player = None;
        let mut via_proxy = false;
        if i16::from(runtime.pose_e_type) == entity_t6::et::PLAYER
            && let Ok(number) = u32::try_from(runtime.next_state.client_num)
            && let Some(ps) = remote_poses.get(&sim::ClientId(number))
        {
            via_proxy = true;
            runtime.origin = ps.origin;
            runtime.angles = ps.viewangles;
            let provenance = remote_provenance[&sim::ClientId(number)];
            let sample_time = match provenance.outcome {
                crate::PresentationSampleOutcome::Exact { snapshot }
                | crate::PresentationSampleOutcome::Starved {
                    held_snapshot: Some(snapshot),
                    ..
                } => snapshot.0 as i32 * AUTHORITY_MS,
                crate::PresentationSampleOutcome::Interpolated { .. } => {
                    provenance.effective_time.0
                }
                crate::PresentationSampleOutcome::Starved {
                    held_snapshot: None,
                    ..
                } => unreachable!("a proxy pose always has a source snapshot"),
            };
            runtime.presented_player = Some((
                crate::player_state_to_entity_state(sim::ClientId(number), ps),
                sample_time,
            ));
        } else {
            runtime.present_pose(body_time_ms);
        }
        if killcam_log_on {
            killcam_log.note_entity(
                runtime.next_state.number,
                i32::from(runtime.next_state.client_num),
                runtime.pose_e_type,
                runtime.origin,
                via_proxy,
            );
        }
    }

    let mut predicted_projectiles = Vec::new();
    prediction
        .0
        .world()
        .visit_projectiles(|projectile| predicted_projectiles.push(*projectile));
    let merged_projectiles = if archived {
        merge_presented_projectiles(snapshot.projectiles.as_slice(), &[], local.0)
    } else {
        merge_presented_projectiles(
            snapshot.projectiles.as_slice(),
            &predicted_projectiles,
            local.0,
        )
    };
    let view_offset = if archived {
        [0.0; 3]
    } else {
        prediction
            .0
            .predicted_error()
            .offset_for_pm_type(predicted.pm_type)
    };
    presented.publish_proxied(
        snap_arc,
        local.0,
        predicted,
        remote_poses,
        remote_provenance,
    );
    // bo2mp gun brain: BO2's viewmodel reads the predicted player state whole, so with the
    // predicted local state its T6 view values come from the predicted brain too (a held
    // previous state keeps the view values it had).
    match present_census.choice {
        Some("predicted") if !archived => presented.set_local_t6_view(
            prediction
                .0
                .world()
                .client_meta(local.0)
                .and_then(|meta| meta.t6_predicted_view()),
        ),
        Some("previous") if !archived => {}
        _ => presented.set_local_t6_view(None),
    }
    presented.set_presented_projectiles(merged_projectiles);
    presented.set_view_offset(view_offset);
    // bo2zm: the camera eases over a stair step (Black Ops II).
    presented.set_view_z_drop(if archived {
        0.0
    } else {
        prediction.0.view_z().camera_drop(cg_clock.time())
    });
    presented.set_snapshot_interpolation(interpolation_previous, frame_interpolation);
    presented.set_trajectory_sample(
        archived.then_some(body_time_ms),
        archived
            .then(|| proxy.0.snapshot_after(cg_clock.time()))
            .flatten(),
    );
}

pub fn reset_cgame_on_match_torn_down(
    mut torn: MessageReader<MatchTornDown>,
    mut clock: ResMut<FrameClock>,
    mut active: ResMut<GameActive>,
    mut join: ResMut<GameJoinCensus>,
    mut adopted: ResMut<LastAdoptedSnapshot>,
    mut received: ResMut<ReceivedTicks>,
    (mut prediction, mut sends, mut signon, mut admission): (
        ResMut<ClientPredictionState>,
        ResMut<PendingClientSends>,
        ResMut<crate::SignonState>,
        ResMut<crate::ClientAdmission>,
    ),
    mut client_clock: ResMut<ClientClock>,
    mut proxy: ResMut<RemoteProxyState>,
    mut presented: ResMut<PresentedSnapshot>,
    mut present_census: ResMut<PresentLocalCensus>,
    mut select: ResMut<WeaponSelect>,
    (mut entity_events, mut pellet_fx, mut debris, mut entity_event_cursor): (
        ResMut<PendingPresentedEntityEvents>,
        ResMut<PendingPelletFx>,
        ResMut<PendingDebris>,
        ResMut<crate::EntityEventCursor>,
    ),
    (mut reliable_ack, mut actions, mut events, mut scores): (
        ResMut<ClientReliableAck>,
        Option<ResMut<ClientActionInbox>>,
        ResMut<Messages<ReliableControlEvent>>,
        ResMut<crate::Scoreboard>,
    ),
    mut birth: Option<ResMut<CEntityBirthCensus>>,
    mut pacer: ResMut<GameplaySendPacer>,
) {
    if torn.read().count() == 0 {
        return;
    }
    *signon = crate::SignonState::default();
    *admission = crate::ClientAdmission::default();
    clock.reset();
    active.0 = false;
    join.clear();
    *adopted = LastAdoptedSnapshot::default();
    received.0.clear();
    prediction.0.disarm();

    *sends = PendingClientSends::default();
    *client_clock = ClientClock::default();
    *pacer = GameplaySendPacer::default();
    *proxy = RemoteProxyState::default();
    presented.clear();
    *present_census = PresentLocalCensus::default();
    *select = WeaponSelect::default();

    *entity_events = PendingPresentedEntityEvents::default();
    pellet_fx.0.clear();
    debris.0.clear();
    *entity_event_cursor = crate::EntityEventCursor::default();

    *reliable_ack = ClientReliableAck::default();
    events.clear();
    *scores = crate::Scoreboard::default();
    if let Some(actions) = actions.as_mut() {
        actions.clear();
    }
    if let Some(birth) = birth.as_mut() {
        **birth = CEntityBirthCensus::default();
    }
    perf::cgame_hold("no_next", 0);
}

pub fn register_client_runtime(app: &mut App) {
    app.add_systems(
        Update,
        enforce_client_work_limits
            .before(predict_local_move)
            .after(reconcile_prediction),
    );
    app.init_resource::<crate::SignonState>()
        .init_resource::<crate::ClientAdmission>()
        .init_resource::<ClientPredictionState>()
        .init_resource::<ClientClock>()
        .init_resource::<FrameClock>()
        .init_resource::<GameActive>()
        .init_resource::<GameJoinCensus>()
        .init_resource::<ClientRealtime>()
        .init_resource::<ReceivedTicks>()
        .init_resource::<LastAdoptedSnapshot>()
        .init_resource::<PresentLocalCensus>()
        .init_resource::<PendingPresentedEntityEvents>()
        .init_resource::<PendingPelletFx>()
        .init_resource::<PendingDebris>()
        .init_resource::<PendingClientSends>()
        .init_resource::<ClientCmdTemplate>()
        .init_resource::<WeaponSelect>()
        .init_resource::<LocationCursor>()
        .init_resource::<ClientReliableAck>()
        .init_resource::<GameplaySendPacer>()
        .add_message::<ReliableControlEvent>()
        .add_message::<frame::MatchInstalled>()
        .add_message::<frame::MatchTornDown>()
        .init_resource::<RemoteProxyState>()
        .init_resource::<ClientShotSamples>()
        .init_resource::<crate::Scoreboard>()
        .add_message::<crate::SvcLocalSound>()
        .add_message::<crate::SvcCardSlotCmd>()
        .add_message::<crate::SvcOpenMenuCmd>()
        .add_message::<crate::SvcHudSplash>()
        .add_message::<crate::SvcGameNotify>();
    crate::client::frame_census::register_update_phase_census(app);
    app.configure_sets(
        Update,
        (
            crate::PresentedPublished
                .in_set(ClientSet::Present)
                .after(publish_presented),
            frame::ClassEquipResolved
                .in_set(ClientSet::Present)
                .after(publish_presented),
        ),
    )
    .add_systems(
        Update,
        (
            advance_cls_realtime.in_set(ClientSet::Load),
            reset_cgame_on_match_torn_down
                .in_set(ClientSet::Load)
                .after(frame::SessionSwapApplied),
            advance_cg_frame_clock
                .in_set(ClientSet::Reconcile)
                .after(reconcile_prediction),
            receive_ticks.in_set(ClientSet::Receive),
            crate::signon::drive_client_admission_facts
                .in_set(ClientSet::Receive)
                .after(receive_ticks),
            crate::signon::drive_signon
                .in_set(ClientSet::Receive)
                .after(crate::signon::drive_client_admission_facts),
            crate::signon::drive_map_loaded
                .in_set(ClientSet::Receive)
                .after(crate::signon::drive_client_admission_facts),
            reconcile_prediction.in_set(ClientSet::Reconcile),
            flush_bootstrap_applied
                .in_set(ClientSet::Reconcile)
                .after(reconcile_prediction),
            apply_weapon_switch_requests
                .in_set(ClientSet::Input)
                .before(sample_client_input),
            sample_client_input.in_set(ClientSet::Input),
            predict_local_move.in_set(ClientSet::Predict),
            send_pending_commands.in_set(ClientSet::Send),
            publish_presented.in_set(ClientSet::Present),
        ),
    );
}

pub fn register_listen_prediction_arm(app: &mut App) {
    app.add_systems(Update, arm_listen_prediction.in_set(ClientSet::Load));
}
