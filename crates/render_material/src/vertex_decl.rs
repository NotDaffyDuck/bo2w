use crate::decl_table::MIX_PATH_TOTAL;
use crate::vertex_layout::VertexFormatLineage;

/// One vertex declaration: which stream each routed source reads from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveVertexFormat {
    pub family: VertexFormatLineage,
    pub name: String,
    pub stream_tally: u8,
    pub has_spare_source: u8,
    pub routing: [[u8; 2]; MIX_PATH_TOTAL],
}

impl LiveVertexFormat {
    /// The routing rows that are actually in use.
    pub fn route_rows(&self) -> &[[u8; 2]] {
        let used = usize::from(self.stream_tally).min(MIX_PATH_TOTAL);
        &self.routing[..used]
    }
}
