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

pub struct BuildMacros {
    pub configure: Option<&'static str>,
    pub build: &'static str,
    pub install: &'static str,
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

    pub fn macros(&self) -> Option<BuildMacros> {
        match self {
            BuildSystem::None => None,
            BuildSystem::Make => Some(BuildMacros {
                configure: None,
                build: "%make_build",
                install: "%make_install",
            }),
            BuildSystem::Autotools => Some(BuildMacros {
                configure: Some("%configure"),
                build: "%make_build",
                install: "%make_install",
            }),
            BuildSystem::Cmake => Some(BuildMacros {
                configure: Some("%cmake"),
                build: "%cmake_build",
                install: "%cmake_install",
            }),
            BuildSystem::Meson => Some(BuildMacros {
                configure: Some("%meson"),
                build: "%meson_build",
                install: "%meson_install",
            }),
            BuildSystem::Cargo => Some(BuildMacros {
                configure: None,
                build: "%cargo_build",
                install: "%cargo_install",
            }),
            BuildSystem::PythonPyproject => Some(BuildMacros {
                configure: None,
                build: "%pyproject_build",
                install: "%pyproject_install",
            }),
        }
    }

    pub fn extra_build_args_string(&self, args: &[String]) -> String {
        args.join(" ")
    }

    pub fn extra_install_args_string(&self, args: &[String]) -> String {
        if args.is_empty() {
            return String::new();
        }
        args.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_has_no_macros() {
        assert!(BuildSystem::None.macros().is_none());
    }

    #[test]
    fn make_macros() {
        let m = BuildSystem::Make.macros().unwrap();
        assert_eq!(m.configure, None);
        assert_eq!(m.build, "%make_build");
        assert_eq!(m.install, "%make_install");
    }

    #[test]
    fn cmake_macros() {
        let m = BuildSystem::Cmake.macros().unwrap();
        assert_eq!(m.configure, Some("%cmake"));
        assert_eq!(m.build, "%cmake_build");
        assert_eq!(m.install, "%cmake_install");
    }

    #[test]
    fn autotools_macros() {
        let m = BuildSystem::Autotools.macros().unwrap();
        assert_eq!(m.configure, Some("%configure"));
        assert_eq!(m.build, "%make_build");
        assert_eq!(m.install, "%make_install");
    }

    #[test]
    fn meson_macros() {
        let m = BuildSystem::Meson.macros().unwrap();
        assert_eq!(m.configure, Some("%meson"));
        assert_eq!(m.build, "%meson_build");
        assert_eq!(m.install, "%meson_install");
    }

    #[test]
    fn cargo_macros() {
        let m = BuildSystem::Cargo.macros().unwrap();
        assert_eq!(m.configure, None);
        assert_eq!(m.build, "%cargo_build");
        assert_eq!(m.install, "%cargo_install");
    }

    #[test]
    fn python_pyproject_macros() {
        let m = BuildSystem::PythonPyproject.macros().unwrap();
        assert_eq!(m.configure, None);
        assert_eq!(m.build, "%pyproject_build");
        assert_eq!(m.install, "%pyproject_install");
    }

    #[test]
    fn extra_build_args_empty() {
        assert_eq!(BuildSystem::Cmake.extra_build_args_string(&[]), "");
    }

    #[test]
    fn extra_build_args_cmake() {
        let args = vec!["-DCMAKE_BUILD_TYPE=Release".into(), "-DBUILD_TESTING=OFF".into()];
        assert_eq!(
            BuildSystem::Cmake.extra_build_args_string(&args),
            "-DCMAKE_BUILD_TYPE=Release -DBUILD_TESTING=OFF"
        );
    }

    #[test]
    fn extra_install_args_empty() {
        assert_eq!(BuildSystem::Cmake.extra_install_args_string(&[]), "");
    }

    #[test]
    fn extra_install_args_with_values() {
        let args = vec!["DESTDIR=%{buildroot}".into()];
        assert_eq!(
            BuildSystem::Make.extra_install_args_string(&args),
            "DESTDIR=%{buildroot}"
        );
    }

    #[test]
    fn cmake_required_build_requires() {
        assert_eq!(BuildSystem::Cmake.required_build_requires(), &["cmake", "gcc-c++"]);
    }

    #[test]
    fn make_required_build_requires_is_empty() {
        assert!(BuildSystem::Make.required_build_requires().is_empty());
    }
}
