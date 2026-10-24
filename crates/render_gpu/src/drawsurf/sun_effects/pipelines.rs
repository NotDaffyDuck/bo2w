use super::SunGpu;
use bevy::prelude::*;
use bevy::render::render_resource::{
    BlendComponent, BlendFactor, BlendOperation, BlendState, ColorTargetState, ColorWrites,
    FragmentState, FrontFace, MultisampleState, PipelineCache, PrimitiveState,
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

const ADD_ONE: BlendComponent = BlendComponent {
    src_factor: BlendFactor::One,
    dst_factor: BlendFactor::One,
    operation: BlendOperation::Add,
};

impl Stage {
    pub(super) const ALL: [Stage; 4] = [Stage::History, Stage::Sprite, Stage::Flare, Stage::Grade];

    pub(super) fn slot(self) -> usize {
        self as usize
    }

    fn label(self) -> &'static str {
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
        let (vertex, fragment) = stage.entries();
        let defs = stage.shader_defs();
        RenderPipelineDescriptor {
            label: Some(stage.label().into()),
            layout: vec![self.layout.clone()],
            immediate_size: 0,
            vertex: VertexState {
                shader: self.shader.clone(),
                shader_defs: defs.clone(),
                entry_point: Some(vertex.into()),
                buffers: Vec::new(),
            },
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                shader_defs: defs,
                entry_point: Some(fragment.into()),
                targets: vec![Some(ColorTargetState {
                    format: stage.format(),
                    blend: stage.blend(),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            primitive: PrimitiveState {
                front_face: FrontFace::Ccw,
                cull_mode: None,
                ..default()
            },
            depth_stencil: None,
            multisample: MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            zero_initialize_workgroup_memory: false,
        }
    }
}

/// Queue the four pipelines once; later frames find them all present and return at once.
pub(super) fn queue_pipelines(
    gpu: Option<ResMut<SunGpu>>,
    cache: Res<PipelineCache>,
    mut specialized: ResMut<SpecializedRenderPipelines<SunGpu>>,
) {
    let Some(mut gpu) = gpu else {
        return;
    };
    if gpu.pipelines.iter().all(Option::is_some) {
        return;
    }
    let ids = Stage::ALL.map(|stage| Some(specialized.specialize(&cache, &gpu, stage)));
    gpu.pipelines = ids;
}
