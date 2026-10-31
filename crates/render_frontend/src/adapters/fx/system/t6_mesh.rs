//! The geometry Black Ops II effects hand the renderer's fallback pass: sprite quads, beams,
//! effect models and particle clouds, plus the dynamic lights effects throw.
//!
//! Their materials have no technique, so the code mesh never draws them; the fallback pass draws
//! these world-space vertices instead.

use bevy::prelude::*;
use render_frame::PropVertex;

/// Element flag: drawn with the viewmodel. The parts of the game's first-person gas flashes
/// carry it; their smoke and the Ray Gun's first-person flash do not.
pub(in crate::adapters::fx) const T6_FX_ELEM_DRAW_WITH_VIEWMODEL: i32 = 0x1000;

/// The most effect lights the fallback pass reads. Black Ops II draws at most 4 dynamic lights in the whole frame
/// (the perk machines' lights summed into a hot spot with more).
pub const T6_DYN_LIGHTS: usize = 4;

/// A draw range into the mesh: first index, index count, catalog material.

/// This frame's effect geometry. Every draw is a range of `indices` with the material it uses.
#[derive(Resource, Clone, Debug, Default)]
pub struct T6FxMesh {
    pub vertices: Vec<PropVertex>,
    pub indices: Vec<u32>,
    pub draws: Vec<(u32, u32, u32)>,
    /// The same for elements drawn with the first-person gun.
    pub viewmodel_draws: Vec<(u32, u32, u32)>,
    /// Particle clouds (embers, ash): each draws the shared box of points, appended once a
    /// frame, with its own placement, colour and size.
    pub clouds: Vec<T6CloudDraw>,
    /// Where the shared box starts in `indices`, once a cloud has appended it this frame.
    cloud_box: Option<u32>,
    /// Per effect model surface (by its first index), its box from the world (columns: the surface's bounds to
    /// -1..1) and its origin. A projected decal (`mc_projecteddecal_*`, the blood a hit paints on a zombie) draws its
    /// box and paints what lies inside it.
    pub model_boxes: std::collections::HashMap<u32, ([f32; 16], [f32; 3])>,
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

/// The lights effects throw this frame (muzzle flashes, explosions, impact flashes), most important first
/// and at most [`T6_DYN_LIGHTS`]: per light its origin and radius, then its linear rgb.
#[derive(Resource, Clone, Debug, Default)]
pub struct T6DynLights {
    pub lights: Vec<[f32; 8]>,
}

/// The cloud box: 8 x 8 x 16 cells of the unit cube, each point jittered inside its cell by the C
/// runtime's `rand` from its default seed, nearest the centre first (a cloud shows its innermost
/// points), as the engine builds its cloud vertex buffer.
fn cloud_box() -> &'static [[f32; 3]] {
    use fx_t6::runtime;
    static BOX: std::sync::OnceLock<Vec<[f32; 3]>> = std::sync::OnceLock::new();
    BOX.get_or_init(|| {
        let mut seed = runtime::CLASSIC_DICE_START;
        let mut keyed = Vec::with_capacity(runtime::SWARM_STAMP_SPECKS);
        for x in 0..runtime::SWARM_LATTICE_ACROSS {
            for y in 0..runtime::SWARM_LATTICE_DEEP {
                for z in 0..runtime::SWARM_LATTICE_TALL {
                    let jitter = [
                        runtime::classic_dice_unit(&mut seed),
                        runtime::classic_dice_unit(&mut seed),
                        runtime::classic_dice_unit(&mut seed),
                    ];
                    let cell = runtime::particle_cloud_cell_xyz(x, y, z, jitter);
                    keyed.push((runtime::particle_cloud_cell_radius_sq(cell), cell));
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

/// A vertex whose normal points straight up, for geometry that is not lit by its normal.
fn upward(position: [f32; 3], tint: [f32; 4], first_texcoord: [f32; 2]) -> PropVertex {
    PropVertex {
        position,
        normal: Vec3::Z.to_array(),
        color: tint,
        first_texcoord,
    }
}

impl T6FxMesh {
    /// The number the next vertex gets.
    fn next_vertex(&self) -> u32 {
        u32::try_from(self.vertices.len()).unwrap_or(u32::MAX)
    }

    /// The position the next index is written at.
    fn next_index(&self) -> u32 {
        self.indices.len().try_into().unwrap_or(u32::MAX)
    }

    /// Appends indices that count from `base`, the vertex they were built against.
    fn append_from(&mut self, base: u32, indices: impl IntoIterator<Item = u32>) {
        self.indices.extend(indices.into_iter().map(|i| base + i));
    }

    /// Empties the mesh for a new frame.
    pub(in crate::adapters::fx) fn clear(&mut self) {
        self.vertices.truncate(0);
        self.indices.truncate(0);
        self.draws.truncate(0);
        self.viewmodel_draws.clear();
        self.clouds.clear();
        self.cloud_box = None;
        self.model_boxes.clear();
    }

    /// One effect model (shell casing, debris), rigid: its bind pose placed by the element
    /// (origin, axis rows, scale), one draw per surface that has a material.
    pub(crate) fn push_model(
        &mut self,
        bone_rig: &asset_model::MeshRig,
        material: impl Fn(usize) -> Option<u32>,
        pos: [f32; 3],
        basis: [[f32; 3]; 3],
        scale: f32,
    ) {
        let origin = Vec3::from_array(pos);
        let [ax, ay, az] = basis.map(Vec3::from_array);
        let scale = if scale > 0.0 { scale } else { 1.0 };
        let surfaces = bone_rig
            .facet_vertex_ranges
            .iter()
            .zip(&bone_rig.facet_index_ranges)
            .enumerate();
        for (facet, (&(first_vertex, vertex_count), &(first_index, index_count))) in surfaces {
            let Some(mat) = material(facet) else {
                continue;
            };
            if first_vertex + vertex_count > bone_rig.positions.len()
                || first_index + index_count > bone_rig.indices.len()
            {
                continue;
            }
            let base = self.next_vertex();
            for v in first_vertex..first_vertex + vertex_count {
                let [x, y, z] = bone_rig.positions[v];
                let [facing_x, facing_y, facing_z] = bone_rig.surface_directions.get(v).copied().unwrap_or([0.0, 0.0, 1.0]);
                self.vertices.push(PropVertex {
                    position: (origin + (ax * x + ay * y + az * z) * scale).to_array(),
                    normal: (ax * facing_x + ay * facing_y + az * facing_z).normalize_or_zero().to_array(),
                    color: bone_rig.colors.get(v).copied().unwrap_or([1.0; 4]),
                    first_texcoord: bone_rig.texcoord_set.get(v).copied().unwrap_or([0.0; 2]),
                });
            }
            let start = self.next_index();
            let rebase = first_vertex as u32;
            let local = bone_rig.indices[first_index..first_index + index_count]
                .iter()
                .map(|&i| i.saturating_sub(rebase));
            self.append_from(base, local);
            self.draws.push((start, index_count as u32, mat));
            let (lo, hi) = bone_rig.positions[first_vertex..first_vertex + vertex_count].iter().fold(
                (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
                |(lo, hi), p| {
                    let p = Vec3::from_array(*p);
                    (lo.min(p), hi.max(p))
                },
            );
            let world_from_local = Mat4::from_cols(
                (ax * scale).extend(0.0),
                (ay * scale).extend(0.0),
                (az * scale).extend(0.0),
                origin.extend(1.0),
            );
            let half = ((hi - lo) * 0.5).max(Vec3::splat(0.001));
            let box_from_world =
                Mat4::from_scale(half.recip()) * Mat4::from_translation(-(lo + hi) * 0.5) * world_from_local.inverse();
            self.model_boxes
                .insert(start, (box_from_world.to_cols_array(), origin.to_array()));
        }
    }

    /// One tracer beam: a world-space strip with its own colours.
    pub(crate) fn push_beam(&mut self, vertex_points: &[fx_t6::RayCorner], index_list: &[u16], mat: u32) {
        let (base, start) = (self.next_vertex(), self.next_index());
        let lit = vertex_points.iter().map(|v| upward(v.spot, unit_color(v.color), v.sheet_uv));
        self.vertices.extend(lit);
        self.append_from(base, index_list.iter().map(|&i| u32::from(i)));
        match self.next_index() - start {
            0 => {}
            count => self.draws.push((start, count, mat)),
        }
    }

    /// One particle cloud: a draw over the shared box (appended the first time in a frame) of as
    /// many of the innermost points as the cloud has particles, or as many as its shape keeps.
    pub(in crate::adapters::fx) fn push_cloud(
        &mut self,
        cloud: &fx_t6::runtime::VisualParticleCloud,
        particles: Option<u32>,
        mat: u32,
    ) {
        use fx_t6::runtime;
        let box_start = self.cloud_box.unwrap_or_else(|| self.append_cloud_box());
        let kept = runtime::particle_cloud_draw_cell_count(cloud.flags) as u32;
        let cells = particles
            .unwrap_or(kept)
            .min(runtime::SWARM_STAMP_SPECKS as u32);
        let placement = Mat4::from_scale_rotation_translation(
            Vec3::splat(cloud.layout_size_ratio),
            Quat::from_array(cloud.turn).normalize(),
            Vec3::from(cloud.spot),
        );
        let material = mat;
        self.clouds.push(T6CloudDraw {
            first_index: box_start,
            index_count: cells * 6,
            material,
            world_from_local: placement.to_cols_array(),
            color: runtime::particle_cloud_color_const(cloud.color),
            size: [cloud.start_extent, cloud.second_extent],
        });
    }

    /// Adds the cloud box's quads and returns where its indices start.
    fn append_cloud_box(&mut self) -> u32 {
        use fx_t6::runtime;
        let start = self.next_index();
        let base = self.next_vertex();
        for (cell, point) in cloud_box().iter().enumerate() {
            let corners = runtime::SWARM_CORNER_UVS.iter();
            self.vertices.extend(corners.map(|&first_texcoord| upward(*point, [1.0; 4], first_texcoord)));
            let quad = runtime::SWARM_RECT_TRIANGLE_SLOTS.iter().map(|&i| u32::from(i));
            self.append_from(base + cell as u32 * 4, quad);
        }
        self.cloud_box = Some(start);
        start
    }

    /// One camera-facing sprite quad, its corners from the unit square placed by `transform`.
    pub(in crate::adapters::fx) fn push_quad(
        &mut self,
        xf: Transform,
        rgba_color: [u8; 4],
        uv: fx_t6::runtime::BillboardSheetCell,
    ) {
        use fx_t6::runtime;
        let base = self.next_vertex();
        let facing = PropVertex {
            normal: (xf.rotation * Vec3::Z).normalize_or_zero().to_array(),
            color: unit_color(rgba_color),
            ..upward([0.0; 3], [0.0; 4], [0.0; 2])
        };
        let placed = |xy: &[f32; 2]| xf.transform_point(Vec3::new(xy[0], xy[1], 0.0));
        let corners = runtime::BILLBOARD_CORNER_OFFSETS.iter().zip(uv.corners());
        self.vertices.extend(corners.map(|(xy, first_texcoord)| PropVertex {
            position: placed(xy).to_array(),
            first_texcoord,
            ..facing
        }));
        self.append_from(base, runtime::BILLBOARD_TRIANGLE_SLOTS.iter().map(|&i| u32::from(i)));
    }

    /// A lens flare at `origin`: BO2's `sw4_3d_superflare` vertex shader
    /// places each of its 25 ghosts on screen itself, so every corner sits
    /// on the flare's point. The ghost's number rides in the red channel
    /// (`floor(v1.x * 255)`), the sprite's alpha in alpha (`v1.w`), and the
    /// corner in the texture coordinate (`v2.xy`).
    pub(in crate::adapters::fx) fn push_flare(&mut self, origin: Vec3, alpha: u8) {
        const GHOSTS: u8 = 25;
        const CORNERS: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let alpha = f32::from(alpha) / 255.0;
        for ghost in 0..GHOSTS {
            let base = self.next_vertex();
            let tint = [(f32::from(ghost) + 0.5) / 255.0, 0.0, 0.0, alpha];
            self.vertices.extend(CORNERS.map(|corner| upward(origin.to_array(), tint, corner)));
            self.append_from(base, [0, 1, 2, 0, 2, 3]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fx_t6::runtime;

    fn cloud(flags: u32) -> runtime::VisualParticleCloud {
        runtime::VisualParticleCloud {
            turn: Quat::IDENTITY.to_array(),
            spot: [1.0, 2.0, 3.0],
            layout_size_ratio: 2.0,
            basis_or_speed: [0.0; 3],
            color: 0xFF80_4020,
            start_extent: 4.0,
            second_extent: 6.0,
            flags,
            scale: 1.0,
        }
    }

    #[test]
    fn a_quad_adds_four_corners_and_its_indices() {
        let mut mesh = T6FxMesh::default();
        mesh.push_quad(Transform::default(), [255, 0, 0, 255], runtime::BillboardSheetCell::FULL);
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.indices.len(), runtime::BILLBOARD_TRIANGLE_SLOTS.len());
        assert_eq!(mesh.vertices[0].color, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(mesh.vertices[0].normal, [0.0, 0.0, 1.0]);
        // A second quad's indices are offset past the first's vertices.
        mesh.push_quad(Transform::default(), [0; 4], runtime::BillboardSheetCell::FULL);
        assert!(mesh.indices[runtime::BILLBOARD_TRIANGLE_SLOTS.len()..].iter().all(|&i| i >= 4));
    }

    #[test]
    fn a_beam_makes_one_draw_unless_it_is_empty() {
        let mut mesh = T6FxMesh::default();
        mesh.push_beam(&[], &[], 9);
        assert!(mesh.draws.is_empty());
        let vert = fx_t6::RayCorner {
            spot: [1.0, 2.0, 3.0],
            color: [255, 128, 0, 255],
            sheet_uv: [0.5, 0.25],
        };
        mesh.push_beam(&[vert, vert, vert], &[0, 1, 2], 9);
        assert_eq!(mesh.draws, vec![(0, 3, 9)]);
        assert_eq!(mesh.vertices[1].position, [1.0, 2.0, 3.0]);
        assert_eq!(mesh.vertices[1].first_texcoord, [0.5, 0.25]);
    }

    #[test]
    fn clouds_share_one_box_per_frame() {
        let mut mesh = T6FxMesh::default();
        mesh.push_cloud(&cloud(0), None, 5);
        let after_first = (mesh.vertices.len(), mesh.indices.len());
        mesh.push_cloud(&cloud(0), Some(10), 6);
        assert_eq!((mesh.vertices.len(), mesh.indices.len()), after_first);
        assert_eq!(mesh.clouds[0].first_index, mesh.clouds[1].first_index);
        assert_eq!(mesh.clouds[0].index_count, runtime::SWARM_STAMP_SPECKS as u32 * 6);
        assert_eq!(mesh.clouds[1].index_count, 60);
    }

    #[test]
    fn a_cloud_never_draws_more_cells_than_the_box_has() {
        let mut mesh = T6FxMesh::default();
        mesh.push_cloud(&cloud(0), Some(u32::MAX), 1);
        assert_eq!(mesh.clouds[0].index_count, runtime::SWARM_STAMP_SPECKS as u32 * 6);
    }

    #[test]
    fn the_flags_halve_the_cells_a_cloud_keeps() {
        let mut mesh = T6FxMesh::default();
        mesh.push_cloud(&cloud(1), None, 1);
        assert_eq!(mesh.clouds[0].index_count, runtime::SWARM_STAMP_SPECKS as u32 / 2 * 6);
    }

    #[test]
    fn the_box_is_nearest_the_centre_first() {
        let radii: Vec<f32> = cloud_box()
            .iter()
            .map(|&cell| runtime::particle_cloud_cell_radius_sq(cell))
            .collect();
        assert_eq!(radii.len(), runtime::SWARM_STAMP_SPECKS);
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
