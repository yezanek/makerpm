use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Default, Serialize)]
pub struct DependencySet {
    #[serde(default)]
    pub build_depends: Vec<String>,
    #[serde(default)]
    pub depends: Vec<String>,
    #[serde(default)]
    pub recommends: Vec<String>,
    #[serde(default)]
    pub suggests: Vec<String>,
    #[serde(default)]
    pub conflicts: Vec<String>,
    #[serde(default)]
    pub provides: Vec<String>,
    #[serde(default)]
    pub obsoletes: Vec<String>,
    #[serde(default)]
    pub supplements: Vec<String>,
    #[serde(default)]
    pub enhances: Vec<String>,
}
