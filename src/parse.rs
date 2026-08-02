use miette::Diagnostic;
use thiserror::Error;

use crate::model::PkgSpecFile;

#[derive(Debug, Error, Diagnostic)]
#[error("failed to parse RPMSPEC.toml")]
pub struct ParseError {
    #[source]
    pub inner: toml::de::Error,
    #[source_code]
    pub source_text: String,
}

pub fn parse_rpmspec(input: &str) -> Result<PkgSpecFile, ParseError> {
    toml::from_str(input).map_err(|inner| ParseError {
        inner,
        source_text: input.to_string(),
    })
}
