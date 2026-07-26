use serde::Deserialize;

#[derive(Debug, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BuildSystem {
    #[default]
    None,
    Make,
    Autotools,
    Cmake,
    Meson,
    Cargo,
    PythonPyproject,
}

#[derive(Debug, Deserialize, Default)]
pub struct BuildSteps {
    #[serde(default)]
    pub prep: Option<String>,
    #[serde(default)]
    pub build: Option<String>,
    #[serde(default)]
    pub install: Option<String>,
    #[serde(default)]
    pub check: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct BuildSpec {
    #[serde(default)]
    pub system: BuildSystem,
    #[serde(default)]
    pub extra_build_args: Vec<String>,
    #[serde(default)]
    pub extra_install_args: Vec<String>,
    #[serde(default)]
    pub run_tests: Option<bool>,
    #[serde(default)]
    pub steps: BuildSteps,
}

impl BuildSystem {
    pub fn required_build_requires(&self) -> &'static [&'static str] {
        match self {
            BuildSystem::None => &[],
            BuildSystem::Make => &[],
            BuildSystem::Autotools => &["autoconf", "automake", "libtool"],
            BuildSystem::Cmake => &["cmake", "gcc-c++"],
            BuildSystem::Meson => &["meson", "ninja-build"],
            BuildSystem::Cargo => &["rust-packaging"],
            BuildSystem::PythonPyproject => &["python3-devel", "pyproject-rpm-macros"],
        }
    }
}
