//! BO2 client HUD and 2D quantities, written from BO2 facts (scratch/hud-facts).
#![no_std]
#![forbid(unsafe_code)]

pub mod ammo;
pub mod centerprint;
pub mod compass;
pub mod flashbang;
pub mod crosshair;
pub mod directive;
pub mod draw_text_cmd;
pub mod expr;
pub mod font;
pub mod gamemsg;
pub mod hudelem;
pub mod fov;
pub mod iris;
pub mod item_text;
pub mod menu_transition;
pub mod overhead_names;
pub mod rangefinder;
pub mod render_commands;
pub mod scrplace;
pub mod set_2d;
pub mod stretch_pic_cmd;
pub mod text_fx;
pub mod view_projection;
pub mod vision_set;
