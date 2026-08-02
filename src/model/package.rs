use serde::{Deserialize, Serialize};

use super::{BuildSpec, ChangelogEntry, DependencySet, FilesSpec, Scriptlets};

fn default_release() -> String {
    "1%{?dist}".to_string()
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Package {
    pub name: String,
    pub version: String,
    #[serde(default = "default_release")]
    pub release: String,
    #[serde(default)]
    pub epoch: Option<u32>,
    pub summary: String,
    pub license: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub noarch: bool,
    pub description: String,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub sha256sums: Vec<String>,
    #[serde(default)]
    pub patches: Vec<String>,
    #[serde(default)]
    pub patch_sha256sums: Vec<String>,
    #[serde(default)]
    pub deps: DependencySet,
    #[serde(default)]
    pub build: BuildSpec,
    #[serde(default)]
    pub files: FilesSpec,
    #[serde(default)]
    pub scriptlets: Scriptlets,
    #[serde(default)]
    pub changelog: Vec<ChangelogEntry>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Subpackage {
    pub suffix: String,
    pub summary: String,
    pub description: String,
    #[serde(default)]
    pub noarch: Option<bool>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub deps: DependencySet,
    #[serde(default)]
    pub files: FilesSpec,
    #[serde(default)]
    pub scriptlets: Scriptlets,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PkgSpecFile {
    pub package: Package,
    #[serde(default, rename = "subpackage")]
    pub subpackages: Vec<Subpackage>,
}
