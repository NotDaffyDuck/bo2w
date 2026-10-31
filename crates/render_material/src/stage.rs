/// Which programmable stage an argument or program belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LiveShaderStage {
    Vertex,
    Pixel,
}
