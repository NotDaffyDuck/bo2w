//! The sorted-material table: the order every material takes among all the
//! others, which becomes the ordinal baked into its draw-surf key.
//!
//! The order is the one the original renderer's material sort produced, so
//! it is fixed by a comparator over each material's sort key, flags, shader
//! names and (for some passes) its pixel constants. The comparator is not
//! guaranteed to be a total order, so the sort is a plain insertion sort:
//! whatever it does with an inconsistent pair, it does the same way every
//! time.

use core::cmp::Ordering;
use std::collections::BTreeMap;

use asset_material::{
    AuthoredMaterial, MaterialDefinitions, OwnedMaterialPass, OwnedShaderArgument, OwnedShaderRef,
    OwnedTechnique, TechsetKey, TechsetResolve,
};

use super::{CatalogBuildError, MaterialAssetId, SortedMaterialOrdinal};

/// What the comparator looks at in the first pass of a technique.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ComparatorPassKey {
    pixel_shader_name: String,
    vertex_shader_name: String,
    code_pixel_constants: Vec<u16>,
    pixel_constants: Vec<(u16, [u32; 4])>,
}

/// What the comparator looks at in one material.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ComparatorRecord {
    asset_id: MaterialAssetId,
    name: String,
    technique_set_name: String,
    sort_key: u8,
    info_game_flags: u8,
    state_flags: u8,
    prepass: u8,
    slot5: Option<ComparatorPassKey>,
    slot9: Option<ComparatorPassKey>,
}

/// The built table, ready to be stored in the catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SortedBuild {
    pub asset_ids_by_ordinal: Vec<MaterialAssetId>,
    pub ordinals_by_asset_id: Vec<Option<SortedMaterialOrdinal>>,
    pub skipped_n: u32,
    pub slot_gap_n: u32,
    pub first_skip: Option<String>,
    pub skip_tech: Option<String>,
    pub skip_cause: Option<String>,
}

// ---------------------------------------------------------------------
// Reading a material into a record
// ---------------------------------------------------------------------

fn shader_name<'a>(
    source: &'a MaterialDefinitions,
    material: MaterialAssetId,
    slot: u8,
    reference: &OwnedShaderRef,
) -> Result<&'a str, CatalogBuildError> {
    reference
        .shader
        .and_then(|index| source.shaders.get(index))
        .map(|shader| shader.name.as_str())
        .ok_or(CatalogBuildError::ShaderIdentityMissing { material, slot })
}

/// The argument-type number the original pass tables use for an argument.
fn argument_type(argument: &OwnedShaderArgument) -> u16 {
    use OwnedShaderArgument as Arg;

    match argument {
        Arg::MaterialVertexConstant { .. } => 0,
        Arg::LiteralVertexConstant { .. } => 1,
        Arg::MaterialPixelSampler { .. } => 2,
        Arg::CodeVertexConstant { .. } => 3,
        Arg::CodePixelSampler { .. } => 4,
        Arg::CodePixelConstant { .. } => 5,
        Arg::MaterialPixelConstant { .. } => 6,
        Arg::LiteralPixelConstant { .. } => 7,
        Arg::Unknown { argument_type, .. } => *argument_type,
    }
}

/// The constants of a pass that the comparator can see.
#[derive(Debug, PartialEq, Eq)]
struct PixelConstants {
    code: Vec<u16>,
    values: Vec<(u16, [u32; 4])>,
}

#[derive(Debug, PartialEq, Eq)]
enum ConstantFault {
    MaterialConstantMissing { name_hash: u32 },
    LiteralMissing,
}

/// Read the pixel constants out of a pass's stable arguments. They sit at
/// the end, in runs: code constants, then material constants, then literals.
/// `material_words` says what the material gives a named constant; a name it
/// does not have is skipped when `may_skip_missing` says so, a fault
/// otherwise.
fn pixel_constants(
    stable: &[OwnedShaderArgument],
    material_words: impl Fn(u32) -> Option<[u32; 4]>,
    may_skip_missing: impl Fn() -> bool,
) -> Result<PixelConstants, ConstantFault> {
    let first_at_least = |type_code: u16| {
        stable
            .iter()
            .position(|argument| argument_type(argument) >= type_code)
            .unwrap_or(stable.len())
    };

    let mut code = Vec::new();
    let mut rest = &stable[first_at_least(5)..];
    while let [
        OwnedShaderArgument::CodePixelConstant { index, .. },
        tail @ ..,
    ] = rest
    {
        code.push(*index);
        rest = tail;
    }

    let mut values = Vec::new();
    let mut rest = &stable[first_at_least(6)..];
    while let [
        OwnedShaderArgument::MaterialPixelConstant {
            destination,
            name_hash,
        },
        tail @ ..,
    ] = rest
    {
        rest = tail;
        match material_words(*name_hash) {
            Some(words) => values.push((*destination, words)),
            None if may_skip_missing() => {}
            None => {
                return Err(ConstantFault::MaterialConstantMissing {
                    name_hash: *name_hash,
                });
            }
        }
    }
    while let [
        OwnedShaderArgument::LiteralPixelConstant { destination, words },
        tail @ ..,
    ] = rest
    {
        values.push((*destination, words.ok_or(ConstantFault::LiteralMissing)?));
        rest = tail;
    }
    values.sort_by_key(|&(destination, _)| destination);
    Ok(PixelConstants { code, values })
}

/// A Black Ops 1 material may name a technique set whose "x0" token the
/// build stripped; its leftover constants then have nowhere to come from,
/// and the comparator passes over them.
fn leftover_x_token_aliased(source: &MaterialDefinitions, material: &AuthoredMaterial) -> bool {
    if material.namespace != asset_core::AssetNamespace::T5 {
        return false;
    }
    let wanted = asset_core::AssetRef::bare_name(material.technique_set.as_str());
    let stripped = asset_material::t5_feature_token_stripped(wanted);
    stripped != wanted
        && source.technique_set_facts().iter().any(|facts| {
            facts.name.is_real() && facts.name.as_str() == stripped && facts.table.is_some()
        })
}

fn stable_arguments(
    pass: &OwnedMaterialPass,
    asset_id: MaterialAssetId,
    slot: u8,
) -> Result<&[OwnedShaderArgument], CatalogBuildError> {
    let start =
        usize::from(pass.per_prim_arg_count).saturating_add(usize::from(pass.per_obj_arg_count));
    let end = start.saturating_add(usize::from(pass.stable_arg_count));
    pass.arguments
        .get(start..end)
        .ok_or(CatalogBuildError::TechniqueBodyMissing {
            material: asset_id,
            slot,
        })
}

fn comparator_pass_key(
    source: &MaterialDefinitions,
    material: &AuthoredMaterial,
    asset_id: MaterialAssetId,
    slot: u8,
    technique: &OwnedTechnique,
) -> Result<ComparatorPassKey, CatalogBuildError> {
    let body_missing = CatalogBuildError::TechniqueBodyMissing {
        material: asset_id,
        slot,
    };
    let pass = technique.passes.first().ok_or(body_missing)?;
    let stable = stable_arguments(pass, asset_id, slot)?;

    let constants = pixel_constants(
        stable,
        |name_hash| {
            material
                .constants
                .iter()
                .find(|constant| constant.name_hash == name_hash)
                .map(|constant| constant.literal.map(f32::to_bits))
        },
        || leftover_x_token_aliased(source, material),
    )
    .map_err(|fault| match fault {
        ConstantFault::MaterialConstantMissing { name_hash } => {
            CatalogBuildError::MaterialConstantMissing {
                material: asset_id,
                name_hash,
            }
        }
        ConstantFault::LiteralMissing => CatalogBuildError::LiteralPixelConstantMissing {
            material: asset_id,
            slot,
        },
    })?;

    Ok(ComparatorPassKey {
        pixel_shader_name: shader_name(source, asset_id, slot, &pass.pixel_shader)?.to_owned(),
        vertex_shader_name: shader_name(source, asset_id, slot, &pass.vertex_shader)?.to_owned(),
        code_pixel_constants: constants.code,
        pixel_constants: constants.values,
    })
}

fn comparator_record(
    source: &MaterialDefinitions,
    index: usize,
) -> Result<ComparatorRecord, CatalogBuildError> {
    let asset_id = MaterialAssetId(
        u16::try_from(index).map_err(|_| CatalogBuildError::MaterialAssetIdOverflow { index })?,
    );
    let material = &source.materials[index];
    let graph_missing = CatalogBuildError::TechniqueGraphMissing { material: asset_id };

    let key = TechsetKey::new(material.namespace, material.technique_set.as_str());
    let facts = match source.resolve_technique_set(key) {
        TechsetResolve::Hit { facts, .. } => facts,
        TechsetResolve::GraphMissing { .. } => return Err(graph_missing),
        TechsetResolve::Foreign { got, .. } => {
            return Err(CatalogBuildError::TechniqueSetNamespaceMismatch {
                material: asset_id,
                want: material.namespace,
                got,
            });
        }
        TechsetResolve::Missing => {
            return Err(CatalogBuildError::TechniqueSetMissing { material: asset_id });
        }
    };
    let table = facts.table.as_ref().ok_or(graph_missing.clone())?;
    let graph = table.graph.as_ref().ok_or(graph_missing)?;

    let pass_key = |slot: usize| -> Result<Option<ComparatorPassKey>, CatalogBuildError> {
        if table.slots & (1 << slot) == 0 {
            return Ok(None);
        }
        let slot = slot as u8;
        let technique = graph
            .slots
            .get(usize::from(slot))
            .and_then(Option::as_ref)
            .ok_or(CatalogBuildError::TechniqueBodyMissing {
                material: asset_id,
                slot,
            })?;
        comparator_pass_key(source, material, asset_id, slot, technique).map(Some)
    };

    let prepass = dpvs_t6::material_prepass(dpvs_t6::MaterialPrepassInput {
        technique0_present: table.slots & 1 != 0,
        technique1_present: table.slots & 2 != 0,
        technique0_flags: table.technique0_flags,
        state_flags: material.state_flags,
        techset_name_has_flag: material.technique_set.as_str().contains("flag_"),
    });
    Ok(ComparatorRecord {
        asset_id,
        name: material.name.to_string(),
        technique_set_name: facts.name.to_string(),
        sort_key: material.sort_key,
        info_game_flags: material.info_game_flags,
        state_flags: material.state_flags,
        prepass,
        slot5: pass_key(5)?,
        slot9: pass_key(9)?,
    })
}

// ---------------------------------------------------------------------
// The comparator
// ---------------------------------------------------------------------

fn compare_float_words(
    a: [u32; 4],
    b: [u32; 4],
    material: MaterialAssetId,
    slot: u8,
) -> Result<Ordering, CatalogBuildError> {
    for (a, b) in a.into_iter().zip(b) {
        let order = f32::from_bits(a)
            .partial_cmp(&f32::from_bits(b))
            .ok_or(CatalogBuildError::NonFiniteComparatorConstant { material, slot })?;
        if order.is_ne() {
            return Ok(order);
        }
    }
    Ok(Ordering::Equal)
}

fn compare_pass_args(
    a: &ComparatorPassKey,
    b: &ComparatorPassKey,
    material: MaterialAssetId,
    slot: u8,
) -> Result<Ordering, CatalogBuildError> {
    let order = a
        .code_pixel_constants
        .cmp(&b.code_pixel_constants)
        .then(a.pixel_constants.len().cmp(&b.pixel_constants.len()));
    if order.is_ne() {
        return Ok(order);
    }
    for (&(a_destination, a_words), &(b_destination, b_words)) in
        a.pixel_constants.iter().zip(&b.pixel_constants)
    {
        let order = a_destination.cmp(&b_destination);
        if order.is_ne() {
            return Ok(order);
        }
        let order = compare_float_words(a_words, b_words, material, slot)?;
        if order.is_ne() {
            return Ok(order);
        }
    }
    Ok(Ordering::Equal)
}

/// Order two materials' passes in `slot`. A material that has the pass goes
/// before one that does not, which the original comparator would not have
/// let happen; each time it does, `slot_gap` counts it.
fn compare_slot(
    slot: u8,
    pair: (&ComparatorRecord, &ComparatorRecord),
    passes: (Option<&ComparatorPassKey>, Option<&ComparatorPassKey>),
    compare_args: bool,
    slot_gap: &mut u32,
) -> Result<Ordering, CatalogBuildError> {
    let (a, b) = pair;
    let (a_pass, b_pass) = match passes {
        (None, None) => return Ok(Ordering::Equal),
        (Some(a_pass), Some(b_pass)) => (a_pass, b_pass),
        (a_pass, b_pass) => {
            *slot_gap = slot_gap.saturating_add(1);
            if *slot_gap == 1 {
                diag::info!(
                    World,
                    "drawsurf sorted materials: slot {slot} occupancy gap {} vs {} \
                     — present-first (not ComparatorInvariant)",
                    a.name,
                    b.name
                );
            }
            return Ok(b_pass.is_some().cmp(&a_pass.is_some()));
        }
    };
    let order = a_pass.pixel_shader_name.cmp(&b_pass.pixel_shader_name);
    if order.is_ne() {
        return Ok(order);
    }
    if compare_args {
        let order = compare_pass_args(a_pass, b_pass, a.asset_id, slot)?;
        if order.is_ne() {
            return Ok(order);
        }
    }
    Ok(a_pass.vertex_shader_name.cmp(&b_pass.vertex_shader_name))
}

fn flag_bit(flags: u8, bit: u8) -> u8 {
    (flags >> bit) & 1
}

fn compare_sorted_materials(
    a: &ComparatorRecord,
    b: &ComparatorRecord,
    slot_gap: &mut u32,
) -> Result<Ordering, CatalogBuildError> {
    let order = a.sort_key.cmp(&b.sort_key);
    if order.is_ne() {
        return Ok(order);
    }
    let order = if a.slot9.is_some() {
        flag_bit(b.info_game_flags, 1).cmp(&flag_bit(a.info_game_flags, 1))
    } else {
        b.slot5.is_some().cmp(&a.slot5.is_some())
    };
    let order = order
        .then(a.prepass.cmp(&b.prepass))
        .then(flag_bit(b.state_flags, 3).cmp(&flag_bit(a.state_flags, 3)));
    if order.is_ne() {
        return Ok(order);
    }

    let by_slot = if a.slot9.is_some() {
        let compare_args = a.state_flags & 8 != 0;
        let passes = (a.slot9.as_ref(), b.slot9.as_ref());
        compare_slot(9, (a, b), passes, compare_args, slot_gap)?
    } else if b.slot9.is_some() {
        let passes = (a.slot5.as_ref(), b.slot5.as_ref());
        compare_slot(5, (a, b), passes, true, slot_gap)?
    } else {
        Ordering::Equal
    };
    Ok(by_slot
        .then_with(|| a.technique_set_name.cmp(&b.technique_set_name))
        .then_with(|| a.name.cmp(&b.name)))
}

/// Insertion sort by [`compare_sorted_materials`]. Each record moves down
/// while it compares less than the one before it.
fn insertion_sort_by_comparator(
    records: &mut [ComparatorRecord],
    slot_gap: &mut u32,
) -> Result<(), CatalogBuildError> {
    for start in 1..records.len() {
        let mut at = start;
        while at != 0 && compare_sorted_materials(&records[at], &records[at - 1], slot_gap)?.is_lt()
        {
            records.swap(at, at - 1);
            at -= 1;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// Skipped materials
// ---------------------------------------------------------------------

/// Whether a material that failed to become a record is left out of the
/// table (and counted) instead of failing the whole build.
fn comparator_skippable(cause: &CatalogBuildError) -> bool {
    matches!(
        cause,
        CatalogBuildError::TechniqueSetMissing { .. }
            | CatalogBuildError::TechniqueSetNamespaceMismatch { .. }
            | CatalogBuildError::TechniqueGraphMissing { .. }
            | CatalogBuildError::TechniqueBodyMissing { .. }
            | CatalogBuildError::ShaderIdentityMissing { .. }
            | CatalogBuildError::MaterialConstantMissing { .. }
            | CatalogBuildError::LiteralPixelConstantMissing { .. }
    )
}

fn skip_cause_key(cause: &CatalogBuildError) -> &'static str {
    match cause {
        CatalogBuildError::TechniqueSetMissing { .. } => "TechniqueSetMissing",
        CatalogBuildError::TechniqueSetNamespaceMismatch { .. } => "TechniqueSetNamespaceMismatch",
        CatalogBuildError::TechniqueGraphMissing { .. } => "TechniqueGraphMissing",
        CatalogBuildError::TechniqueBodyMissing { .. } => "TechniqueBodyMissing",
        CatalogBuildError::ShaderIdentityMissing { .. } => "ShaderIdentityMissing",
        CatalogBuildError::MaterialConstantMissing { .. } => "MaterialConstantMissing",
        CatalogBuildError::LiteralPixelConstantMissing { .. } => "LiteralPixelConstantMissing",
        CatalogBuildError::SortedMaterialCapacity { .. } => "SortedMaterialCapacity",
        _ => "other",
    }
}

/// The six most common entries of `counts`, most common first (ties by
/// name), as `name:count,...`.
fn skip_tech_label(counts: &BTreeMap<String, u32>) -> Option<String> {
    if counts.is_empty() {
        return None;
    }
    let mut ranked: Vec<_> = counts.iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let top: Vec<_> = ranked
        .into_iter()
        .take(6)
        .map(|(name, count)| format!("{name}:{count}"))
        .collect();
    Some(top.join(","))
}

/// What was learned about the materials that were left out.
#[derive(Default)]
struct Skips {
    count: u32,
    by_tech: BTreeMap<String, u32>,
    by_cause: BTreeMap<String, u32>,
    first: Option<(usize, CatalogBuildError)>,
    first_x0: Option<String>,
}

impl Skips {
    fn note(&mut self, index: usize, cause: CatalogBuildError, material: &AuthoredMaterial) {
        let tech = material.technique_set.as_str();
        *self.by_tech.entry(tech.to_owned()).or_default() += 1;
        *self
            .by_cause
            .entry(skip_cause_key(&cause).to_owned())
            .or_default() += 1;
        if self.first_x0.is_none() && tech.contains("x0") {
            self.first_x0 = Some(format!("{cause:?} {} tech={tech}", material.name));
        }
        if self.first.is_none() {
            self.first = Some((index, cause));
        }
        self.count = self.count.saturating_add(1);
    }
}

// ---------------------------------------------------------------------
// The table
// ---------------------------------------------------------------------

/// Sorted records, the ordinal of each asset id, and the slot-gap count.
type Ranked = (
    Vec<ComparatorRecord>,
    Vec<Option<SortedMaterialOrdinal>>,
    u32,
);

/// Sort `records` and number them. `material_count` is how many materials
/// the catalog has, ranked or not.
fn rank(
    mut records: Vec<ComparatorRecord>,
    material_count: usize,
) -> Result<Ranked, CatalogBuildError> {
    let mut slot_gap = 0;
    insertion_sort_by_comparator(&mut records, &mut slot_gap)?;
    if records.len() > SortedMaterialOrdinal::RETAIL_LIMIT {
        return Err(CatalogBuildError::SortedMaterialCapacity {
            count: records.len(),
            capacity: SortedMaterialOrdinal::RETAIL_LIMIT,
        });
    }
    let mut ordinals = vec![None; material_count];
    for (ordinal, record) in records.iter().enumerate() {
        let ordinal = u32::try_from(ordinal)
            .ok()
            .ok_or(CatalogBuildError::SortedMaterialCapacity {
                count: records.len(),
                capacity: SortedMaterialOrdinal::LIMIT,
            })
            .and_then(SortedMaterialOrdinal::new)?;
        ordinals[usize::from(record.asset_id.0)] = Some(ordinal);
    }
    Ok((records, ordinals, slot_gap))
}

pub(super) fn build_sorted_material_table(
    source: &MaterialDefinitions,
) -> Result<SortedBuild, CatalogBuildError> {
    let mut records = Vec::new();
    let mut skips = Skips::default();
    for (index, material) in source.materials.iter().enumerate() {
        match comparator_record(source, index) {
            Ok(record) => records.push(record),
            Err(cause) if comparator_skippable(&cause) => skips.note(index, cause, material),
            Err(cause) => return Err(cause),
        }
    }
    let (records, ordinals_by_asset_id, slot_gap_n) = rank(records, source.materials.len())?;

    let first_skip = skips.first.as_ref().map(|(index, cause)| {
        let name = source
            .materials
            .get(*index)
            .map_or("<out-of-range>", |material| material.name.as_str());
        format!("{cause:?} {name}")
    });
    let skip_tech = skip_tech_label(&skips.by_tech);
    let skip_cause = skip_tech_label(&skips.by_cause);
    if first_skip.is_some() {
        let or_dash = |text: &Option<String>| text.clone().unwrap_or_else(|| "-".into());
        diag::info!(
            World,
            "drawsurf sorted materials: skipped={} ranked={} first_skip={} skip_cause={} \
             skip_tech={} first_x0={}",
            skips.count,
            records.len(),
            or_dash(&first_skip),
            or_dash(&skip_cause),
            or_dash(&skip_tech),
            or_dash(&skips.first_x0),
        );
    }
    Ok(SortedBuild {
        asset_ids_by_ordinal: records.iter().map(|record| record.asset_id).collect(),
        ordinals_by_asset_id,
        skipped_n: skips.count,
        slot_gap_n,
        first_skip,
        skip_tech,
        skip_cause,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pass_key(pixel: &str, vertex: &str) -> ComparatorPassKey {
        ComparatorPassKey {
            pixel_shader_name: pixel.into(),
            vertex_shader_name: vertex.into(),
            code_pixel_constants: Vec::new(),
            pixel_constants: Vec::new(),
        }
    }

    fn record(id: u16, name: &str) -> ComparatorRecord {
        ComparatorRecord {
            asset_id: MaterialAssetId(id),
            name: name.into(),
            technique_set_name: "set".into(),
            sort_key: 0,
            info_game_flags: 0,
            state_flags: 0,
            prepass: 0,
            slot5: None,
            slot9: None,
        }
    }

    fn compare(a: &ComparatorRecord, b: &ComparatorRecord) -> Ordering {
        compare_sorted_materials(a, b, &mut 0).expect("comparable")
    }

    fn words(a: f32, b: f32, c: f32, d: f32) -> [u32; 4] {
        [a, b, c, d].map(f32::to_bits)
    }

    // ----- argument reading ------------------------------------------

    fn code_const(index: u16) -> OwnedShaderArgument {
        OwnedShaderArgument::CodePixelConstant {
            destination: 0,
            index,
            first_row: 0,
            row_count: 1,
        }
    }

    fn material_const(destination: u16, name_hash: u32) -> OwnedShaderArgument {
        OwnedShaderArgument::MaterialPixelConstant {
            destination,
            name_hash,
        }
    }

    fn literal_const(destination: u16, value: Option<[u32; 4]>) -> OwnedShaderArgument {
        OwnedShaderArgument::LiteralPixelConstant {
            destination,
            words: value,
        }
    }

    fn sampler(destination: u16) -> OwnedShaderArgument {
        OwnedShaderArgument::MaterialPixelSampler {
            destination,
            name_hash: 1,
        }
    }

    #[test]
    fn argument_types_use_the_original_numbering() {
        let cases = [
            (
                OwnedShaderArgument::MaterialVertexConstant {
                    destination: 0,
                    name_hash: 0,
                },
                0,
            ),
            (
                OwnedShaderArgument::LiteralVertexConstant {
                    destination: 0,
                    words: None,
                },
                1,
            ),
            (sampler(0), 2),
            (
                OwnedShaderArgument::CodeVertexConstant {
                    destination: 0,
                    index: 0,
                    first_row: 0,
                    row_count: 1,
                },
                3,
            ),
            (
                OwnedShaderArgument::CodePixelSampler {
                    destination: 0,
                    index: 0,
                },
                4,
            ),
            (code_const(0), 5),
            (material_const(0, 0), 6),
            (literal_const(0, None), 7),
            (
                OwnedShaderArgument::Unknown {
                    argument_type: 99,
                    raw: [0; 8],
                },
                99,
            ),
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
        let read = pixel_constants(
            &stable,
            |hash| match hash {
                100 => Some(words(9.0, 9.0, 9.0, 9.0)),
                200 => Some(words(8.0, 8.0, 8.0, 8.0)),
                _ => None,
            },
            || false,
        )
        .expect("every constant is there");
        assert_eq!(read.code, [11, 12]);
        // Sorted by destination register, whatever order they came in.
        assert_eq!(
            read.values,
            [
                (2, words(8.0, 8.0, 8.0, 8.0)),
                (3, words(1.0, 2.0, 3.0, 4.0)),
                (4, words(9.0, 9.0, 9.0, 9.0)),
            ]
        );
    }

    #[test]
    fn a_pass_without_pixel_constants_reads_as_empty() {
        let read = pixel_constants(&[sampler(0)], no_words, || false).expect("nothing to read");
        assert_eq!(
            read,
            PixelConstants {
                code: Vec::new(),
                values: Vec::new()
            }
        );
        let none = pixel_constants(&[], no_words, || false).expect("nothing to read");
        assert!(none.values.is_empty() && none.code.is_empty());
    }

    #[test]
    fn a_material_constant_the_material_lacks_is_a_fault() {
        let stable = [material_const(1, 77)];
        assert_eq!(
            pixel_constants(&stable, no_words, || false),
            Err(ConstantFault::MaterialConstantMissing { name_hash: 77 })
        );
    }

    #[test]
    fn a_missing_constant_is_passed_over_when_the_caller_allows_it() {
        let stable = [material_const(1, 77), material_const(2, 78)];
        let read = pixel_constants(
            &stable,
            |hash| (hash == 78).then(|| words(5.0, 5.0, 5.0, 5.0)),
            || true,
        )
        .expect("the missing one is skipped");
        assert_eq!(read.values, [(2, words(5.0, 5.0, 5.0, 5.0))]);
    }

    #[test]
    fn a_literal_without_words_is_a_fault() {
        let stable = [literal_const(1, None)];
        assert_eq!(
            pixel_constants(&stable, no_words, || true),
            Err(ConstantFault::LiteralMissing)
        );
    }

    // ----- comparing -------------------------------------------------

    #[test]
    fn float_words_order_by_the_first_difference() {
        let id = MaterialAssetId(0);
        let low = words(1.0, 5.0, 0.0, 0.0);
        let high = words(1.0, 6.0, 0.0, 0.0);
        assert_eq!(compare_float_words(low, high, id, 5), Ok(Ordering::Less));
        assert_eq!(compare_float_words(high, low, id, 5), Ok(Ordering::Greater));
        assert_eq!(compare_float_words(low, low, id, 5), Ok(Ordering::Equal));
        // Words are compared as numbers, not as bit patterns.
        let negative = words(-1.0, 0.0, 0.0, 0.0);
        let positive = words(1.0, 0.0, 0.0, 0.0);
        assert_eq!(
            compare_float_words(negative, positive, id, 5),
            Ok(Ordering::Less)
        );
    }

    #[test]
    fn a_nan_constant_cannot_be_ordered() {
        let id = MaterialAssetId(3);
        let nan = [f32::NAN, 0.0, 0.0, 0.0].map(f32::to_bits);
        assert_eq!(
            compare_float_words(nan, words(0.0, 0.0, 0.0, 0.0), id, 9),
            Err(CatalogBuildError::NonFiniteComparatorConstant {
                material: id,
                slot: 9
            })
        );
        // The difference is found before the NaN is reached.
        let early = [1.0, f32::NAN, 0.0, 0.0].map(f32::to_bits);
        let other = [2.0, f32::NAN, 0.0, 0.0].map(f32::to_bits);
        assert_eq!(compare_float_words(early, other, id, 9), Ok(Ordering::Less));
    }

    #[test]
    fn pass_arguments_order_by_code_constants_then_count_then_values() {
        let id = MaterialAssetId(0);
        let base = pass_key("p", "v");
        let with_code = |codes: &[u16]| ComparatorPassKey {
            code_pixel_constants: codes.to_vec(),
            ..pass_key("p", "v")
        };
        let with_values = |values: &[(u16, [u32; 4])]| ComparatorPassKey {
            pixel_constants: values.to_vec(),
            ..pass_key("p", "v")
        };
        let one = words(1.0, 0.0, 0.0, 0.0);
        let two = words(2.0, 0.0, 0.0, 0.0);

        assert_eq!(compare_pass_args(&base, &base, id, 5), Ok(Ordering::Equal));
        assert_eq!(
            compare_pass_args(&with_code(&[1]), &with_code(&[2]), id, 5),
            Ok(Ordering::Less)
        );
        // A shorter run of code constants sorts first.
        assert_eq!(
            compare_pass_args(&base, &with_code(&[0]), id, 5),
            Ok(Ordering::Less)
        );
        // Code constants decide before the pixel constants are looked at.
        let a = ComparatorPassKey {
            code_pixel_constants: vec![1],
            pixel_constants: vec![(0, two)],
            ..pass_key("p", "v")
        };
        let b = ComparatorPassKey {
            code_pixel_constants: vec![2],
            pixel_constants: vec![(0, one)],
            ..pass_key("p", "v")
        };
        assert_eq!(compare_pass_args(&a, &b, id, 5), Ok(Ordering::Less));
        // Fewer pixel constants sort first, whatever their values.
        assert_eq!(
            compare_pass_args(&base, &with_values(&[(0, one)]), id, 5),
            Ok(Ordering::Less)
        );
        // Then the destination, then the value.
        assert_eq!(
            compare_pass_args(&with_values(&[(1, two)]), &with_values(&[(2, one)]), id, 5),
            Ok(Ordering::Less)
        );
        assert_eq!(
            compare_pass_args(&with_values(&[(1, two)]), &with_values(&[(1, one)]), id, 5),
            Ok(Ordering::Greater)
        );
    }

    #[test]
    fn the_sort_key_comes_first() {
        let mut low = record(0, "z");
        low.sort_key = 1;
        let mut high = record(1, "a");
        high.sort_key = 2;
        assert_eq!(compare(&low, &high), Ordering::Less);
        assert_eq!(compare(&high, &low), Ordering::Greater);
    }

    #[test]
    fn with_a_slot_9_pass_the_info_flag_decides_and_without_one_slot_5_does() {
        let (mut flagged, mut plain) = (record(0, "a"), record(1, "b"));
        flagged.slot9 = Some(pass_key("p", "v"));
        plain.slot9 = Some(pass_key("p", "v"));
        flagged.info_game_flags = 0b10;
        // The flagged material goes first.
        assert_eq!(compare(&flagged, &plain), Ordering::Less);
        assert_eq!(compare(&plain, &flagged), Ordering::Greater);

        let (mut with5, without5) = (record(0, "z"), record(1, "a"));
        with5.slot5 = Some(pass_key("p", "v"));
        // Having a slot 5 pass goes before not having one, ahead of the name.
        assert_eq!(compare(&with5, &without5), Ordering::Less);
        assert_eq!(compare(&without5, &with5), Ordering::Greater);
    }

    #[test]
    fn prepass_then_the_state_bit_then_names_break_ties() {
        let (mut early, mut late) = (record(0, "z"), record(1, "a"));
        early.prepass = 1;
        late.prepass = 2;
        assert_eq!(compare(&early, &late), Ordering::Less);

        let (mut on, off) = (record(0, "z"), record(1, "a"));
        on.state_flags = 8;
        assert_eq!(compare(&on, &off), Ordering::Less);

        let (mut first, mut second) = (record(0, "z"), record(1, "a"));
        first.technique_set_name = "alpha".into();
        second.technique_set_name = "beta".into();
        assert_eq!(compare(&first, &second), Ordering::Less);

        assert_eq!(compare(&record(0, "a"), &record(1, "b")), Ordering::Less);
        assert_eq!(compare(&record(0, "a"), &record(1, "a")), Ordering::Equal);
    }

    #[test]
    fn slot_9_passes_order_by_pixel_shader_then_arguments_then_vertex_shader() {
        let with9 = |pass: ComparatorPassKey, state_flags: u8| {
            let mut r = record(0, "m");
            r.slot9 = Some(pass);
            r.state_flags = state_flags;
            r
        };
        // Pixel shader name first.
        assert_eq!(
            compare(&with9(pass_key("a", "z"), 0), &with9(pass_key("b", "a"), 0)),
            Ordering::Less
        );
        // The vertex shader decides when the pixel shaders match.
        assert_eq!(
            compare(&with9(pass_key("a", "a"), 0), &with9(pass_key("a", "b"), 0)),
            Ordering::Less
        );
        // Arguments are only looked at when the (first) material has state bit 3 ...
        let two = ComparatorPassKey {
            code_pixel_constants: vec![2],
            ..pass_key("a", "a")
        };
        let one = ComparatorPassKey {
            code_pixel_constants: vec![1],
            ..pass_key("a", "z")
        };
        // ... and then they come before the vertex shader ...
        // (bit 3 of the state flags also sorts the materials, equally here)
        assert_eq!(
            compare(&with9(two.clone(), 8), &with9(one.clone(), 8)),
            Ordering::Greater
        );
        // ... otherwise the vertex shader name alone decides.
        assert_eq!(compare(&with9(two, 0), &with9(one, 0)), Ordering::Less);
    }

    #[test]
    fn when_only_the_second_has_slot_9_the_slot_5_passes_are_compared() {
        let mut a = record(0, "m");
        a.slot5 = Some(pass_key("a", "v"));
        let mut b = record(1, "m");
        b.slot5 = Some(pass_key("b", "v"));
        b.slot9 = Some(pass_key("p", "v"));
        // `a` has no slot 9, so slot 5 is not ranked first by the flag rule,
        // and the comparison falls through to slot 5's shader names.
        assert_eq!(compare(&a, &b), Ordering::Less);
    }

    #[test]
    fn a_slot_present_on_one_side_only_counts_a_gap_and_goes_first() {
        let mut gap = 0;
        let mut has = record(0, "m");
        has.slot9 = Some(pass_key("p", "v"));
        let lacks = record(1, "m");
        // `has` has slot 9, so the comparison looks at slot 9; `lacks` has none.
        let order = compare_sorted_materials(&has, &lacks, &mut gap).expect("comparable");
        assert_eq!(order, Ordering::Less);
        assert_eq!(gap, 1);
        let order = compare_sorted_materials(&lacks, &has, &mut gap).expect("comparable");
        // `lacks` has no slot 9 but the other does, so slot 5 (empty on both) is compared.
        assert_eq!(order, Ordering::Equal);
        assert_eq!(gap, 1);

        let both_sides = compare_slot(
            9,
            (&has, &lacks),
            (None, Some(&pass_key("p", "v"))),
            false,
            &mut gap,
        );
        assert_eq!(both_sides, Ok(Ordering::Greater));
        assert_eq!(gap, 2);
        assert_eq!(
            compare_slot(9, (&has, &lacks), (None, None), false, &mut gap),
            Ok(Ordering::Equal)
        );
        assert_eq!(gap, 2);
    }

    #[test]
    fn the_sort_is_stable_and_leaves_an_ordered_list_alone() {
        let mut records: Vec<_> = (0..5).map(|i| record(i, "same")).collect();
        let before: Vec<_> = records.iter().map(|r| r.asset_id).collect();
        insertion_sort_by_comparator(&mut records, &mut 0).expect("comparable");
        let after: Vec<_> = records.iter().map(|r| r.asset_id).collect();
        assert_eq!(before, after);
    }

    #[test]
    fn the_sort_puts_records_in_comparator_order() {
        let mut records = vec![record(0, "c"), record(1, "a"), record(2, "b")];
        records[0].sort_key = 0;
        records[1].sort_key = 2;
        records[2].sort_key = 1;
        insertion_sort_by_comparator(&mut records, &mut 0).expect("comparable");
        let order: Vec<_> = records.iter().map(|r| r.asset_id.0).collect();
        assert_eq!(order, [0, 2, 1]);
    }

    #[test]
    fn a_comparator_failure_stops_the_sort() {
        let nan = ComparatorPassKey {
            pixel_constants: vec![(0, [f32::NAN, 0.0, 0.0, 0.0].map(f32::to_bits))],
            ..pass_key("p", "v")
        };
        let mut a = record(0, "m");
        a.slot9 = Some(nan.clone());
        a.state_flags = 8;
        let mut b = record(1, "m");
        b.slot9 = Some(nan);
        b.state_flags = 8;
        let mut records = vec![a, b];
        assert!(matches!(
            insertion_sort_by_comparator(&mut records, &mut 0),
            Err(CatalogBuildError::NonFiniteComparatorConstant { .. })
        ));
    }

    // ----- skipping and numbering ------------------------------------

    #[test]
    fn only_missing_data_is_skipped_rather_than_fatal() {
        let id = MaterialAssetId(0);
        let skippable = [
            CatalogBuildError::TechniqueSetMissing { material: id },
            CatalogBuildError::TechniqueGraphMissing { material: id },
            CatalogBuildError::TechniqueBodyMissing {
                material: id,
                slot: 5,
            },
            CatalogBuildError::ShaderIdentityMissing {
                material: id,
                slot: 5,
            },
            CatalogBuildError::MaterialConstantMissing {
                material: id,
                name_hash: 1,
            },
            CatalogBuildError::LiteralPixelConstantMissing {
                material: id,
                slot: 5,
            },
        ];
        for cause in skippable {
            assert!(comparator_skippable(&cause), "{cause:?}");
            assert_ne!(skip_cause_key(&cause), "other", "{cause:?}");
        }
        let fatal = [
            CatalogBuildError::MaterialAssetIdOverflow { index: 70_000 },
            CatalogBuildError::NonFiniteComparatorConstant {
                material: id,
                slot: 5,
            },
            CatalogBuildError::ComparatorInvariant {
                material: id,
                other: id,
                slot: 5,
            },
        ];
        for cause in fatal {
            assert!(!comparator_skippable(&cause), "{cause:?}");
            assert_eq!(skip_cause_key(&cause), "other", "{cause:?}");
        }
        assert_eq!(
            skip_cause_key(&CatalogBuildError::SortedMaterialCapacity {
                count: 1,
                capacity: 0
            }),
            "SortedMaterialCapacity"
        );
    }

    #[test]
    fn skip_labels_list_the_six_most_common_largest_first() {
        assert_eq!(skip_tech_label(&BTreeMap::new()), None);
        let counts: BTreeMap<String, u32> = [
            ("a", 1),
            ("b", 5),
            ("c", 5),
            ("d", 2),
            ("e", 9),
            ("f", 3),
            ("g", 4),
            ("h", 1),
        ]
        .into_iter()
        .map(|(name, count)| (name.to_owned(), count))
        .collect();
        assert_eq!(
            skip_tech_label(&counts).as_deref(),
            Some("e:9,b:5,c:5,g:4,f:3,d:2")
        );
    }

    #[test]
    fn records_are_numbered_in_sorted_order() {
        let mut records = vec![record(0, "c"), record(1, "a"), record(2, "b")];
        records[0].sort_key = 2;
        let (sorted, ordinals, gap) = rank(records, 4).expect("ranks");
        let ids: Vec<_> = sorted.iter().map(|r| r.asset_id.0).collect();
        assert_eq!(ids, [1, 2, 0]);
        let numbers: Vec<_> = ordinals
            .iter()
            .map(|o| o.map(SortedMaterialOrdinal::get))
            .collect();
        // Asset 3 was never ranked.
        assert_eq!(numbers, [Some(2), Some(0), Some(1), None]);
        assert_eq!(gap, 0);
    }

    #[test]
    fn more_ranked_materials_than_the_retail_limit_is_refused() {
        let count = SortedMaterialOrdinal::RETAIL_LIMIT + 1;
        let records: Vec<_> = (0..count)
            .map(|i| record(i as u16, &format!("{i:05}")))
            .collect();
        assert_eq!(
            rank(records, count).unwrap_err(),
            CatalogBuildError::SortedMaterialCapacity {
                count,
                capacity: SortedMaterialOrdinal::RETAIL_LIMIT
            }
        );
        let records: Vec<_> = (0..SortedMaterialOrdinal::RETAIL_LIMIT)
            .map(|i| record(i as u16, &format!("{i:05}")))
            .collect();
        assert!(rank(records, SortedMaterialOrdinal::RETAIL_LIMIT).is_ok());
    }

    #[test]
    fn no_materials_make_an_empty_table() {
        let built = build_sorted_material_table(&MaterialDefinitions::default()).expect("builds");
        assert_eq!(
            built,
            SortedBuild {
                asset_ids_by_ordinal: Vec::new(),
                ordinals_by_asset_id: Vec::new(),
                skipped_n: 0,
                slot_gap_n: 0,
                first_skip: None,
                skip_tech: None,
                skip_cause: None,
            }
        );
    }
}
