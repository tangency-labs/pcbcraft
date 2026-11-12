//! Exact immutable item references, independent of files and display names.
use serde::{Deserialize, Serialize};
/// An immutable definition address. The containing field supplies the item kind; paths are not identity.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub library: String,
    pub id: String,
    pub revision: u32,
}
impl Reference {
    /// Reject invalid library/item identities and unversioned references before resolution.
    pub fn validate(&self) -> Result<(), String> {
        crate::library::check_id(&self.library)?;
        crate::library::check_id(&self.id)?;
        if self.revision == 0 {
            return Err("Definition references require a positive revision.".into());
        }
        Ok(())
    }
}
