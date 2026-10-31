//! The netcode: the host's fixed-rate authority, the client's input and prediction, the
//! snapshot transport between them, and the sign-on / match lifecycle that joins the two.
//!
//! Everything other crates call is re-exported here, grouped by the part of the netcode it
//! belongs to.
// The modules, one per part of the netcode:
pub mod host_tick; // the host's tick, command inbox and action ledger
pub mod client; // input, prediction, interpolation and presentation
pub mod identity_slips; // named holes the netcode knows it has
mod feel_trace; // per-frame diagnostics lines
pub mod parse_scores; // the scoreboard command text
pub mod app_wiring; // `NetworkingPlugin`, the app wiring for each runtime role
pub mod viewer_policy; // who sees what: seats and killcams
pub mod snapshot_apply; // what a snapshot may overwrite on the client
pub mod role_wiring_check; // which role runs which half
pub mod tick_sets; // system sets for the host and the client
pub mod match_flow; // the match / peer state machines
pub mod join_walk; // joining a match and staying in it
pub mod notice_msgs; // print and disconnect notices
pub mod card_msgs; // player cards and open-menu commands
pub mod board_msgs; // scoreboard commands
pub mod local_sound_msgs; // local sound commands
pub mod net_links; // snapshots, packets, links

/// bo2zm: a lobby member's id, as the host's kick names it.
pub use master_protocol::RoomOccupantId;

// Host side: the tick, the inboxes and the action ledger.
pub use host_tick::action_books::ActionAcceptanceResult;
pub use host_tick::action_books::ActionRequestCounter;
pub use host_tick::action_books::PlayerActionBook;
pub use host_tick::action_books::RECALLED_COMMANDS_PER_RECEIVER_CAP;
pub use host_tick::peer_mail::ActionQueueRefusal;
pub use host_tick::peer_mail::HOST_TICK_FREQUENCY;
pub use host_tick::peer_mail::HOST_STEP_MILLIS;
pub use host_tick::peer_mail::HostTimeKeeper;
pub use host_tick::peer_mail::PlayerActionIntake;
pub use host_tick::peer_mail::PlayerMoveIntake;
pub use host_tick::peer_mail::CollectedMoves;
pub use host_tick::peer_mail::PLAYER_ACTION_QUEUE_LIMIT;
pub use host_tick::peer_mail::MOVE_RESEND_WINDOW;
pub use host_tick::peer_mail::execute_steady_host_loop;
pub use host_tick::peer_mail::HostInstant;
pub use host_tick::tick_systems::host_stage_upkeep;
pub use host_tick::tick_systems::host_due_for_step;
pub use host_tick::tick_systems::HostInputValve;
pub use host_tick::tick_systems::HostLoadingPause;
pub use host_tick::tick_systems::HostStageTally;
pub use host_tick::tick_systems::HostStageTrail;
pub use host_tick::tick_systems::HostSimulationWorld;
pub use host_tick::tick_systems::PlayerShotOrigins;
pub use host_tick::tick_systems::DumpConfigurationChangeLog;
pub use host_tick::tick_systems::DeathLogDumper;
pub use host_tick::tick_systems::GrantLogDumper;
pub use host_tick::tick_systems::StepUpdateTally;
pub use host_tick::tick_systems::LocalHostFanoutTally;
pub use host_tick::tick_systems::LinkDiagnosticsReport;
pub use host_tick::tick_systems::StagedReceipts;
pub use host_tick::tick_systems::StagedHostTickInput;
pub use host_tick::tick_systems::StagedStepOutput;
pub use host_tick::tick_systems::HostStepNumber;
pub use host_tick::tick_systems::HostStepOutput;

// Client side: the frame clock, input, prediction and the drive systems.
pub use client::frame_tally::PlayerStageTally;
pub use client::frame_tally::OVERLAY_STAGE_TOTAL;
pub use client::frame_tally::FrameUpdateTally;
pub use client::frame_clock::FrameClock;
pub use client::frame_clock::GameActive;
pub use client::frame_clock::GameJoinCensus;
pub use client::local_input::fold_aim_input;
pub use client::local_input::assemble_input_record;
pub use client::local_input::PlayerControlSnapshot;
pub use client::local_input::frame_time_msec;
pub use client::local_input::resting_command;
pub use client::local_input::pressed_frame_millis;
pub use client::local_input::INPUT_SPAN_MILLIS_LIMIT;
pub use client::local_input::aim_angles_from_degrees;
pub use client::local_input::ViewAimAccumulator;
pub use client::run_ahead::ReceiptLookup;
pub use client::run_ahead::LocalForecast;
pub use client::run_ahead::InputCommandNumber;
pub use client::run_ahead::DEFAULT_MOVE_LOG_LIMIT;
pub use client::run_ahead::UNCONFIRMED_STATE_CAP;
pub use client::run_ahead::ForecastMoveLog;
pub use client::run_ahead::ForecastMoveEntry;
pub use client::run_ahead::ForecastCounters;
pub use client::run_ahead::CorrectionResult;
pub use client::run_ahead::ground_entity_kind;
pub use crate::client::{correction_fade::ForecastDrift, realtime::ClientRealtime};
pub use client::drive::step_presentation_clock;
pub use client::drive::advance_receiver_wall_clock;
pub use client::drive::enable_local_host_forecast;
pub use client::drive::PlayerTimeStepper;
pub use client::drive::PlayerMoveBlueprint;
pub use client::drive::ReceiverStageLog;
pub use client::drive::ClientGuessState;
pub use client::drive::PlayerGuaranteedReceipt;
pub use client::drive::TimerStepResult;
pub use client::drive::cycle_weapon_select;
pub use client::drive::follow_held_weapon_select;
pub use client::drive::NewestTakenSnapshot;
pub use client::drive::forecast_needs_rearm;
pub use client::drive::LocationCursor;
pub use client::drive::QueuedOutgoingPlayerMessages;
pub use client::drive::PendingDebris;
pub use client::drive::QueuedPelletEffect;
pub use client::drive::StagedShownEntityEvents;
pub use client::drive::forecast_own_move;
pub use client::drive::expose_shown_frame;
pub use client::drive::ingest_host_steps;
pub use client::drive::ArrivedStepQueue;
pub use client::drive::correct_forecast;
pub use client::drive::enroll_receiver_systems;
pub use client::drive::enroll_hosted_forecast_hookup;
pub use client::drive::GuaranteedSimEvent;
pub use client::drive::OtherPlayerSmootherHolder;
pub use client::drive::poll_receiver_controls;
pub use client::drive::transmit_queued_inputs;
pub use client::drive::WeaponSelect;

// What the client draws from.
pub use client::presentation::entity_records::EntityModelHandle;
pub use client::presentation::entity_records::EntityEffectHandle;
pub use client::presentation::entity_records::ClientEntityPoseState;
pub use client::presentation::entity_records::cadaver_slot_to_object_state;
pub use client::presentation::entity_records::BlendedPoseNow;
pub use client::presentation::entity_records::DEAD_ENTITY_FLAG;
pub use client::presentation::entity_records::SNAP_MOVE_FLAG;
pub use client::presentation::entity_records::avatar_entity_row;
pub use client::presentation::entity_records::PlayerDrawGate;
pub use client::presentation::entity_records::peer_avatar_submission_class;
pub use client::presentation::entity_records::remote_body_draws;
pub use client::presentation::entity_records::other_player_pose;
pub use client::presentation::entity_records::OtherPlayerDrawRoute;
pub use client::presentation::entity_records::OtherPlayerPoseReading;
pub use client::presentation::entity_slots::ClientEntityRecord;
pub use client::presentation::entity_slots::EntitySpawnTally;
pub use client::presentation::entity_slots::EntityTableCells;
pub use client::presentation::entity_slots::ENTITY_NUMBER_COUNT;
pub use client::presentation::entity_slots::enroll_receiver_objects;
pub use client::presentation::entity_slots::mirror_entities;
pub use client::presentation::event_raise::InstalledEntityEventTraversal;
pub use client::presentation::event_raise::RoutedEntityEvent;
pub use client::presentation::event_raise::EntityBoltImpact;
pub use client::presentation::event_raise::BulletStrike;
pub use client::presentation::event_raise::EntityEarthquake;
pub use client::presentation::event_raise::ShellCaseOut;
pub use client::presentation::event_raise::EntityEventReadPos;
pub use client::presentation::event_raise::SoundCueAt;
pub use client::presentation::event_raise::BlastAt;
pub use client::presentation::event_raise::EntityFireMelee;
pub use client::presentation::event_raise::GrenadeBounceAt;
pub use client::presentation::event_raise::MeleeBloodAt;
pub use client::presentation::event_raise::StepSoundAt;
pub use client::presentation::event_raise::EntityKillNotice;
pub use client::presentation::event_raise::EffectAt;
pub use client::presentation::event_raise::EntityRemoteMissileBoost;
pub use client::presentation::event_raise::EntitySightsLowered;
pub use client::presentation::event_raise::EntityRumble;
pub use client::presentation::event_raise::EntityShieldBlast;
pub use client::presentation::event_raise::GunFiredAt;
pub use client::presentation::event_raise::KillcamFxTransition;
pub use client::presentation::event_raise::enroll_object_occurrence_routing;
pub use client::presentation::event_raise::UnhandledEntityEvents;
pub use client::presentation::event_table::EntityEventHandlerKind;
pub use client::presentation::event_table::EntityEventTableEntry;
pub use client::presentation::event_table::EVENT_ROUTE_TABLE;
pub use client::presentation::event_table::event_route_of;
pub use client::presentation::shown_frame::FirstPersonEventHints;
pub use client::presentation::shown_frame::ViewHandEntry;
pub use client::presentation::shown_frame::ViewHandRawReading;
pub use client::presentation::shown_frame::HostViewClient;
pub use client::presentation::shown_frame::ShownWorldFrame;
pub use client::presentation::shown_frame::OwnViewerTally;
pub use client::presentation::shown_frame::ViewweaponAim;
pub use client::presentation::seen_throws::throws_shown_for;
pub use client::presentation::seen_throws::combine_shown_missiles;
pub use client::presentation::seen_throws::ShownProjectile;
pub use client::remote_buffer::PoseFillIn;
pub use client::remote_buffer::CONSTANT_LAG_POLICY_GENERATION;
pub use client::remote_buffer::SmoothingSampleVerdict;
pub use client::remote_buffer::SmoothingSampleOrigin;
pub use client::remote_buffer::SmoothingInstant;
pub use client::remote_buffer::SMOOTHER_FRAME_CAPACITY;
pub use client::remote_buffer::STANDIN_LAG_MILLIS;
pub use client::remote_buffer::SmootherStrategy;
pub use client::remote_buffer::SmootherPolicyGeneration;
pub use client::remote_buffer::SmootherReading;
pub use client::remote_buffer::RemoteBufferShortfall;
pub use client::remote_buffer::OtherPlayerSmoother;

// Sign-on, the match lifecycle and the reconciliation contract.
pub use identity_slips::LinkDeficiency;
pub use identity_slips::LinkDeficiencyOrigin;
pub use identity_slips::LinkIdentityDeficits;
pub use identity_slips::ScriptEventKind;
pub use crate::app_wiring::NetworkingPlugin;
pub use viewer_policy::cam_view::LiveDeathReplays;
pub use viewer_policy::cam_view::stamp_seat_into_world_state;
pub use viewer_policy::cam_view::deathreplay_secs_to_millis;
pub use viewer_policy::cam_view::DeathReplayRun;
pub use viewer_policy::cam_view::read_deathreplay_seat;
pub use viewer_policy::cam_view::ViewerSeatReading;
pub use viewer_policy::cam_view::state_and_reading_for_viewer;
pub use viewer_policy::cam_view::state_for_viewer;
pub use snapshot_apply::place_of_tick;
pub use snapshot_apply::CorrectionCutoff;
pub use snapshot_apply::CorrectionStrategy;
pub use snapshot_apply::IDENTITY_RULES;
pub use snapshot_apply::IdentityRuleEntry;
pub use snapshot_apply::AFTEREFFECT_RULES;
pub use snapshot_apply::AftereffectRuleEntry;
pub use snapshot_apply::AftereffectRerunPolicy;
pub use snapshot_apply::FramePosition;
pub use snapshot_apply::STATE_RULES;
pub use snapshot_apply::StateCategory;
pub use snapshot_apply::StateRuleEntry;
pub use tick_sets::SERVER_STAGES;
pub use tick_sets::ServerStage;
pub use tick_sets::CLIENT_STAGES;
pub use tick_sets::ClientStep;
pub use tick_sets::order_server_side_stages;
pub use tick_sets::order_player_side_stages;
pub use tick_sets::ShownStatePosted;
pub use match_flow::loadout_pick_permitted;
pub use match_flow::PlayerRoundCore;
pub use match_flow::acknowledge_tagged_scene_loaded;
pub use match_flow::AbortStep;
pub use match_flow::HostRoundInstall;
pub use match_flow::HostRoundCore;
pub use match_flow::HostRoundConsequence;
pub use match_flow::HostRoundOccurrence;
pub use match_flow::HostMapLoaded;
pub use match_flow::level_ready_agrees_with_host;
pub use match_flow::teardown_hits_epoch;
pub use match_flow::teardown_hits_key;
pub use match_flow::GameRoundStage;
pub use match_flow::PeerEntryGate;
pub use match_flow::PlayerJoinStage;
pub use match_flow::LoadStageKind;
pub use match_flow::LoadStageTimer;
pub use match_flow::SessionInstallParams;
pub use match_flow::SessionStateCore;
pub use match_flow::GameSessionNotice;
pub use match_flow::SessionAbortRecord;
pub use match_flow::SessionSnapshotView;
pub use join_walk::PlayerEntryTicket;
pub use join_walk::HoldTransferInstalled;
pub use join_walk::HoldMapLoadedSignal;
pub use join_walk::join_stage_now;
pub use join_walk::world_ready_held_back;
pub use join_walk::current_round_key;
pub use join_walk::GameRoundSerial;
pub use join_walk::stage_of_link;
pub use join_walk::JoinFailureCause;
pub use join_walk::JoinSequenceStep;
pub use join_walk::JoinProgressRecord;

// Server commands carried beside the snapshot.
pub use notice_msgs::receiver_display_text;
pub use notice_msgs::StagedAnnouncements;
pub use notice_msgs::HOST_MSG_LEAVE_ALERT;
pub use notice_msgs::HOST_MSG_TEXT_LINE;
pub use notice_msgs::SVC_PRINT_BOLD;
pub use notice_msgs::HostGameAnnouncement;
pub use card_msgs::StagedCardUpdate;
pub use card_msgs::HOST_MSG_NAMEPLATE;
pub use card_msgs::HOST_MSG_MENU_OPEN;
pub use card_msgs::HostCardSlotMsg;
pub use card_msgs::CardSlotCommand;
pub use card_msgs::HostScreenFlash;
pub use card_msgs::HostMenuOpenMsg;
pub use card_msgs::MenuOpenCommand;
pub use board_msgs::compose_standings_command;
pub use board_msgs::compose_standings_from_state;
pub use board_msgs::read_board_command;
pub use board_msgs::StagedScoreboardSend;
pub use board_msgs::Scoreboard;
pub use board_msgs::SCOREBOARD_POLL_MILLIS;
pub use board_msgs::HOST_MSG_SCOREBOARD;
pub use local_sound_msgs::StagedHostSounds;
pub use local_sound_msgs::HOST_SOUND_START_TAG;
pub use local_sound_msgs::HOST_SOUND_HALT_TAG;
pub use local_sound_msgs::HostLocalSoundMsg;
pub use local_sound_msgs::HostSoundCue;

// Transport: snapshots on the wire, the packet protocol, the UDP link.
pub use net_links::peer_baseline::ConfirmedReferenceTable;
pub use net_links::peer_baseline::LossySimulatedQueue;
pub use net_links::snapshot_history::archive_attainable_ticks;
pub use net_links::snapshot_history::HISTORY_SIZE_LIMIT_BYTES;
pub use net_links::snapshot_history::HISTORY_CACHED_RECEIVER_LIMIT;
pub use net_links::snapshot_history::ARCHIVE_LOOKUP_WINDOW_TICKS;
pub use net_links::snapshot_history::HISTORY_FRAME_CAPACITY;
pub use net_links::snapshot_history::VAULT_STEP_MS;
pub use net_links::snapshot_history::StoredHistoryFrame;
pub use net_links::snapshot_history::HistoryQueryResult;
pub use net_links::snapshot_history::HistoryStoreStats;
pub use net_links::snapshot_history::SnapshotVault;
pub use crate::net_links::match_handoff::TransferReceiptMsg;
pub use net_links::snapshot_diff::ProjectileChangeRecord;
pub use net_links::snapshot_diff::FrameDiffReader;
pub use net_links::snapshot_diff::FrameDiff;
pub use net_links::snapshot_diff::FrameDiffWriter;
pub use net_links::tick_frames::host_world_state_checksum;
pub use net_links::tick_frames::Frame;
pub use net_links::tick_frames::frame_for_confirmed_tick;
pub use net_links::tick_frames::frame_for_tick;
pub use net_links::tick_frames::FrameChunkLengths;
pub use net_links::tick_frames::InProcessLink;
pub use net_links::tick_frames::Transport;
pub use net_links::tick_frames::LinkIoFailure;
pub use net_links::listen_link::LocalHostTickPipe;
pub use net_links::listen_link::ArrivedHostStep;
pub use net_links::lobby_link::ContentCatalog;
pub use net_links::lobby_link::ContentPack;
pub use net_links::lobby_link::DirectoryListing;
pub use net_links::lobby_link::DirectoryLink;
pub use net_links::lobby_link::DirectoryLinkStatus;
pub use net_links::lobby_link::DirectoryBrowser;
pub use net_links::lobby_link::DirectoryBrowserView;
pub use net_links::lobby_link::DirectoryLaunchRequest;
pub use net_links::lobby_link::DirectoryLifecycleNote;
pub use net_links::lobby_link::DirectoryGameOffer;
pub use net_links::lobby_link::DirectoryMatchBegin;
pub use net_links::lobby_link::DirectoryMenuChoice;
pub use net_links::lobby_link::QueuedMenuChoice;
pub use net_links::lobby_link::LobbySessionLabel;
pub use net_links::lobby_link::LinkFailureRecord;
pub use net_links::snapshot_wire::parse_scene_prop_resync;
pub use net_links::snapshot_wire::pack_scene_prop_resync;
pub use net_links::snapshot_wire::MetaSectionSizes;
pub use net_links::snapshot_wire::SCENE_RESYNC_INTERVAL_TICKS;
pub use net_links::snapshot_wire::WorldPropStreamDecoder;
pub use net_links::snapshot_wire::WorldPropStreamEncoder;
pub use net_links::crossing_fields::adopted_field_names;
pub use net_links::crossing_fields::players_hash;
pub use net_links::crossing_fields::FieldDrift;
pub use net_links::crossing_fields::player_state_diff;
pub use net_links::crossing_fields::avatar_drift;
pub use net_links::crossing_fields::PLAYER_WIRE_FIELD_TOTAL;
pub use net_links::crossing_fields::PLAYER_STATE_WIRE_FIELDS;
pub use net_links::crossing_fields::PlayerStateWireField;
pub use net_links::crossing_fields::SyncPolicy;
pub use net_links::crossing_fields::FieldCheck;
pub use net_links::datagrams::PlayerToHostDatagram;
pub use net_links::datagrams::LinkNumber;
pub use net_links::datagrams::LinkRegistry;
pub use net_links::datagrams::AssetDigestSet;
pub use net_links::datagrams::parse_receiver_datagram;
pub use net_links::datagrams::parse_host_datagram;
pub use net_links::datagrams::judge_greeting;
pub use net_links::datagrams::JoinGreeting;
pub use net_links::datagrams::JoinRefusal;
pub use net_links::datagrams::LinkIdentityFault;
pub use net_links::datagrams::GameSessionSummary;
pub use net_links::datagrams::DatagramPreamble;
pub use net_links::datagrams::NegotiatedCaps;
pub use net_links::datagrams::HostToPlayerDatagram;
pub use net_links::datagrams::DATAGRAM_TRANSPORT_READY;
pub use net_links::once_queue::ActionRulingNotice;
pub use net_links::once_queue::unpack_guaranteed_body;
pub use net_links::once_queue::pack_guaranteed_body;
pub use net_links::once_queue::GUARANTEED_BACKLOG_LIMIT;
pub use net_links::once_queue::GuaranteedEventRouter;
pub use net_links::once_queue::GuaranteedEventBacklog;
pub use net_links::once_queue::GuaranteedEnvelope;
pub use net_links::once_queue::GuaranteedEntry;
pub use net_links::relay::FinalizedEntry;
pub use net_links::relay::HostUdpSwitchboard;
pub use net_links::relay::PlayerUdpSession;
pub use crate::net_links::send_fault::UdpTransmitFailure;
pub use net_links::byte_codec::PacketDecodeFault;
pub use net_links::byte_codec::ByteCursorReader;
pub use net_links::byte_codec::ByteSinkWriter;
// The packet version:
/// Bumped whenever a packet or snapshot layout changes, so mismatched builds refuse each other.
pub const WIRE_REVISION_NUMBER: u32 = 96;
