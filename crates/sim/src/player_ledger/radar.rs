//! The minimap sweep mode.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RadarMode {
    #[default]
    Off,
    Normal,
    Fast,
    Constant,
}

impl RadarMode {
    const ALL: [RadarMode; 4] = [Self::Off, Self::Normal, Self::Fast, Self::Constant];

    /// The byte that stands for this mode on the wire (its position in the list above).
    pub fn net_code(self) -> u8 {
        self as u8
    }

    pub fn from_net_code(tag: u8) -> Option<Self> {
        Self::ALL.get(usize::from(tag)).copied()
    }
}
