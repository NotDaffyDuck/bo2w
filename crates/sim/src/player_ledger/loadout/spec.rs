use crate::tick_feed::LoadoutClass;

/// The loadout a player spawned with: which class, and the revision of it that was current at the time.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KitPlan {
    pub class_id: LoadoutClass,
    pub revision: u32,
    pub primary: u32,
    pub secondary: u32,
    pub primary_attachments: [u32; 4],
    pub secondary_attachments: [u32; 4],
    pub lethal: u32,
    pub tactical: u32,
    pub perks: [u32; 3],
}
/// Older spelling LoadoutSpec still calls; the rename sweep moves it over.
pub use self::KitPlan as LoadoutSpec;
