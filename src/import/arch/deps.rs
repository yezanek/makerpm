#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranslatedDependency {
    pub value: String,
    pub note: String,
}

pub fn translate(raw: &str) -> TranslatedDependency {
    let raw = raw.trim();
    for operator in [">=", "<=", "=", ">", "<"] {
        if let Some((name, version)) = raw.split_once(operator) {
            if !name.trim().is_empty() && !version.trim().is_empty() {
                return TranslatedDependency {
                    value: format!("{} {operator} {}", name.trim(), version.trim()),
                    note: format!(
                        "translated Arch version syntax for {}; verify the package name on Fedora",
                        name.trim()
                    ),
                };
            }
        }
    }
    TranslatedDependency {
        value: raw.to_string(),
        note: format!("passed Arch package name {raw} through unchanged; verify it on Fedora"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spaces_arch_version_constraints_for_rpm() {
        assert_eq!(translate("openssl>=3.0").value, "openssl >= 3.0");
        assert_eq!(translate("python-click").value, "python-click");
    }
}
