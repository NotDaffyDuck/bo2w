//! The geometry Black Ops II effects hand the renderer's fallback pass: sprite quads, beams,
//! effect models and particle clouds, plus the dynamic lights effects throw.
//!
//! Their materials have no technique, so the code mesh never draws them; the fallback pass draws
//! these world-space vertices instead.

use bevy::prelude::*;
use render_frame::SmodelVertex;

/// Element flag: drawn with the viewmodel. The parts of the game's first-person gas flashes
/// carry it; their smoke and the Ray Gun's first-person flash do not.
pub(in crate::adapters::fx) const T6_FX_ELEM_DRAW_WITH_VIEWMODEL: i32 = 0x1000;

/// The most effect lights the fallback pass reads.
pub const T6_DYN_LIGHTS: usize = 16;

/// A draw range into the mesh: first index, index count, catalog material.
pub type DrawRange = (u32, u32, u32);

/// This frame's effect geometry. Every draw is a range of `indices` with the material it uses.
#[derive(Resource, Clone, Debug, Default)]
pub struct T6FxMesh {
    pub vertices: Vec<SmodelVertex>,
    pub indices: Vec<u32>,
    pub draws: Vec<DrawRange>,
    /// The same for elements drawn with the first-person gun.
    pub viewmodel_draws: Vec<DrawRange>,
    /// Particle clouds (embers, ash): each draws the shared box of points, appended once a
    /// frame, with its own placement, colour and size.
    pub clouds: Vec<T6CloudDraw>,
    /// Where the shared box starts in `indices`, once a cloud has appended it this frame.
    cloud_box: Option<u32>,
}

/// One particle cloud. Its vertex shader takes each point of the box to the world by the cloud's
/// placement, then out to a quad facing the camera, sized by the particle's width and height;
/// the pixel shader is the texture times the cloud colour, halved.
#[derive(Clone, Debug)]
pub struct T6CloudDraw {
    pub first_index: u32,
    pub index_count: u32,
    pub material: u32,
    /// World from the box (columns): the cloud's rotation, scale and origin.
    pub world_from_local: [f32; 16],
    /// The cloud's colour (rgba, 0..1), lit.
    pub color: [f32; 4],
    /// The particle's width and height.
    pub size: [f32; 2],
}

/// The lights effects throw this frame (muzzle flashes, explosions, impact flashes), nearest the
/// eye first and at most [`T6_DYN_LIGHTS`]: per light its origin and radius, then its linear rgb.
#[derive(Resource, Clone, Debug, Default)]
pub struct T6DynLights {
    pub lights: Vec<[f32; 8]>,
}

/// The cloud box: 8 x 8 x 16 cells of the unit cube, each point jittered inside its cell by the C
/// runtime's `rand` from its default seed, nearest the centre first (a cloud shows its innermost
/// points), as the engine builds its cloud vertex buffer.
fn cloud_box() -> &'static [[f32; 3]] {
    use fx_t6::runtime as rt;
    static BOX: std::sync::OnceLock<Vec<[f32; 3]>> = std::sync::OnceLock::new();
    BOX.get_or_init(|| {
        let mut seed = rt::MSVCRT_HOLDRAND_DEFAULT;
        let mut keyed = Vec::with_capacity(rt::FX_PARTICLE_CLOUD_TEMPLATE_CELLS);
        for x in 0..rt::FX_PARTICLE_CLOUD_GRID_X {
            for y in 0..rt::FX_PARTICLE_CLOUD_GRID_Y {
                for z in 0..rt::FX_PARTICLE_CLOUD_GRID_Z {
                    let jitter = [
                        rt::msvcrt_rand01(&mut seed),
                        rt::msvcrt_rand01(&mut seed),
                        rt::msvcrt_rand01(&mut seed),
                    ];
                    let cell = rt::particle_cloud_cell_xyz(x, y, z, jitter);
                    keyed.push((rt::particle_cloud_cell_radius_sq(cell), cell));
                }
            }
        }
        keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
        keyed.into_iter().map(|(_, cell)| cell).collect()
    })
}

fn unit_color(rgba: [u8; 4]) -> [f32; 4] {
    rgba.map(|channel| f32::from(channel) / 255.0)
}

impl T6FxMesh {
    /// Empties the mesh for a new frame.
    pub(in crate::adapters::fx) fn clear(&mut self) {
        self.vertices.clear();
        self.indices.clear();
        self.draws.clear();
        self.viewmodel_draws.clear();
        self.clouds.clear();
        self.cloud_box = None;
    }

    /// One effect model (shell casing, debris), rigid: its bind pose placed by the element
    /// (origin, axis rows, scale), one draw per surface that has a material.
    pub(crate) fn push_model(
        &mut self,
        skel: &asset_model::ModelSkel,
        material: impl Fn(usize) -> Option<u32>,
        origin: [f32; 3],
        axis: [[f32; 3]; 3],
        scale: f32,
    ) {
        let origin = Vec3::from_array(origin);
        let [ax, ay, az] = axis.map(Vec3::from_array);
        let scale = if scale > 0.0 { scale } else { 1.0 };
        let surfaces = skel
            .surface_vertex_ranges
            .iter()
            .zip(&skel.surface_index_ranges)
            .enumerate();
        for (surface, (&(first_vertex, vertex_count), &(first_index, index_count))) in surfaces {
            let Some(material) = material(surface) else {
                continue;
            };
            if first_vertex + vertex_count > skel.positions.len()
                || first_index + index_count > skel.indices.len()
            {
                continue;
            }
            let base = self.vertices.len() as u32;
            for v in first_vertex..first_vertex + vertex_count {
                let [x, y, z] = skel.positions[v];
                let [nx, ny, nz] = skel.normals.get(v).copied().unwrap_or([0.0, 0.0, 1.0]);
                self.vertices.push(SmodelVertex {
                    position: (origin + (ax * x + ay * y + az * z) * scale).to_array(),
                    normal: (ax * nx + ay * ny + az * nz).normalize_or_zero().to_array(),
                    color: skel.colors.get(v).copied().unwrap_or([1.0; 4]),
                    uv0: skel.uvs.get(v).copied().unwrap_or([0.0; 2]),
                });
            }
            let start = self.indices.len() as u32;
            let rebase = first_vertex as u32;
            self.indices.extend(
                skel.indices[first_index..first_index + index_count]
                    .iter()
                    .map(|&i| base + i.saturating_sub(rebase)),
            );
            self.draws.push((start, index_count as u32, material));
        }
    }

    /// One tracer beam: a world-space strip with its own colours.
    pub(crate) fn push_beam(&mut self, verts: &[fx_t6::FxBeamVert], indices: &[u16], material: u32) {
        let base = self.vertices.len() as u32;
        let start = self.indices.len() as u32;
        self.vertices.extend(verts.iter().map(|v| SmodelVertex {
            position: v.xyz,
            normal: [0.0, 0.0, 1.0],
            color: unit_color(v.color),
            uv0: v.uv,
        }));
        self.indices.extend(indices.iter().map(|&i| base + u32::from(i)));
        let count = self.indices.len() as u32 - start;
        if count > 0 {
            self.draws.push((start, count, material));
        }
    }

    /// One particle cloud: a draw over the shared box (appended the first time in a frame) of as
    /// many of the innermost points as the cloud has particles, or as many as its shape keeps.
    pub(in crate::adapters::fx) fn push_cloud(
        &mut self,
        cloud: &fx_t6::runtime::GfxParticleCloud,
        particles: Option<u32>,
        material: u32,
    ) {
        use fx_t6::runtime as rt;
        let first_index = match self.cloud_box {
            Some(first) => first,
            None => self.append_cloud_box(),
        };
        let kept = rt::particle_cloud_draw_cell_count(cloud.flags) as u32;
        let cells = particles
            .unwrap_or(kept)
            .min(rt::FX_PARTICLE_CLOUD_TEMPLATE_CELLS as u32);
        let placement = Mat4::from_scale_rotation_translation(
            Vec3::splat(cloud.placement_scale),
            Quat::from_array(cloud.quat).normalize(),
            Vec3::from_array(cloud.pos),
        );
        self.clouds.push(T6CloudDraw {
            first_index,
            index_count: cells * 6,
            material,
            world_from_local: placement.to_cols_array(),
            color: rt::particle_cloud_color_const(cloud.color),
            size: [cloud.size0, cloud.size1],
        });
    }

    /// Adds the cloud box's quads and returns where its indices start.
    fn append_cloud_box(&mut self) -> u32 {
        use fx_t6::runtime as rt;
        let first_index = self.indices.len() as u32;
        let base = self.vertices.len() as u32;
        for (cell, point) in cloud_box().iter().enumerate() {
            self.vertices
                .extend(rt::FX_PARTICLE_CLOUD_UV.iter().map(|&uv0| SmodelVertex {
                    position: *point,
                    normal: [0.0, 0.0, 1.0],
                    color: [1.0; 4],
                    uv0,
                }));
            let at = base + cell as u32 * 4;
            self.indices
                .extend(rt::FX_PARTICLE_CLOUD_QUAD_INDICES.iter().map(|&i| at + u32::from(i)));
        }
        self.cloud_box = Some(first_index);
        first_index
    }

    /// One camera-facing sprite quad, its corners from the unit square placed by `transform`.
    pub(in crate::adapters::fx) fn push_quad(
        &mut self,
        transform: Transform,
        color_rgba: [u8; 4],
        uv: fx_t6::runtime::FxSpriteAtlasUv,
    ) {
        use fx_t6::runtime as rt;
        let base = self.vertices.len() as u32;
        let normal = (transform.rotation * Vec3::Z).normalize_or_zero().to_array();
        let color = unit_color(color_rgba);
        let corners = uv.corners();
        self.vertices
            .extend(rt::FX_SPRITE_QUAD_LOCAL_XY.iter().zip(corners).map(|(xy, uv0)| SmodelVertex {
                position: transform
                    .transform_point(Vec3::new(xy[0], xy[1], 0.0))
                    .to_array(),
                normal,
                color,
                uv0,
            }));
        self.indices
            .extend(rt::FX_SPRITE_QUAD_INDICES.iter().map(|&i| base + u32::from(i)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fx_t6::runtime as rt;

    fn cloud(flags: u32) -> rt::GfxParticleCloud {
        rt::GfxParticleCloud {
            quat: [0.0, 0.0, 0.0, 1.0],
            pos: [1.0, 2.0, 3.0],
            placement_scale: 2.0,
            axis_or_vel: [0.0; 3],
            color: 0xFF80_4020,
            size0: 4.0,
            size1: 6.0,
            flags,
            scale: 1.0,
        }
    }

    #[test]
    fn a_quad_adds_four_corners_and_its_indices() {
        let mut mesh = T6FxMesh::default();
        mesh.push_quad(Transform::default(), [255, 0, 0, 255], rt::FxSpriteAtlasUv::FULL);
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.indices.len(), rt::FX_SPRITE_QUAD_INDICES.len());
        assert_eq!(mesh.vertices[0].color, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(mesh.vertices[0].normal, [0.0, 0.0, 1.0]);
        // A second quad's indices are offset past the first's vertices.
        mesh.push_quad(Transform::default(), [0; 4], rt::FxSpriteAtlasUv::FULL);
        assert!(mesh.indices[rt::FX_SPRITE_QUAD_INDICES.len()..].iter().all(|&i| i >= 4));
    }

    #[test]
    fn a_beam_makes_one_draw_unless_it_is_empty() {
        let mut mesh = T6FxMesh::default();
        mesh.push_beam(&[], &[], 9);
        assert!(mesh.draws.is_empty());
        let vert = fx_t6::FxBeamVert {
            xyz: [1.0, 2.0, 3.0],
            color: [255, 128, 0, 255],
            uv: [0.5, 0.25],
        };
        mesh.push_beam(&[vert, vert, vert], &[0, 1, 2], 9);
        assert_eq!(mesh.draws, vec![(0, 3, 9)]);
        assert_eq!(mesh.vertices[1].position, [1.0, 2.0, 3.0]);
        assert_eq!(mesh.vertices[1].uv0, [0.5, 0.25]);
    }

    #[test]
    fn clouds_share_one_box_per_frame() {
        let mut mesh = T6FxMesh::default();
        mesh.push_cloud(&cloud(0), None, 5);
        let after_first = (mesh.vertices.len(), mesh.indices.len());
        mesh.push_cloud(&cloud(0), Some(10), 6);
        assert_eq!((mesh.vertices.len(), mesh.indices.len()), after_first);
        assert_eq!(mesh.clouds[0].first_index, mesh.clouds[1].first_index);
        assert_eq!(mesh.clouds[0].index_count, rt::FX_PARTICLE_CLOUD_TEMPLATE_CELLS as u32 * 6);
        assert_eq!(mesh.clouds[1].index_count, 60);
    }

    #[test]
    fn a_cloud_never_draws_more_cells_than_the_box_has() {
        let mut mesh = T6FxMesh::default();
        mesh.push_cloud(&cloud(0), Some(u32::MAX), 1);
        assert_eq!(mesh.clouds[0].index_count, rt::FX_PARTICLE_CLOUD_TEMPLATE_CELLS as u32 * 6);
    }

    #[test]
    fn the_flags_halve_the_cells_a_cloud_keeps() {
        let mut mesh = T6FxMesh::default();
        mesh.push_cloud(&cloud(1), None, 1);
        assert_eq!(mesh.clouds[0].index_count, rt::FX_PARTICLE_CLOUD_TEMPLATE_CELLS as u32 / 2 * 6);
    }

    #[test]
    fn the_box_is_nearest_the_centre_first() {
        let radii: Vec<f32> = cloud_box()
            .iter()
            .map(|&cell| rt::particle_cloud_cell_radius_sq(cell))
            .collect();
        assert_eq!(radii.len(), rt::FX_PARTICLE_CLOUD_TEMPLATE_CELLS);
        assert!(radii.windows(2).all(|pair| pair[0] <= pair[1]));
    }

    #[test]
    fn clearing_lets_the_next_frame_append_the_box_again() {
        let mut mesh = T6FxMesh::default();
        mesh.push_cloud(&cloud(0), None, 1);
        mesh.clear();
        assert!(mesh.vertices.is_empty() && mesh.indices.is_empty() && mesh.clouds.is_empty());
        mesh.push_cloud(&cloud(0), None, 1);
        assert_eq!(mesh.clouds[0].first_index, 0);
        assert!(!mesh.vertices.is_empty());
    }
}
