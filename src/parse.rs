use std::path::Path;

use miette::Diagnostic;
use thiserror::Error;

use crate::model::PkgSpecFile;

#[derive(Debug, Error, Diagnostic)]
#[error("failed to parse PKGSPEC.toml")]
pub struct ParseError {
    #[source]
    pub inner: toml::de::Error,
    #[source_code]
    pub source_text: String,
}

pub fn parse_pkgspec(input: &str) -> Result<PkgSpecFile, ParseError> {
    toml::from_str(input).map_err(|inner| ParseError {
        inner,
        source_text: input.to_string(),
    })
}

pub fn parse_pkgspec_file(path: &Path) -> Result<(String, PkgSpecFile), ParseError> {
    let input = std::fs::read_to_string(path).unwrap_or_else(|e| {
        panic!("failed to read {}: {e}", path.display());
    });
    let spec = parse_pkgspec(&input)?;
    Ok((input, spec))
}
