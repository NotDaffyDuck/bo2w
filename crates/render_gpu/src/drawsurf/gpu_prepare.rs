use render_frame::{WgpuBindLayoutEntry, WgpuPassLayout};
use render_material::{ConstantSource, PackedCodeConstantLane, RuntimeShaderStage};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PassConstantBuffers {
    pub vertex: Vec<[u32; 4]>,
    pub pixel: Vec<[u32; 4]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConstantPackRefusal {
    ExecutablePairMismatch,
    MissingLocalBanks,
    DestinationPastConstantBlock {
        stage: RuntimeShaderStage,
        destination: u16,
        block_len: usize,
    },
    MissingConstantRegister {
        stage: RuntimeShaderStage,
        register: u16,
        source: ConstantSource,
    },
}

pub fn overlay_packed_code_on_banks(
    vertex: &mut [[u32; 4]],
    pixel: &mut [[u32; 4]],
    lanes: &[PackedCodeConstantLane],
) -> Result<u32, ConstantPackRefusal> {
    let mut written = 0u32;
    for lane in lanes {
        let start = usize::from(lane.first_row);
        let end = start.saturating_add(usize::from(lane.row_count));
        overlay_write_rows(
            lane.stage,
            lane.destination,
            &lane.rows[start..end],
            vertex,
            pixel,
        )?;
        written = written.saturating_add(u32::from(lane.row_count));
    }
    Ok(written)
}

fn overlay_write_rows(
    stage: RuntimeShaderStage,
    destination: u16,
    rows: &[[u32; 4]],
    vertex: &mut [[u32; 4]],
    pixel: &mut [[u32; 4]],
) -> Result<(), ConstantPackRefusal> {
    let bank = match stage {
        RuntimeShaderStage::Vertex => &mut *vertex,
        RuntimeShaderStage::Pixel => &mut *pixel,
    };
    for (row, words) in rows.iter().enumerate() {
        let register = destination
            .checked_add(u16::try_from(row).expect("row index fits u16"))
            .ok_or(ConstantPackRefusal::DestinationPastConstantBlock {
                stage,
                destination,
                block_len: bank.len(),
            })?;
        let index = usize::from(register);
        if index >= bank.len() {
            return Err(ConstantPackRefusal::DestinationPastConstantBlock {
                stage,
                destination: register,
                block_len: bank.len(),
            });
        }
        bank[index] = *words;
    }
    Ok(())
}

pub fn split_bind_layout(
    layout: &WgpuPassLayout,
) -> (Vec<WgpuBindLayoutEntry>, Vec<WgpuBindLayoutEntry>) {
    let mut constants = Vec::new();
    let mut textures = Vec::new();
    for entry in &layout.bind_entries {
        match entry.group {
            0 => constants.push(*entry),
            1 => textures.push(*entry),
            _ => {}
        }
    }
    (constants, textures)
}
