//! bo2zm: text in Black Ops II's own HUD fonts (`assets::T6HudFonts`): each
//! letter a piece of the font sheet, placed as the font's letter_shape table says
//! (its box against the pen and the baseline, then the pen moves on by the
//! letter's advance).

use std::collections::HashMap;

use bevy::prelude::*;

use crate::ScreenBox;

/// The sheet as an image and each font by the name the HUD's Lua scripts
/// give it (Default, Big, Morris, ...).
#[derive(Resource)]
pub(crate) struct Bo2Fonts {
    pub sheet: Handle<Image>,
    /// The sheet's pixels as plain RGBA8 (for blurring covered text).
    cpu: Option<std::sync::Arc<Image>>,
    fonts: HashMap<String, Bo2Font>,
    /// bo2zm: the PlayStation face buttons (□ ○ △ ×), drawn here: BO2 on PC
    /// has only Xbox button pictures and its fonts have no such shapes.
    button_pictures: HashMap<char, Handle<Image>>,
}

/// The characters drawn as button icons: the PlayStation shapes and BO2's
/// Xbox button pictures (`frame::BO2_XBOX_GLYPHS`).
pub(crate) fn is_button_icon(c: char) -> bool {
    matches!(c, '□' | '○' | '△' | '×' | '\u{E100}'..='\u{E114}')
}

/// One PlayStation face button, 64 x 64: a dark disc with a light rim and
/// the shape in its own colour (square pink, circle red, triangle green,
/// cross blue), edges smoothed.
fn button_icon(c: char) -> Image {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{TextureDimension, TextureFormat};
    const N: usize = 64;
    let (rgb, kind) = match c {
        '□' => ([0.95, 0.55, 0.82], 0),
        '○' => ([1.0, 0.33, 0.36], 1),
        '△' => ([0.3, 0.86, 0.72], 2),
        _ => ([0.48, 0.62, 1.0], 3),
    };
    let mut px = vec![0u8; N * N * 4];
    let edge = |d: f32| (0.5 - d).clamp(0.0, 1.0);
    for y in 0..N {
        for x in 0..N {
            let (fx, fy) = (x as f32 + 0.5 - 32.0, y as f32 + 0.5 - 32.0);
            let r = (fx * fx + fy * fy).sqrt();
            // The disc and its rim.
            let disc = edge(r - 30.5);
            let rim = edge((r - 29.0).abs() - 1.5);
            // The shape's stroke (distance from its outline, 4.5 wide).
            let d = match kind {
                0 => (fx.abs().max(fy.abs()) - 13.0).abs(),
                1 => (r - 14.0).abs(),
                2 => {
                    // An upward triangle, centre slightly low.
                    let (qx, qy) = (fx, fy - 3.0);
                    let k = 3f32.sqrt();
                    let a = (k * qx.abs() - qy) * 0.5 - 8.0;
                    let b = qy - 8.0;
                    a.max(b).abs()
                }
                _ => {
                    let a = (fx - fy).abs() / 2f32.sqrt();
                    let b = (fx + fy).abs() / 2f32.sqrt();
                    let inside = fx.abs().max(fy.abs()) <= 13.0;
                    if inside { a.min(b) } else { 99.0 }
                }
            };
            let shape = edge(d - 2.25);
            let base = [0.08, 0.08, 0.09];
            let mut col = [0.0f32; 3];
            for i in 0..3 {
                let with_rim = base[i] + (0.85 - base[i]) * rim * 0.6;
                col[i] = with_rim + (rgb[i] - with_rim) * shape;
            }
            let o = (y * N + x) * 4;
            for i in 0..3 {
                px[o + i] = (255.0 * col[i].clamp(0.0, 1.0)).round() as u8;
            }
            px[o + 3] = (disc * 255.0).round() as u8;
        }
    }
    Image::new(
        crate::lui_scene::picture_size(N, N),
        TextureDimension::D2,
        px,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

struct Bo2Font {
    raster_rows: f32,
    /// Above and below the baseline, over every letter_shape's box.
    ascent: f32,
    descent: f32,
    glyphs: HashMap<char, Bo2Glyph>,
}

#[derive(Clone, Copy)]
struct Bo2Glyph {
    x0: f32,
    y0: f32,
    step_along_x: f32,
    w: f32,
    h: f32,
    rect: Rect,
}

impl Bo2Fonts {
    fn font(&self, name: &str) -> Option<&Bo2Font> {
        self.fonts.get(name).or_else(|| self.fonts.get("Default"))
    }

    /// Each font's pixel height, letter advances and space advance (the
    /// LUI scripts' text measure).
    pub fn advances(&self) -> HashMap<String, (f32, HashMap<char, f32>, f32)> {
        self.fonts
            .iter()
            .map(|(name, f)| {
                let space = f.glyphs.get(&' ').map_or(f.raster_rows * 0.3, |g| g.step_along_x);
                let mut adv: HashMap<char, f32> = f.glyphs.iter().map(|(c, g)| (*c, g.step_along_x)).collect();
                // Button pictures: a little wider than the font is tall.
                for c in self.button_pictures.keys() {
                    adv.insert(*c, f.raster_rows * 1.1);
                }
                (name.clone(), (f.raster_rows, adv, space))
            })
            .collect()
    }

    /// How wide `text` is in `font` at `px` (as `spawn_line` lays it out).
    pub fn line_width(&self, font: &str, words: &str, px: f32) -> f32 {
        let Some(f) = self.font(font) else {
            return 0.0;
        };
        let s = px / f.raster_rows.max(1.0);
        let space = f.glyphs.get(&' ').map_or(f.raster_rows * 0.3, |g| g.step_along_x);
        words.chars()
            .map(|c| match f.glyphs.get(&c).or_else(|| f.glyphs.get(&'?')) {
                _ if is_button_icon(c) => f.raster_rows * 1.1,
                Some(g) if c != ' ' => g.step_along_x,
                _ => space,
            })
            .sum::<f32>()
            * s
    }

    /// bo2mp: the letter_shape coverage of `lines` (text, left edge in units from the
    /// block's left) in `font` at `px_u` units tall, laid out as `spawn_line`
    /// does one line under the other, as a white RGBA8 picture whose alpha is
    /// the coverage: `w_u` x block height units plus `pad` units clear all
    /// round, `res` texels to the unit. Returns it with its size in texels
    /// and the block's height in units; the caller blurs it, as BO2's engine
    /// blurs the menu layer it has drawn the text into.
    pub fn coverage(&self, font: &str, lines: &[(String, f32)], px_u: f32, w_u: f32, pad: f32, res: f32) -> Option<(Vec<u8>, usize, usize, f32)> {
        let f = self.font(font)?;
        let sheet = self.cpu.as_ref()?;
        let data = sheet.data.as_ref()?;
        let (sw, sh) = (sheet.width() as usize, sheet.height() as usize);
        if data.len() < sw * sh * 4 {
            return None;
        }
        let s = px_u / f.raster_rows.max(1.0);
        let line_h = (f.ascent + f.descent) * s;
        let block_h = line_h * lines.len() as f32;
        let tw = (((w_u + 2.0 * pad) * res).ceil() as usize).max(1);
        let th = (((block_h + 2.0 * pad) * res).ceil() as usize).max(1);
        let mut px = vec![255u8; tw * th * 4];
        for p in px.chunks_exact_mut(4) {
            p[3] = 0;
        }
        let space = f.glyphs.get(&' ').map_or(f.raster_rows * 0.3, |g| g.step_along_x);
        let at = |x: f32, y: f32| -> f32 {
            let (xi, yi) = (x.floor() as isize, y.floor() as isize);
            if xi < 0 || yi < 0 || xi as usize >= sw || yi as usize >= sh {
                return 0.0;
            }
            f32::from(data[(yi as usize * sw + xi as usize) * 4 + 3]) / 255.0
        };
        for (i, (text, left)) in lines.iter().enumerate() {
            let mut pen = 0.0f32;
            for c in text.chars() {
                let g = match f.glyphs.get(&c).or_else(|| f.glyphs.get(&'?')) {
                    Some(g) if c != ' ' => g,
                    _ => {
                        pen += space;
                        continue;
                    }
                };
                // The letter_shape's box in units, then in texels of the block.
                let gx = left + (pen + g.x0) * s;
                let gy = i as f32 * line_h + (f.ascent + g.y0) * s;
                let (gw, gh) = ((g.w * s).max(1e-3), (g.h * s).max(1e-3));
                let tx0 = ((gx + pad) * res).floor().max(0.0) as usize;
                let ty0 = ((gy + pad) * res).floor().max(0.0) as usize;
                let tx1 = (((gx + gw + pad) * res).ceil() as usize).min(tw);
                let ty1 = (((gy + gh + pad) * res).ceil() as usize).min(th);
                for ty in ty0..ty1 {
                    for tx in tx0..tx1 {
                        // Each texel averages a 3 x 3 sampling of the sheet over its footprint.
                        let mut sum = 0.0f32;
                        for sy in 0..3 {
                            for sx in 0..3 {
                                let ux = (tx as f32 + (sx as f32 + 0.5) / 3.0) / res - pad;
                                let uy = (ty as f32 + (sy as f32 + 0.5) / 3.0) / res - pad;
                                let (fx, fy) = ((ux - gx) / gw, (uy - gy) / gh);
                                if !(0.0..1.0).contains(&fx) || !(0.0..1.0).contains(&fy) {
                                    continue;
                                }
                                sum += at(g.rect.min.x + fx * g.rect.width(), g.rect.min.y + fy * g.rect.height());
                            }
                        }
                        let a = (sum / 9.0 * 255.0).round() as u32;
                        let o = (ty * tw + tx) * 4 + 3;
                        px[o] = (u32::from(px[o]) + a).min(255) as u8;
                    }
                }
                pen += g.step_along_x;
            }
        }
        Some((px, tw, th, block_h))
    }

    /// bo2mp: one line's height in units at `px_u` (what `coverage` stacks).
    pub fn coverage_height(&self, font: &str, px_u: f32) -> f32 {
        self.font(font).map_or(0.0, |f| (f.ascent + f.descent) * px_u / f.raster_rows.max(1.0))
    }

    /// One line of coloured runs in `font`, `px` tall for the font's own
    /// pixel height, as a node of its size holding a letter_shape image per letter
    /// (a dark copy under each, offset by `shadow` pixels, when it is not
    /// zero). Returns the line's size.
    pub fn spawn_line(
        &self,
        holder: &mut ChildSpawnerCommands,
        font: &str,
        runs: &[(String, Color)],
        px: f32,
        shadow: f32,
    ) -> Vec2 {
        self.spawn_line_shadowed(holder, font, runs, px, shadow, 0.75)
    }

    /// bo2mp: `font`'s own pixel height (the em its glyphs were cut at).
    pub fn raster_rows(&self, font: &str) -> f32 {
        self.font(font).map_or(0.0, |f| f.raster_rows)
    }

    /// bo2mp: `spawn_line` with the shadow's alpha as a share of the text's
    /// (BO2's engine draws a hudelem's drop shadow at the text's own alpha).
    pub fn spawn_line_shadowed(
        &self,
        holder: &mut ChildSpawnerCommands,
        font: &str,
        runs: &[(String, Color)],
        px: f32,
        shadow: f32,
        shadow_alpha: f32,
    ) -> Vec2 {
        let Some(f) = self.font(font) else {
            return Vec2::ZERO;
        };
        let s = px / f.raster_rows.max(1.0);
        let space = f.glyphs.get(&' ').map_or(f.raster_rows * 0.3, |g| g.step_along_x);
        let mut placed: Vec<(Bo2Glyph, f32, Color)> = Vec::new();
        let mut icons = Vec::new();
        let mut pen = 0.0f32;
        for (words, tint) in runs {
            for c in words.chars() {
                if is_button_icon(c) && self.button_pictures.contains_key(&c) {
                    icons.push((c, pen));
                    pen += f.raster_rows * 1.1;
                    continue;
                }
                match f.glyphs.get(&c).or_else(|| f.glyphs.get(&'?')) {
                    Some(g) if c != ' ' => {
                        placed.push((*g, pen, *tint));
                        pen += g.step_along_x;
                    }
                    _ => pen += space,
                }
            }
        }
        let size = Vec2::new(pen * s, (f.ascent + f.descent) * s);
        let sheet = self.sheet.clone();
        holder
            .spawn(Node {
                width: Val::Px(size.x),
                height: Val::Px(size.y),
                ..default()
            })
            .with_children(|line| {
                let mut letter_shape = |g: &Bo2Glyph, pen: f32, tint: Color, off: f32| {
                    line.spawn((
                        ScreenBox {
                            left_px: (pen + g.x0) * s + off,
                            top_px: (f.ascent + g.y0) * s + off,
                            width_px: g.w * s,
                            height_px: g.h * s,
                        }.node(),
                        ImageNode {
                            image: sheet.clone(),
                            rect: Some(g.rect),
                            color: tint,
                            ..default()
                        },
                    ));
                };
                if shadow != 0.0 {
                    for (g, pen, tint) in &placed {
                        let a = tint.alpha() * shadow_alpha;
                        letter_shape(g, *pen, Color::srgba(0.0, 0.0, 0.0, a), shadow);
                    }
                }
                for (g, pen, tint) in &placed {
                    letter_shape(g, *pen, *tint, 0.0);
                }
                // Button icons: a font's height, centred on the line.
                let side = f.raster_rows * s;
                for (c, pen) in &icons {
                    let Some(image) = self.button_pictures.get(c) else { continue };
                    line.spawn((
                        ScreenBox {
                            left_px: pen * s + side * 0.05,
                            top_px: ((f.ascent + f.descent) * s - side) * 0.5,
                            width_px: side,
                            height_px: side,
                        }.node(),
                        ImageNode::new(image.clone()),
                    ));
                }
            });
        size
    }
}

/// Once the map's fonts arrive: the sheet as an image, each font's glyphs.
pub(crate) fn load_bo2_fonts(
    mut commands: Commands,
    staged: Option<Res<assets::T6HudFonts>>,
    hud_icons: Option<Res<assets::T6HudIcons>>,
    mut pics: ResMut<Assets<Image>>,
) {
    let Some(staged) = staged else {
        return;
    };
    if !staged.is_changed() && !hud_icons.as_ref().is_some_and(|i| i.is_changed()) {
        return;
    }
    let Some(sheet) = staged.sheet.as_ref() else {
        commands.remove_resource::<Bo2Fonts>();
        return;
    };
    let w = sheet.texture_descriptor.size.width as f32;
    let h = sheet.texture_descriptor.size.height as f32;
    let fonts = staged
        .fonts
        .iter()
        .map(|font| {
            let glyphs: HashMap<char, Bo2Glyph> = font
                .glyphs
                .iter()
                .filter_map(|(letter, m, st)| {
                    let c = char::from_u32(u32::from(*letter))?;
                    Some((
                        c,
                        Bo2Glyph {
                            x0: f32::from(m[0]),
                            y0: f32::from(m[1]),
                            step_along_x: f32::from(m[2]),
                            w: f32::from(m[3]),
                            h: f32::from(m[4]),
                            rect: Rect::new(st[0] * w, st[1] * h, st[2] * w, st[3] * h),
                        },
                    ))
                })
                .collect();
            // The letter_shape tables' y0 is against the line's reference, which
            // the engine puts one pixelHeight below the line's top (the line
            // step of `RB_DrawText` and `R_TextHeight` is the font's
            // pixelHeight, T5's `rb_backend.cpp`): ExtraSmall's tallest letter_shape
            // reaches 21 over a 20 px font and simply stands out of its box.
            let ascent = font.raster_rows;
            let descent = glyphs.values().map(|g| g.y0 + g.h).fold(0.0, f32::max);
            (
                font.name.clone(),
                Bo2Font {
                    raster_rows: font.raster_rows,
                    ascent,
                    descent,
                    glyphs,
                },
            )
        })
        .collect();
    let mut button_pictures: HashMap<char, Handle<Image>> = ['□', '○', '△', '×']
        .into_iter()
        .map(|c| (c, pics.add(button_icon(c))))
        .collect();
    // BO2's own Xbox button pictures, once the HUD's pictures are in.
    if let Some(hud) = hud_icons.as_deref() {
        for (c, name) in frame::BO2_XBOX_GLYPHS {
            if let Some(image) = crate::hud_picture(hud, name) {
                button_pictures.insert(c, pics.add((**image).clone()));
            }
        }
    }
    commands.insert_resource(Bo2Fonts {
        sheet: pics.add((**sheet).clone()),
        cpu: assets::plain_rgba8(sheet).map(std::sync::Arc::new),
        fonts,
        button_pictures,
    });
}
