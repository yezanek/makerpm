use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ChangelogEntry {
    pub version: String,
    pub date: String,
    pub packager: String,
    pub entries: Vec<String>,
}
