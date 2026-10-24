//! The prepared first-person rig.
//!
//! Preparation happens once per weapon, off the frame path:
//!
//! 1. every first-person mesh surface gets a verdict (`FpvMaterialAdmission`),
//! 2. a mesh with its hidden parts applied becomes a `PreparedFpvModel` (skin
//!    layout plus the surfaces that will draw),
//! 3. an assembly's parts become a `PreparedFpvComposition`,
//! 4. a composition plus the clips' track tables becomes a `PreparedFpvRig`,
//!    whose `geometry` is the draw plan a frame publishes.
//!
//! A frame then only poses bones (`pose_hand`) and skins vertices (`skin_into`).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bevy::math::{Mat4, Vec3};
use render_scene::SmodelPassMaterial;

use crate::anim::fpv_pose::{
    FpvBoltFrame, FpvBoltTags, PosedClip, tag_camera_lens_local, tag_view_to_bevy_camera,
};
use crate::anim::xmodel_pose::{FpvSurfOwner, SkinLayout, build_skin_layout, skin_packed_into};
use crate::draw::FpvSurfaceDraw;
use asset_game::{FpvAssembly, FpvClipTracks, FpvPartRole};
use asset_model::FpvMeshCatalog;
use dobj_t6::{PartBits, hide_part_bit, surface_hidden, surface_hidden_whole};
use xmodel_runtime::{AnimInstance, DObjAnim};

// ---------------------------------------------------------------------------
// Surface admission
// ---------------------------------------------------------------------------

/// What became of one mesh surface when its material was looked up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FpvSurfaceVerdict {
    /// Drawn with this row of the admitted material table.
    Admitted(u32),
    /// Not meant to draw (it has no colour map, for one); not a fault.
    Inapplicable(&'static str),
    /// Should have drawn and could not.
    Refused {
        material: String,
        cause: &'static str,
    },
}

/// The materials admitted so far, shared by every model of a preparation, and
/// the per-surface verdicts that produced them.
#[derive(Default)]
pub struct FpvMaterialAdmission {
    pub materials: Vec<SmodelPassMaterial>,
    /// Authored material index -> its row in `materials`.
    pub by_authored: HashMap<usize, u32>,
    verdicts: HashMap<usize, Vec<(usize, FpvSurfaceVerdict)>>,
}

impl FpvMaterialAdmission {
    pub fn record(&mut self, catalog_entry: usize, verdicts: Vec<(usize, FpvSurfaceVerdict)>) {
        self.verdicts.insert(catalog_entry, verdicts);
    }

    pub fn verdicts_of(&self, catalog_entry: usize) -> &[(usize, FpvSurfaceVerdict)] {
        self.verdicts
            .get(&catalog_entry)
            .map_or(&[][..], Vec::as_slice)
    }

    pub fn verdict(&self, catalog_entry: usize, surface: usize) -> Option<&FpvSurfaceVerdict> {
        self.verdicts_of(catalog_entry)
            .iter()
            .find_map(|(index, verdict)| (*index == surface).then_some(verdict))
    }

    /// The material row a surface draws with, if it was admitted.
    fn row_of(&self, catalog_entry: usize, surface: usize) -> Option<u32> {
        match self.verdict(catalog_entry, surface) {
            Some(FpvSurfaceVerdict::Admitted(row)) => Some(*row),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// One prepared model
// ---------------------------------------------------------------------------

/// One surface that will be drawn, positioned within its model's index list.
#[derive(Clone, Copy, Debug)]
struct PreparedFpvSurface {
    index_start: u32,
    index_count: u32,
    /// Row in the admitted material table.
    material: u32,
    /// Authored material index, which is what two surfaces share a draw
    /// material by.
    authored: usize,
    lens_named: bool,
}

/// A model with its hidden parts applied: how its vertices are packed for
/// skinning and which surfaces draw.
pub struct PreparedFpvModel {
    catalog_entry: usize,
    posed_surface_n: usize,
    packed_ok: bool,
    layout: SkinLayout,
    surfaces: Vec<PreparedFpvSurface>,
    refusal: Option<FpvMandatoryRefusal>,
}

/// A surface that had to draw and could not, with enough to say which.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FpvMandatoryRefusal {
    pub model: String,
    pub surface: usize,
    pub material: String,
    pub cause: &'static str,
}

impl core::fmt::Display for FpvMandatoryRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{} surface {} `{}`: {}",
            self.model, self.surface, self.material, self.cause
        )
    }
}

#[derive(Debug)]
pub enum FpvRigError {
    Catalog(&'static str),
    Layout(&'static str),
}

impl core::fmt::Display for FpvRigError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Catalog(what) => write!(f, "{what} missing from the first-person catalog"),
            Self::Layout(what) => f.write_str(what),
        }
    }
}

/// A scope's surface that carries the lens, by its material's name.
pub fn leftover_scope_surf_is_lens(name: &str) -> bool {
    name.contains("lens")
}

/// Whether the hidden tags take a surface out of the model.
///
/// bo2zm: Black Ops II hides a tag's geometry, not every surface that touches
/// it (the DSR-50's body shares surfaces with the iron sights its scope
/// variant hides), so a surface goes only when all its bones are hidden; the
/// rest lose the hidden bones' vertices when posed. Other games drop a surface
/// as soon as any of its bones is hidden.
fn hides_surface(t6: bool, part_bits: &[u32; 5], hidden_tags: &[u32; 5]) -> bool {
    if t6 {
        surface_hidden_whole(part_bits, hidden_tags, 0)
    } else {
        surface_hidden(part_bits, hidden_tags, 0)
    }
}

impl PreparedFpvModel {
    pub fn build(
        catalog: &FpvMeshCatalog,
        catalog_entry: usize,
        hide: Option<&[u32; 5]>,
        admission: &FpvMaterialAdmission,
    ) -> Result<Self, FpvRigError> {
        let entry = catalog
            .get_at(catalog_entry)
            .ok_or(FpvRigError::Catalog("model"))?;
        let skel = &entry.skel;
        let lod0 = skel.surfaces_for_lod(0);
        let t6 = entry.namespace == asset_core::AssetNamespace::T6;

        let is_hidden = |surface: usize| {
            let (Some(tags), Some(bits)) = (hide, skel.surface_part_bits.get(surface)) else {
                return false;
            };
            hides_surface(t6, bits, tags)
        };
        let is_drawn = |surface: usize| {
            !is_hidden(surface) && admission.row_of(catalog_entry, surface).is_some()
        };

        let layout = build_skin_layout(skel, 0, |within_lod| is_drawn(lod0.start + within_lod))
            .ok_or(FpvRigError::Layout("skin layout refused the model"))?;

        // The first visible surface that should have drawn and could not.
        let refusal = lod0.clone().filter(|&s| !is_hidden(s)).find_map(|surface| {
            match admission.verdict(catalog_entry, surface)? {
                FpvSurfaceVerdict::Refused { material, cause } => Some(FpvMandatoryRefusal {
                    model: skel.name.clone(),
                    surface,
                    material: material.clone(),
                    cause,
                }),
                _ => None,
            }
        });

        let surfaces = layout
            .surfaces
            .iter()
            .filter(|laid| laid.visible && laid.index_count != 0)
            .filter_map(|laid| {
                let authored = skel
                    .surface_materials
                    .get(laid.surface_index)
                    .copied()
                    .flatten()?
                    .get();
                let material = admission.row_of(catalog_entry, laid.surface_index)?;
                let lens_named = entry
                    .material_keys
                    .get(laid.surface_index)
                    .and_then(|key| Some(key.as_ref()?.name.as_str()))
                    .is_some_and(leftover_scope_surf_is_lens);
                Some(PreparedFpvSurface {
                    index_start: laid.index_start,
                    index_count: laid.index_count,
                    material,
                    authored,
                    lens_named,
                })
            })
            .collect();

        Ok(Self {
            catalog_entry,
            posed_surface_n: lod0.len(),
            packed_ok: skel.packed_vertices.len() == skel.positions.len(),
            layout,
            surfaces,
            refusal,
        })
    }

    pub fn refusal(&self) -> Option<&FpvMandatoryRefusal> {
        self.refusal.as_ref()
    }
}

// ---------------------------------------------------------------------------
// Composition of an assembly's parts
// ---------------------------------------------------------------------------

struct ComposedPart {
    model: Arc<PreparedFpvModel>,
    owner: FpvSurfOwner,
    /// First bone of this part within the assembly's shared skeleton.
    bone_base: usize,
}

/// An assembly (hands, gun, and whatever hangs off the gun) with each part's
/// model prepared.
pub struct PreparedFpvComposition {
    assembly: Arc<FpvAssembly>,
    parts: Vec<ComposedPart>,
    refusal: Option<FpvMandatoryRefusal>,
}

fn surf_owner(role: FpvPartRole) -> FpvSurfOwner {
    match role {
        FpvPartRole::Hands => FpvSurfOwner::Hands,
        FpvPartRole::Gun | FpvPartRole::Knife => FpvSurfOwner::Gun,
        FpvPartRole::Attachment => FpvSurfOwner::Scope,
        FpvPartRole::Rocket => FpvSurfOwner::Rocket,
    }
}

impl PreparedFpvComposition {
    /// `model_of` supplies the prepared model for a catalog order and the
    /// hidden tags the part asks for; the first error it reports ends the
    /// composition.
    pub fn compose(
        assembly: Arc<FpvAssembly>,
        mut model_of: impl FnMut(usize, Option<[u32; 5]>) -> Result<Arc<PreparedFpvModel>, String>,
    ) -> Result<Self, String> {
        let mut parts = Vec::with_capacity(assembly.parts.len());
        for part in &assembly.parts {
            parts.push(ComposedPart {
                model: model_of(part.model.order(), part.hide)?,
                owner: surf_owner(part.role),
                bone_base: part.bone_base,
            });
        }
        let refusal = parts.iter().find_map(|part| part.model.refusal().cloned());
        Ok(Self {
            assembly,
            parts,
            refusal,
        })
    }

    pub fn assembly(&self) -> &Arc<FpvAssembly> {
        &self.assembly
    }

    pub fn refusal(&self) -> Option<&FpvMandatoryRefusal> {
        self.refusal.as_ref()
    }
}

/// Narrows composed bone indices to the table's 16-bit form; a track with no
/// bone, or a bone beyond 16 bits, becomes `FpvClipTracks::NONE`.
fn narrow_bones(bones: Vec<Option<usize>>) -> Arc<[u16]> {
    bones
        .into_iter()
        .map(|bone| {
            bone.and_then(|bone| u16::try_from(bone).ok())
                .unwrap_or(FpvClipTracks::NONE)
        })
        .collect()
}

/// For one clip, which bone of the assembly's shared skeleton each of the
/// clip's tracks drives.
pub fn compose_clip_tracks(
    assembly: &FpvAssembly,
    clip: usize,
    tracks: &FpvClipTracks,
) -> Option<Arc<[u16]>> {
    assembly.compose_tracks(clip, tracks).map(narrow_bones)
}

/// A track table back into bone indices, `None` for tracks with no bone.
fn bones_of(table: &[u16]) -> Vec<Option<usize>> {
    table
        .iter()
        .map(|&bone| (bone != FpvClipTracks::NONE).then_some(usize::from(bone)))
        .collect()
}

// ---------------------------------------------------------------------------
// The rig: geometry plan
// ---------------------------------------------------------------------------

/// One model drawn for one hand. The same prepared model appears twice in a
/// dual-wield rig, once per hand, with a destination range of its own.
struct PlanSlot {
    hand: usize,
    part: usize,
    /// First row of this slot in the skinned vertex buffer.
    dest_base: usize,
}

/// The draw plan of a rig: every list a frame publishes, built once.
#[derive(Default)]
pub struct PreparedFpvGeometry {
    /// Each slot's model and the vertex offset its indices are rebased by.
    segments: Vec<(Arc<PreparedFpvModel>, u32)>,
    pub index_n: usize,
    pub surface_ranges: Vec<(u32, u32)>,
    pub materials: Vec<SmodelPassMaterial>,
    pub draws: Vec<FpvSurfaceDraw>,
    pub dest_n: usize,
    pub packed_ok: bool,

    pub hands_plan_n: u32,
    pub gun_plan_n: u32,
    pub scope_plan_n: u32,
    pub scope_house_plan_n: u32,
    pub scope_lens_plan_n: u32,
    pub plan_draw_n: u32,
    pub plan_skip_n: u32,
}

impl PreparedFpvGeometry {
    /// The index list of every slot in plan order, each rebased onto its own
    /// destination rows.
    pub fn write_indices(&self, out: &mut Vec<u32>) {
        out.clear();
        out.reserve(self.index_n);
        for (model, rebase) in &self.segments {
            out.extend(model.layout.indices.iter().map(|&index| index + rebase));
        }
    }
}

/// The order slots are laid out in: every hand's arms first, then every hand's
/// gun and whatever hangs off it. The scope and the rocket hang off the right
/// gun only. Pairs are (hand, part).
fn slot_order(part_count: usize, dual: bool) -> Vec<(usize, usize)> {
    let hands = if dual { 2 } else { 1 };
    let arms = (0..hands).map(|hand| (hand, 0));
    let guns = (0..hands).flat_map(|hand| {
        (1..part_count)
            .filter(move |&part| hand == 0 || part == 1)
            .map(move |part| (hand, part))
    });
    arms.chain(guns).collect()
}

/// Which counters a drawn surface moves.
struct SurfaceClass {
    first_hand: bool,
    owner: FpvSurfOwner,
    lens: bool,
}

impl PreparedFpvGeometry {
    fn count(&mut self, class: &SurfaceClass) {
        if class.first_hand {
            match class.owner {
                FpvSurfOwner::Hands => self.hands_plan_n += 1,
                _ => self.gun_plan_n += 1,
            }
        }
        if class.owner == FpvSurfOwner::Scope {
            self.scope_plan_n += 1;
            if class.lens {
                self.scope_lens_plan_n += 1;
            } else {
                self.scope_house_plan_n += 1;
            }
        }
    }

    /// Adds one slot's surfaces, returning the next free destination row.
    fn push_slot(
        &mut self,
        admission: &FpvMaterialAdmission,
        draw_material_of: &mut HashMap<usize, u32>,
        part: &ComposedPart,
        hand: usize,
        dest_base: usize,
    ) -> usize {
        let model = &part.model;
        let index_base = self.index_n as u32;
        let is_scope = part.owner == FpvSurfOwner::Scope;
        for surface in &model.surfaces {
            let Some(material) = admission.materials.get(surface.material as usize) else {
                continue;
            };
            let lens = is_scope && surface.lens_named;
            let range = self.surface_ranges.len() as u32;
            self.surface_ranges
                .push((index_base + surface.index_start, surface.index_count));
            let draw_material = *draw_material_of.entry(surface.authored).or_insert_with(|| {
                self.materials.push(material.clone());
                self.materials.len() as u32 - 1
            });
            self.draws.push(FpvSurfaceDraw {
                surface: range,
                material: draw_material,
                is_scope: is_scope && !lens,
            });
            self.count(&SurfaceClass {
                first_hand: hand == 0,
                owner: part.owner,
                lens,
            });
        }
        self.segments.push((Arc::clone(model), dest_base as u32));
        self.index_n += model.layout.indices.len();
        dest_base + model.layout.dest_vertex_n
    }
}

// ---------------------------------------------------------------------------
// The rig: posing and skinning
// ---------------------------------------------------------------------------

/// One hand's pose for a frame: the skin matrices the vertices are bound to
/// and the bones as the effects see them.
pub struct FpvHandPose {
    skin: Vec<Mat4>,
    eye_from_world: Mat4,
    offset: Vec3,
    pub lens: Mat4,
    pub bolt: FpvBoltFrame,
}

pub struct PreparedFpvRig {
    composition: Arc<PreparedFpvComposition>,
    generation: u64,
    parts: PartBits,
    tags: FpvBoltTags,
    slots: Vec<PlanSlot>,
    /// Per hand (right, left): per scheduler node, the bone each track of
    /// that node's clip drives.
    tracks: [Vec<Option<Arc<[u16]>>>; 2],
    pub geometry: PreparedFpvGeometry,
}

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

impl PreparedFpvRig {
    /// A plan holding this generation is a plan whose indices, ranges,
    /// materials and draws are this rig's.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn build(
        composition: Arc<PreparedFpvComposition>,
        dual: bool,
        admission: &FpvMaterialAdmission,
        tracks: [Vec<Option<Arc<[u16]>>>; 2],
    ) -> Self {
        let order = slot_order(composition.parts.len(), dual);
        let mut geometry = PreparedFpvGeometry {
            segments: Vec::with_capacity(order.len()),
            packed_ok: true,
            ..PreparedFpvGeometry::default()
        };
        let mut slots = Vec::with_capacity(order.len());
        let mut draw_material_of = HashMap::new();
        let mut dest_base = 0;
        let mut posed_surfaces = 0;
        for (hand, part_index) in order {
            let part = &composition.parts[part_index];
            posed_surfaces += part.model.posed_surface_n;
            geometry.packed_ok &= part.model.packed_ok;
            slots.push(PlanSlot {
                hand,
                part: part_index,
                dest_base,
            });
            dest_base = geometry.push_slot(admission, &mut draw_material_of, part, hand, dest_base);
        }
        geometry.dest_n = dest_base;
        geometry.plan_draw_n = geometry.draws.len() as u32;
        geometry.plan_skip_n = posed_surfaces.saturating_sub(geometry.draws.len()) as u32;

        let assembly = &composition.assembly;
        let found = &assembly.tags;
        let tags = FpvBoltTags {
            flash: found.flash,
            flash_silenced: found.flash_silenced,
            brass: found.brass,
            knife: found.knife,
            laser: found.laser,
            tracker_screen: found.tracker_screen,
            tracker_light: found.tracker_light,
            flash_left: found.flash_left,
            brass_left: found.brass_left,
            target: found.target,
            hud: found.hud,
        };
        Self {
            parts: assembly.dobj.all_parts(),
            composition,
            generation: NEXT_GENERATION.fetch_add(1, Ordering::Relaxed),
            tags,
            slots,
            tracks,
            geometry,
        }
    }

    /// One hand's bones for this frame. No skeleton is built here.
    /// `camera` (bo2zm) is a camera-only clip that takes the camera tag over.
    pub fn pose_hand(
        &self,
        hand: usize,
        anims: &[PosedClip<'_>],
        offset: Vec3,
        camera: Option<&PosedClip<'_>>,
    ) -> Option<FpvHandPose> {
        let listed: Vec<_> = anims.iter().map(|anim| (hand, anim)).collect();
        self.pose_listed(hand, &listed, offset, camera)
    }

    /// bo2zm: Black Ops II dual wield: one set of arms holding both guns, the
    /// right hand's clips (bound by the right table) and the left hand's (by
    /// the left table) posed together. Their animations key disjoint bones
    /// (`j_gun`, `tag_weapon`, the right arm / `j_gun1`, `tag_weapon1`, the
    /// left arm), so each bone takes its own side's.
    pub fn pose_combined(
        &self,
        right: &[PosedClip<'_>],
        left: &[PosedClip<'_>],
        camera: Option<&PosedClip<'_>>,
    ) -> Option<FpvHandPose> {
        let listed: Vec<_> = right
            .iter()
            .map(|anim| (0, anim))
            .chain(left.iter().map(|anim| (1, anim)))
            .collect();
        self.pose_listed(0, &listed, Vec3::ZERO, camera)
    }

    /// The camera clip's sample for the camera tag: the bone, the sample, and
    /// how strongly it takes the bone over.
    fn camera_override(&self, camera: &PosedClip<'_>) -> Option<(usize, CameraSample, f32)> {
        let bone = self.composition.assembly.camera_bone?;
        let table = self.tracks[0].get(camera.node)?.as_ref()?;
        let track = track_of_bone(table, bone)?;
        let sample = camera.clip.sample_track(track, camera.time)?;
        Some((
            bone,
            CameraSample {
                rotation: sample.rotation,
                translation: sample.translation,
            },
            camera.weight.clamp(0.0, 1.0),
        ))
    }

    /// Poses `anims`, each bound by its own hand's track table, as `hand`.
    fn pose_listed(
        &self,
        hand: usize,
        anims: &[(usize, &PosedClip<'_>)],
        offset: Vec3,
        camera: Option<&PosedClip<'_>>,
    ) -> Option<FpvHandPose> {
        let bound: Vec<(&PosedClip<'_>, Vec<Option<usize>>)> = anims
            .iter()
            .filter_map(|&(table, anim)| {
                let tracks = self.tracks[table].get(anim.node)?.as_ref()?;
                Some((anim, bones_of(tracks)))
            })
            .collect();
        if !bound
            .iter()
            .any(|(_, bones)| bones.iter().any(Option::is_some))
        {
            return None;
        }
        let instances: Vec<AnimInstance<'_>> = bound
            .iter()
            .map(|(anim, bones)| AnimInstance {
                clip: anim.clip,
                tracks: bones,
                time: anim.time,
                weight: anim.weight,
                parts: None,
            })
            .collect();

        let assembly = &self.composition.assembly;
        // bo2zm: a camera-only clip (the climb's) takes the camera tag over
        // the other clips by its weight; the lens follows the camera tag.
        let takeover = camera.and_then(|clip| self.camera_override(clip));
        let world = assembly.dobj.pose_with_controller(
            &instances,
            &self.parts,
            Mat4::IDENTITY,
            |_, _, locals| {
                if let Some((bone, sample, weight)) = &takeover
                    && let Some(local) = locals.get_mut(*bone)
                {
                    sample.blend_into(&mut local.rotation, &mut local.translation, *weight);
                }
            },
        );

        let mut skin = assembly.dobj.skin_matrices(&world);
        for (part, model) in assembly.parts.iter().zip(&assembly.dobj.models) {
            if let Some(tags) = &part.hide {
                collapse_hidden_bones(&mut skin, part.bone_base, model.bone_count, tags);
            }
        }

        let view = world[assembly.view_bone];
        let eye_from_world = tag_view_to_bevy_camera() * view.inverse();
        let lens = assembly.camera_bone.map_or(Mat4::IDENTITY, |camera| {
            tag_camera_lens_local(view, world[camera])
        });
        let published = published_bone_count(hand, world.len(), assembly.paired_bones);
        let shift = (offset != Vec3::ZERO).then(|| Mat4::from_translation(offset));
        let bones = world[..published]
            .iter()
            .map(|&bone| {
                let in_eye = eye_from_world * bone;
                shift.map_or(in_eye, |shift| shift * in_eye)
            })
            .collect();
        Some(FpvHandPose {
            skin,
            eye_from_world,
            offset,
            lens,
            bolt: FpvBoltFrame {
                bones,
                tags: self.tags,
            },
        })
    }

    /// bo2mp test log (`BO2MP_VMLOG` only): each clip's track count and how
    /// many of them each hand's table binds (and how many land on left-side
    /// bones), then where the guns, wrists and shoulders sit in eye space and
    /// whether their skin is hidden.
    pub fn debug_pose_line(&self, anims: &[PosedClip<'_>], pose: &FpvHandPose) -> String {
        use std::fmt::Write as _;
        let dobj = &self.composition.assembly.dobj;
        let on_left = |bone: usize| {
            dobj.bones
                .get(bone)
                .is_some_and(|b| b.name.ends_with("_le") || b.name.ends_with('1'))
        };
        let bound_by = |hand: usize, node: usize| -> (usize, usize) {
            let Some(table) = self.tracks[hand].get(node).and_then(Option::as_ref) else {
                return (0, 0);
            };
            let bones: Vec<usize> = bones_of(table).into_iter().flatten().collect();
            (bones.len(), bones.iter().filter(|&&b| on_left(b)).count())
        };
        let mut line = String::new();
        for anim in anims {
            let (right, right_left) = bound_by(0, anim.node);
            let (left, left_left) = bound_by(1, anim.node);
            let _ = write!(
                line,
                "{}:{:.2}@{:.2} n{} t{} R{right}/{right_left} L{left}/{left_left} | ",
                anim.clip.name,
                anim.weight,
                anim.time,
                anim.node,
                anim.clip.tracks.len()
            );
        }
        const WATCHED: [&str; 9] = [
            "tag_weapon",
            "tag_weapon1",
            "j_gun",
            "j_gun1",
            "j_wrist_ri",
            "j_wrist_le",
            "j_shoulder_ri",
            "j_shoulder_le",
            "tag_camera",
        ];
        for name in WATCHED {
            let Some(index) = dobj
                .bones
                .iter()
                .position(|b| b.name.eq_ignore_ascii_case(name))
            else {
                continue;
            };
            let Some(at) = pose.bolt.bones.get(index) else {
                continue;
            };
            let hidden = if pose.skin.get(index) == Some(&Mat4::ZERO) {
                "H"
            } else {
                ""
            };
            let _ = write!(
                line,
                "{name}{hidden}=({:.1},{:.1},{:.1}) ",
                at.w_axis.x, at.w_axis.y, at.w_axis.z
            );
        }
        line
    }

    /// Skins every slot into the destination buffer the plan published. A
    /// vertex update only: indices, surface ranges and materials are untouched.
    pub fn skin_into(
        &self,
        catalog: &FpvMeshCatalog,
        poses: &[Option<FpvHandPose>; 2],
        dest: &mut [[u8; asset_t6::T6_PACKED_VERTEX]],
    ) {
        for slot in &self.slots {
            let Some(pose) = poses[slot.hand].as_ref() else {
                continue;
            };
            let part = &self.composition.parts[slot.part];
            let model = &part.model;
            let Some(entry) = catalog.get_at(model.catalog_entry) else {
                continue;
            };
            let end = slot.dest_base.saturating_add(model.layout.dest_vertex_n);
            let Some(rows) = dest.get_mut(slot.dest_base..end) else {
                continue;
            };
            skin_packed_into(
                &entry.skel,
                |bone| pose.eye_from_world * pose.skin[part.bone_base + bone],
                |_| false,
                0,
                &model.layout,
                rows,
            );
            if pose.offset != Vec3::ZERO {
                translate_packed_rows(rows, pose.offset);
            }
        }
    }
}

/// A sampled camera track.
struct CameraSample {
    rotation: Option<[f32; 4]>,
    translation: Option<[f32; 3]>,
}

impl CameraSample {
    /// Blends the sample into a bone's local transform by `weight`.
    fn blend_into(&self, rotation: &mut [f32; 4], translation: &mut [f32; 3], weight: f32) {
        if let Some(sample) = self.rotation {
            *rotation = blend_rotation(*rotation, sample, weight);
        }
        if let Some(sample) = self.translation {
            *translation = xanim_t6::lerp3(*translation, sample, weight);
        }
    }
}

/// Black Ops II's anim-matrix blend of two rotations: the sample is flipped
/// when it points away, the two are summed by weight, then normalised.
fn blend_rotation(base: [f32; 4], sample: [f32; 4], weight: f32) -> [f32; 4] {
    let mut sum = dobj_t6::AnimMat::ZERO;
    sum.add_quat(base, 1.0 - weight);
    sum.add_quat(sample, weight);
    sum.finish().0
}

/// The track of a clip's table that drives `bone`.
fn track_of_bone(table: &[u16], bone: usize) -> Option<usize> {
    table
        .iter()
        .position(|&driven| driven != FpvClipTracks::NONE && usize::from(driven) == bone)
}

/// A hidden tag's vertices collapse to nothing (surfaces that also hold other
/// bones still draw the rest).
fn collapse_hidden_bones(skin: &mut [Mat4], bone_base: usize, bone_count: usize, tags: &[u32; 5]) {
    for bone in (0..bone_count).filter(|&bone| hide_part_bit(tags, bone)) {
        if let Some(matrix) = skin.get_mut(bone_base + bone) {
            *matrix = Mat4::ZERO;
        }
    }
}

/// How many bones a hand publishes to the effects: the right hand every bone,
/// the left only those of the shared arms-and-left-gun prefix.
fn published_bone_count(hand: usize, world_len: usize, paired_bones: usize) -> usize {
    if hand == 0 {
        world_len
    } else {
        paired_bones.min(world_len)
    }
}

/// Moves every packed vertex position by `delta`; the rest of the row stays.
fn translate_packed_rows(rows: &mut [[u8; asset_t6::T6_PACKED_VERTEX]], delta: Vec3) {
    for row in rows {
        for (axis, shift) in [delta.x, delta.y, delta.z].into_iter().enumerate() {
            let at = axis * 4..axis * 4 + 4;
            let moved = f32::from_le_bytes(row[at.clone()].try_into().expect("four bytes")) + shift;
            row[at].copy_from_slice(&moved.to_le_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bit(bone: usize) -> [u32; 5] {
        let mut words = [0; 5];
        words[bone >> 5] = 0x8000_0000 >> (bone & 31);
        words
    }

    fn either(a: [u32; 5], b: [u32; 5]) -> [u32; 5] {
        std::array::from_fn(|i| a[i] | b[i])
    }

    #[test]
    fn leftover_lens_surfaces_are_found_by_name() {
        assert!(leftover_scope_surf_is_lens("mtl_scope_lens_glass"));
        assert!(!leftover_scope_surf_is_lens("mtl_scope_body"));
    }

    #[test]
    fn admission_answers_by_entry_and_surface() {
        let mut admission = FpvMaterialAdmission::default();
        admission.record(
            7,
            vec![
                (0, FpvSurfaceVerdict::Admitted(3)),
                (1, FpvSurfaceVerdict::Inapplicable("no colour map")),
                (
                    2,
                    FpvSurfaceVerdict::Refused {
                        material: "m".into(),
                        cause: "why",
                    },
                ),
            ],
        );
        assert_eq!(admission.row_of(7, 0), Some(3));
        assert_eq!(admission.row_of(7, 1), None, "inapplicable draws nothing");
        assert_eq!(admission.row_of(7, 2), None, "refused draws nothing");
        assert_eq!(admission.row_of(7, 9), None, "unknown surface");
        assert_eq!(admission.row_of(8, 0), None, "unknown entry");
        assert_eq!(admission.verdicts_of(7).len(), 3);
        assert!(admission.verdicts_of(8).is_empty());
        assert!(matches!(
            admission.verdict(7, 2),
            Some(FpvSurfaceVerdict::Refused { cause: "why", .. })
        ));
    }

    #[test]
    fn a_refusal_names_the_model_surface_and_material() {
        let refusal = FpvMandatoryRefusal {
            model: "viewmodel_gun".into(),
            surface: 2,
            material: "mtl_gun".into(),
            cause: "colour map not uploaded",
        };
        assert_eq!(
            refusal.to_string(),
            "viewmodel_gun surface 2 `mtl_gun`: colour map not uploaded"
        );
        assert_eq!(
            FpvRigError::Catalog("model").to_string(),
            "model missing from the first-person catalog"
        );
        assert_eq!(FpvRigError::Layout("bad").to_string(), "bad");
    }

    #[test]
    fn black_ops_drops_a_surface_only_when_all_its_bones_are_hidden() {
        let both = either(bit(1), bit(2));
        assert!(!hides_surface(true, &both, &bit(1)), "one bone left");
        assert!(hides_surface(true, &both, &both));
        assert!(!hides_surface(true, &[0; 5], &both), "bound to nothing");
    }

    #[test]
    fn other_games_drop_a_surface_when_any_bone_is_hidden() {
        let both = either(bit(1), bit(2));
        assert!(hides_surface(false, &both, &bit(1)));
        assert!(!hides_surface(false, &both, &bit(3)));
    }

    #[test]
    fn narrowing_marks_missing_and_oversized_bones() {
        let narrowed = narrow_bones(vec![Some(4), None, Some(70_000), Some(0)]);
        assert_eq!(
            &narrowed[..],
            [4, FpvClipTracks::NONE, FpvClipTracks::NONE, 0]
        );
    }

    #[test]
    fn a_track_table_binds_back_to_bones() {
        let table = [5, FpvClipTracks::NONE, 0];
        assert_eq!(bones_of(&table), [Some(5), None, Some(0)]);
        assert_eq!(track_of_bone(&table, 0), Some(2));
        assert_eq!(track_of_bone(&table, 5), Some(0));
        assert_eq!(track_of_bone(&table, 9), None);
        assert_eq!(
            track_of_bone(&[FpvClipTracks::NONE], usize::from(FpvClipTracks::NONE)),
            None
        );
    }

    #[test]
    fn one_hand_lays_out_arms_then_everything_on_the_gun() {
        assert_eq!(slot_order(3, false), [(0, 0), (0, 1), (0, 2)]);
        assert_eq!(slot_order(1, false), [(0, 0)]);
    }

    #[test]
    fn two_hands_lay_out_both_arms_first_and_give_the_left_only_a_gun() {
        assert_eq!(
            slot_order(4, true),
            [(0, 0), (1, 0), (0, 1), (0, 2), (0, 3), (1, 1)]
        );
    }

    #[test]
    fn a_rotation_blend_at_zero_weight_keeps_the_base() {
        let base = [0.0, 0.0, 0.0, 1.0];
        let other = [0.0, 1.0, 0.0, 0.0];
        assert_eq!(blend_rotation(base, other, 0.0), base);
        assert_eq!(blend_rotation(base, other, 1.0), other);
    }

    #[test]
    fn a_rotation_blend_flips_a_sample_that_points_away() {
        let base = [0.0, 0.0, 0.0, 1.0];
        let opposite = [0.0, 0.0, 0.0, -1.0];
        assert_eq!(blend_rotation(base, opposite, 0.5), base);
    }

    #[test]
    fn a_camera_sample_only_moves_what_it_carries() {
        let mut rotation = [0.0, 0.0, 0.0, 1.0];
        let mut translation = [1.0, 2.0, 3.0];
        let nothing = CameraSample {
            rotation: None,
            translation: None,
        };
        nothing.blend_into(&mut rotation, &mut translation, 1.0);
        assert_eq!(
            (rotation, translation),
            ([0.0, 0.0, 0.0, 1.0], [1.0, 2.0, 3.0])
        );

        let moves = CameraSample {
            rotation: None,
            translation: Some([3.0, 2.0, 1.0]),
        };
        moves.blend_into(&mut rotation, &mut translation, 0.5);
        assert_eq!(translation, [2.0, 2.0, 2.0]);
        assert_eq!(rotation, [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn hidden_tags_zero_only_their_own_bones_skin() {
        let mut skin = vec![Mat4::IDENTITY; 6];
        let tags = either(bit(0), bit(2));
        collapse_hidden_bones(&mut skin, 2, 3, &tags);
        assert_eq!(skin[2], Mat4::ZERO);
        assert_eq!(skin[3], Mat4::IDENTITY);
        assert_eq!(skin[4], Mat4::ZERO);
        assert_eq!(skin[0], Mat4::IDENTITY, "bones before the part are not its");
        assert_eq!(skin[5], Mat4::IDENTITY, "bones past its count are not its");
    }

    #[test]
    fn hidden_bones_beyond_the_skin_are_ignored() {
        let mut skin = vec![Mat4::IDENTITY; 2];
        collapse_hidden_bones(&mut skin, 1, 4, &bit(3));
        assert!(skin.iter().all(|m| *m == Mat4::IDENTITY));
    }

    #[test]
    fn the_left_hand_publishes_only_the_paired_prefix() {
        assert_eq!(published_bone_count(0, 40, 12), 40);
        assert_eq!(published_bone_count(1, 40, 12), 12);
        assert_eq!(published_bone_count(1, 8, 12), 8);
    }

    #[test]
    fn translating_rows_moves_positions_and_leaves_the_rest() {
        let mut row = [0xAB; asset_t6::T6_PACKED_VERTEX];
        for (axis, value) in [1.0f32, 2.0, 3.0].into_iter().enumerate() {
            row[axis * 4..axis * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        let mut rows = [row, row];
        translate_packed_rows(&mut rows, Vec3::new(0.5, -1.0, 2.0));
        for moved in rows {
            let read =
                |axis: usize| f32::from_le_bytes(moved[axis * 4..axis * 4 + 4].try_into().unwrap());
            assert_eq!([read(0), read(1), read(2)], [1.5, 1.0, 5.0]);
            assert!(moved[12..].iter().all(|&b| b == 0xAB));
        }
    }

    #[test]
    fn a_geometry_plan_counts_scope_housing_and_lens_apart() {
        let mut geometry = PreparedFpvGeometry::default();
        for class in [
            SurfaceClass {
                first_hand: true,
                owner: FpvSurfOwner::Hands,
                lens: false,
            },
            SurfaceClass {
                first_hand: true,
                owner: FpvSurfOwner::Gun,
                lens: false,
            },
            SurfaceClass {
                first_hand: false,
                owner: FpvSurfOwner::Gun,
                lens: false,
            },
            SurfaceClass {
                first_hand: true,
                owner: FpvSurfOwner::Scope,
                lens: false,
            },
            SurfaceClass {
                first_hand: true,
                owner: FpvSurfOwner::Scope,
                lens: true,
            },
            SurfaceClass {
                first_hand: true,
                owner: FpvSurfOwner::Rocket,
                lens: false,
            },
        ] {
            geometry.count(&class);
        }
        assert_eq!(geometry.hands_plan_n, 1);
        assert_eq!(
            geometry.gun_plan_n, 4,
            "gun, scope housing, scope lens and rocket of the first hand"
        );
        assert_eq!(geometry.scope_plan_n, 2);
        assert_eq!(geometry.scope_house_plan_n, 1);
        assert_eq!(geometry.scope_lens_plan_n, 1);
    }
}
