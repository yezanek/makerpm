use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct ChangelogEntry {
    pub version: String,
    pub date: String,
    pub packager: String,
    pub entries: Vec<String>,
}
