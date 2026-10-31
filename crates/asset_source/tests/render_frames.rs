//! Draws the Gravity Gun's view model through each of its sequences into
//! PNGs (`BO2MP_SOURCE_SHOTS`, default target/gravgun-shots), so its
//! skinning and animation decoding can be judged by eye. Needs Half-Life 2
//! or Garry's Mod installed; run with `--ignored`. `render_standin` draws
//! the Minecraft stand-in the same way (no Valve files needed).

use asset_source::{Source, mdl, vtf};
use glam::{Mat4, Vec2, Vec3, Vec4};

const W: usize = 640;
const H: usize = 480;

struct Canvas {
    rgb: Vec<[u8; 3]>,
    depth: Vec<f32>,
}

fn sample(tex: &vtf::Rgba, uv: Vec2) -> [f32; 4] {
    let x = ((uv.x.rem_euclid(1.0)) * tex.width as f32) as usize % tex.width as usize;
    let y = ((uv.y.rem_euclid(1.0)) * tex.height as f32) as usize % tex.height as usize;
    let p = &tex.pixels[(y * tex.width as usize + x) * 4..][..4];
    [p[0] as f32, p[1] as f32, p[2] as f32, p[3] as f32]
}

/// `glow`: the texture's alpha is its self-lit mask (the stand-in's).
fn draw(model: &mdl::Model, textures: &[Option<vtf::Rgba>], palette: &[Mat4], proj: Mat4, canvas: &mut Canvas, glow: bool) {
    let light = Vec3::new(-0.4, 0.5, 0.8).normalize();
    for mesh in &model.meshes {
        let tex = textures.get(mesh.texture).and_then(Option::as_ref);
        let vertex_points: Vec<(Vec4, Vec3, Vec2)> = mesh
            .vertices
            .iter()
            .map(|v| {
                let mut p = Vec3::ZERO;
                let mut n = Vec3::ZERO;
                for k in 0..3 {
                    if v.blend[k] > 0.0 {
                        let m = palette[v.bones[k] as usize];
                        p += m.transform_point3(v.pos) * v.blend[k];
                        n += m.transform_vector3(v.normal) * v.blend[k];
                    }
                }
                // Source view: x forward, y left, z up -> right, up, back.
                let view = Vec3::new(-p.y, p.z, -p.x);
                let nv = Vec3::new(-n.y, n.z, -n.x).normalize_or_zero();
                (proj * view.extend(1.0), nv, v.uv)
            })
            .collect();
        for tri in mesh.tri_list.chunks_exact(3) {
            let [a, b, c] = [tri[0] as usize, tri[1] as usize, tri[2] as usize].map(|i| vertex_points[i]);
            if a.0.w <= 0.01 || b.0.w <= 0.01 || c.0.w <= 0.01 {
                continue;
            }
            let s = |v: Vec4| Vec3::new((v.x / v.w * 0.5 + 0.5) * W as f32, (0.5 - v.y / v.w * 0.5) * H as f32, v.w);
            let (pa, pb, pc) = (s(a.0), s(b.0), s(c.0));
            let area = (pb.x - pa.x) * (pc.y - pa.y) - (pb.y - pa.y) * (pc.x - pa.x);
            if area.abs() < 1e-6 {
                continue;
            }
            let (x0, x1) = (pa.x.min(pb.x).min(pc.x).max(0.0) as usize, (pa.x.max(pb.x).max(pc.x).ceil() as usize).min(W - 1));
            let (y0, y1) = (pa.y.min(pb.y).min(pc.y).max(0.0) as usize, (pa.y.max(pb.y).max(pc.y).ceil() as usize).min(H - 1));
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let w0 = ((pb.x - px) * (pc.y - py) - (pb.y - py) * (pc.x - px)) / area;
                    let w1 = ((pc.x - px) * (pa.y - py) - (pc.y - py) * (pa.x - px)) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    // Perspective-correct by 1/w.
                    let iw = w0 / pa.z + w1 / pb.z + w2 / pc.z;
                    let depth = 1.0 / iw;
                    let i = y * W + x;
                    if depth >= canvas.depth[i] {
                        continue;
                    }
                    let uv = (a.2 * (w0 / pa.z) + b.2 * (w1 / pb.z) + c.2 * (w2 / pc.z)) / iw;
                    let n = (a.1 * w0 + b.1 * w1 + c.1 * w2).normalize_or_zero();
                    let t = tex.map_or([200.0, 200.0, 200.0, 255.0], |t| sample(t, uv));
                    let lit = 0.45 + 0.55 * n.dot(light).abs();
                    let lit = if glow { lit + (1.0 - lit) * t[3] / 255.0 } else { lit };
                    canvas.depth[i] = depth;
                    canvas.rgb[i] = [(t[0] * lit) as u8, (t[1] * lit) as u8, (t[2] * lit) as u8];
                }
            }
        }
    }
}

fn save(canvas: &Canvas, path: &std::path::Path) {
    let file = std::fs::File::create(path).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), W as u32, H as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().unwrap();
    let bytes: Vec<u8> = canvas.rgb.iter().flatten().copied().collect();
    w.write_image_data(&bytes).unwrap();
}

#[test]
#[ignore]
fn render_frames() {
    let out = std::path::PathBuf::from(std::env::var("BO2MP_SOURCE_SHOTS").unwrap_or("target/gravgun-shots".into()));
    std::fs::create_dir_all(&out).unwrap();
    let source = Source::find(std::path::Path::new("."));
    for name in ["v_physcannon", "v_superphyscannon"] {
        let read = |ext: &str| source.read(&format!("models/weapons/{name}.{ext}")).unwrap();
        let model = mdl::read(&read("mdl"), &read("vvd"), &read("dx90.vtx")).unwrap();
        let textures: Vec<Option<vtf::Rgba>> = model
            .textures
            .iter()
            .map(|t| {
                t.folders.iter().find_map(|f| source.read(&format!("materials/{f}{}.vtf", t.name)).and_then(|b| vtf::decode(&b)))
            })
            .collect();
        eprintln!("{name}: textures {:?}", textures.iter().map(|t| t.as_ref().map(|t| (t.width, t.height))).collect::<Vec<_>>());
        // Source's viewmodel_fov 54 is horizontal at 4:3.
        let proj = Mat4::perspective_rh_gl(2.0 * ((54f32 / 2.0).to_radians().tan() * 0.75).atan(), W as f32 / H as f32, 1.0, 1000.0);
        let active = model.pose_param("active");
        for (si, seq) in model.sequences.iter().enumerate() {
            let len = model.duration(si).max(0.01);
            for k in 0..4 {
                let t = len * k as f32 / 3.0;
                let mut params = vec![0.0; model.pose_params.len()];
                if let Some(a) = active {
                    params[a] = if seq.name == "prong_blend" || seq.name == "hold_idle" { k as f32 / 3.0 } else { 0.0 };
                }
                let pose = model.pose(si, t, &params, t);
                let skin = model.skin(&pose);
                let mut canvas = Canvas { rgb: vec![[40, 44, 52]; W * H], depth: vec![f32::INFINITY; W * H] };
                draw(&model, &textures, &skin, proj, &mut canvas, false);
                save(&canvas, &out.join(format!("{name}_{}_{k}.png", seq.name)));
            }
            if seq.name == "idle" {
                // Claws shut and open on the same frame, for the layer check.
                for (tag, open) in [("shut", 0.0), ("open", 1.0)] {
                    let mut params = vec![0.0; model.pose_params.len()];
                    if let Some(a) = active {
                        let (lo, hi) = (model.pose_params[a].start, model.pose_params[a].end);
                        params[a] = lo + (hi - lo) * open;
                    }
                    let skin = model.skin(&model.pose(si, 0.0, &params, 0.0));
                    let mut canvas = Canvas { rgb: vec![[40, 44, 52]; W * H], depth: vec![f32::INFINITY; W * H] };
                    draw(&model, &textures, &skin, proj, &mut canvas, false);
                    save(&canvas, &out.join(format!("{name}_claws_{tag}.png")));
                }
            }
            eprintln!("{name} {} flags {:#x} layers {:?} fps {} frames {} len {len:.2}s act {}", seq.name, seq.studio_bits, seq.layers.iter().map(|l| (l.sequence, l.pose, l.layer_bits, l.start, l.end)).collect::<Vec<_>>(), model.animations[seq.anims[0]].fps, model.animations[seq.anims[0]].frames.len(), seq.activity);
        }
    }
}

#[test]
#[ignore]
fn render_standin() {
    let out = std::path::PathBuf::from(std::env::var("BO2MP_SOURCE_SHOTS").unwrap_or("target/gravgun-shots".into()));
    std::fs::create_dir_all(&out).unwrap();
    let proj = Mat4::perspective_rh_gl(2.0 * ((54f32 / 2.0).to_radians().tan() * 0.75).atan(), W as f32 / H as f32, 1.0, 1000.0);
    for upgraded in [false, true] {
        let model = asset_source::standin::model(upgraded);
        let textures: Vec<Option<vtf::Rgba>> = model.textures.iter().map(|t| asset_source::standin::texture(&t.name)).collect();
        assert!(textures.iter().all(Option::is_some));
        // Turned on the spot in front of the eye, shut and open.
        let idle = model.sequence("idle").unwrap();
        let centre = Vec3::new(45.0, -10.0, -12.5);
        for (view, yaw) in [("side", 90f32), ("quarter", 140.0), ("front", 180.0)] {
            for (tag, open) in [("shut", 0.0), ("open", 1.0)] {
                let turn = Mat4::from_translation(Vec3::new(70.0, 0.0, 0.0))
                    * Mat4::from_rotation_z(yaw.to_radians())
                    * Mat4::from_translation(-centre);
                let skin: Vec<Mat4> = model.skin(&model.pose(idle, 0.0, &[open], 0.0)).iter().map(|m| turn * *m).collect();
                let mut canvas = Canvas { rgb: vec![[40, 44, 52]; W * H], depth: vec![f32::INFINITY; W * H] };
                draw(&model, &textures, &skin, proj, &mut canvas, true);
                save(&canvas, &out.join(format!("{}_{view}_{tag}.png", model.name)));
            }
        }
        for (si, seq) in model.sequences.iter().enumerate() {
            let len = model.duration(si).max(0.01);
            for k in 0..4 {
                let t = len * k as f32 / 3.0;
                for (tag, open) in [("shut", 0.0), ("open", 1.0)] {
                    if open > 0.0 && k > 0 {
                        continue;
                    }
                    let skin = model.skin(&model.pose(si, t, &[open], t));
                    let mut canvas = Canvas { rgb: vec![[40, 44, 52]; W * H], depth: vec![f32::INFINITY; W * H] };
                    draw(&model, &textures, &skin, proj, &mut canvas, true);
                    let (model_name, seq_name) = (&model.name, &seq.name);
                    save(&canvas, &out.join(format!("{model_name}_{seq_name}_{k}_{tag}.png")));
                }
            }
        }
    }
}
