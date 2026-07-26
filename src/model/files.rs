use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
pub struct FilesSpec {
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub docs: Vec<String>,
    #[serde(default)]
    pub licenses: Vec<String>,
    #[serde(default)]
    pub configs_noreplace: Vec<String>,
    #[serde(default)]
    pub configs: Vec<String>,
    #[serde(default)]
    pub dirs: Vec<String>,
}

impl FilesSpec {
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
            && self.docs.is_empty()
            && self.licenses.is_empty()
            && self.configs_noreplace.is_empty()
            && self.configs.is_empty()
            && self.dirs.is_empty()
    }

    pub fn all_paths(&self) -> impl Iterator<Item = &str> {
        self.paths
            .iter()
            .chain(self.docs.iter())
            .chain(self.licenses.iter())
            .chain(self.configs_noreplace.iter())
            .chain(self.configs.iter())
            .chain(self.dirs.iter())
            .map(String::as_str)
    }
}
