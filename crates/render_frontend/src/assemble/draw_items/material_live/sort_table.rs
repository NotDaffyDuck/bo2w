//! The sorted-material table: each material's place among all the others,
//! which becomes the ordinal baked into its draw-surf key.
//!
//! The places are the ones the original renderer's material sort gave, so
//! they come from the same comparator: sort key, a few flag bits, shader
//! names and, for some passes, pixel constants. That comparator need not be
//! a total order, so the sort is a plain insertion sort, which treats an
//! inconsistent pair the same way on every run.

use core::cmp::{Ordering, Reverse};
use std::collections::BTreeMap;

use asset_material::{
    DesignerMaterial, MaterialDefinitions, SelfContainedShaderPass, HeldShaderArg, SelfContainedShaderLink,
    SelfContainedRenderRecipe, RecipeBundleId, RecipeBundleLookup,
};

use crate::assemble::draw_items::material_live::{LedgerBuildError, MaterialAssetKey, SortedMaterialRank};

/// How two records order, or why they could not be compared.
/// Skip counts keyed by name.

/// The technique slots whose first pass the comparator reads, in the order
/// [`SortRecord::passes`] holds them.
const PASS_SLOTS: [u8; 2] = [5, 9];

/// The first pass of a technique, as the comparator sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PassKey {
    pixel_shader: String,
    vertex_shader: String,
    constants: PixelConstants,
}

/// The bits of a material the comparator ranks by after the sort key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RankBits {
    info_game: u8,
    state: u8,
    early_pass: u8,
}

/// One material, as the comparator sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SortRecord {
    id: MaterialAssetKey,
    name: String,
    techset: String,
    order_key: u8,
    bits: RankBits,
    /// The passes in [`PASS_SLOTS`], when the technique set has them.
    passes: [Option<PassKey>; 2],
}

impl SortRecord {
    fn pass(&self, slot: u8) -> Option<&PassKey> {
        let at = PASS_SLOTS.iter().position(|&s| s == slot)?;
        self.passes[at].as_ref()
    }
}

/// The built table, ready to be stored in the catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SortedBuild {
    pub asset_ids_by_ordinal: Vec<MaterialAssetKey>,
    pub ranks_by_asset_id: Vec<Option<SortedMaterialRank>>,
    pub skipped_n: u32,
    pub slot_gap_n: u32,
    pub first_skip: Option<String>,
    pub skipped_tech: Option<String>,
    pub skipped_cause: Option<String>,
}

// ---------------------------------------------------------------------
// Reading a material into a record
// ---------------------------------------------------------------------

/// The argument-type number the original pass tables use for an argument.
fn argument_type(arg: &HeldShaderArg) -> u16 {

    match arg {
        HeldShaderArg::MaterialVertConst { .. } => 0,
        HeldShaderArg::FixedVertexConstant { .. } => 1,
        HeldShaderArg::OwnPixelSampler { .. } => 2,
        HeldShaderArg::EngineVertexConstant { .. } => 3,
        HeldShaderArg::EnginePixelSampler { .. } => 4,
        HeldShaderArg::EnginePixelConstant { .. } => 5,
        HeldShaderArg::OwnPixelConstant { .. } => 6,
        HeldShaderArg::FixedPixelConstant { .. } => 7,
        HeldShaderArg::Unknown { param_type: arg_type, .. } => *arg_type,
    }
}

/// The constants of a pass that the comparator can see.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PixelConstants {
    code: Vec<u16>,
    /// Destination register and value words, by register.
    values: Vec<(u16, [u32; 4])>,
}

#[derive(Debug, PartialEq, Eq)]
enum ConstantFault {
    MaterialConstantMissing { label_digest: u32 },
    LiteralMissing,
}

/// Reads the pixel constants from a pass's stable arguments, where they end
/// the list in runs: code constants, material constants, then literals.
/// `material_words` gives the material's value for a named constant; a name
/// it lacks is passed over if `may_skip_missing` allows, a fault if not.
fn ps_constants(
    stable: &[HeldShaderArg],
    material_words: impl Fn(u32) -> Option<[u32; 4]>,
    may_skip_missing: impl Fn() -> bool,
) -> Result<PixelConstants, ConstantFault> {
    let start_of = |type_code: u16| {
        let at = stable.iter().position(|a| argument_type(a) >= type_code);
        at.unwrap_or(stable.len())
    };

    let code = stable[start_of(5)..]
        .iter()
        .map_while(|a| match a {
            HeldShaderArg::EnginePixelConstant { const_slot: index, .. } => Some(*index),
            _ => None,
        })
        .collect();

    let constants = &stable[start_of(6)..];
    let named: Vec<(u16, u32)> = constants
        .iter()
        .map_while(|a| match a {
            HeldShaderArg::OwnPixelConstant { slot: dest, label_digest } => {
                Some((*dest, *label_digest))
            }
            _ => None,
        })
        .collect();

    let mut values = Vec::with_capacity(constants.len());
    for &(destination, label_digest) in &named {
        if let Some(words) = material_words(label_digest) {
            values.push((destination, words));
        } else if !may_skip_missing() {
            return Err(ConstantFault::MaterialConstantMissing { label_digest });
        }
    }
    for argument in &constants[named.len()..] {
        let HeldShaderArg::FixedPixelConstant { slot: dest, words } = argument else {
            break;
        };
        let words = words.ok_or(ConstantFault::LiteralMissing)?;
        values.push((*dest, words));
    }
    values.sort_by_key(|entry| entry.0);
    Ok(PixelConstants { code, values })
}

/// A Black Ops 1 material can name a technique set whose "x0" token the
/// build stripped; the constants that token brought have no source, and the
/// comparator passes over them.
fn leftover_x_token_aliased(source: &MaterialDefinitions, material: &DesignerMaterial) -> bool {
    if material.namespace != asset_core::AssetCategoryScope::T5 {
        return false;
    }
    let named = asset_core::AssetPointer::bare_name(material.method_set.as_str());
    let stripped = asset_material::without_t5_feature_tag(named);
    if stripped == named {
        return false;
    }
    source
        .method_set_facts()
        .iter()
        .filter(|set| set.table.is_some() && set.name.is_genuine())
        .any(|set| set.name.as_str() == stripped)
}

/// Turns one material of the definitions into a [`SortRecord`].
struct RecordingFileReader<'a> {
    source: &'a MaterialDefinitions,
    material: &'a DesignerMaterial,
    id: MaterialAssetKey,
}

impl<'a> RecordingFileReader<'a> {
    fn body_missing(&self, slot: u8) -> LedgerBuildError {
        LedgerBuildError::RecipeBodyAbsent { material: self.id, slot }
    }

    fn graph_missing(&self) -> LedgerBuildError {
        LedgerBuildError::TechniqueGraphMissing { material: self.id }
    }

    fn shader(&self, slot: u8, reference: &SelfContainedShaderLink) -> Result<String, LedgerBuildError> {
        let shader = reference.shader_slot.and_then(|i| self.source.shaders.get(i));
        let missing = || LedgerBuildError::ShaderPairAbsent { material: self.id, slot };
        shader.map(|s| s.name.as_str().to_owned()).ok_or_else(missing)
    }

    /// The stable run of a pass's arguments (after the per-primitive and
    /// per-object ones).
    fn stable<'p>(
        &self,
        pass: &'p SelfContainedShaderPass,
        slot: u8,
    ) -> Result<&'p [HeldShaderArg], LedgerBuildError> {
        let [prim, obj, stable] =
            [pass.per_prim_arg_tally, pass.per_obj_arg_tally, pass.stable_arg_tally].map(usize::from);
        let first = prim.saturating_add(obj);
        let range = first..first.saturating_add(stable);
        pass.arguments.get(range).ok_or_else(|| self.body_missing(slot))
    }

    fn material_words(&self, label_digest: u32) -> Option<[u32; 4]> {
        let constant = self.material.constants.iter().find(|c| c.label_digest == label_digest)?;
        Some(constant.literal.map(f32::to_bits))
    }

    fn pass_key(&self, slot: u8, technique: &SelfContainedRenderRecipe) -> Result<PassKey, LedgerBuildError> {
        let pass = technique.passes.first().ok_or_else(|| self.body_missing(slot))?;
        let stable = self.stable(pass, slot)?;
        let skip_missing = || leftover_x_token_aliased(self.source, self.material);
        let constants = ps_constants(stable, |hash| self.material_words(hash), skip_missing)
            .map_err(|fault| match fault {
                ConstantFault::MaterialConstantMissing { label_digest } => {
                    LedgerBuildError::LocalConstantAbsent { material: self.id, label_digest }
                }
                ConstantFault::LiteralMissing => {
                    LedgerBuildError::LiteralFragmentAbsent { material: self.id, slot }
                }
            })?;
        let pixel_shader = self.shader(slot, &pass.pixel_shader)?;
        let vertex_shader = self.shader(slot, &pass.vertex_shader)?;
        Ok(PassKey { pixel_shader, vertex_shader, constants })
    }

    fn record(&self) -> Result<SortRecord, LedgerBuildError> {
        let material = self.material;
        let key = RecipeBundleId::new(material.namespace, material.method_set.as_str());
        let set = match self.source.resolve_method_set(key) {
            RecipeBundleLookup::Hit { facts, .. } => facts,
            RecipeBundleLookup::GraphMissing { .. } => return Err(self.graph_missing()),
            RecipeBundleLookup::Foreign { holder: got, .. } => {
                let want = material.namespace;
                let id = self.id;
                return Err(LedgerBuildError::SetNamespaceDiffers { material: id, want, got });
            }
            RecipeBundleLookup::Missing => {
                return Err(LedgerBuildError::TechniqueSetMissing { material: self.id });
            }
        };
        let Some(table) = set.table.as_ref() else {
            return Err(self.graph_missing());
        };
        let Some(graph) = table.graph.as_ref() else {
            return Err(self.graph_missing());
        };
        let has_slot = |slot: u8| table.cells & (1 << slot) != 0;

        let prepass = dpvs_t6::prepass_flag_of(dpvs_t6::MaterialPrepassInput {
            technique0_present: has_slot(0),
            has_second_pass: has_slot(1),
            technique0_flags: table.technique0_flags,
            condition_bits: material.condition_bits,
            techset_name_has_flag: material.method_set.as_str().contains("flag_"),
        });

        let mut passes: [Option<PassKey>; 2] = [None, None];
        for (out, slot) in passes.iter_mut().zip(PASS_SLOTS) {
            if !has_slot(slot) {
                continue;
            }
            let technique = graph.slots.get(usize::from(slot)).and_then(Option::as_ref);
            let technique = technique.ok_or_else(|| self.body_missing(slot))?;
            *out = Some(self.pass_key(slot, technique)?);
        }

        let (id, name) = (self.id, material.name.to_string());
        Ok(SortRecord {
            id,
            name,
            techset: set.name.to_string(),
            order_key: material.draw_order,
            bits: RankBits {
                info_game: material.game_flag_info,
                state: material.condition_bits,
                early_pass: prepass,
            },
            passes,
        })
    }
}

fn checker_record(source: &MaterialDefinitions, index: usize) -> Result<SortRecord, LedgerBuildError> {
    let Ok(raw_id) = u16::try_from(index) else {
        return Err(LedgerBuildError::MaterialAssetNumberTooLarge { index });
    };
    let reader = RecordingFileReader {
        source,
        material: &source.materials[index],
        id: MaterialAssetKey(raw_id),
    };
    reader.record()
}

// ---------------------------------------------------------------------
// The comparator
// ---------------------------------------------------------------------

/// Which material and slot a comparison is reading, for its error.
#[derive(Clone, Copy, Debug)]
struct CompareAt {
    material: MaterialAssetKey,
    slot: u8,
}

/// Orders two constants' words as floats, word by word. A NaN met before
/// any difference cannot be ordered.
fn compare_float_words(a: &[u32; 4], b: &[u32; 4], at: CompareAt) -> Result<Ordering, LedgerBuildError> {
    let first_difference = a
        .iter()
        .zip(b)
        .map(|(&x, &y)| f32::from_bits(x).partial_cmp(&f32::from_bits(y)))
        .find(|order| *order != Some(Ordering::Equal));
    match first_difference {
        None => Ok(Ordering::Equal),
        Some(Some(order)) => Ok(order),
        Some(None) => Err(LedgerBuildError::CompareConstantNotFinite {
            material: at.material,
            slot: at.slot,
        }),
    }
}

/// Orders two passes' constants: code constants, then how many pixel
/// constants, then each pixel constant's register and words.
fn diff_pass_args(a: &PassKey, b: &PassKey, at: CompareAt) -> Result<Ordering, LedgerBuildError> {
    let (left, right) = (&a.constants, &b.constants);
    let head = left.code.cmp(&right.code).then(left.values.len().cmp(&right.values.len()));
    if head.is_ne() {
        return Ok(head);
    }
    for ((reg_a, words_a), (reg_b, words_b)) in left.values.iter().zip(&right.values) {
        let seq = match reg_a.cmp(reg_b) {
            Ordering::Equal => compare_float_words(words_a, words_b, at)?,
            differs => differs,
        };
        if seq.is_ne() {
            return Ok(seq);
        }
    }
    Ok(head)
}

fn flag_bit(flags: u8, bit: u8) -> u8 {
    (flags >> bit) & 1
}

/// The comparator and the sort, with the count of slot gaps they met.
#[derive(Default)]
struct Sorter {
    /// Comparisons where only one material had the slot's pass. The
    /// original comparator never let that happen; the one who has it goes
    /// first.
    slot_spacing: u32,
}

impl Sorter {
    /// Orders two materials' passes in `slot` (pixel shader, then the
    /// constants when `with_args`, then the vertex shader).
    fn slot_order(
        &mut self,
        slot: u8,
        a: &SortRecord,
        b: &SortRecord,
        with_args: bool,
    ) -> Result<Ordering, LedgerBuildError> {
        let (pass_a, pass_b) = match (a.pass(slot), b.pass(slot)) {
            (Some(pass_a), Some(pass_b)) => (pass_a, pass_b),
            (None, None) => return Ok(Ordering::Equal),
            (pass_a, pass_b) => {
                self.slot_spacing = self.slot_spacing.saturating_add(1);
                if self.slot_spacing == 1 {
                    diag::info!(
                        World,
                        "drawsurf sorted materials: slot {slot} occupancy gap {} vs {} \
                         — present-first (not ComparatorInvariant)",
                        a.name,
                        b.name
                    );
                }
                return Ok(pass_b.is_some().cmp(&pass_a.is_some()));
            }
        };
        let by_pixel = pass_a.pixel_shader.cmp(&pass_b.pixel_shader);
        if by_pixel.is_ne() {
            return Ok(by_pixel);
        }
        if with_args {
            let at = CompareAt { material: a.id, slot };
            let by_args = diff_pass_args(pass_a, pass_b, at)?;
            if by_args.is_ne() {
                return Ok(by_args);
            }
        }
        Ok(pass_a.vertex_shader.cmp(&pass_b.vertex_shader))
    }

    fn compare(&mut self, a: &SortRecord, b: &SortRecord) -> Result<Ordering, LedgerBuildError> {
        let a_has_9 = a.pass(9).is_some();
        let lead = if a_has_9 {
            flag_bit(b.bits.info_game, 1).cmp(&flag_bit(a.bits.info_game, 1))
        } else {
            b.pass(5).is_some().cmp(&a.pass(5).is_some())
        };
        let head = a
            .order_key
            .cmp(&b.order_key)
            .then(lead)
            .then(a.bits.early_pass.cmp(&b.bits.early_pass))
            .then(flag_bit(b.bits.state, 3).cmp(&flag_bit(a.bits.state, 3)));
        if head.is_ne() {
            return Ok(head);
        }
        let by_slot = match (a_has_9, b.pass(9).is_some()) {
            (true, _) => self.slot_order(9, a, b, a.bits.state & 8 != 0)?,
            (false, true) => self.slot_order(5, a, b, true)?,
            (false, false) => Ordering::Equal,
        };
        Ok(by_slot
            .then_with(|| a.techset.cmp(&b.techset))
            .then_with(|| a.name.cmp(&b.name)))
    }

    /// Insertion sort by [`Self::compare`]: each record sinks while it
    /// compares less than its left neighbour.
    fn sort(&mut self, records: &mut [SortRecord]) -> Result<(), LedgerBuildError> {
        for placed in 1..records.len() {
            for at in (1..=placed).rev() {
                if !self.compare(&records[at], &records[at - 1])?.is_lt() {
                    break;
                }
                records.swap(at, at - 1);
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------
// Skipped materials
// ---------------------------------------------------------------------

/// The causes that leave a material out of the table (and counted) rather
/// than fail the whole build: data the material needs is missing.
const SKIPPED_CAUSES: [&str; 7] = [
    "TechniqueSetMissing",
    "SetNamespaceDiffers",
    "TechniqueGraphMissing",
    "RecipeBodyAbsent",
    "ShaderPairAbsent",
    "LocalConstantAbsent",
    "LiteralFragmentAbsent",
];

/// The variant name of a build error.
fn cause_name(cause: &LedgerBuildError) -> String {
    let text = format!("{cause:?}");
    let end = text.find([' ', '{', '(']).unwrap_or(text.len());
    text[..end].to_owned()
}

fn comparator_skippable(cause: &LedgerBuildError) -> bool {
    SKIPPED_CAUSES.contains(&cause_name(cause).as_str())
}

/// The name a cause is tallied under: its variant name when it is a
/// skipped cause or the capacity refusal, "other" for anything else.
fn skip_cause_key(cause: &LedgerBuildError) -> String {
    let name = cause_name(cause);
    if SKIPPED_CAUSES.contains(&name.as_str()) || name == "RankTableFull" {
        name
    } else {
        "other".to_owned()
    }
}

/// The six biggest entries of `counts`, biggest first (ties by name), as
/// `name:count,...`; `None` when nothing was counted.
fn six_biggest(counts: &BTreeMap<String, u32>) -> Option<String> {
    let mut ranked: Vec<_> = counts.iter().collect();
    // The map is already in name order, and a stable sort keeps that order among equal counts.
    ranked.sort_by_key(|&(_, count)| Reverse(*count));
    let shown: Vec<String> = ranked.iter().take(6).map(|(name, count)| format!("{name}:{count}")).collect();
    (!shown.is_empty()).then(|| shown.join(","))
}

/// What was learned about the materials left out.
#[derive(Default)]
struct Skips {
    by_tech: BTreeMap<String, u32>,
    by_cause: BTreeMap<String, u32>,
    /// The first one, as `cause name`.
    first: Option<String>,
    /// The first one whose technique set carries an "x0" token.
    first_x0: Option<String>,
}

impl Skips {
    fn record_once(&mut self, cause: LedgerBuildError, material: &DesignerMaterial) {
        let tech = material.method_set.as_str();
        let tally = |map: &mut BTreeMap<String, u32>, key: String| *map.entry(key).or_default() += 1;
        tally(&mut self.by_tech, tech.to_owned());
        tally(&mut self.by_cause, skip_cause_key(&cause));
        if tech.contains("x0") {
            self.first_x0
                .get_or_insert_with(|| format!("{cause:?} {} tech={tech}", material.name));
        }
        self.first.get_or_insert_with(|| format!("{cause:?} {}", material.name));
    }

    /// How many were left out (each one tallied once by technique set).
    fn count(&self) -> u32 {
        self.by_tech.values().fold(0, |sum, &n| sum.saturating_add(n))
    }
}

// ---------------------------------------------------------------------
// The table
// ---------------------------------------------------------------------

/// The records in sorted order, the ordinal of each asset id, and the
/// sort's slot-gap count.
struct Ranking {
    records: Vec<SortRecord>,
    ordinals: Vec<Option<SortedMaterialRank>>,
    slot_spacing: u32,
}

/// Sorts `records` and numbers them; `material_tally` is how many
/// materials the catalog has, ranked or not.
fn rank(mut records: Vec<SortRecord>, material_tally: usize) -> Result<Ranking, LedgerBuildError> {
    let mut sorter = Sorter::default();
    sorter.sort(&mut records)?;
    let ranked = records.len();
    let full = |capacity| LedgerBuildError::RankTableFull { count: ranked, capacity };
    if ranked > SortedMaterialRank::SHIPPED_SLOT_CEILING {
        return Err(full(SortedMaterialRank::SHIPPED_SLOT_CEILING));
    }
    let mut ordinals = vec![None; material_tally];
    for (place, rec) in records.iter().enumerate() {
        let number = u32::try_from(place).map_err(|_| full(SortedMaterialRank::SLOT_CEILING))?;
        ordinals[usize::from(rec.id.0)] = Some(SortedMaterialRank::new(number)?);
    }
    Ok(Ranking { records, ordinals, slot_spacing: sorter.slot_spacing })
}

pub(super) fn build_sorted_material_table(
    source: &MaterialDefinitions,
) -> Result<SortedBuild, LedgerBuildError> {
    let mut skips = Skips::default();
    let materials = &source.materials;
    let kept = materials
        .iter()
        .enumerate()
        .filter_map(|(at, material)| match checker_record(source, at) {
            Err(why) if comparator_skippable(&why) => {
                skips.record_once(why, material);
                None
            }
            outcome => Some(outcome),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let ranking = rank(kept, source.materials.len())?;

    let skipped = skips.count();
    let skip_tech = six_biggest(&skips.by_tech);
    let skip_cause = six_biggest(&skips.by_cause);
    if let Some(first) = &skips.first {
        let dash = |text: &Option<String>| text.as_deref().unwrap_or("-").to_owned();
        let ranked = ranking.records.len();
        diag::info!(
            World,
            "drawsurf sorted materials: skipped={} ranked={} first_skip={} skip_cause={} \
             skip_tech={} first_x0={}",
            skipped,
            ranked,
            first,
            dash(&skip_cause),
            dash(&skip_tech),
            dash(&skips.first_x0),
        );
    }
    Ok(SortedBuild {
        asset_ids_by_ordinal: ranking.records.iter().map(|r| r.id).collect(),
        ranks_by_asset_id: ranking.ordinals,
        skipped_n: skipped,
        slot_gap_n: ranking.slot_spacing,
        first_skip: skips.first,
        skipped_tech: skip_tech,
        skipped_cause: skip_cause,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pass_key(pixel: &str, vertex: &str) -> PassKey {
        PassKey {
            pixel_shader: pixel.into(),
            vertex_shader: vertex.into(),
            constants: PixelConstants { code: Vec::new(), values: Vec::new() },
        }
    }

    fn with_constants(code: &[u16], values: &[(u16, [u32; 4])], vertex: &str) -> PassKey {
        let mut key = pass_key("p", vertex);
        key.constants = PixelConstants { code: code.to_vec(), values: values.to_vec() };
        key
    }

    fn record(id: u16, name: &str) -> SortRecord {
        SortRecord {
            id: MaterialAssetKey(id),
            name: name.into(),
            techset: "set".into(),
            order_key: 0,
            bits: RankBits::default(),
            passes: [None, None],
        }
    }

    /// Slot 5 and slot 9 passes, by index into `passes`.
    const FIVE: usize = 0;
    const NINE: usize = 1;

    fn compare(a: &SortRecord, b: &SortRecord) -> Ordering {
        Sorter::default().compare(a, b).expect("comparable")
    }

    fn words(a: f32, b: f32, c: f32, d: f32) -> [u32; 4] {
        [a, b, c, d].map(f32::to_bits)
    }

    fn at(material: u16, slot: u8) -> CompareAt {
        CompareAt { material: MaterialAssetKey(material), slot }
    }

    // ----- argument reading ------------------------------------------

    fn code_const(index: u16) -> HeldShaderArg {
        HeldShaderArg::EnginePixelConstant { const_slot: index, line_total: 1, into: 0, row_first: 0 }
    }

    fn material_const(dest: u16, label_digest: u32) -> HeldShaderArg {
        HeldShaderArg::OwnPixelConstant { slot: dest, label_digest }
    }

    fn literal_const(target_dst: u16, word_list: Option<[u32; 4]>) -> HeldShaderArg {
        HeldShaderArg::FixedPixelConstant { slot: target_dst, words: word_list }
    }

    fn sampler(dest: u16) -> HeldShaderArg {
        HeldShaderArg::OwnPixelSampler {
            slot: dest,
            label_digest: 1,
        }
    }

    #[test]
    fn argument_types_use_the_original_numbering() {
        let dst = 0;
        let vertex_material =
            HeldShaderArg::MaterialVertConst { slot: dst, label_digest: 0 };
        let vertex_literal = HeldShaderArg::FixedVertexConstant { slot: dst, bit_words: None };
        let vertex_code =
            HeldShaderArg::EngineVertexConstant { index: 0, line_total: 1, slot: dst, start_row: 0 };
        let pixel_code_sampler = HeldShaderArg::EnginePixelSampler { index: 0, slot: dst };
        let unknown = HeldShaderArg::Unknown { raw: [0; 8], param_type: 99 };
        let cases = [
            (vertex_material, 0),
            (vertex_literal, 1),
            (sampler(0), 2),
            (vertex_code, 3),
            (pixel_code_sampler, 4),
            (code_const(0), 5),
            (material_const(0, 0), 6),
            (literal_const(0, None), 7),
            (unknown, 99),
        ];
        for (argument, code) in cases {
            assert_eq!(argument_type(&argument), code, "{argument:?}");
        }
    }

    fn no_words(_: u32) -> Option<[u32; 4]> {
        None
    }

    #[test]
    fn pixel_constants_are_read_from_the_runs_at_the_end() {
        let stable = [
            sampler(0),
            code_const(11),
            code_const(12),
            material_const(4, 100),
            material_const(2, 200),
            literal_const(3, Some(words(1.0, 2.0, 3.0, 4.0))),
        ];
        let lookup = |hash| match hash {
            100 => Some(words(9.0, 9.0, 9.0, 9.0)),
            200 => Some(words(8.0, 8.0, 8.0, 8.0)),
            _ => None,
        };
        let read = ps_constants(&stable, lookup, || false).expect("every constant is there");
        assert_eq!(read.code, [11, 12]);
        // Sorted by destination register, whatever order they came in.
        let expected = [
            (2, words(8.0, 8.0, 8.0, 8.0)),
            (3, words(1.0, 2.0, 3.0, 4.0)),
            (4, words(9.0, 9.0, 9.0, 9.0)),
        ];
        assert_eq!(read.values, expected);
    }

    #[test]
    fn a_pass_without_pixel_constants_reads_as_empty() {
        let empty = PixelConstants { code: Vec::new(), values: Vec::new() };
        let read = ps_constants(&[sampler(0)], no_words, || false).expect("nothing to read");
        assert_eq!(read, empty);
        let none = ps_constants(&[], no_words, || false).expect("nothing to read");
        assert!(none.values.is_empty() && none.code.is_empty());
    }

    #[test]
    fn a_material_constant_the_material_lacks_is_a_fault() {
        let read = ps_constants(&[material_const(1, 77)], no_words, || false);
        assert_eq!(read, Err(ConstantFault::MaterialConstantMissing { label_digest: 77 }));
    }

    #[test]
    fn a_missing_constant_is_passed_over_when_the_caller_allows_it() {
        let stable = [material_const(1, 77), material_const(2, 78)];
        let five = words(5.0, 5.0, 5.0, 5.0);
        let read = ps_constants(&stable, |hash| (hash == 78).then_some(five), || true)
            .expect("the missing one is skipped");
        assert_eq!(read.values, [(2, five)]);
    }

    #[test]
    fn a_literal_without_words_is_a_fault() {
        let read = ps_constants(&[literal_const(1, None)], no_words, || true);
        assert_eq!(read, Err(ConstantFault::LiteralMissing));
    }

    // ----- comparing -------------------------------------------------

    #[test]
    fn float_words_order_by_the_first_difference() {
        let low = words(1.0, 5.0, 0.0, 0.0);
        let high = words(1.0, 6.0, 0.0, 0.0);
        let cmp = |x: &[u32; 4], y: &[u32; 4]| compare_float_words(x, y, at(0, 5));
        assert_eq!(cmp(&low, &high), Ok(Ordering::Less));
        assert_eq!(cmp(&high, &low), Ok(Ordering::Greater));
        assert_eq!(cmp(&low, &low), Ok(Ordering::Equal));
        // Words are compared as numbers, not as bit patterns.
        assert_eq!(cmp(&words(-1.0, 0.0, 0.0, 0.0), &words(1.0, 0.0, 0.0, 0.0)), Ok(Ordering::Less));
    }

    #[test]
    fn a_nan_constant_cannot_be_ordered() {
        let nan = words(f32::NAN, 0.0, 0.0, 0.0);
        let refused = compare_float_words(&nan, &words(0.0, 0.0, 0.0, 0.0), at(3, 9));
        let expected =
            LedgerBuildError::CompareConstantNotFinite { material: MaterialAssetKey(3), slot: 9 };
        assert_eq!(refused, Err(expected));
        // The difference is found before the NaN is reached.
        let early = words(1.0, f32::NAN, 0.0, 0.0);
        let other = words(2.0, f32::NAN, 0.0, 0.0);
        assert_eq!(compare_float_words(&early, &other, at(3, 9)), Ok(Ordering::Less));
    }

    #[test]
    fn pass_arguments_order_by_code_constants_then_count_then_values() {
        let base = pass_key("p", "v");
        let codes = |c: &[u16]| with_constants(c, &[], "v");
        let values = |v: &[(u16, [u32; 4])]| with_constants(&[], v, "v");
        let one = words(1.0, 0.0, 0.0, 0.0);
        let two = words(2.0, 0.0, 0.0, 0.0);
        let cmp = |x: &PassKey, y: &PassKey| diff_pass_args(x, y, at(0, 5));

        assert_eq!(cmp(&base, &base), Ok(Ordering::Equal));
        assert_eq!(cmp(&codes(&[1]), &codes(&[2])), Ok(Ordering::Less));
        // A shorter run of code constants sorts first.
        assert_eq!(cmp(&base, &codes(&[0])), Ok(Ordering::Less));
        // Code constants decide before the pixel constants are looked at.
        let a = with_constants(&[1], &[(0, two)], "v");
        let b = with_constants(&[2], &[(0, one)], "v");
        assert_eq!(cmp(&a, &b), Ok(Ordering::Less));
        // Fewer pixel constants sort first, whatever their values.
        assert_eq!(cmp(&base, &values(&[(0, one)])), Ok(Ordering::Less));
        // Then the destination, then the value.
        assert_eq!(cmp(&values(&[(1, two)]), &values(&[(2, one)])), Ok(Ordering::Less));
        assert_eq!(cmp(&values(&[(1, two)]), &values(&[(1, one)])), Ok(Ordering::Greater));
    }

    #[test]
    fn the_sort_key_comes_first() {
        let (mut low, mut high) = (record(0, "z"), record(1, "a"));
        (low.order_key, high.order_key) = (1, 2);
        assert_eq!(compare(&low, &high), Ordering::Less);
        assert_eq!(compare(&high, &low), Ordering::Greater);
    }

    #[test]
    fn with_a_slot_9_pass_the_info_flag_decides_and_without_one_slot_5_does() {
        let (mut flagged, mut plain) = (record(0, "a"), record(1, "b"));
        flagged.passes[NINE] = Some(pass_key("p", "v"));
        plain.passes[NINE] = Some(pass_key("p", "v"));
        flagged.bits.info_game = 0b10;
        // The flagged material goes first.
        assert_eq!(compare(&flagged, &plain), Ordering::Less);
        assert_eq!(compare(&plain, &flagged), Ordering::Greater);

        let (mut with5, without5) = (record(0, "z"), record(1, "a"));
        with5.passes[FIVE] = Some(pass_key("p", "v"));
        // Having a slot 5 pass goes before not having one, ahead of the name.
        assert_eq!(compare(&with5, &without5), Ordering::Less);
        assert_eq!(compare(&without5, &with5), Ordering::Greater);
    }

    #[test]
    fn prepass_then_the_state_bit_then_names_break_ties() {
        let (mut early, mut late) = (record(0, "z"), record(1, "a"));
        (early.bits.early_pass, late.bits.early_pass) = (1, 2);
        assert_eq!(compare(&early, &late), Ordering::Less);

        let (mut on, off) = (record(0, "z"), record(1, "a"));
        on.bits.state = 8;
        assert_eq!(compare(&on, &off), Ordering::Less);

        let (mut first, mut second) = (record(0, "z"), record(1, "a"));
        (first.techset, second.techset) = ("alpha".into(), "beta".into());
        assert_eq!(compare(&first, &second), Ordering::Less);

        assert_eq!(compare(&record(0, "a"), &record(1, "b")), Ordering::Less);
        assert_eq!(compare(&record(0, "a"), &record(1, "a")), Ordering::Equal);
    }

    #[test]
    fn slot_9_passes_order_by_pixel_shader_then_arguments_then_vertex_shader() {
        let with9 = |pass: PassKey, state: u8| {
            let mut r = record(0, "m");
            r.passes[NINE] = Some(pass);
            r.bits.state = state;
            r
        };
        // Pixel shader name first.
        assert_eq!(compare(&with9(pass_key("a", "z"), 0), &with9(pass_key("b", "a"), 0)), Ordering::Less);
        // The vertex shader decides when the pixel shaders match.
        assert_eq!(compare(&with9(pass_key("a", "a"), 0), &with9(pass_key("a", "b"), 0)), Ordering::Less);
        // Arguments are only looked at when the (first) material has state bit 3 ...
        let mut two = with_constants(&[2], &[], "a");
        let mut one = with_constants(&[1], &[], "z");
        two.pixel_shader = "a".into();
        one.pixel_shader = "a".into();
        // ... and then they come before the vertex shader ...
        // (bit 3 of the state flags also sorts the materials, equally here)
        assert_eq!(compare(&with9(two.clone(), 8), &with9(one.clone(), 8)), Ordering::Greater);
        // ... otherwise the vertex shader name alone decides.
        assert_eq!(compare(&with9(two, 0), &with9(one, 0)), Ordering::Less);
    }

    #[test]
    fn when_only_the_second_has_slot_9_the_slot_5_passes_are_compared() {
        let (mut a, mut b) = (record(0, "m"), record(1, "m"));
        a.passes[FIVE] = Some(pass_key("a", "v"));
        b.passes = [Some(pass_key("b", "v")), Some(pass_key("p", "v"))];
        // `a` has no slot 9, so slot 5 is not ranked first by the flag rule,
        // and the comparison falls through to slot 5's shader names.
        assert_eq!(compare(&a, &b), Ordering::Less);
    }

    #[test]
    fn a_slot_present_on_one_side_only_counts_a_gap_and_goes_first() {
        let mut sorter = Sorter::default();
        let mut has = record(0, "m");
        has.passes[NINE] = Some(pass_key("p", "v"));
        let lacks = record(1, "m");
        // `has` has slot 9, so the comparison looks at slot 9; `lacks` has none.
        assert_eq!(sorter.compare(&has, &lacks), Ok(Ordering::Less));
        assert_eq!(sorter.slot_spacing, 1);
        // `lacks` has no slot 9 but the other does, so slot 5 (empty on both) is compared.
        assert_eq!(sorter.compare(&lacks, &has), Ok(Ordering::Equal));
        assert_eq!(sorter.slot_spacing, 1);

        // Only the second has the pass: it goes first, and the gap counts.
        assert_eq!(sorter.slot_order(9, &lacks, &has, false), Ok(Ordering::Greater));
        assert_eq!(sorter.slot_spacing, 2);
        assert_eq!(sorter.slot_order(9, &lacks, &lacks, false), Ok(Ordering::Equal));
        assert_eq!(sorter.slot_spacing, 2);
    }

    fn sorted_ids(records: &mut [SortRecord]) -> Vec<u16> {
        Sorter::default().sort(records).expect("comparable");
        records.iter().map(|r| r.id.0).collect()
    }

    #[test]
    fn the_sort_is_stable_and_leaves_an_ordered_list_alone() {
        let mut records: Vec<_> = (0..5).map(|i| record(i, "same")).collect();
        assert_eq!(sorted_ids(&mut records), [0, 1, 2, 3, 4]);
    }

    #[test]
    fn the_sort_puts_records_in_comparator_order() {
        let mut records = vec![record(0, "c"), record(1, "a"), record(2, "b")];
        for (r, key) in records.iter_mut().zip([0, 2, 1]) {
            r.order_key = key;
        }
        assert_eq!(sorted_ids(&mut records), [0, 2, 1]);
    }

    #[test]
    fn a_comparator_failure_stops_the_sort() {
        let nan = with_constants(&[], &[(0, words(f32::NAN, 0.0, 0.0, 0.0))], "v");
        let mut records = vec![record(0, "m"), record(1, "m")];
        for r in &mut records {
            r.passes[NINE] = Some(nan.clone());
            r.bits.state = 8;
        }
        assert!(matches!(
            Sorter::default().sort(&mut records),
            Err(LedgerBuildError::CompareConstantNotFinite { .. })
        ));
    }

    // ----- skipping and numbering ------------------------------------

    #[test]
    fn only_missing_data_is_skipped_rather_than_fatal() {
        let id = MaterialAssetKey(0);
        let skippable = [
            LedgerBuildError::TechniqueSetMissing { material: id },
            LedgerBuildError::SetNamespaceDiffers {
                material: id,
                want: asset_core::AssetCategoryScope::T6,
                got: asset_core::AssetCategoryScope::T5,
            },
            LedgerBuildError::TechniqueGraphMissing { material: id },
            LedgerBuildError::RecipeBodyAbsent { material: id, slot: 5 },
            LedgerBuildError::ShaderPairAbsent { material: id, slot: 5 },
            LedgerBuildError::LocalConstantAbsent { material: id, label_digest: 1 },
            LedgerBuildError::LiteralFragmentAbsent { material: id, slot: 5 },
        ];
        for cause in skippable {
            assert!(comparator_skippable(&cause), "{cause:?}");
            assert_eq!(skip_cause_key(&cause), cause_name(&cause), "{cause:?}");
            assert_ne!(skip_cause_key(&cause), "other", "{cause:?}");
        }
        let fatal = [
            LedgerBuildError::MaterialAssetNumberTooLarge { index: 70_000 },
            LedgerBuildError::CompareConstantNotFinite { material: id, slot: 5 },
            LedgerBuildError::CompareRuleContradiction { material: id, other: id, slot: 5 },
        ];
        for cause in fatal {
            assert!(!comparator_skippable(&cause), "{cause:?}");
            assert_eq!(skip_cause_key(&cause), "other", "{cause:?}");
        }
        let full = LedgerBuildError::RankTableFull { count: 1, capacity: 0 };
        assert!(!comparator_skippable(&full));
        assert_eq!(skip_cause_key(&full), "RankTableFull");
    }

    #[test]
    fn skip_labels_list_the_six_most_common_largest_first() {
        assert_eq!(six_biggest(&BTreeMap::new()), None);
        let tallies = [("a", 1), ("b", 5), ("c", 5), ("d", 2), ("e", 9), ("f", 3), ("g", 4), ("h", 1)];
        let counts: BTreeMap<String, u32> =
            tallies.into_iter().map(|(name, n)| (name.to_owned(), n)).collect();
        assert_eq!(six_biggest(&counts).as_deref(), Some("e:9,b:5,c:5,g:4,f:3,d:2"));
    }

    #[test]
    fn records_are_numbered_in_sorted_order() {
        let mut records = vec![record(0, "c"), record(1, "a"), record(2, "b")];
        records[0].order_key = 2;
        let ranking = rank(records, 4).expect("ranks");
        let ids: Vec<u16> = ranking.records.iter().map(|r| r.id.0).collect();
        assert_eq!(ids, [1, 2, 0]);
        let numbers: Vec<Option<u32>> =
            ranking.ordinals.iter().map(|o| o.map(SortedMaterialRank::get)).collect();
        // Asset 3 was never ranked.
        assert_eq!(numbers, [Some(2), Some(0), Some(1), None]);
        assert_eq!(ranking.slot_spacing, 0);
    }

    #[test]
    fn more_ranked_materials_than_the_retail_limit_is_refused() {
        let limit = SortedMaterialRank::SHIPPED_SLOT_CEILING;
        let many = |n: usize| -> Vec<SortRecord> {
            (0..n).map(|i| record(i as u16, &format!("{i:05}"))).collect()
        };
        let refused = rank(many(limit + 1), limit + 1).err();
        let expected = LedgerBuildError::RankTableFull { count: limit + 1, capacity: limit };
        assert_eq!(refused, Some(expected));
        assert!(rank(many(limit), limit).is_ok());
    }

    #[test]
    fn no_materials_make_an_empty_table() {
        let built = build_sorted_material_table(&MaterialDefinitions::default()).expect("builds");
        assert!(built.asset_ids_by_ordinal.is_empty() && built.ranks_by_asset_id.is_empty());
        assert_eq!((built.skipped_n, built.slot_gap_n), (0, 0));
        assert_eq!((built.first_skip, built.skipped_tech, built.skipped_cause), (None, None, None));
    }
}
