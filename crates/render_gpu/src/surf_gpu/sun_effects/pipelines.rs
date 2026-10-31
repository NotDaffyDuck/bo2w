use super::SunGpu;
use bevy::prelude::*;
use bevy::render::render_resource::{
    BlendComponent, BlendFactor, BlendOperation, BlendState, ColorTargetState, ColorWrites,
    FragmentState, MultisampleState, PipelineCache, PrimitiveState,
    RenderPipelineDescriptor, SpecializedRenderPipeline, SpecializedRenderPipelines, TextureFormat,
    VertexState,
};

/// The four draws of the sun effect, in the order they run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Stage {
    History,
    Sprite,
    Flare,
    Grade,
}

/// Plain additive: source and destination both at full weight.
const ADD_ONE: BlendComponent = BlendComponent { operation: BlendOperation::Add, src_factor: BlendFactor::One, dst_factor: BlendFactor::One };

impl Stage {
    pub(super) const ALL: [Stage; 4] = [Stage::History, Stage::Sprite, Stage::Flare, Stage::Grade];

    pub(super) fn bay(self) -> usize {
        self as usize
    }

    fn name(self) -> &'static str {
        match self {
            Stage::History => "bo2_sun_history",
            Stage::Sprite => "bo2_sun_sprite",
            Stage::Flare => "bo2_sun_flare",
            Stage::Grade => "bo2_sun_blind",
        }
    }

    fn entries(self) -> (&'static str, &'static str) {
        match self {
            Stage::History => ("vs_history", "fs_history"),
            Stage::Sprite => ("vs_quad", "fs_sprite"),
            Stage::Flare => ("vs_quad", "fs_flare"),
            Stage::Grade => ("vs_grade", "fs_grade"),
        }
    }

    fn format(self) -> TextureFormat {
        match self {
            Stage::History => TextureFormat::Rgba16Float,
            _ => TextureFormat::Rgba8Unorm,
        }
    }

    fn blend(self) -> Option<BlendState> {
        matches!(self, Stage::Sprite | Stage::Flare).then_some(BlendState {
            color: ADD_ONE,
            alpha: ADD_ONE,
        })
    }

    fn shader_defs(self) -> Vec<bevy::shader::ShaderDefVal> {
        match self {
            Stage::Flare => vec!["SUN_FLARE".into()],
            _ => Vec::new(),
        }
    }
}

impl SpecializedRenderPipeline for SunGpu {
    type Key = Stage;

    fn specialize(&self, stage: Stage) -> RenderPipelineDescriptor {
        let (vs_entry, fs_entry) = stage.entries();
        let stage_defs = stage.shader_defs();
        // A full-screen draw with no vertex buffers, no depth, no culling and one colour target.
        let out_tgt = ColorTargetState { format: stage.format(), blend: stage.blend(), write_mask: ColorWrites::all() };
        let vertex = VertexState { shader: self.program.clone(), shader_defs: stage_defs.clone(), entry_point: Some(vs_entry.into()), buffers: vec![] };
        let fragment = FragmentState { shader: self.program.clone(), shader_defs: stage_defs, entry_point: Some(fs_entry.into()), targets: vec![Some(out_tgt)] };
        RenderPipelineDescriptor {
            label: Some(stage.name().into()),
            layout: vec![self.layout.clone()],
            immediate_size: 0,
            vertex,
            fragment: Some(fragment),
            primitive: PrimitiveState { cull_mode: None, ..default() },
            depth_stencil: None,
            multisample: MultisampleState::default(),
            zero_initialize_workgroup_memory: false,
        }
    }
}

/// Queue the four pipelines once; later frames find them all present and return at once.
pub(super) fn queue_pipelines(
    gpu: Option<ResMut<SunGpu>>,
    pipes: Res<PipelineCache>,
    mut specialized: ResMut<SpecializedRenderPipelines<SunGpu>>,
) {
    let Some(mut gpu) = gpu else {
        return;
    };
    if gpu.pipelines.iter().all(Option::is_some) {
        return;
    }
    let ids = Stage::ALL.map(|stage| Some(specialized.specialize(&pipes, &gpu, stage)));
    gpu.pipelines = ids;
}
