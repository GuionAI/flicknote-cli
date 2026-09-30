//! Editable document save receipt; editing algorithms remain daemon-owned.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditableSaveResult {
    pub title_changed: bool,
    pub content_changed: bool,
    pub stored_content: String,
}
