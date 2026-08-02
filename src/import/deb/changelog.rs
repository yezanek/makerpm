use thiserror::Error;
use time::format_description::well_known::Rfc2822;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebianChangelogEntry {
    pub package: String,
    pub version: String,
    pub maintainer: String,
    pub date: String,
    pub entries: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebianVersion {
    pub epoch: Option<u32>,
    pub upstream: String,
    pub revision: Option<String>,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ChangelogError {
    #[error("line {line}: invalid Debian changelog header")]
    InvalidHeader { line: usize },

    #[error("line {line}: invalid Debian changelog trailer")]
    InvalidTrailer { line: usize },

    #[error("debian/changelog contains no entries")]
    Empty,

    #[error("invalid Debian version: {0}")]
    InvalidVersion(String),
}

pub fn parse(input: &str) -> Result<Vec<DebianChangelogEntry>, ChangelogError> {
    let lines = input.lines().collect::<Vec<_>>();
    let mut entries = Vec::new();
    let mut index = 0;

    while index < lines.len() {
        while index < lines.len() && lines[index].trim().is_empty() {
            index += 1;
        }
        if index == lines.len() {
            break;
        }

        let header_line = index + 1;
        let (package, version) = parse_header(lines[index])
            .ok_or(ChangelogError::InvalidHeader { line: header_line })?;
        index += 1;
        let mut changes: Vec<String> = Vec::new();
        let mut trailer = None;

        while index < lines.len() {
            let line = lines[index].trim_end_matches('\r');
            if line.trim_start().starts_with("-- ") {
                trailer = Some((index + 1, line));
                index += 1;
                break;
            }
            let trimmed = line.trim_start();
            if let Some(change) = trimmed
                .strip_prefix("* ")
                .or_else(|| trimmed.strip_prefix("- "))
            {
                changes.push(change.trim().to_string());
            } else if !trimmed.is_empty() {
                if let Some(previous) = changes.last_mut() {
                    previous.push(' ');
                    previous.push_str(trimmed);
                }
            }
            index += 1;
        }

        let (trailer_line, trailer) = trailer.ok_or(ChangelogError::InvalidTrailer {
            line: index.max(header_line),
        })?;
        let (maintainer, date) =
            parse_trailer(trailer).ok_or(ChangelogError::InvalidTrailer { line: trailer_line })?;
        entries.push(DebianChangelogEntry {
            package,
            version,
            maintainer: maintainer.to_string(),
            date: normalize_date(date),
            entries: changes,
        });
    }

    if entries.is_empty() {
        return Err(ChangelogError::Empty);
    }
    Ok(entries)
}

pub fn split_version(version: &str) -> Result<DebianVersion, ChangelogError> {
    let (epoch, remainder) = if let Some((epoch, remainder)) = version.split_once(':') {
        let epoch = epoch
            .parse::<u32>()
            .map_err(|_| ChangelogError::InvalidVersion(version.to_string()))?;
        (Some(epoch), remainder)
    } else {
        (None, version)
    };
    if remainder.is_empty() {
        return Err(ChangelogError::InvalidVersion(version.to_string()));
    }
    let (upstream, revision) = match remainder.rsplit_once('-') {
        Some((upstream, revision)) if !upstream.is_empty() && !revision.is_empty() => {
            (upstream, Some(revision.to_string()))
        }
        _ => (remainder, None),
    };
    Ok(DebianVersion {
        epoch,
        upstream: upstream.to_string(),
        revision,
    })
}

fn parse_header(line: &str) -> Option<(String, String)> {
    let open = line.find(" (")?;
    let after_open = open + 2;
    let close = line[after_open..].find(')')? + after_open;
    if !line[close + 1..].trim_start().contains(';') {
        return None;
    }
    Some((
        line[..open].trim().to_string(),
        line[after_open..close].trim().to_string(),
    ))
}

fn parse_trailer(line: &str) -> Option<(&str, &str)> {
    let trailer = line.trim_start().strip_prefix("-- ")?;
    let (maintainer, date) = trailer.rsplit_once("  ")?;
    Some((maintainer.trim(), date.trim()))
}

fn normalize_date(date: &str) -> String {
    time::OffsetDateTime::parse(date, &Rfc2822)
        .map(|parsed| {
            let format = time::macros::format_description!("[year]-[month]-[day]");
            parsed
                .format(format)
                .expect("static ISO date format is valid")
        })
        .unwrap_or_else(|_| date.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHANGELOG: &str = r#"sample (2:1.4.0~rc1-3) unstable; urgency=medium

  * Add the first change.
  * Continue a long change
    on another line.

 -- Jane Doe <jane@example.org>  Sun, 02 Aug 2026 12:34:56 +0200

sample (1.3.0-1) stable; urgency=low

  * Previous release.

 -- John Doe <john@example.org>  Sat, 01 Aug 2026 10:00:00 +0200
"#;

    #[test]
    fn parses_full_history_and_normalizes_dates() {
        let entries = parse(CHANGELOG).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].version, "2:1.4.0~rc1-3");
        assert_eq!(entries[0].date, "2026-08-02");
        assert_eq!(
            entries[0].entries,
            [
                "Add the first change.",
                "Continue a long change on another line."
            ]
        );
        assert_eq!(entries[1].maintainer, "John Doe <john@example.org>");
    }

    #[test]
    fn splits_epoch_upstream_and_revision() {
        assert_eq!(
            split_version("2:1.4.0~rc1-3").unwrap(),
            DebianVersion {
                epoch: Some(2),
                upstream: "1.4.0~rc1".to_string(),
                revision: Some("3".to_string()),
            }
        );
    }
}
