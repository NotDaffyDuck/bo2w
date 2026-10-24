use crate::decl_table::ROUTING_COUNT;
use crate::vertex_layout::VertexLayoutFamily;

/// One vertex declaration: which stream each routed source reads from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeVertexDecl {
    pub family: VertexLayoutFamily,
    pub name: String,
    pub stream_count: u8,
    pub has_optional_source: u8,
    pub routing: [[u8; 2]; ROUTING_COUNT],
}

impl RuntimeVertexDecl {
    /// The routing rows that are actually in use.
    pub fn routed(&self) -> &[[u8; 2]] {
        let used = usize::from(self.stream_count).min(ROUTING_COUNT);
        &self.routing[..used]
    }
}
