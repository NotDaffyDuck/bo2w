//! Half-Life 2's Gravity Gun on a plain Black Ops II map (the Special
//! Weapons tab; Minecraft Zombies' `minecraft_gravgun` without the
//! character's arms or the Minecraft parts): the real view models, textures, glows and sounds,
//! read from the player's own Half-Life 2 / Garry's Mod files (or the free
//! Garry's Mod Dedicated Server, or the loose copy an install carries).
//! Nothing of Valve's is in this repository.
//!
//! This part is the view model: it loads the files in the background, runs
//! the weapon's animations as `weapon_physcannon` sends them (draw, idle,
//! the shake while holding, fire, altfire), opens the claws with the
//! "active" pose parameter, sways and bobs it as Half-Life 2's view models
//! do, and draws its glow sprites.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};

use asset_source::glam::{Mat4, Quat, Vec3};
use bytemuck::Pod;
use bytemuck::Zeroable;
use asset_source::{mdl, vtf};
use diag::{info, warn};

/// One vertex of a guest weapon (`render_gpu`'s guest view reads the same
/// 28 bytes): view-space position, texture point, shade and layer (alpha),
/// light, and a flag (`pad[0]` 255: the enchanted shine).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
#[derive(Pod, Zeroable)]
pub struct GuestVertex {
    pub position: [f32; 3],
    pub tex_uv: [f32; 2],
    pub tint: [u8; 4],
    pub light: [u8; 2],
    pub pad: [u8; 2],
}

/// A guest weapon's textures: square RGBA8 layers of one size, each with
/// its levels largest first.
pub struct GuestTexture {
    pub size: u32,
    pub layers: Vec<Vec<Vec<u8>>>,
}

/// Source's `viewmodel_fov`: 54 degrees across a 4:3 screen.
const VIEWMODEL_FOV: f32 = 54.0;
/// `cl_wpn_sway_scale` / `g_fMaxViewModelLag`.
const MAX_VIEWMODEL_LAG: f32 = 1.5;
/// The claws' open and shut time (`m_ElementParameter`, 0.2 s spline).
const CLAW_TIME: f32 = 0.2;
/// Map units in a block, for the bob's speed.
const UNITS_PER_BLOCK: f32 = 36.0;

/// The glow sprites, as `c_weapon_physcannon`'s materials (the `.vmt`s'
/// base textures). Their layer is `SPRITE_BASE + index`.
const SPRITES: &[&str] = &[
    "materials/sprites/orangecore1.vtf",
    "materials/sprites/orangecore2.vtf",
    "materials/sprites/orangeflare1.vtf",
    "materials/sprites/glow04.vtf",
    "materials/effects/fluttercore.vtf",
    "materials/sprites/blueflare1.vtf",
    "materials/sprites/physcannon_bluecore1b.vtf",
    "materials/sprites/physcannon_bluecore2b.vtf",
    "materials/sprites/physcannon_blueflare1.vtf",
    "materials/sprites/physcannon_blueglow.vtf",
];
const SPRITE_BASE: u32 = 3;
const ORANGE_CORE: usize = 0;
const ORANGE_BLAST: usize = 1;
const ORANGE_FLARE: usize = 2;
const GLOW: usize = 3;
const FLUTTER_CORE: usize = 4;
const BLUE_FLARE: usize = 5;

/// The sounds, by the name the game asks for them.
pub const SOUNDS: &[(&str, &str)] = &[
    ("hold_loop", "sound/weapons/physcannon/hold_loop.wav"),
    ("superphys_hold_loop", "sound/weapons/physcannon/superphys_hold_loop.wav"),
    ("pickup", "sound/weapons/physcannon/physcannon_pickup.wav"),
    ("drop", "sound/weapons/physcannon/physcannon_drop.wav"),
    ("dryfire", "sound/weapons/physcannon/physcannon_dryfire.wav"),
    ("tooheavy", "sound/weapons/physcannon/physcannon_tooheavy.wav"),
    ("claws_open", "sound/weapons/physcannon/physcannon_claws_open.wav"),
    ("claws_close", "sound/weapons/physcannon/physcannon_claws_close.wav"),
    ("charge", "sound/weapons/physcannon/physcannon_charge.wav"),
    ("launch1", "sound/weapons/physcannon/superphys_launch1.wav"),
    ("launch2", "sound/weapons/physcannon/superphys_launch2.wav"),
    ("launch3", "sound/weapons/physcannon/superphys_launch3.wav"),
    ("launch4", "sound/weapons/physcannon/superphys_launch4.wav"),
    ("zap1", "sound/weapons/physcannon/superphys_small_zap1.wav"),
    ("zap2", "sound/weapons/physcannon/superphys_small_zap2.wav"),
    ("zap3", "sound/weapons/physcannon/superphys_small_zap3.wav"),
    ("zap4", "sound/weapons/physcannon/superphys_small_zap4.wav"),
];

/// One of the two view models with what it needs to draw.
struct Gun {
    mdl: mdl::Model,
    /// The texture layer of each of the model's textures.
    layers: Vec<u32>,
    active: Option<usize>,
    idle: usize,
    hold: usize,
    draw: usize,
    fire: usize,
    altfire: usize,
    /// The claw attachments: bases, middles, tips (fork 1 then fork 2).
    forks: [Option<usize>; 6],
    muzzle: Option<usize>,
    /// The Portal Gun's core: its texture, and the orange core's layer
    /// (the last portal shot was orange).
    tint: Option<(usize, u32)>,
    /// The Portal Gun's handle bone (Half-Life 2's hand rides it).
    handle: Option<usize>,
}

impl Gun {
    fn new(mdl_in: mdl::Model, layers: Vec<u32>) -> Option<Self> {
        let seq = |name: &str| mdl_in.sequence(name);
        let att = |name: &str| mdl_in.attachments.iter().position(|a| a.name.eq_ignore_ascii_case(name));
        Some(Self {
            active: mdl_in.pose_param("active"),
            idle: seq("idle")?,
            hold: seq("hold_idle").or(seq("idle"))?,
            draw: seq("draw").or(seq("idle"))?,
            fire: seq("fire").or(seq("idle"))?,
            altfire: seq("altfire").or(seq("fire"))?,
            forks: ["fork1b", "fork1m", "fork1t", "fork2b", "fork2m", "fork2t"].map(att),
            muzzle: att("muzzle"),
            tint: None,
            handle: None,
            layers,
            mdl: mdl_in,
        })
    }
}

/// Half-Life 2's hand off the Gravity Gun, as it sits at rest (its layer,
/// and each vertex's place, normal and uv), for the Portal Gun to hold.
struct HandMesh {
    layer: u32,
    verts: Vec<(Vec3, Vec3, [f32; 2])>,
    index_list: Vec<u32>,
}

/// The Gravity Gun's hand meshes (their texture is the hands' sheet) at
/// the idle's first frame.
fn rest_hand(gun: &Gun) -> Vec<HandMesh> {
    let model = &gun.mdl;
    let mut params = vec![0.0; model.pose_params.len()];
    if let Some(a) = gun.active {
        params[a] = model.pose_params[a].start;
    }
    let bones = model.joint_global_transform(&model.pose(gun.idle, 0.0, &params, 0.0));
    let skin: Vec<Mat4> = bones.iter().zip(&model.bones).map(|(w, b)| *w * b.pose_to_bone).collect();
    model
        .meshes
        .iter()
        .filter(|m| model.textures.get(m.texture).is_some_and(|t| t.name.to_ascii_lowercase().contains("hand")))
        .map(|mesh| HandMesh {
            layer: gun.layers.get(mesh.texture).copied().unwrap_or(0),
            verts: mesh
                .vertices
                .iter()
                .map(|v| {
                    let (mut p, mut n) = (Vec3::ZERO, Vec3::ZERO);
                    for k in 0..3 {
                        if v.blend[k] > 0.0 {
                            let m = skin[v.bones[k] as usize];
                            p += m.transform_point3(v.pos) * v.blend[k];
                            n += m.transform_vector3(v.normal) * v.blend[k];
                        }
                    }
                    (p, n, [v.uv.x, v.uv.y])
                })
                .collect::<_>(),
            index_list: mesh.tri_list.clone(),
        })
        .collect::<_>()
}

/// The 3D Portal Gun and the Super one (`asset_source::portalgun`), their
/// textures added as layers.
fn portal_guns(layers: &mut Vec<Vec<Vec<u8>>>, size: u32) -> Vec<Gun> {
    use asset_source::portalgun;
    [false, true]
        .into_iter()
        .map(|upgraded| {
            let model = portalgun::model(upgraded);
            let mut of = Vec::new();
            for t in &model.textures {
                of.push(layers.len() as u32);
                let rgba = portalgun::texture(&t.name).unwrap_or(vtf::Rgba { width: 1, height: 1, pixels: vec![200, 200, 200, 0] });
                layers.push(pixel_levels(&rgba, size));
            }
            let orange = of[portalgun::CORE_ORANGE];
            let mut gun = Gun::new(model, of).expect("the Portal Gun has every sequence");
            gun.tint = Some((portalgun::CORE, orange));
            gun.handle = gun.mdl.bones.iter().position(|b| b.name.eq_ignore_ascii_case("Handle"));
            gun
        })
        .collect::<_>()
}

/// Everything read from the Source files.
pub struct Assets {
    /// The Gravity Gun and the Super one, then the Portal Gun and the
    /// Super one (`gun_of`).
    guns: Vec<Gun>,
    /// Half-Life 2's hand for the Portal Gun (none with the stand-in).
    hand: Vec<HandMesh>,
    pub texture: Arc<GuestTexture>,
    sprites: Vec<bool>,
    /// The sounds' WAV bytes by their `SOUNDS` name.
    pub sounds: HashMap<&'static str, Arc<[u8]>>,
}

enum Load {
    Idle,
    Loading(mpsc::Receiver<Result<Assets, String>>),
    Ready(Arc<Assets>),
    Missing,
}

impl Default for Load {
    fn default() -> Self {
        Self::Idle
    }
}

/// The folder the game runs from (its `guest/` sits beside the exe).
fn app_home_folder() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from)).unwrap_or_else(|| PathBuf::from("."))
}

/// A texture resampled (bilinear) to `size` square, then every level by a
/// box filter.
fn layer_levels(rgba: Option<&vtf::Rgba>, size: u32) -> Vec<Vec<u8>> {
    let mut level: Vec<u8> = match rgba {
        Some(t) if t.width > 0 && t.height > 0 => {
            let mut out = vec![0u8; (size * size * 4) as usize];
            let (w, h) = (t.width as usize, t.height as usize);
            for y in 0..size as usize {
                let fy = ((y as f32 + 0.5) * h as f32 / size as f32 - 0.5).clamp(0.0, (h - 1) as f32);
                let (y0, ty) = (fy.floor() as usize, fy.fract());
                let y1 = (y0 + 1).min(h - 1);
                for x in 0..size as usize {
                    let fx = ((x as f32 + 0.5) * w as f32 / size as f32 - 0.5).clamp(0.0, (w - 1) as f32);
                    let (x0, tx) = (fx.floor() as usize, fx.fract());
                    let x1 = (x0 + 1).min(w - 1);
                    for ch in 0..4 {
                        let p = |xx: usize, yy: usize| f32::from(t.pixels[(yy * w + xx) * 4 + ch]);
                        let top = p(x0, y0) + (p(x1, y0) - p(x0, y0)) * tx;
                        let bottom = p(x0, y1) + (p(x1, y1) - p(x0, y1)) * tx;
                        out[(y * size as usize + x) * 4 + ch] = (top + (bottom - top) * ty).round() as u8;
                    }
                }
            }
            out
        }
        // A missing texture is clear (a sprite) or white.
        _ => vec![0u8; (size * size * 4) as usize],
    };
    let mut levels = Vec::new();
    let mut side = size as usize;
    loop {
        let next_side = (side / 2).max(1);
        let next: Vec<u8> = if side > 1 {
            let mut n = vec![0u8; next_side * next_side * 4];
            for y in 0..next_side {
                for x in 0..next_side {
                    for ch in 0..4 {
                        let p = |xx: usize, yy: usize| u32::from(level[(yy * side + xx) * 4 + ch]);
                        let sum = p(2 * x, 2 * y) + p(2 * x + 1, 2 * y) + p(2 * x, 2 * y + 1) + p(2 * x + 1, 2 * y + 1);
                        n[(y * next_side + x) * 4 + ch] = ((sum + 2) / 4) as u8;
                    }
                }
            }
            n
        } else {
            Vec::new()
        };
        levels.push(std::mem::replace(&mut level, next));
        if side == 1 {
            break;
        }
        side = next_side;
    }
    levels
}

/// Half-Life 2's gun when its files are found, the Minecraft stand-in when
/// not (`BO2MP_GRAVGUN_STANDIN=1` forces it, for a test).
fn load() -> Result<Assets, String> {
    if std::env::var_os("BO2MP_GRAVGUN_STANDIN").is_some() {
        info!(World, "gravity gun: BO2MP_GRAVGUN_STANDIN, the Minecraft stand-in");
        return Ok(standin());
    }
    load_hl2().or_else(|why| {
        info!(World, "gravity gun: no Half-Life 2 files ({why}); the Minecraft stand-in");
        Ok(standin())
    })
}

/// A pixel texture blown up to `len_bytes` by nearest pixel (kept sharp), then
/// its levels.
fn pixel_levels(rgba: &vtf::Rgba, len_bytes: u32) -> Vec<Vec<u8>> {
    let (w, h) = (rgba.width.max(1) as usize, rgba.height.max(1) as usize);
    let mut pixels = Vec::with_capacity((len_bytes * len_bytes * 4) as usize);
    for y in 0..len_bytes as usize {
        for x in 0..len_bytes as usize {
            let at = ((y * h / len_bytes as usize) * w + x * w / len_bytes as usize) * 4;
            pixels.extend_from_slice(&rgba.pixels[at..at + 4]);
        }
    }
    layer_levels(Some(&vtf::Rgba { width: len_bytes, height: len_bytes, pixels }), len_bytes)
}

/// The Minecraft-style stand-in (`asset_source::standin`): no sounds of its
/// own.
fn standin() -> Assets {
    use asset_source::standin;
    let size = 256;
    // Layers 0..2 (Half-Life 2's sheets) stay empty; the sprites, then
    // each gun's materials.
    let mut layers: Vec<Vec<Vec<u8>>> = (0..SPRITE_BASE).map(|_| layer_levels(None, size)).collect();
    for path in SPRITES {
        layers.push(pixel_levels(&standin::sprite(path), size));
    }
    let mut gun = |upgraded: bool| -> Gun {
        let model = standin::model(upgraded);
        let mut of = Vec::new();
        for t in &model.textures {
            of.push(layers.len() as u32);
            let rgba = standin::texture(&t.name).unwrap_or(vtf::Rgba { width: 1, height: 1, pixels: vec![128, 128, 128, 0] });
            layers.push(pixel_levels(&rgba, size));
        }
        Gun::new(model, of).expect("the stand-in has every sequence")
    };
    let mut guns = vec![gun(false), gun(true)];
    guns.extend(portal_guns(&mut layers, size));
    info!(World, "gravity gun: the Minecraft stand-in, textures {size}px x{}", layers.len());
    Assets {
        guns,
        hand: Vec::new(),
        texture: Arc::new(GuestTexture { size, layers }),
        sprites: vec![true; SPRITES.len()],
        sounds: HashMap::new(),
    }
}

/// Reads both view models, their textures, the sprites and the sounds.
fn load_hl2() -> Result<Assets, String> {
    let source = asset_source::Source::find(&app_home_folder());
    for line in &source.notes {
        info!(World, "gravity gun: {line}");
    }
    let read_model = |name: &str| -> Result<mdl::Model, String> {
        let file = |ext: &str| source.read(&format!("models/weapons/{name}.{ext}")).ok_or(format!("no {name}.{ext}"));
        mdl::read(&file("mdl")?, &file("vvd")?, &file("dx90.vtx")?).ok_or(format!("{name} unreadable"))
    };
    let normal = read_model("v_physcannon")?;
    let mega = read_model("v_superphyscannon")?;
    // Layers 0..2: the gun sheet, the super gun's sheet, the hands.
    let sheet = |model: &mdl::Model, t: usize| -> Option<vtf::Rgba> {
        let t = &model.textures[t];
        t.folders.iter().find_map(|f| source.read(&format!("materials/{f}{}.vtf", t.name)).and_then(|b| vtf::decode(&b)))
    };
    // The sheets' alpha is their `$selfillum` mask (the orange parts glow);
    // a sheet without it has none.
    let self_illum = |model: &mdl::Model, t: usize| -> bool {
        let t = &model.textures[t];
        t.folders.iter().find_map(|f| source.read(&format!("materials/{f}{}.vmt", t.name))).is_some_and(|b| {
            String::from_utf8_lossy(&b)
                .lines()
                .map(|l| l.split("//").next().unwrap_or("").to_ascii_lowercase())
                .any(|l| l.contains("\"$selfillum\"") && l.contains("\"1\""))
        })
    };
    let mut named: Vec<(String, Option<vtf::Rgba>)> = Vec::new();
    for model in [&normal, &mega] {
        for t in 0..model.textures.len() {
            let name = model.textures[t].name.to_ascii_lowercase();
            if !named.iter().any(|(n, _)| *n == name) {
                let mut rgba = sheet(model, t);
                if !self_illum(model, t)
                    && let Some(rgba) = rgba.as_mut()
                {
                    rgba.pixels.chunks_exact_mut(4).for_each(|p| p[3] = 0);
                }
                named.push((name, rgba));
            }
        }
    }
    let order = |name: &str| match name {
        n if n.contains("superphyscannon") => 1,
        n if n.contains("physcannon") => 0,
        n if n.contains("hand") => 2,
        _ => 3,
    };
    named.sort_by_key(|(n, _)| order(n));
    if named.len() > SPRITE_BASE as usize {
        return Err(format!("unexpected textures {:?}", named.iter().map(|(n, _)| n).collect::<Vec<_>>()));
    }
    let size = named
        .iter()
        .filter_map(|(_k, t)| t.as_ref().map(|t| t.width.max(t.height)))
        .max()
        .unwrap_or(256)
        .next_power_of_two()
        .clamp(64, 1024);
    let mut layers: Vec<Vec<Vec<u8>>> = Vec::new();
    for slot in 0..SPRITE_BASE as usize {
        layers.push(layer_levels(named.get(slot).and_then(|(_, t)| t.as_ref()), size));
    }
    let mut sprites = Vec::new();
    for path in SPRITES {
        let rgba = source.read(path).and_then(|b| vtf::decode(&b));
        sprites.push(rgba.is_some());
        layers.push(layer_levels(rgba.as_ref(), size));
    }
    let layer_of = |model: &mdl::Model| -> Vec<u32> {
        model
            .textures
            .iter()
            .map(|t| named.iter().position(|(n, _)| n.eq_ignore_ascii_case(&t.name)).unwrap_or(0) as u32)
            .collect::<_>()
    };
    let (normal_layers, mega_layers) = (layer_of(&normal), layer_of(&mega));
    let mut guns = vec![
        Gun::new(normal, normal_layers).ok_or("v_physcannon lacks its sequences")?,
        Gun::new(mega, mega_layers).ok_or("v_superphyscannon lacks its sequences")?,
    ];
    let hand = rest_hand(&guns[0]);
    guns.extend(portal_guns(&mut layers, size));
    let sounds = SOUNDS
        .iter()
        .filter_map(|(clip_name, path)| source.read(path).map(|bytes| (*clip_name, Arc::<[u8]>::from(bytes))))
        .collect::<HashMap<_, _>>();
    diag::info!(
        World,
        "gravity gun: loaded, textures {size}px x{} ({} sprites), {} sounds",
        layers.len(),
        sprites.iter().filter(|s| **s).count(),
        sounds.len()
    );
    Ok(Assets { guns, hand, texture: Arc::new(GuestTexture { size, layers }), sprites, sounds })
}

/// What the gun is doing, from the game side, for the view model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Action {
    #[default]
    None,
    /// Primary fire: a launch or a punt (`ACT_VM_PRIMARYATTACK`).
    Fire,
    /// Primary fire with nothing to hit (the same animation, no blast).
    DryFire,
    /// The pull caught something, or it was let go (`ACT_VM_SECONDARYATTACK`).
    AltFire,
}

/// A sound the gun makes (`Weapon_PhysCannon.*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    Pickup,
    Drop,
    TooHeavy,
    Launch,
    DryFire,
    ClawsOpen,
    ClawsClose,
    /// The hum while it holds something (`Some(upgraded)`), or its end.
    Hum(Option<bool>),
}

/// The hum's name in the sound queue.
const HUM: &str = "gravgun_hum";

/// Each cue's Half-Life 2 sound (`SOUNDS` names; a launch picks one of
/// four) and volume.
fn hl2_sound(cue: Cue, pick: u32) -> Option<(&'static str, f32)> {
    Some(match cue {
        Cue::Pickup => ("pickup", 0.8),
        Cue::Drop => ("drop", 0.8),
        Cue::TooHeavy => ("tooheavy", 0.8),
        Cue::Launch => (["launch1", "launch2", "launch3", "launch4"][pick as usize % 4], 1.0),
        Cue::DryFire => ("dryfire", 0.8),
        Cue::ClawsOpen => ("claws_open", 0.5),
        Cue::ClawsClose => ("claws_close", 0.5),
        Cue::Hum(Some(false)) => ("hold_loop", 0.5),
        Cue::Hum(Some(true)) => ("superphys_hold_loop", 0.5),
        Cue::Hum(None) => return None,
    })
}

/// One frame's input to the view model.
pub struct ViewInput {
    /// The held weapon's name (BO2's), if any.
    pub weapon: Option<String>,
    pub dt: f32,
    /// The look direction in Minecraft's axes, and the pitch in degrees
    /// (down positive).
    pub look: [f32; 3],
    pub tilt: f32,
    /// His feet in blocks, for the bob's speed.
    pub feet: [f64; 3],
    pub aspect_ratio: f32,
    /// Sky and block light at the eye, 0..15.
    pub eye_light: [f32; 2],
    /// Something is held in the beam.
    pub holding: bool,
    /// The claws want to open (aiming at something it can take, or pulling).
    pub open: bool,
    pub action: Action,
    /// The Portal Gun's last shot was orange (its core and glow show it).
    pub orange: bool,
}

/// The view model's mesh for the renderer.
pub struct ViewMesh {
    pub verts: Vec<u8>,
    pub idx_list: Vec<u32>,
    pub solid_count: u32,
    pub to_clip: [f32; 16],
    pub texture: Arc<GuestTexture>,
}

/// The Gravity Gun's state in this world.
#[derive(Default)]
pub struct Gravgun {
    load: Load,
    /// The gun out last frame (`gun_of`).
    out: Option<usize>,
    seq: usize,
    seq_time: f32,
    clock: f32,
    claws: f32,
    claws_from: f32,
    claws_to: f32,
    claws_time: f32,
    last_facing: Option<Vec3>,
    last_feet: Option<[f64; 3]>,
    bob_time: f32,
    /// A launch's blast flash: seconds since it.
    blast: Option<f32>,
    rng: u32,
}

/// Whether a BO2 weapon name is the Gravity Gun, and which one.
pub fn gravgun_of(weapon: &str) -> Option<bool> {
    let w = weapon.to_ascii_lowercase();
    w.starts_with("physcannon").then(|| w.contains("upgraded"))
}

/// Whether a BO2 weapon name is the Portal Gun, and which one.
pub fn portalgun_of(weapon: &str) -> Option<bool> {
    let w = weapon.to_ascii_lowercase();
    w.starts_with("portalgun").then(|| w.contains("upgraded"))
}

/// The view model a BO2 weapon shows: 0 the Gravity Gun, 1 the Super one,
/// 2 the Portal Gun, 3 the Super one.
fn gun_of(weapon: &str) -> Option<usize> {
    gravgun_of(weapon).map(usize::from).or_else(|| portalgun_of(weapon).map(|u| 2 + usize::from(u)))
}

impl Gravgun {
    /// Starts loading the files (once), and takes them when read.
    pub fn assets(&mut self) -> Option<Arc<Assets>> {
        match &self.load {
            Load::Idle => {
                let (send, receive) = mpsc::channel();
                let _ = std::thread::Builder::new().name("gravity-gun-load".into()).spawn(move || {
                    let _ = send.send(load());
                });
                self.load = Load::Loading(receive);
                None
            }
            Load::Loading(receive) => match receive.try_recv() {
                Ok(Ok(assets)) => {
                    let assets = Arc::new(assets);
                    self.load = Load::Ready(assets.clone());
                    Some(assets)
                }
                Ok(Err(why)) => {
                    warn!(World, "gravity gun: nothing to show ({why})");
                    self.load = Load::Missing;
                    None
                }
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.load = Load::Missing;
                    None
                }
            },
            Load::Ready(assets) => Some(assets.clone()),
            Load::Missing => None,
        }
    }

    /// The gun's sounds this frame at his ear: Half-Life 2's own (the
    /// stand-in has none).
    pub(crate) fn play_cues(&mut self, cues: &[Cue], queue: &mut Vec<audio::GuestSoundRequest>) {
        if cues.is_empty() {
            return;
        }
        let assets = self.assets();
        // BO2MP_SOUND_LOG=1 (tests, muted): each cue reaches the log.
        let log = std::env::var_os("BO2MP_SOUND_LOG").is_some();
        for &cue in cues {
            if cue == Cue::Hum(None) {
                if log {
                    info!(World, "gravity gun sound: hum stops");
                }
                queue.push(audio::GuestSoundRequest::stop(HUM));
                continue;
            }
            let hold = matches!(cue, Cue::Hum(_)).then_some(HUM);
            self.rng = self.rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let hl2 = hl2_sound(cue, self.rng >> 16)
                .and_then(|(hl2_name, vol)| Some((hl2_name, vol, assets.as_ref()?.sounds.get(hl2_name)?.clone())));
            if let Some((hl2_name, vol, bytes)) = hl2 {
                if log {
                    info!(World, "gravity gun sound: {cue:?} HL2 {hl2_name}");
                }
                queue.push(audio::GuestSoundRequest { key: format!("hl2:{hl2_name}"), bytes, volume: vol, pitch_mul: 1.0, hold });
            }
        }
    }

    fn random(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Runs the view model a frame; its mesh when the gun is out and read.
    pub fn view(&mut self, input: &ViewInput) -> Option<ViewMesh> {
        let assets = self.assets();
        let which = input.weapon.as_deref().and_then(gun_of);
        let Some(which) = which else {
            self.out = None;
            return None;
        };
        let (upgraded, portal) = (which % 2 == 1, which >= 2);
        let assets = assets?;
        let gun = assets.guns.get(which)?;
        let model = &gun.mdl;
        let dt = input.dt.clamp(0.0, 0.1);
        self.clock += dt;
        if self.out != Some(which) {
            // Raised (or Pack-a-Punched in his hands): the draw.
            self.out = Some(which);
            self.seq = gun.draw;
            self.seq_time = 0.0;
            self.last_facing = None;
            self.claws = if upgraded && !portal { 1.0 } else { 0.0 };
            self.claws_from = self.claws;
            self.claws_to = self.claws;
            self.blast = None;
        } else {
            self.seq_time += dt;
        }
        match input.action {
            Action::Fire | Action::DryFire => {
                self.seq = gun.fire;
                self.seq_time = 0.0;
                if input.action == Action::Fire {
                    self.blast = Some(0.0);
                }
            }
            Action::AltFire => {
                self.seq = gun.altfire;
                self.seq_time = 0.0;
            }
            Action::None => {}
        }
        // `WeaponIdle`: once a one-shot is done, the shake while holding
        // (ACT_VM_RELOAD) or the idle.
        let idle = if input.holding { gun.hold } else { gun.idle };
        let one_shot = !model.sequences[self.seq].repeats && self.seq != gun.idle && self.seq != gun.hold;
        if (one_shot && self.seq_time >= model.duration(self.seq)) || (!one_shot && self.seq != idle) {
            self.seq = idle;
            self.seq_time = 0.0;
        }
        if let Some(t) = self.blast.as_mut() {
            *t += dt;
            if *t > 0.2 {
                self.blast = None;
            }
        }
        // The claws: the super gun's stay open.
        let want = if !portal && (upgraded || input.holding || input.open) { 1.0 } else { 0.0 };
        if want != self.claws_to {
            self.claws_from = self.claws;
            self.claws_to = want;
            self.claws_time = 0.0;
        }
        self.claws_time += dt;
        let s = (self.claws_time / CLAW_TIME).clamp(0.0, 1.0);
        self.claws = self.claws_from + (self.claws_to - self.claws_from) * (s * s * (3.0 - 2.0 * s));
        let mut params = vec![0.0; model.pose_params.len()];
        if let Some(a) = gun.active {
            let p = &model.pose_params[a];
            params[a] = p.start + (p.end - p.start) * self.claws;
        }
        let pose = model.pose(self.seq, self.seq_time, &params, self.clock);
        let bones = model.joint_global_transform(&pose);
        let skin: Vec<Mat4> = bones.iter().zip(&model.bones).map(|(w, b)| *w * b.pose_to_bone).collect();

        // The view model's place: `CalcViewModelLag` and HL2's bob.
        let look = Vec3::from_array(input.look).normalize_or(Vec3::Z);
        let up_world = Vec3::Y;
        let right = look.cross(up_world).normalize_or(Vec3::X);
        let up = right.cross(look);
        let last = self.last_facing.get_or_insert(look);
        let difference = look - *last;
        let mut speed = 5.0;
        let lag = difference.length();
        if lag > MAX_VIEWMODEL_LAG {
            speed *= lag / MAX_VIEWMODEL_LAG;
        }
        *last = (*last + difference * (speed * dt)).normalize_or(look);
        let world_offset = difference * -5.0;
        let mut offset = Vec3::new(world_offset.dot(right), world_offset.dot(up), -world_offset.dot(look));
        let pitch = input.tilt;
        offset += Vec3::new(-pitch * 0.03, -pitch * 0.02, pitch * 0.035);
        let moved = self.last_feet.replace(input.feet).map_or(0.0, |l| {
            let (step_along_x, dz) = (input.feet[0] - l[0], input.feet[2] - l[2]);
            (step_along_x * step_along_x + dz * dz).sqrt() as f32
        });
        // A teleport is not a run.
        let speed = if dt > 0.0 && moved < 2.0 { moved / dt } else { 0.0 };
        let ground = (speed * UNITS_PER_BLOCK).clamp(0.0, 320.0);
        self.bob_time += dt * (ground / 320.0);
        let bob = |time: f32, cycle_max: f32| {
            let cycle = (time - (time / cycle_max).floor() * cycle_max) / cycle_max;
            let cycle = if cycle < 0.5 { std::f32::consts::PI * cycle / 0.5 } else { std::f32::consts::PI + std::f32::consts::PI * (cycle - 0.5) / 0.5 };
            let base = ground * 0.005;
            (base * 0.3 + base * 0.7 * cycle.sin()).clamp(-7.0, 4.0)
        };
        let vertical = bob(self.bob_time, 0.45);
        let lateral = bob(self.bob_time, 0.9);
        // `AddViewmodelBob`: forward and up by a tenth of the vertical bob,
        // right by 0.8 of the lateral; roll, pitch and yaw by them.
        offset += Vec3::new(lateral * 0.8, vertical * 0.1, -vertical * 0.1);
        let bob_rotation = Quat::from_rotation_z((-lateral * 0.3).to_radians())
            * Quat::from_rotation_y((-vertical * 0.4).to_radians())
            * Quat::from_rotation_x((vertical * 0.5).to_radians());
        // Source (x forward, y left, z up) to view (right, up, back).
        let to_view = |p: Vec3| -> Vec3 {
            let p = bob_rotation * p;
            Vec3::new(-p.y, p.z, -p.x) + offset
        };
        let light_dir = Vec3::new(0.35, 0.8, 0.45).normalize();
        let light = [
            (input.eye_light[0].clamp(0.0, 15.0) * 16.0).round() as u8,
            (input.eye_light[1].clamp(0.0, 15.0) * 16.0).round() as u8,
        ];
        let mut vertices: Vec<GuestVertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        for mesh in &model.meshes {
            let layer = match gun.tint {
                Some((core, orange)) if input.orange && mesh.texture == core => orange,
                _ => gun.layers.get(mesh.texture).copied().unwrap_or(0),
            };
            let first = vertices.len() as u32;
            for v in &mesh.vertices {
                let (mut p, mut n) = (Vec3::ZERO, Vec3::ZERO);
                for k in 0..3 {
                    if v.blend[k] > 0.0 {
                        let m = skin[v.bones[k] as usize];
                        p += m.transform_point3(v.pos) * v.blend[k];
                        n += m.transform_vector3(v.normal) * v.blend[k];
                    }
                }
                let n = bob_rotation * n;
                let n = Vec3::new(-n.y, n.z, -n.x).normalize_or_zero();
                let shade = (0.5 + 0.5 * n.dot(light_dir).max(0.0)).clamp(0.0, 1.0);
                let c = (shade * 255.0).round() as u8;
                vertices.push(GuestVertex {
                    position: to_view(p).to_array(),
                    tex_uv: [v.uv.x, v.uv.y],
                    tint: [c, c, c, layer as u8],
                    light,
                    pad: [0; 2],
                });
            }
            indices.extend(mesh.tri_list.iter().map(|i| first + i));
        }
        // The Portal Gun in Half-Life 2's hand: the hand as it holds the
        // Gravity Gun's handle, on this gun's handle (it sits where the
        // Gravity Gun's did, so the grip lands on its side grip).
        if let Some(handle) = gun.handle.filter(|_| portal) {
            let place = bones[handle] * model.bones[handle].pose_to_bone;
            for mesh in &assets.hand {
                let first = vertices.len() as u32;
                for (p, n, uv) in &mesh.verts {
                    let n = bob_rotation * place.transform_vector3(*n);
                    let n = Vec3::new(-n.y, n.z, -n.x).normalize_or_zero();
                    let shade = (0.5 + 0.5 * n.dot(light_dir).max(0.0)).clamp(0.0, 1.0);
                    let c = (shade * 255.0).round() as u8;
                    vertices.push(GuestVertex {
                        position: to_view(place.transform_point3(*p)).to_array(),
                        tex_uv: *uv,
                        tint: [c, c, c, mesh.layer as u8],
                        light,
                        pad: [0; 2],
                    });
                }
                indices.extend(mesh.index_list.iter().map(|i| first + i));
            }
        }
        let solid = indices.len() as u32;

        // The glow sprites (`DoEffectIdle` / `DoEffect`), facing the view,
        // drawn over the model as Source's `$ignorez` sprites: each is
        // pulled halfway to the eye at half size (the same on screen).
        let attachment = |a: Option<usize>| -> Option<Vec3> {
            let a = &model.attachments[a?];
            Some(to_view((bones[a.joint] * a.local).w_axis.truncate()))
        };
        let sprite = |vertices: &mut Vec<GuestVertex>,
                          idx_out: &mut Vec<u32>,
                          at: Vec3,
                          which: usize,
                          half: f32,
                          colour: [f32; 3]| {
            if half <= 0.0 || !assets.sprites.get(which).copied().unwrap_or(false) {
                return;
            }
            let (at, half) = (at * 0.5, half * 0.5);
            let first = vertices.len() as u32;
            let rgb = colour.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
            for (step_along_x, dy, u, v) in [(-1.0, -1.0, 0.0, 1.0), (1.0, -1.0, 1.0, 1.0), (1.0, 1.0, 1.0, 0.0), (-1.0, 1.0, 0.0, 0.0)] {
                vertices.push(GuestVertex {
                    position: [at.x + step_along_x * half, at.y + dy * half, at.z],
                    tex_uv: [u, v],
                    tint: [rgb[0], rgb[1], rgb[2], (SPRITE_BASE as usize + which) as u8],
                    light,
                    pad: [0; 2],
                });
            }
            idx_out.extend([first, first + 1, first + 2, first, first + 2, first + 3]);
        };
        let (core, flare, glow) = if portal {
            // The Portal Gun's tip glows the last portal's colour.
            if input.orange { (ORANGE_CORE, ORANGE_FLARE, GLOW) } else { (FLUTTER_CORE, BLUE_FLARE, BLUE_FLARE) }
        } else if upgraded {
            (FLUTTER_CORE, BLUE_FLARE, BLUE_FLARE)
        } else {
            (ORANGE_CORE, ORANGE_FLARE, GLOW)
        };
        let scale = match (portal, upgraded) {
            (true, _) => 0.6,
            (false, true) => 1.5,
            (false, false) => 1.0,
        };
        // Ready: a dim core; holding: a bright one.
        if let Some(at) = attachment(gun.muzzle) {
            let (half, bright) = if input.holding { (4.5, 1.0) } else { (2.5 + 1.5 * self.claws, 0.45 + 0.25 * self.claws) };
            sprite(&mut vertices, &mut indices, at, core, half * scale, [bright; 3]);
            if let Some(t) = self.blast {
                let s = t / 0.2;
                let blast = if portal { core } else if upgraded { FLUTTER_CORE } else { ORANGE_BLAST };
                sprite(&mut vertices, &mut indices, at, blast, (2.0 + 8.0 * s) * scale, [1.0 - s; 3]);
            }
        }
        // The claws' flickering glows and their tips' flares.
        for (i, fork) in gun.forks.iter().enumerate() {
            let Some(at) = attachment(*fork) else { continue };
            let flicker = self.random();
            let tip = i % 3 == 2;
            if tip {
                sprite(&mut vertices, &mut indices, at, flare, (1.0 + 0.5 * flicker) * scale, [0.25 + 0.15 * flicker + 0.4 * self.claws; 3]);
            } else {
                sprite(&mut vertices, &mut indices, at, glow, (1.2 + 0.4 * flicker) * scale, [0.1 + 0.05 * flicker + 0.15 * self.claws; 3]);
            }
        }

        // Reverse-Z with no far plane; Source keeps the vertical angle.
        let f = 1.0 / ((VIEWMODEL_FOV / 2.0).to_radians().tan() * 0.75);
        let near = 0.5;
        let clip = [
            f / input.aspect_ratio.max(0.1), 0.0, 0.0, 0.0, //
            0.0, f, 0.0, 0.0, //
            0.0, 0.0, 0.0, -1.0, //
            0.0, 0.0, near, 0.0,
        ];
        Some(ViewMesh {
            verts: bytemuck::cast_slice(&vertices).to_vec(),
            idx_list: indices,
            solid_count: solid,
            to_clip: clip,
            texture: assets.texture.clone(),
        })
    }
}
