use super::super::Confidence;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranslatedDependency {
    pub value: String,
    pub confidence: Confidence,
    pub note: String,
}

pub fn split_dependencies(raw: &str) -> Vec<&str> {
    raw.split(',')
        .map(str::trim)
        .filter(|dependency| !dependency.is_empty())
        .collect()
}

pub fn translate(dependency: &str) -> TranslatedDependency {
    let dependency = dependency.trim();
    if dependency.contains('|') || dependency.contains('[') || dependency.contains(']') {
        return unsupported(dependency, "complex Debian dependency not translated");
    }

    let (name, constraint) = split_constraint(dependency);
    if name.contains('$') {
        return unsupported(
            "",
            "Debian substitution variable in package name was not evaluated",
        );
    }
    if name.contains(['<', '>']) {
        return unsupported("", "Debian build-profile qualifier was not evaluated");
    }
    if constraint.is_some_and(|constraint| constraint.contains('$')) {
        return unsupported(
            dependency,
            "Debian substitution variable in version constraint was not evaluated",
        );
    }
    let (translated_name, name_confidence, name_note) = translate_name(name);
    let translated_constraint = constraint.and_then(translate_constraint);

    if constraint.is_some() && translated_constraint.is_none() {
        return unsupported(dependency, "unrecognized Debian version constraint");
    }

    let value = match translated_constraint {
        Some((operator, version)) => format!("{translated_name} {operator} {version}"),
        None => translated_name.clone(),
    };
    let confidence = name_confidence;
    let note = match (constraint, name_confidence) {
        (Some(_), Confidence::Unsupported) => {
            "version constraint translated mechanically; package name passed through unchanged"
                .to_string()
        }
        (Some(_), _) => format!("{name_note}; version constraint translated mechanically"),
        (None, _) => name_note,
    };

    TranslatedDependency {
        value,
        confidence,
        note,
    }
}

fn split_constraint(dependency: &str) -> (&str, Option<&str>) {
    let Some(open) = dependency.find(" (") else {
        return (dependency, None);
    };
    if !dependency.ends_with(')') {
        return (dependency, None);
    }
    (
        dependency[..open].trim(),
        Some(dependency[open + 2..dependency.len() - 1].trim()),
    )
}

fn translate_constraint(constraint: &str) -> Option<(&'static str, &str)> {
    let (operator, version) = constraint.split_once(char::is_whitespace)?;
    let operator = match operator {
        "<<" => "<",
        "<=" => "<=",
        "=" => "=",
        ">=" => ">=",
        ">>" => ">",
        _ => return None,
    };
    Some((operator, version.trim()))
}

fn translate_name(name: &str) -> (String, Confidence, String) {
    if let Some(stem) = name
        .strip_prefix("lib")
        .and_then(|name| name.strip_suffix("-dev"))
    {
        return (
            format!("lib{stem}-devel"),
            Confidence::BestEffort,
            format!("translated Debian development package {name} heuristically"),
        );
    }
    if name.starts_with("lib") && name.ends_with(|character: char| character.is_ascii_digit()) {
        let stripped = name
            .trim_end_matches(|character: char| character.is_ascii_digit())
            .trim_end_matches(['.', '-', '_']);
        if stripped.is_empty() {
            return (
                name.to_string(),
                Confidence::Unsupported,
                format!("could not derive a Fedora package name from {name}"),
            );
        }
        return (
            stripped.to_string(),
            Confidence::BestEffort,
            format!("stripped Debian soname version from {name}"),
        );
    }
    if name.starts_with("python3-") {
        return (
            name.to_string(),
            Confidence::BestEffort,
            format!("kept likely cross-distribution Python package name {name}"),
        );
    }
    (
        name.to_string(),
        Confidence::Unsupported,
        format!("package name {name} was not translated; verify it exists on Fedora"),
    )
}

fn unsupported(value: &str, reason: &str) -> TranslatedDependency {
    TranslatedDependency {
        value: value.to_string(),
        confidence: Confidence::Unsupported,
        note: format!("{reason}; verify the package name and constraint on Fedora"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_supported_name_and_constraint_patterns() {
        assert_eq!(translate("libfoo-dev").value, "libfoo-devel");
        assert_eq!(translate("libfoo2").value, "libfoo");
        assert_eq!(translate("python3-click").value, "python3-click");

        let constrained = translate("foo (>= 1.2)");
        assert_eq!(constrained.value, "foo >= 1.2");
        assert_eq!(constrained.confidence, Confidence::Unsupported);
        assert!(constrained.note.contains("passed through unchanged"));

        let upper_bound = translate("libfoo (<< 2.0)");
        assert_eq!(upper_bound.value, "libfoo < 2.0");
        assert_eq!(upper_bound.confidence, Confidence::Unsupported);

        let qualified = translate("libfoo (>= 1.0) [amd64]");
        assert_eq!(qualified.confidence, Confidence::Unsupported);
        assert_eq!(qualified.value, "libfoo (>= 1.0) [amd64]");
    }

    #[test]
    fn unsupported_names_and_complex_relations_are_loud() {
        assert_eq!(translate("debhelper").confidence, Confidence::Unsupported);
        assert_eq!(translate("foo | bar").confidence, Confidence::Unsupported);
        assert_eq!(
            translate("sample (= ${binary:Version})").confidence,
            Confidence::Unsupported
        );
        for dependency in ["${misc:Depends}", "foo <!nocheck>"] {
            let translated = translate(dependency);
            assert_eq!(translated.confidence, Confidence::Unsupported);
            assert!(translated.value.is_empty());
        }
    }
}
