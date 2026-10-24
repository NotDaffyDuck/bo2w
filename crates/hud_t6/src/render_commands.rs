//! BO2 render command buffer (scratch/hud-facts/render_commands.md): the 4-byte header, the 33 command
//! ids, the critical/noncritical allocator and the ui3d-filtered walker.

/// Header: u16 byteCount @0, u8 id @2, u8 ui3d @3.
pub const GFX_CMD_HEADER_SIZE: u32 = 4;

pub const RC_END_OF_LIST: u8 = 0;
pub const RC_SET_CUSTOM_CONSTANT: u8 = 1;
pub const RC_SET_MATERIAL_COLOR: u8 = 2;
pub const RC_SAVE_SCREEN: u8 = 3;
pub const RC_SAVE_SCREEN_SECTION: u8 = 4;
pub const RC_CLEAR_SCREEN: u8 = 5;
pub const RC_BEGIN_VIEW: u8 = 6;
pub const RC_SET_VIEWPORT: u8 = 7;
pub const RC_SET_SCISSOR: u8 = 8;
pub const RC_RESOLVE_COMPOSITE: u8 = 9;
pub const RC_PC_COPY_IMAGE_GEN_MIP: u8 = 10;
pub const RC_STRETCH_PIC: u8 = 11;
pub const RC_STRETCH_PIC_FLIP_ST: u8 = 12;
pub const RC_STRETCH_PIC_ROTATE_XY: u8 = 13;
pub const RC_STRETCH_PIC_ROTATE_ST: u8 = 14;
pub const RC_DRAW_QUAD_PIC: u8 = 15;
pub const RC_DRAW_FULL_SCREEN_COLORED_QUAD: u8 = 16;
pub const RC_DRAW_TEXT_2D: u8 = 17;
pub const RC_DRAW_TEXT_3D: u8 = 18;
pub const RC_BLEND_SAVED_SCREEN_BLURRED: u8 = 19;
pub const RC_BLEND_SAVED_SCREEN_FLASHED: u8 = 20;
pub const RC_DRAW_POINTS: u8 = 21;
pub const RC_DRAW_LINES: u8 = 22;
pub const RC_DRAW_UI_QUADS: u8 = 23;
pub const RC_DRAW_UI_QUADS_REPLACE_IMAGE: u8 = 24;
pub const RC_DRAW_TRIANGLES: u8 = 25;
pub const RC_DRAW_UI_TRIANGLES: u8 = 26;
pub const RC_DRAW_QUADLIST_2D: u8 = 27;
pub const RC_DRAW_EMBLEM_LAYER: u8 = 28;
pub const RC_STRETCH_COMPOSITE: u8 = 29;
pub const RC_PROJECTION_SET: u8 = 30;
pub const RC_DRAW_FRAMED: u8 = 31;
pub const RC_CONSTANT_SET: u8 = 32;
pub const RC_COUNT: u8 = 33;
/// Ids below this are critical (never dropped); this one and above may be dropped.
pub const RC_FIRST_NONCRITICAL: u8 = RC_STRETCH_PIC;

/// Fixed command sizes.
pub const RC_END_OF_LIST_SIZE: u32 = 4;
pub const RC_SET_CUSTOM_CONSTANT_SIZE: u32 = 0x18;
pub const RC_SET_VIEWPORT_SIZE: u32 = 0x14;
pub const RC_SET_SCISSOR_SIZE: u32 = 0x18;
pub const RC_RESOLVE_COMPOSITE_SIZE: u32 = 8;
pub const RC_PC_COPY_IMAGE_GEN_MIP_SIZE: u32 = 0x10;
pub const RC_CLEAR_SCREEN_SIZE: u32 = 0x1c;
pub const RC_STRETCH_PIC_ROTATE_SIZE: u32 = 0x34;
pub const RC_DRAW_QUAD_PIC_SIZE: u32 = 0x30;
pub const RC_PROJECTION_SET_SIZE: u32 = 8;
pub const RC_DRAW_FRAMED_SIZE: u32 = 0x2c;
pub const RC_CONSTANT_SET_SIZE: u32 = 0x7c;

/// DRAW_QUADLIST_2D size for `count` quads.
#[must_use]
pub fn rc_draw_quadlist_2d_size(count: u32) -> u32 {
    count * 0x50 + 0x0c
}

/// Tail kept for critical commands; it shrinks as critical commands are written.
pub const GFX_RENDER_CMD_CRITICAL_RESERVE: u32 = 0x2000;
/// Critical usage past this only warns.
pub const GFX_RENDER_CMD_CRITICAL_WARN: u32 = 0x1e00;

/// ui3d header byte: 0 outside a ui3d window, else 0x80 | window index (index max 0x7f).
#[must_use]
pub fn ui3d_byte(window: Option<u8>) -> u8 {
    window.map_or(0, |w| 0x80 | (w & 0x7f))
}

#[must_use]
pub fn is_critical(id: u8) -> bool {
    id < RC_FIRST_NONCRITICAL
}

/// The per-frame command list (BO2 GfxCmdArray). Buffer size and warn size are runtime values.
#[derive(Debug)]
pub struct GfxRenderCommandBuf<'a> {
    pub buf: &'a mut [u8],
    pub used_total: u32,
    pub used_critical: u32,
    /// Offset of the last command written; None after a dropped command.
    pub last: Option<u32>,
    pub byte_size: u32,
    pub warn_size: u32,
    /// ui3d byte stamped on new commands.
    pub ui3d: u8,
    pub critical_warned: bool,
    pub size_warned: bool,
}

impl<'a> GfxRenderCommandBuf<'a> {
    #[must_use]
    pub fn new(buf: &'a mut [u8], warn_size: u32) -> Self {
        let byte_size = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        Self {
            buf,
            used_total: 0,
            used_critical: 0,
            last: None,
            byte_size,
            warn_size,
            ui3d: 0,
            critical_warned: false,
            size_warned: false,
        }
    }

    /// Room for a command with this id: critical commands may use the reserved tail.
    #[must_use]
    pub fn space_for(&self, id: u8) -> u32 {
        let free = self.byte_size.saturating_sub(self.used_total);
        if is_critical(id) {
            free
        } else {
            (free + self.used_critical).saturating_sub(GFX_RENDER_CMD_CRITICAL_RESERVE)
        }
    }

    /// R_GetCommandBuffer: reserve `bytes` for command `id`, write its header and return the whole
    /// command slice. A noncritical command that does not fit is dropped (None, last cleared).
    pub fn get_command_buffer(&mut self, id: u8, bytes: u32) -> Option<&mut [u8]> {
        debug_assert!(id < RC_COUNT, "render command id {id} out of range");
        debug_assert!(bytes.is_multiple_of(4) && bytes >= GFX_CMD_HEADER_SIZE);
        if id >= RC_COUNT || !bytes.is_multiple_of(4) || bytes > u32::from(u16::MAX) || bytes >= self.byte_size
        {
            self.last = None;
            return None;
        }
        if bytes > self.space_for(id) {
            debug_assert!(!is_critical(id), "critical render command {id} does not fit");
            self.last = None;
            return None;
        }
        let off = self.used_total;
        self.used_total += bytes;
        if is_critical(id) {
            self.used_critical += bytes;
            self.critical_warned |= self.used_critical > GFX_RENDER_CMD_CRITICAL_WARN;
        }
        self.size_warned |= self.warn_size != 0 && self.used_total > self.warn_size;
        self.last = Some(off);
        let cmd = &mut self.buf[off as usize..(off + bytes) as usize];
        cmd[0..2].copy_from_slice(&(bytes as u16).to_le_bytes());
        cmd[2] = id;
        cmd[3] = self.ui3d;
        Some(cmd)
    }

    /// R_AddCmdEndOfList.
    pub fn add_end_of_list(&mut self) -> bool {
        self.get_command_buffer(RC_END_OF_LIST, RC_END_OF_LIST_SIZE).is_some()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GfxCmdView<'a> {
    pub id: u8,
    pub ui3d: u8,
    pub size: u16,
    pub offset: u32,
    pub bytes: &'a [u8],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GfxCmdWalkRefuse {
    TruncatedHeader { offset: u32 },
    SizeBelowHeader { offset: u32, size: u16 },
    SlicePastUsed { offset: u32, size: u16 },
    BadId { offset: u32, id: u8 },
}

/// RB_ExecuteRenderCommandsLoop's walk: stops at END_OF_LIST. With no ui3d filter only commands whose
/// ui3d byte is 0 are yielded; with a filter only those with the high bit set and that window index.
/// `used` bounds the walk as a safety net (BO2 relies on the terminator alone).
pub struct GfxCmdWalk<'a> {
    buf: &'a [u8],
    used: u32,
    off: u32,
    filter: Option<u8>,
}

impl<'a> GfxCmdWalk<'a> {
    #[must_use]
    pub fn new(buf: &'a [u8], used: u32, ui3d_filter: Option<u8>) -> Self {
        let used = used.min(u32::try_from(buf.len()).unwrap_or(u32::MAX));
        Self { buf, used, off: 0, filter: ui3d_filter }
    }

    fn runs(&self, ui3d: u8) -> bool {
        match self.filter {
            None => ui3d == 0,
            Some(w) => ui3d & 0x80 != 0 && ui3d & 0x7f == w & 0x7f,
        }
    }
}

impl<'a> Iterator for GfxCmdWalk<'a> {
    type Item = Result<GfxCmdView<'a>, GfxCmdWalkRefuse>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.off >= self.used {
                return None;
            }
            let offset = self.off;
            let start = offset as usize;
            let fail = |s: &mut Self, e| {
                s.off = s.used;
                Some(Err(e))
            };
            if self.used - offset < GFX_CMD_HEADER_SIZE {
                return fail(self, GfxCmdWalkRefuse::TruncatedHeader { offset });
            }
            let size = u16::from_le_bytes([self.buf[start], self.buf[start + 1]]);
            let id = self.buf[start + 2];
            let ui3d = self.buf[start + 3];
            if id == RC_END_OF_LIST {
                self.off = self.used;
                return None;
            }
            if id >= RC_COUNT {
                return fail(self, GfxCmdWalkRefuse::BadId { offset, id });
            }
            if u32::from(size) < GFX_CMD_HEADER_SIZE {
                return fail(self, GfxCmdWalkRefuse::SizeBelowHeader { offset, size });
            }
            let end = offset + u32::from(size);
            if end > self.used {
                return fail(self, GfxCmdWalkRefuse::SlicePastUsed { offset, size });
            }
            self.off = end;
            if !self.runs(ui3d) {
                continue;
            }
            let bytes = &self.buf[start..end as usize];
            return Some(Ok(GfxCmdView { id, ui3d, size, offset, bytes }));
        }
    }
}

#[must_use]
pub fn walk_render_commands(buf: &[u8], used: u32, ui3d_filter: Option<u8>) -> GfxCmdWalk<'_> {
    GfxCmdWalk::new(buf, used, ui3d_filter)
}

#[cfg(test)]
mod tests {
    extern crate alloc;

    use super::*;

    #[test]
    fn header_layout_and_critical_split() {
        let mut mem = [0u8; 0x4000];
        let mut b = GfxRenderCommandBuf::new(&mut mem, 0);
        let cmd = b.get_command_buffer(RC_SET_VIEWPORT, RC_SET_VIEWPORT_SIZE).unwrap();
        assert_eq!(&cmd[0..4], &[0x14, 0, RC_SET_VIEWPORT, 0]);
        assert_eq!(b.used_critical, 0x14);
        b.ui3d = ui3d_byte(Some(3));
        let cmd = b.get_command_buffer(RC_STRETCH_PIC, 0x30).unwrap();
        assert_eq!(&cmd[0..4], &[0x30, 0, RC_STRETCH_PIC, 0x83]);
        assert_eq!(b.used_total, 0x44);
        assert_eq!(b.used_critical, 0x14);
        assert_eq!(b.last, Some(0x14));
    }

    #[test]
    fn noncritical_respects_reserve_that_shrinks_with_critical_use() {
        let mut mem = [0u8; 0x3000];
        let mut b = GfxRenderCommandBuf::new(&mut mem, 0);
        // 0x3000 - 0x2000 = 0x1000 bytes for noncritical commands.
        assert_eq!(b.space_for(RC_STRETCH_PIC), 0x1000);
        assert!(b.get_command_buffer(RC_DRAW_TEXT_2D, 0x1000).is_some());
        assert!(b.get_command_buffer(RC_STRETCH_PIC, 0x30).is_none());
        assert_eq!(b.last, None);
        // Critical commands still fit in the tail, and writing them frees noncritical room.
        assert!(b.get_command_buffer(RC_CLEAR_SCREEN, 0x100).is_some());
        assert_eq!(b.space_for(RC_STRETCH_PIC), 0);
        assert_eq!(b.space_for(RC_SET_SCISSOR), 0x1f00);
    }

    #[test]
    fn warnings_never_refuse() {
        let mut mem = [0u8; 0x8000];
        let mut b = GfxRenderCommandBuf::new(&mut mem, 0x100);
        assert!(b.get_command_buffer(RC_CLEAR_SCREEN, 0x1f00).is_some());
        assert!(b.critical_warned);
        assert!(b.size_warned);
    }

    #[test]
    fn walk_stops_at_end_and_filters_ui3d() {
        let mut mem = [0u8; 0x4000];
        let mut b = GfxRenderCommandBuf::new(&mut mem, 0);
        b.get_command_buffer(RC_SET_SCISSOR, RC_SET_SCISSOR_SIZE).unwrap();
        b.ui3d = ui3d_byte(Some(2));
        b.get_command_buffer(RC_STRETCH_PIC, 0x30).unwrap();
        b.ui3d = 0;
        b.get_command_buffer(RC_DRAW_TEXT_2D, 0x68).unwrap();
        b.add_end_of_list();
        // Junk after the terminator is never read.
        b.get_command_buffer(RC_STRETCH_PIC, 0x30).unwrap();
        let used = b.used_total;
        let ids = |f| {
            walk_render_commands(&mem, used, f)
                .map(|c| c.unwrap().id)
                .collect::<alloc::vec::Vec<_>>()
        };
        assert_eq!(ids(None), [RC_SET_SCISSOR, RC_DRAW_TEXT_2D]);
        assert_eq!(ids(Some(2)), [RC_STRETCH_PIC]);
        assert!(ids(Some(1)).is_empty());
    }

    #[test]
    fn walk_refuses_bad_headers() {
        let mut mem = [0u8; 16];
        mem[0..4].copy_from_slice(&[2, 0, RC_STRETCH_PIC, 0]);
        let r: alloc::vec::Vec<_> = walk_render_commands(&mem, 16, None).collect();
        assert_eq!(r, [Err(GfxCmdWalkRefuse::SizeBelowHeader { offset: 0, size: 2 })]);
        mem[0..4].copy_from_slice(&[4, 0, 40, 0]);
        let r: alloc::vec::Vec<_> = walk_render_commands(&mem, 16, None).collect();
        assert_eq!(r, [Err(GfxCmdWalkRefuse::BadId { offset: 0, id: 40 })]);
        assert_eq!(rc_draw_quadlist_2d_size(2), 0xac);
    }
}
