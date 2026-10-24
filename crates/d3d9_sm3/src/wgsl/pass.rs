//! Assembling the whole pass module: bindings, the vertex entry, and one
//! fragment entry per alpha test.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use d3d9_decl::{D3DDECLUSAGE_COLOR, D3DDECLUSAGE_TEXCOORD, DeclType};
use d3d9_state::{ALPHA_REF_SCALE, AlphaTest, CompareFunc};

use super::body::{Defined, emit_body};
use super::plan::{LoweringPlan, plan_program};
use super::{
    PASS_FRAGMENT_ENTRY, PassWgsl, Sm3WgslError, TEXTURE_TABLE_BINDING_2D,
    TEXTURE_TABLE_BINDING_3D, TEXTURE_TABLE_BINDING_CUBE, TEXTURE_TABLE_BINDING_SAMPLERS,
    TEXTURE_TABLE_GROUP, component_name, register_name,
};
use crate::abi::{ConstantSlot, PassLoweringAbi, SamplerTextureDimension, VaryingLink, VertexInput};
use crate::ir::{Sm3ProgramIr, Sm3Register, Sm3RegisterFile};

/// WGSL functions that unpack the compressed vertex element formats a few
/// games use (`DeclType::T6*`) into the register values their shaders expect.
/// They are emitted only when an attribute needs one.
pub const T6_VERTEX_HELPERS: &str = "
fn t6_unit_vec(packed: u32) -> vec4<f32> {
    let raw = vec3<i32>(bitcast<i32>(packed << 22u), bitcast<i32>(packed << 12u), bitcast<i32>(packed << 2u)) >> vec3<u32>(22u);
    let v = vec3<f32>(raw) / 511.0;
    let len = sqrt(v.x * v.x + v.y * v.y + v.z * v.z);
    let n = select(vec3<f32>(0.0, 0.0, 1.0), v / len, len > 1e-6);
    let b = clamp(vec3<i32>(n * 127.0 + 127.5), vec3<i32>(0), vec3<i32>(255));
    return vec4<f32>(vec3<f32>(b), 63.0);
}

fn t6_texcoord(packed: u32) -> vec4<f32> {
    let r = (packed << 16u) | (packed >> 16u);
    return vec4<f32>(vec4<u32>(r & 255u, (r >> 8u) & 255u, (r >> 16u) & 255u, r >> 24u));
}

fn t6_world_texcoord(row: vec4<u32>) -> vec4<f32> {
    let uv = unpack2x16float(row.x);
    return vec4<f32>(uv, f32(row.w & 65535u) / 65535.0, f32(row.w >> 16u) / 65535.0);
}
";

/// The name of the fragment entry point that applies alpha test `index`.
pub fn pass_fragment_alpha_test_entry(index: usize) -> String {
    format!("{PASS_FRAGMENT_ENTRY}_atest{index}")
}

/// Constant rows the host must upload for a stage: one past the highest
/// register it reads that the program does not define itself.
fn constant_block_len(
    plan: &LoweringPlan,
    bindings: &[ConstantSlot],
) -> Result<usize, Sm3WgslError> {
    let mut len = 0usize;
    for register in float_constants(plan) {
        let bound = bindings
            .iter()
            .find(|binding| binding.register == register.index)
            .ok_or(Sm3WgslError::UnboundConstantRegister { register })?;
        if !bound.program_defined {
            len = len.max(usize::from(register.index) + 1);
        }
    }
    if plan.uses_relative_float {
        // A runtime index can reach any bound constant.
        for binding in bindings.iter().filter(|binding| !binding.program_defined) {
            len = len.max(usize::from(binding.register) + 1);
        }
    }
    Ok(len)
}

fn float_constants(plan: &LoweringPlan) -> impl Iterator<Item = Sm3Register> + '_ {
    plan.external
        .iter()
        .copied()
        .filter(|register| register.file == Sm3RegisterFile::FloatConstant)
}

/// The attribute a vertex input reads, when the pass supplies one.
fn attribute_of(input: &VertexInput) -> Option<(u32, DeclType)> {
    match (input.location, input.decl_type) {
        (Some(location), Some(decl_type)) if !matches!(decl_type, DeclType::Unknown(_)) => {
            Some((location, decl_type))
        }
        _ => None,
    }
}

fn is_packed_format(decl_type: DeclType) -> bool {
    matches!(
        decl_type,
        DeclType::T6UnitVec | DeclType::T6TexCoord | DeclType::T6WorldTexLmap
    )
}

fn attribute_type(decl_type: DeclType) -> &'static str {
    match decl_type {
        DeclType::Float2 | DeclType::Float16x2 => "vec2<f32>",
        DeclType::Float3 => "vec3<f32>",
        DeclType::UByte4 => "vec4<u32>",
        DeclType::T6UnitVec | DeclType::T6TexCoord => "u32",
        DeclType::T6WorldTexLmap => "vec4<u32>",
        _ => "vec4<f32>",
    }
}

/// The four-component register value an attribute of `decl_type` presents.
fn attribute_value(decl_type: DeclType, location: u32) -> String {
    let attribute = format!("attribute_{location}");
    match decl_type {
        DeclType::Float2 | DeclType::Float16x2 => {
            format!("vec4<f32>({attribute}.x, {attribute}.y, 0.0, 1.0)")
        }
        DeclType::Float3 => format!("vec4<f32>({attribute}, 1.0)"),
        DeclType::D3dColor => {
            format!("vec4<f32>({attribute}.z, {attribute}.y, {attribute}.x, {attribute}.w)")
        }
        DeclType::UByte4 => format!("vec4<f32>({attribute})"),
        DeclType::T6UnitVec => format!("t6_unit_vec({attribute})"),
        DeclType::T6TexCoord => format!("t6_texcoord({attribute})"),
        DeclType::T6WorldTexLmap => format!("t6_world_texcoord({attribute})"),
        _ => attribute,
    }
}

/// What an input with no attribute reads: zero, with w at one for colours and
/// texture coordinates.
fn default_value(input: &VertexInput) -> String {
    let usage = input.semantic.usage;
    let w = if usage == D3DDECLUSAGE_COLOR || usage == D3DDECLUSAGE_TEXCOORD {
        1.0f32
    } else {
        0.0
    };
    format!("vec4<f32>(0.0, 0.0, 0.0, {w:?})")
}

/// Constants the program reads from the host, one `let` each.
fn emit_constant_prologue(
    out: &mut String,
    plan: &LoweringPlan,
    bindings: &[ConstantSlot],
    base: &str,
) -> Result<(), Sm3WgslError> {
    for register in float_constants(plan) {
        let bound = bindings
            .iter()
            .find(|binding| binding.register == register.index)
            .ok_or(Sm3WgslError::UnboundConstantRegister { register })?;
        if bound.program_defined {
            continue;
        }
        emit!(
            out,
            "    let {} = sm3_constants.c[{base} + {}u];\n",
            register_name(register),
            register.index
        );
    }
    Ok(())
}

/// Texture and sampler slot numbers travel in the constant arena, after both
/// stages' constants, four to a row.
fn emit_texture_slot_prologue(
    out: &mut String,
    plan: &LoweringPlan,
    abi: &PassLoweringAbi,
    base: &str,
    slot_row_base: usize,
) -> Result<(), Sm3WgslError> {
    for &register in plan.samplers.keys() {
        let slot = abi
            .samplers
            .iter()
            .position(|sampler| sampler.register == register)
            .ok_or(Sm3WgslError::UnboundSamplerRegister { register })?;
        emit!(
            out,
            "    let sm3_slot_s{register} = bitcast<vec4<u32>>(sm3_constants.c[{base} + {}u]).{};\n",
            slot_row_base + slot / 4,
            component_name((slot % 4) as u8)
        );
    }
    Ok(())
}

/// The program's own constants, then a zeroed variable for every register it
/// writes. Returns the components that are readable from the start.
fn emit_prologue(out: &mut String, plan: &LoweringPlan) -> Defined {
    let mut defined = Defined::new();
    for (register, values) in &plan.float_definitions {
        emit!(
            out,
            "    let {} = vec4<f32>(bitcast<f32>(0x{:08x}u), bitcast<f32>(0x{:08x}u), bitcast<f32>(0x{:08x}u), bitcast<f32>(0x{:08x}u));\n",
            register_name(*register),
            values[0],
            values[1],
            values[2],
            values[3]
        );
        defined.insert(*register, 0x0f);
    }
    for (register, values) in &plan.integer_definitions {
        emit!(
            out,
            "    let {} = vec4<i32>(bitcast<i32>(0x{:08x}u), bitcast<i32>(0x{:08x}u), bitcast<i32>(0x{:08x}u), bitcast<i32>(0x{:08x}u));\n",
            register_name(*register),
            values[0],
            values[1],
            values[2],
            values[3]
        );
        defined.insert(*register, 0x0f);
    }
    for register in &plan.writable {
        if register.file == Sm3RegisterFile::Address {
            emit!(out, "    var {} = vec4<i32>(0);\n", register_name(*register));
        } else {
            emit!(out, "    var {} = vec4<f32>(0.0);\n", register_name(*register));
        }
    }
    defined
}

/// Discard pixels that fail the alpha test, comparing in the byte range the
/// reference was given in.
fn emit_alpha_test(out: &mut String, test: AlphaTest) -> Result<(), Sm3WgslError> {
    let comparison = match test.func {
        CompareFunc::Never => {
            out.push_str("    discard;\n");
            return Ok(());
        }
        CompareFunc::Always => return Ok(()),
        CompareFunc::Less => "<",
        CompareFunc::Equal => "==",
        CompareFunc::LessEqual => "<=",
        CompareFunc::Greater => ">",
        CompareFunc::NotEqual => "!=",
        CompareFunc::GreaterEqual => ">=",
        CompareFunc::Unknown(raw) => {
            return Err(Sm3WgslError::UnsupportedAlphaTestFunc {
                raw_func: Some(raw),
            });
        }
    };
    emit!(
        out,
        "    let sm3_alpha_ref = round(clamp(oc0.a, 0.0, 1.0) * {ALPHA_REF_SCALE:.1});\n"
    );
    emit!(
        out,
        "    if !(sm3_alpha_ref {comparison} {:.1}) {{ discard; }}\n",
        f32::from(test.reference)
    );
    Ok(())
}

fn emit_texture_table(out: &mut String, abi: &PassLoweringAbi) {
    if abi.samplers.is_empty() {
        return;
    }
    let tables = [
        (
            SamplerTextureDimension::D2,
            TEXTURE_TABLE_BINDING_2D,
            "sm3_textures_2d",
            "texture_2d<f32>",
        ),
        (
            SamplerTextureDimension::Cube,
            TEXTURE_TABLE_BINDING_CUBE,
            "sm3_textures_cube",
            "texture_cube<f32>",
        ),
        (
            SamplerTextureDimension::D3,
            TEXTURE_TABLE_BINDING_3D,
            "sm3_textures_3d",
            "texture_3d<f32>",
        ),
    ];
    for (dimension, binding, name, texture_type) in tables {
        if abi.samplers.iter().any(|sampler| sampler.dimension == dimension) {
            emit!(
                out,
                "@group({TEXTURE_TABLE_GROUP}) @binding({binding}) var {name}: binding_array<{texture_type}>;\n"
            );
        }
    }
    emit!(
        out,
        "@group({TEXTURE_TABLE_GROUP}) @binding({TEXTURE_TABLE_BINDING_SAMPLERS}) var sm3_samplers: binding_array<sampler>;\n"
    );
}

/// Check that the stages agree with the ABI and with each other.
fn check_links(
    abi: &PassLoweringAbi,
    vertex: &LoweringPlan,
    pixel: &LoweringPlan,
) -> Result<(), Sm3WgslError> {
    let linked: Vec<&VaryingLink> = core::iter::once(&abi.position)
        .chain(&abi.varyings)
        .collect();
    for link in &linked {
        if !vertex.output_masks.contains_key(&link.vertex_register) {
            return Err(Sm3WgslError::DeclaredOutputNeverWritten {
                register: link.vertex_register,
            });
        }
    }
    for register in vertex.output_masks.keys() {
        if !linked.iter().any(|link| link.vertex_register == *register) {
            return Err(Sm3WgslError::UnlinkedVertexOutput {
                register: *register,
            });
        }
    }
    let colour = Sm3Register {
        file: Sm3RegisterFile::ColourOutput,
        index: 0,
    };
    if let Some(register) = pixel.output_masks.keys().find(|register| **register != colour) {
        return Err(Sm3WgslError::UnsupportedPixelOutput {
            register: *register,
        });
    }
    if !pixel.output_masks.contains_key(&colour) {
        return Err(Sm3WgslError::MissingColourOutput);
    }
    Ok(())
}

fn emit_vertex_entry(
    out: &mut String,
    abi: &PassLoweringAbi,
    vertex: &Sm3ProgramIr,
    plan: &LoweringPlan,
    vertex_len: usize,
    pixel_len: usize,
) -> Result<(), Sm3WgslError> {
    // `@invariant` keeps the position bit-identical across the pipelines of a
    // pass, which depth-equal passes rely on.
    out.push_str("\nstruct Sm3Varyings {\n    @builtin(position) @invariant position: vec4<f32>,\n");
    for link in &abi.varyings {
        emit!(out, "    @location({0}) varying_{0}: vec4<f32>,\n", link.location);
    }
    emit!(
        out,
        "    @interpolate(flat) @location({}) sm3_constant_base: u32,\n",
        abi.varyings.len()
    );
    out.push_str("}\n\n@vertex\nfn sm3_vertex_main(\n");
    for (location, decl_type) in abi.vertex_inputs.iter().filter_map(attribute_of) {
        emit!(
            out,
            "    @location({location}) attribute_{location}: {},\n",
            attribute_type(decl_type)
        );
    }
    out.push_str("    @builtin(instance_index) sm3_constant_base: u32,\n) -> Sm3Varyings {\n");

    for input in &abi.vertex_inputs {
        let value = match attribute_of(input) {
            Some((location, decl_type)) => attribute_value(decl_type, location),
            None => default_value(input),
        };
        emit!(out, "    let {} = {value};\n", register_name(input.register));
    }
    emit_constant_prologue(out, plan, &abi.vertex_constants, "sm3_constant_base")?;
    emit_texture_slot_prologue(out, plan, abi, "sm3_constant_base", vertex_len + pixel_len)?;
    let defined = emit_prologue(out, plan);
    let helper = plan.uses_relative_float.then_some("sm3_vs_c");
    emit_body(out, vertex, plan, defined, helper)?;

    emit!(
        out,
        "    return Sm3Varyings(\n        {},\n",
        register_name(abi.position.vertex_register)
    );
    for link in &abi.varyings {
        emit!(out, "        {},\n", register_name(link.vertex_register));
    }
    out.push_str("        sm3_constant_base,\n    );\n}\n");
    Ok(())
}

fn emit_fragment_entry(
    out: &mut String,
    abi: &PassLoweringAbi,
    pixel: &Sm3ProgramIr,
    plan: &LoweringPlan,
    name: &str,
    alpha_test: Option<AlphaTest>,
    vertex_len: usize,
    pixel_len: usize,
) -> Result<(), Sm3WgslError> {
    let misc = |index: u16| Sm3Register {
        file: Sm3RegisterFile::Misc,
        index,
    };
    emit!(out, "\n@fragment\nfn {name}(varyings: Sm3Varyings");
    if plan.external.contains(&misc(1)) {
        out.push_str(", @builtin(front_facing) sm3_front_facing: bool");
    }
    out.push_str(") -> @location(0) vec4<f32> {\n");

    for register in plan
        .external
        .iter()
        .filter(|register| register.file == Sm3RegisterFile::Misc)
    {
        match register.index {
            0 => {
                let read = plan.external_read_masks.get(register).copied().unwrap_or(0);
                if read & !0b11 != 0 {
                    return Err(Sm3WgslError::UnsupportedSourceFile {
                        at_word: 0,
                        file: Sm3RegisterFile::Misc,
                    });
                }
                out.push_str("    let misc0 = varyings.position.xy - vec2<f32>(0.5);\n");
            }
            1 => out.push_str(
                "    let misc1 = vec4<f32>(select(-1.0, 1.0, sm3_front_facing));\n",
            ),
            _ => {
                return Err(Sm3WgslError::UnsupportedSourceFile {
                    at_word: 0,
                    file: Sm3RegisterFile::Misc,
                });
            }
        }
    }
    for link in &abi.varyings {
        if let Some(register) = link.pixel_register {
            emit!(
                out,
                "    let {} = varyings.varying_{};\n",
                register_name(register),
                link.location
            );
        }
    }
    let constant_base = format!("varyings.sm3_constant_base + {vertex_len}u");
    emit_constant_prologue(out, plan, &abi.pixel_constants, &constant_base)?;
    emit_texture_slot_prologue(
        out,
        plan,
        abi,
        "varyings.sm3_constant_base",
        vertex_len + pixel_len,
    )?;
    let defined = emit_prologue(out, plan);
    emit_body(out, pixel, plan, defined, None)?;
    if let Some(test) = alpha_test {
        emit_alpha_test(out, test)?;
    }
    out.push_str("    return oc0;\n}\n");
    Ok(())
}

/// Lower a vertex and pixel program into one module for `abi`.
pub fn lower_pass_to_wgsl(
    abi: &PassLoweringAbi,
    vertex: &Sm3ProgramIr,
    pixel: &Sm3ProgramIr,
) -> Result<PassWgsl, Sm3WgslError> {
    let vertex_plan = plan_program(vertex)?;
    let pixel_plan = plan_program(pixel)?;
    let vertex_len = constant_block_len(&vertex_plan, &abi.vertex_constants)?;
    let pixel_len = constant_block_len(&pixel_plan, &abi.pixel_constants)?;
    check_links(abi, &vertex_plan, &pixel_plan)?;

    let mut out = String::new();
    out.push_str("struct Sm3ConstantArena { c: array<vec4<f32>> }\n");
    out.push_str("@group(0) @binding(0) var<storage, read> sm3_constants: Sm3ConstantArena;\n");
    if vertex_plan.uses_relative_float {
        out.push_str("fn sm3_vs_c(base: u32, i: i32) -> vec4<f32> {\n");
        if vertex_len == 0 {
            out.push_str("    return vec4<f32>(0.0);\n}\n");
        } else {
            emit!(
                out,
                "    if i < 0 || i >= {vertex_len} {{ return vec4<f32>(0.0); }}\n    return sm3_constants.c[base + u32(i)];\n}}\n"
            );
        }
    }
    emit_texture_table(&mut out, abi);
    if abi
        .vertex_inputs
        .iter()
        .filter_map(attribute_of)
        .any(|(_, decl_type)| is_packed_format(decl_type))
    {
        out.push_str(T6_VERTEX_HELPERS);
    }
    emit_vertex_entry(&mut out, abi, vertex, &vertex_plan, vertex_len, pixel_len)?;
    for index in 0..=abi.alpha_tests.len() {
        let (name, test) = match index.checked_sub(1) {
            None => (String::from(PASS_FRAGMENT_ENTRY), None),
            Some(test) => (
                pass_fragment_alpha_test_entry(test),
                Some(abi.alpha_tests[test]),
            ),
        };
        emit_fragment_entry(
            &mut out,
            abi,
            pixel,
            &pixel_plan,
            &name,
            test,
            vertex_len,
            pixel_len,
        )?;
    }

    Ok(PassWgsl {
        source: out,
        attribute_count: abi.vertex_inputs.iter().filter_map(attribute_of).count(),
        varying_count: abi.varyings.len(),
        vertex_constant_len: vertex_len,
        pixel_constant_len: pixel_len,
        sampler_count: abi.samplers.len(),
    })
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use d3d9_decl::Semantic;

    use super::*;
    use crate::ir::decode_sm3_program;
    use crate::token::ShaderStage;

    const VS_3_0: u32 = 0xfffe_0300;
    const PS_3_0: u32 = 0xffff_0300;
    const END: u32 = 0x0000_ffff;

    fn bytes(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|word| word.to_le_bytes()).collect()
    }

    fn register(file: Sm3RegisterFile, index: u16) -> Sm3Register {
        Sm3Register { file, index }
    }

    /// `mov o0, v0`
    fn vertex_program() -> Sm3ProgramIr {
        let words = [VS_3_0, 0x0200_0001, 0xe00f_0000, 0x90e4_0000, END];
        decode_sm3_program(&bytes(&words), ShaderStage::Vertex).unwrap()
    }

    /// `mov oC0, c0`
    fn pixel_program() -> Sm3ProgramIr {
        let words = [PS_3_0, 0x0200_0001, 0x800f_0800, 0xa0e4_0000, END];
        decode_sm3_program(&bytes(&words), ShaderStage::Pixel).unwrap()
    }

    fn abi(decl_type: DeclType, alpha_tests: Vec<AlphaTest>) -> PassLoweringAbi {
        let position = register(Sm3RegisterFile::Output, 0);
        PassLoweringAbi {
            vertex_inputs: vec![VertexInput {
                register: register(Sm3RegisterFile::Input, 0),
                location: Some(0),
                decl_type: Some(decl_type),
                semantic: Semantic::position(0),
            }],
            position: VaryingLink {
                semantic: Semantic::position(0),
                vertex_register: position,
                pixel_register: None,
                location: 0,
            },
            varyings: Vec::new(),
            vertex_constants: Vec::new(),
            pixel_constants: vec![ConstantSlot {
                register: 0,
                program_defined: false,
            }],
            samplers: Vec::new(),
            alpha_tests,
        }
    }

    #[test]
    fn a_minimal_pass_lowers_to_both_entries() {
        let abi = abi(DeclType::Float4, Vec::new());
        let pass = lower_pass_to_wgsl(&abi, &vertex_program(), &pixel_program()).unwrap();
        assert!(pass.source.contains("fn sm3_vertex_main("));
        assert!(pass.source.contains("fn sm3_fragment_main(varyings: Sm3Varyings)"));
        assert!(pass.source.contains("@location(0) attribute_0: vec4<f32>,"));
        assert!(pass.source.contains("let v0 = attribute_0;"));
        assert!(pass.source.contains("let c0 = sm3_constants.c[varyings.sm3_constant_base + 0u + 0u];"));
        assert!(pass.source.ends_with("    return oc0;\n}\n"));
        assert_eq!(pass.attribute_count, 1);
        assert_eq!(pass.varying_count, 0);
        assert_eq!(pass.vertex_constant_len, 0);
        assert_eq!(pass.pixel_constant_len, 1);
        assert_eq!(pass.sampler_count, 0);
        assert!(!pass.source.contains("t6_unit_vec"));
    }

    #[test]
    fn lowering_is_deterministic() {
        let abi = abi(DeclType::Float4, Vec::new());
        let first = lower_pass_to_wgsl(&abi, &vertex_program(), &pixel_program()).unwrap();
        let second = lower_pass_to_wgsl(&abi, &vertex_program(), &pixel_program()).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn each_alpha_test_gets_its_own_fragment_entry() {
        let tests = vec![AlphaTest::from_raw(5, 128), AlphaTest::from_raw(1, 0)];
        let abi = abi(DeclType::Float4, tests);
        let pass = lower_pass_to_wgsl(&abi, &vertex_program(), &pixel_program()).unwrap();
        assert_eq!(pass_fragment_alpha_test_entry(1), "sm3_fragment_main_atest1");
        assert!(pass.source.contains("fn sm3_fragment_main_atest0("));
        assert!(pass.source.contains("fn sm3_fragment_main_atest1("));
        assert!(pass.source.contains("if !(sm3_alpha_ref > 128.0) { discard; }"));
        assert!(pass.source.contains("    discard;\n    return oc0;"));
    }

    #[test]
    fn alpha_test_always_adds_nothing_and_unknown_is_refused() {
        let mut text = String::new();
        emit_alpha_test(&mut text, AlphaTest::from_raw(8, 7)).unwrap();
        assert_eq!(text, "");
        assert_eq!(
            emit_alpha_test(&mut text, AlphaTest::from_raw(99, 7)),
            Err(Sm3WgslError::UnsupportedAlphaTestFunc { raw_func: Some(99) })
        );
    }

    #[test]
    fn packed_formats_pull_in_their_helpers() {
        let abi = abi(DeclType::T6TexCoord, Vec::new());
        let pass = lower_pass_to_wgsl(&abi, &vertex_program(), &pixel_program()).unwrap();
        assert!(pass.source.contains(T6_VERTEX_HELPERS));
        assert!(pass.source.contains("@location(0) attribute_0: u32,"));
        assert!(pass.source.contains("let v0 = t6_texcoord(attribute_0);"));
    }

    #[test]
    fn an_unwritten_declared_position_is_refused() {
        let mut abi = abi(DeclType::Float4, Vec::new());
        abi.position.vertex_register = register(Sm3RegisterFile::Output, 1);
        assert_eq!(
            lower_pass_to_wgsl(&abi, &vertex_program(), &pixel_program()),
            Err(Sm3WgslError::DeclaredOutputNeverWritten {
                register: register(Sm3RegisterFile::Output, 1),
            })
        );
    }

    #[test]
    fn a_constant_the_abi_does_not_bind_is_refused() {
        let mut abi = abi(DeclType::Float4, Vec::new());
        abi.pixel_constants.clear();
        assert_eq!(
            lower_pass_to_wgsl(&abi, &vertex_program(), &pixel_program()),
            Err(Sm3WgslError::UnboundConstantRegister {
                register: register(Sm3RegisterFile::FloatConstant, 0),
            })
        );
    }
}
