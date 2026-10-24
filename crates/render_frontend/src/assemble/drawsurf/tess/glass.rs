use bevy::prelude::{Res, ResMut, Resource};

use crate::prepare::scene::world::WorldScene;

/// codmp: what each Black Ops II window's client plays as its state changes
/// this frame (`GlassClient::SetState`): its def's crack sound as it cracks;
/// its shatter sound (auto-shatter sound when it broke by itself, no hit
/// direction) and its shatter effect along its edges as it shatters. Filled by
/// `apply_glass_host`, played by the effects pass right after it.
#[derive(Resource, Default, Debug)]
pub struct T6GlassCues(pub Vec<T6GlassCue>);

#[derive(Clone, Debug)]
pub struct T6GlassCue {
    pub pane: u32,
    pub shattered: bool,
    /// Direction it was broken in; zero when it broke by itself.
    pub dir: [f32; 3],
    pub client: asset_world::T6GlassClientDef,
}

/// The windows' client state and the clock/epoch they were last applied at.
#[derive(Clone, Debug, Default, Resource)]
pub struct GlassTable {
    pub map_round_epoch: u32,
    pub generation: u32,
    pub last_msec: i32,
    /// bo2mp: Black Ops II's windows as this client holds them; `None`
    /// until the first glass message, which applies as the gamestate.
    pub t6: Option<glass_t6::GlassesClient>,
}

impl GlassTable {
    /// Forgets the windows and the clock; the epoch stays, `generation` moves
    /// so readers see the reset.
    pub fn reset(&mut self) {
        *self = Self {
            map_round_epoch: self.map_round_epoch,
            generation: self.generation.wrapping_add(1),
            ..Self::default()
        };
    }
}

/// The snapshot whose world objects are on screen: the presented one, or the
/// next one adopted.
fn on_screen_snapshot<'a>(
    presented: Option<&'a net::PresentedSnapshot>,
    adopted: Option<&'a net::LastAdoptedSnapshot>,
) -> Option<&'a sim::Snapshot> {
    presented
        .and_then(net::PresentedSnapshot::snapshot)
        .or_else(|| adopted.and_then(net::LastAdoptedSnapshot::next))
}

/// The clock the windows are applied against: the snapshot's own time while
/// the local player watches an archived frame, the effects clock otherwise.
fn presentation_clock(snap: Option<&sim::Snapshot>, local: Option<sim::ClientId>, fx_msec: i32) -> i32 {
    let archived = match (snap, local) {
        (Some(snap), Some(local)) => snap
            .players
            .iter()
            .any(|(id, ps)| *id == local && !ps.is_live_frame()),
        _ => false,
    };
    if archived {
        snap.map_or(0, |s| s.meta.world_objects.as_of_ms)
    } else {
        fx_msec
    }
}

/// A new map round: the epoch moved from one that was already set.
fn new_round(seen: u32, incoming: u32) -> bool {
    seen != 0 && seen != incoming
}

pub(crate) fn apply_glass_host(
    presented: Option<Res<net::PresentedSnapshot>>,
    adopted: Option<Res<net::LastAdoptedSnapshot>>,
    local: Option<Res<net::LocalPresentClient>>,
    world: Option<Res<WorldScene>>,
    mut table: ResMut<GlassTable>,
    mut fx_host: Option<ResMut<render_fx::HostFxSystem>>,
    mut cues: ResMut<T6GlassCues>,
) {
    cues.0.clear();
    let Some(world) = world else {
        return;
    };
    let Some(glass) = world.t6_glass.as_ref() else {
        return;
    };
    let snap = on_screen_snapshot(presented.as_deref(), adopted.as_deref());
    let objects = snap.map(|s| &s.meta.world_objects);
    let epoch = objects.map_or(0, |o| o.map_round_epoch);
    // bo2mp: Black Ops II's windows come in BO2's own glass message.
    let rows = objects.map_or(&[][..], |o| o.t6_glass.as_slice());
    let now = presentation_clock(
        snap,
        local.map(|c| c.0),
        fx_host.as_ref().map_or(0, |h| h.0.msec_now),
    );
    let live = !glass.panes.is_empty() && rows.len() == glass.panes.len();
    if new_round(table.map_round_epoch, epoch)
        || (live && (now < table.last_msec || t6_glass_went_back(table.t6.as_ref(), rows)))
    {
        table.reset();
        if let Some(host) = fx_host.as_mut() {
            host.0.glass = fx::FxGlassSystemHost::default();
        }
    }
    // bo2mp: Black Ops II's windows break into their own shards (`fx::T6Glass`);
    // a glass reset clears them, so they are set up again after one.
    if let Some(host) = fx_host.as_mut()
        && host.0.glass.t6.panes.len() != glass.panes.len()
    {
        let floor = world.world_bounds.map_or(-1.0e6, |b| b[2] - b[5] + 5.0);
        host.0.glass.t6.set_panes(t6_shard_panes(&glass.panes), floor);
        host.0.glass.shattered = vec![false; glass.panes.len()];
    }
    table.last_msec = now;
    table.map_round_epoch = epoch;
    if !live {
        return;
    }
    // bo2mp: each state applied as BO2's client does (`CG_Glass_SetState`);
    // the first message, or the first after a rebuild, is the gamestate:
    // no sounds or effects, and its loose shards go at once.
    let gamestate = table.t6.is_none();
    let client = table
        .t6
        .get_or_insert_with(|| glass_t6::GlassesClient::new(rows.len()));
    for u in rows {
        let (pos, dir) = u.hit.unwrap_or_default();
        client.set_state(u32::from(u.pane), u.state, pos, dir, gamestate);
    }
    // codmp: a Black Ops II window's own def plays as it changes.
    let mut play = |pane: u32, shattered: bool, dir: [f32; 3]| {
        if let Some(draw) = glass.panes.get(pane as usize) {
            cues.0.push(T6GlassCue { pane, shattered, dir, client: draw.client.clone() });
        }
    };
    for cue in client.take_cues() {
        match cue {
            glass_t6::Cue::RemoveShards { pane } => {
                if let Some(host) = fx_host.as_mut() {
                    host.0.glass.t6.clear_pane(pane);
                }
            }
            glass_t6::Cue::CrackSound { pane } => play(pane, false, [0.0; 3]),
            glass_t6::Cue::Shatter { pane, hit_pos, hit_dir, gamestate } => {
                if let Some(host) = fx_host.as_mut() {
                    host.0.glass.t6.shatter(pane, hit_pos, hit_dir, None, !gamestate);
                    if let Some(gone) = host.0.glass.shattered.get_mut(pane as usize) {
                        *gone = true;
                    }
                    host.0.marks.hide_glass_marks(pane as u16);
                }
                client.shatter_started(pane);
                if !gamestate {
                    play(pane, true, hit_dir);
                }
            }
            _ => {}
        }
    }
}

/// bo2mp: a Black Ops II window the client holds as further broken than the
/// snapshot says (a rewind): the windows are set up again from it.
fn t6_glass_went_back(client: Option<&glass_t6::GlassesClient>, rows: &[glass_t6::PaneUpdate]) -> bool {
    let Some(client) = client else {
        return false;
    };
    rows.iter()
        .any(|u| client.state(u32::from(u.pane)).is_some_and(|held| held as u8 > u.state as u8))
}

/// bo2mp: each Black Ops II window's shard source, from its draw data.
fn t6_shard_panes(panes: &[asset_world::T6GlassPaneDraw]) -> Vec<fx::T6PaneShape> {
    panes
        .iter()
        .map(|p| fx::T6PaneShape {
            outline: p.client.outline.clone(),
            axis: p.client.axis,
            outline_origin: p.client.outline_origin,
            thickness: p.shards.thickness,
            uv_scale: p.shards.uv_scale,
            is_planar: p.shards.is_planar,
            min_shard_size: p.shards.min_shard_size,
            max_shard_size: p.shards.max_shard_size,
            shard_life_probability: p.shards.shard_life_probability,
            max_shards: p.shards.max_shards,
            pool_shards: p.shards.pool_shards,
            pool_physics: p.shards.pool_physics,
        })
        .collect()
}

/// codmp: Black Ops II's windows, drawn as BO2 draws them
/// (`GlassesClient::Update` -> `R_AddGlassBrushToScene`): each window's own
/// surfaces where it stands, in its def's pristine material while whole and
/// its cracked one once damaged (`GlassClient::SetBrushMaterial`), and not
/// at all once shattered. They go out with the T6 marks (T6 draw codes; the
/// legacy glass tess wants an legacy material ordinal these have none of). Each
/// surface is lit by its own lightmap at its corners, as a mark on a wall
/// is: its vertex colours hold that light (colour times sqrt(light / 32)).
#[derive(Clone, Debug, Default, Resource)]
pub struct T6GlassMesh {
    pub vertices: Vec<render_frame::SmodelVertex>,
    pub indices: Vec<u32>,
    /// As `T6MarkMesh::draws`: index start and count, material, a point on
    /// it, its lightmap's primary light and visibility (`None`: light grid).
    pub draws: Vec<(u32, u32, u32, [f32; 3], Option<(u8, f32)>)>,
    /// What the lit vertices were built from (lightmap generation, panes).
    built: Option<(u64, usize, usize)>,
    /// Per surface: its pane, index start and count, a point on it, its light.
    surfaces: Vec<(usize, u32, u32, [f32; 3], Option<(u8, f32)>)>,
    /// The (pristine, cracked) counts last logged: the runtime catalog
    /// gets the map's materials after the panes are built.
    logged: Option<(usize, usize)>,
    /// bo2mp: the panes' own vertices and indices; the shards' follow and
    /// are rebuilt every frame.
    static_len: (usize, usize),
    /// Whether shards were drawn, and some lacked their material, as last logged.
    shards_logged: (usize, usize, usize),
    /// Shard groups drawn at high LOD, by (material, cell), with the shards'
    /// revision they were built at: high to low waits for a rebuild.
    shard_high: std::collections::HashMap<(u32, [i32; 3]), u64>,
    /// The shard draw's unknowns, logged once.
    shard_facts_logged: bool,
}

pub(crate) fn draw_t6_glass_panes(
    world: Option<Res<WorldScene>>,
    runtime: Option<Res<crate::assemble::drawsurf::MaterialGeneration>>,
    table: Res<GlassTable>,
    fx_host: Option<Res<render_fx::HostFxSystem>>,
    view: Option<Res<crate::prepare::scene::view_parms::PreparedSceneView>>,
    mut mesh: ResMut<T6GlassMesh>,
) {
    let mesh = &mut *mesh;
    mesh.draws.clear();
    let panes = world
        .as_deref()
        .and_then(|s| s.t6_glass.as_ref())
        .map_or(&[][..], |g| g.panes.as_slice());
    if panes.is_empty() {
        if mesh.built.take().is_some() {
            mesh.vertices.clear();
            mesh.indices.clear();
            mesh.surfaces.clear();
        }
        return;
    }
    let key = (
        lighting_t6::t6_lightmap_generation(),
        panes.as_ptr() as usize,
        panes.len(),
    );
    if mesh.built != Some(key) {
        mesh.built = Some(key);
        mesh.logged = None;
        build_t6_glass_mesh(panes, mesh);
        mesh.static_len = (mesh.vertices.len(), mesh.indices.len());
    }
    mesh.vertices.truncate(mesh.static_len.0);
    mesh.indices.truncate(mesh.static_len.1);
    let Some(runtime) = runtime else {
        return;
    };
    let material = |name: &str| {
        runtime
            .catalog
            .material_for_name(asset_game::material_bind_name(name))
            .filter(|m| m.t6_draw.is_some())
            .map(|m| u32::from(m.asset_id.0))
    };
    // Applied states: 0 whole, 1 damaged, 2 shattered, 3 gone; BO2's own
    // client states when the server sends them.
    let wears: Vec<Option<u32>> = panes
        .iter()
        .enumerate()
        .map(|(i, pane)| {
            let applied = match table.t6.as_ref() {
                Some(c) if !c.draws_brush(i as u32) => 2,
                Some(c) => u8::from(c.state(i as u32).is_some_and(|s| s != glass_t6::PaneState::Pristine)),
                None => 0,
            };
            match applied {
                0 => material(&pane.pristine),
                1 => material(&pane.cracked).or_else(|| material(&pane.pristine)),
                _ => None,
            }
        })
        .collect();
    let whole = panes.iter().filter(|p| material(&p.pristine).is_some()).count();
    let cracked = panes.iter().filter(|p| material(&p.cracked).is_some()).count();
    if mesh.logged != Some((whole, cracked)) {
        mesh.logged = Some((whole, cracked));
        diag::info!(
            World,
            "codmp t6 glass panes: {} panes, {} surfaces ({} lit by their lightmap), {} triangles; pristine material found for {}, cracked for {}",
            panes.len(),
            mesh.surfaces.len(),
            mesh.surfaces.iter().filter(|s| s.4.is_some()).count(),
            mesh.indices.len() / 3,
            whole,
            cracked
        );
    }
    for &(pane, start, count, at, lit) in &mesh.surfaces {
        if let Some(Some(material)) = wears.get(pane) {
            mesh.draws.push((start, count, *material, at, lit));
        }
    }
    if let Some(t6) = fx_host.as_deref().map(|h| &h.0.glass.t6) {
        let shard_materials: Vec<Option<u32>> = panes.iter().map(|p| material(&p.shards.material)).collect();
        let view = view.as_deref().filter(|v| v.ready);
        draw_t6_glass_shards(t6, &shard_materials, view, mesh);
    }
}

/// Whether a shard group `dist` from the eye draws at high LOD (side walls):
/// below `T6_GLASS_LOW_LOD_DIST`, or still high since its last build (BO2
/// rebuilds a group going low to high, but not high to low until the group
/// is invalidated). `built_high` is the shards' revision it was built high
/// at. TODO(fact): what invalidates one BO2 group is unknown; here any
/// change to the shards counts.
fn t6_shard_group_high(dist: f32, built_high: Option<u64>, revision: u64) -> bool {
    dist < glass_t6::T6_GLASS_LOW_LOD_DIST || built_high == Some(revision)
}

/// bo2mp: every shard drawn as Black Ops II draws them
/// (`ShardGroup::GenerateVerts`): the shards of one def in one grid cell
/// (32 units) make one surface in the def's shard material, white, lit at
/// the centre of their box (`R_AllocModelLighting` at the group origin; the
/// box leaves out shards flying with no collision when there are others).
/// Each shard has its front and back faces, and its side walls while the
/// group is at high LOD (`t6_shard_group_high`). A group wholly outside the
/// view is not drawn (`cullGlassShards`, on by default).
fn draw_t6_glass_shards(
    t6: &fx::T6Glass,
    materials: &[Option<u32>],
    view: Option<&crate::prepare::scene::view_parms::PreparedSceneView>,
    mesh: &mut T6GlassMesh,
) {
    // TODO(fact): BO2 sizes the cells from an extent (x/y max(32, extent *
    // 1.1 / 2048), z max(32, extent * 1.1 / 1024)); which extent is unknown,
    // so the 32 minimum stands.
    const CELL: f32 = 32.0;
    let mut groups: std::collections::BTreeMap<(u32, [i32; 3]), Vec<usize>> = Default::default();
    let mut missing = 0;
    for (i, shard) in t6.shards.iter().enumerate() {
        let Some(Some(material)) = materials.get(shard.pane as usize) else {
            missing += 1;
            continue;
        };
        let cell = shard.origin.map(|v| (v / CELL).floor() as i32);
        groups.entry((*material, cell)).or_default().push(i);
    }
    let eye = view.map(|v| v.eye.to_array());
    let planes = view.map_or(&[][..], |v| v.frustum_planes.as_slice());
    mesh.shard_high.retain(|key, _| groups.contains_key(key));
    let (mut high_n, mut culled) = (0, 0);
    let (mut verts, mut indices) = (Vec::new(), Vec::new());
    for (key, members) in &groups {
        let still = members.iter().any(|&i| t6.shards[i].motion != fx::T6ShardMotion::Simple);
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for &i in members {
            let shard = &t6.shards[i];
            if !still || shard.motion != fx::T6ShardMotion::Simple {
                for k in 0..3 {
                    lo[k] = lo[k].min(shard.origin[k]);
                    hi[k] = hi[k].max(shard.origin[k]);
                }
            }
        }
        let at = [0, 1, 2].map(|k| (lo[k] + hi[k]) * 0.5);
        // TODO(fact): BO2 scales this distance by the view's FOV; how is
        // unknown, so it is the plain distance.
        let dist = eye.map_or(0.0, |e| {
            ((at[0] - e[0]).powi(2) + (at[1] - e[1]).powi(2) + (at[2] - e[2]).powi(2)).sqrt()
        });
        let high = t6_shard_group_high(dist, mesh.shard_high.get(key).copied(), t6.revision);
        if high {
            mesh.shard_high.insert(*key, t6.revision);
            high_n += 1;
        } else {
            mesh.shard_high.remove(key);
        }
        let (v0, start) = (mesh.vertices.len(), mesh.indices.len() as u32);
        for &i in members {
            verts.clear();
            indices.clear();
            t6.shard_mesh(i, high, &mut verts, &mut indices);
            let base = mesh.vertices.len() as u32;
            // TODO(fact): BO2 gives each vertex a tangent (axis0) and a
            // binormal sign (front s, back and walls -s); the model vertex
            // this draws through has neither.
            mesh.vertices.extend(verts.iter().map(|v| render_frame::SmodelVertex {
                position: v.position,
                normal: v.normal,
                color: [1.0; 4],
                uv0: v.uv,
            }));
            mesh.indices.extend(indices.iter().map(|&k| base + k));
        }
        let (mut vlo, mut vhi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for v in &mesh.vertices[v0..] {
            for k in 0..3 {
                vlo[k] = vlo[k].min(v.position[k]);
                vhi[k] = vhi[k].max(v.position[k]);
            }
        }
        if mesh.vertices.len() > v0 && dpvs_t6::box_culled(planes, vlo, vhi) {
            mesh.vertices.truncate(v0);
            mesh.indices.truncate(start as usize);
            culled += 1;
            continue;
        }
        let count = mesh.indices.len() as u32 - start;
        if count > 0 {
            mesh.draws.push((start, count, key.0, at, None));
        }
    }
    if !groups.is_empty() && !mesh.shard_facts_logged {
        mesh.shard_facts_logged = true;
        diag::info!(
            World,
            "bo2mp t6 glass shards: TODO(fact) low LOD uses the plain distance (BO2's FOV scaling unknown), cells stay 32 units (BO2's extent unknown), no tangent or binormal sign (model vertex has none)"
        );
    }
    // Logged as shards first appear or all go, or their material turns up.
    let key = (usize::from(!t6.shards.is_empty()), usize::from(missing > 0), 0);
    if key != mesh.shards_logged {
        mesh.shards_logged = key;
        diag::info!(
            World,
            "bo2mp t6 glass shards drawn: {} shards in {} groups ({} high LOD, {} culled, {} triangles), {} with no shard material",
            t6.shards.len(),
            groups.len(),
            high_n,
            culled,
            (mesh.indices.len() - mesh.static_len.1) / 3,
            missing
        );
    }
}

fn build_t6_glass_mesh(panes: &[asset_world::T6GlassPaneDraw], mesh: &mut T6GlassMesh) {
    mesh.vertices.clear();
    mesh.indices.clear();
    mesh.surfaces.clear();
    for (pane_i, pane) in panes.iter().enumerate() {
        let base = mesh.vertices.len() as u32;
        mesh.vertices.extend(pane.vertices.iter().map(|v| render_frame::SmodelVertex {
            position: v.position,
            normal: v.normal,
            color: v.color,
            uv0: v.uv,
        }));
        for surf in &pane.surfaces {
            let Some(indices) = pane
                .indices
                .get(surf.start as usize..(surf.start + surf.count) as usize)
            else {
                continue;
            };
            let start = mesh.indices.len() as u32;
            mesh.indices.extend(indices.iter().map(|&i| base + i));
            let at = indices
                .first()
                .and_then(|&i| pane.vertices.get(i as usize))
                .map_or([0.0; 3], |v| v.position);
            let lit = light_t6_glass_surface(pane, surf, indices, &mut mesh.vertices[base as usize..]);
            mesh.surfaces.push((pane_i, start, surf.count, at, lit));
        }
    }
}

/// A window surface's corners lit by its lightmap, as a mark on a
/// lightmapped wall is: each corner's colour becomes its colour times
/// sqrt(light / 32); the result is the surface's primary light and its mean
/// baked visibility (times the mean colour squared), or `None` (some corner
/// has no lightmap light) to keep the light grid.
fn light_t6_glass_surface(
    pane: &asset_world::T6GlassPaneDraw,
    surf: &asset_world::T6GlassSurface,
    indices: &[u32],
    out: &mut [render_frame::SmodelVertex],
) -> Option<(u8, f32)> {
    let mut seen: Vec<u32> = indices.to_vec();
    seen.sort_unstable();
    seen.dedup();
    let lights = seen
        .iter()
        .map(|&i| {
            let v = pane.vertices.get(i as usize)?;
            lighting_t6::t6_lightmap_light(surf.lightmap, v.lightmap_uv, v.normal)
        })
        .collect::<Option<Vec<_>>>()?;
    let mut vis_sum = 0.0;
    let mut tint_sq = 0.0;
    for (&i, (light, vis)) in seen.iter().zip(&lights) {
        let Some(v) = out.get_mut(i as usize) else {
            continue;
        };
        tint_sq += (v.color[0] * v.color[0] + v.color[1] * v.color[1] + v.color[2] * v.color[2]) / 3.0;
        for c in 0..3 {
            v.color[c] *= (light[c] / 32.0).clamp(0.0, 1.0).sqrt();
        }
        vis_sum += vis;
    }
    let n = seen.len().max(1) as f32;
    Some((surf.primary_light, vis_sum / n * (tint_sq / n)))
}

#[cfg(test)]
mod tests {
    use super::t6_shard_group_high;
    use glass_t6::T6_GLASS_LOW_LOD_DIST as LOD;

    #[test]
    fn shard_groups_go_low_only_once_rebuilt() {
        // Near: high. Far and never built high: low.
        assert!(t6_shard_group_high(LOD - 1.0, None, 7));
        assert!(!t6_shard_group_high(LOD, None, 7));
        // Built high at this revision: stays high when it moves away.
        assert!(t6_shard_group_high(LOD + 500.0, Some(7), 7));
        // The shards changed since: it goes low.
        assert!(!t6_shard_group_high(LOD + 500.0, Some(7), 8));
    }
}
