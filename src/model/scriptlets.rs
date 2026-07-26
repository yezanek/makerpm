use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
pub struct Scriptlets {
    #[serde(default)]
    pub pretrans: Option<String>,
    #[serde(default)]
    pub pre: Option<String>,
    #[serde(default)]
    pub post: Option<String>,
    #[serde(default)]
    pub preun: Option<String>,
    #[serde(default)]
    pub postun: Option<String>,
    #[serde(default)]
    pub posttrans: Option<String>,
    #[serde(default)]
    pub interpreter: Option<String>,
}
