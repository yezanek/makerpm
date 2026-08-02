use std::collections::BTreeMap;

use thiserror::Error;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Stanza {
    fields: BTreeMap<String, String>,
}

impl Stanza {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.fields
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlFile {
    pub source: Stanza,
    pub binaries: Vec<Stanza>,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ControlError {
    #[error("line {line}: continuation without a preceding field")]
    OrphanContinuation { line: usize },

    #[error("line {line}: expected a Field: value entry")]
    InvalidField { line: usize },

    #[error("debian/control has no Source stanza")]
    MissingSource,

    #[error("debian/control has no binary Package stanzas")]
    MissingBinary,
}

pub fn parse(input: &str) -> Result<ControlFile, ControlError> {
    let mut stanzas = Vec::new();
    let mut current = Stanza::default();
    let mut previous_field: Option<String> = None;

    for (index, raw_line) in input.lines().enumerate() {
        let line_number = index + 1;
        let line = raw_line.trim_end_matches('\r');
        if line.trim().is_empty() {
            if !current.fields.is_empty() {
                stanzas.push(std::mem::take(&mut current));
            }
            previous_field = None;
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        if line.starts_with([' ', '\t']) {
            let key = previous_field
                .as_ref()
                .ok_or(ControlError::OrphanContinuation { line: line_number })?;
            let value = current
                .fields
                .get_mut(key)
                .expect("previous_field always refers to the current stanza");
            value.push('\n');
            value.push_str(line.trim_start());
            continue;
        }

        let (name, value) = line
            .split_once(':')
            .ok_or(ControlError::InvalidField { line: line_number })?;
        let name = name.trim();
        if name.is_empty() || name.chars().any(char::is_whitespace) {
            return Err(ControlError::InvalidField { line: line_number });
        }
        let key = name.to_ascii_lowercase();
        current
            .fields
            .insert(key.clone(), value.trim_start().to_string());
        previous_field = Some(key);
    }

    if !current.fields.is_empty() {
        stanzas.push(current);
    }

    let source_index = stanzas
        .iter()
        .position(|stanza| stanza.get("Source").is_some())
        .ok_or(ControlError::MissingSource)?;
    let source = stanzas.remove(source_index);
    let binaries = stanzas
        .into_iter()
        .filter(|stanza| stanza.get("Package").is_some())
        .collect::<Vec<_>>();
    if binaries.is_empty() {
        return Err(ControlError::MissingBinary);
    }

    Ok(ControlFile { source, binaries })
}

pub fn description(stanza: &Stanza) -> (String, String) {
    let raw = stanza.get("Description").unwrap_or_default();
    let mut lines = raw.lines();
    let summary = lines.next().unwrap_or_default().trim().to_string();
    let long = lines
        .map(|line| if line.trim() == "." { "" } else { line })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    let description = if long.is_empty() {
        summary.clone()
    } else {
        long
    };
    (summary, description)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MULTI_BINARY_CONTROL: &str = r#"Source: libsample
Maintainer: Jane Doe <jane@example.org>
Build-Depends: debhelper-compat (= 13),
 libssl-dev,
 python3-setuptools
Homepage: https://example.org/libsample

Package: libsample
Architecture: any
Depends: ${shlibs:Depends}, ${misc:Depends}
Description: Sample runtime library
 The first paragraph wraps across
 multiple physical lines.
 .
 This is a second paragraph.

Package: libsample-dev
Architecture: all
Depends: libsample1 (= ${binary:Version})
Description: Sample development files
 Headers and static libraries.
"#;

    #[test]
    fn parses_wrapped_fields_and_multiple_binary_stanzas() {
        let control = parse(MULTI_BINARY_CONTROL).unwrap();
        assert_eq!(control.source.get("Source"), Some("libsample"));
        assert_eq!(
            control.source.get("Build-Depends"),
            Some("debhelper-compat (= 13),\nlibssl-dev,\npython3-setuptools")
        );
        assert_eq!(control.binaries.len(), 2);
        assert_eq!(control.binaries[1].get("Architecture"), Some("all"));

        let (summary, long) = description(&control.binaries[0]);
        assert_eq!(summary, "Sample runtime library");
        assert_eq!(
            long,
            "The first paragraph wraps across\nmultiple physical lines.\n\nThis is a second paragraph."
        );
    }

    #[test]
    fn rejects_missing_source_or_binary_stanzas() {
        assert_eq!(
            parse("Package: only-binary\n"),
            Err(ControlError::MissingSource)
        );
        assert_eq!(
            parse("Source: only-source\n"),
            Err(ControlError::MissingBinary)
        );
    }
}
