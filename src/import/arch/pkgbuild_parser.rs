use std::collections::BTreeMap;

use thiserror::Error;

const CAPTURED_FUNCTIONS: &[&str] = &["build", "package", "prepare", "check", "pkgver"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AssignmentValue {
    Scalar(String),
    Array(Vec<String>),
}

impl AssignmentValue {
    pub fn values(&self) -> Vec<&str> {
        match self {
            Self::Scalar(value) => vec![value],
            Self::Array(values) => values.iter().map(String::as_str).collect(),
        }
    }

    pub fn contains_command_substitution(&self) -> bool {
        self.values().into_iter().any(contains_command_substitution)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ParsedPkgbuild {
    pub assignments: BTreeMap<String, AssignmentValue>,
    pub functions: BTreeMap<String, String>,
    pub has_additional_logic: bool,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ParseError {
    #[error("line {line}: unterminated array assignment")]
    UnterminatedArray { line: usize },

    #[error("line {line}: unterminated function body")]
    UnterminatedFunction { line: usize },

    #[error("line {line}: unterminated quoted value")]
    UnterminatedQuote { line: usize },
}

pub fn parse(input: &str) -> Result<ParsedPkgbuild, ParseError> {
    let lines = input.lines().collect::<Vec<_>>();
    let mut parsed = ParsedPkgbuild::default();
    let mut index = 0;

    while index < lines.len() {
        let line_number = index + 1;
        let trimmed = lines[index].trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            index += 1;
            continue;
        }

        if control_block_terminator(trimmed).is_some() {
            parsed.has_additional_logic = true;
            index = skip_control_block(&lines, index);
            continue;
        }

        if let Some(function_name) = function_name(trimmed) {
            let (body, next_index) = capture_function(&lines, index)?;
            if CAPTURED_FUNCTIONS.contains(&function_name) {
                parsed.functions.insert(function_name.to_string(), body);
            } else {
                parsed.has_additional_logic = true;
            }
            index = next_index;
            continue;
        }

        if let Some((name, raw_value)) = assignment(trimmed) {
            if raw_value.trim_start().starts_with('(') {
                let (raw_array, next_index) = capture_array(&lines, index, raw_value)?;
                let values = tokenize_array(&raw_array)
                    .map_err(|_| ParseError::UnterminatedQuote { line: line_number })?;
                parsed
                    .assignments
                    .insert(name.to_string(), AssignmentValue::Array(values));
                index = next_index;
            } else {
                let value = parse_scalar(raw_value)
                    .map_err(|_| ParseError::UnterminatedQuote { line: line_number })?;
                parsed
                    .assignments
                    .insert(name.to_string(), AssignmentValue::Scalar(value));
                index += 1;
            }
            continue;
        }

        parsed.has_additional_logic = true;
        index += 1;
    }

    Ok(parsed)
}

pub fn contains_command_substitution(value: &str) -> bool {
    value.contains("$(") || value.contains('`')
}

fn control_block_terminator(line: &str) -> Option<&'static str> {
    let keyword = line.split_whitespace().next()?;
    match keyword {
        "if" => Some("fi"),
        "case" => Some("esac"),
        "for" | "while" | "until" | "select" => Some("done"),
        _ => None,
    }
}

fn skip_control_block(lines: &[&str], start: usize) -> usize {
    let mut depth = 0_usize;
    for (offset, line) in lines[start..].iter().enumerate() {
        depth += control_block_opener_count(line);
        depth = depth.saturating_sub(control_block_terminator_count(line));
        if depth == 0 {
            return start + offset + 1;
        }
    }
    lines.len()
}

fn control_block_opener_count(line: &str) -> usize {
    ["if", "case", "for", "while", "until", "select"]
        .into_iter()
        .map(|token| control_token_count(line, token))
        .sum()
}

fn control_block_terminator_count(line: &str) -> usize {
    ["fi", "esac", "done"]
        .into_iter()
        .map(|token| control_token_count(line, token))
        .sum()
}

fn control_token_count(line: &str, token: &str) -> usize {
    let mut count = 0;
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;

    for character in line.chars() {
        if escaped {
            escaped = false;
            current.push(character);
            continue;
        }
        if character == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(active_quote) = quote {
            if character == active_quote {
                quote = None;
            }
            continue;
        }
        if matches!(character, '\'' | '"') {
            quote = Some(character);
        } else if character == '#' {
            break;
        } else if character.is_whitespace() || character == ';' {
            if current == token {
                count += 1;
            }
            current.clear();
        } else {
            current.push(character);
        }
    }
    count + usize::from(current == token)
}

fn assignment(line: &str) -> Option<(&str, &str)> {
    let (name, value) = line.split_once('=')?;
    let name = name.trim();
    if name.is_empty()
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return None;
    }
    Some((name, value.trim_start()))
}

fn function_name(line: &str) -> Option<&str> {
    let open = line.find("()")?;
    let name = line[..open].trim();
    if name.is_empty()
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return None;
    }
    let remainder = line[open + 2..].trim_start();
    if remainder.is_empty() || remainder.starts_with('{') {
        Some(name)
    } else {
        None
    }
}

fn capture_array(
    lines: &[&str],
    start: usize,
    first_value: &str,
) -> Result<(String, usize), ParseError> {
    let mut raw = String::new();
    let mut depth = 0_i32;
    let mut quote = None;
    let mut escaped = false;

    for (offset, line) in lines[start..].iter().enumerate() {
        let fragment = if offset == 0 { first_value } else { line };
        if offset > 0 {
            raw.push('\n');
        }
        for character in fragment.chars() {
            if escaped {
                escaped = false;
                raw.push(character);
                continue;
            }
            if character == '\\' && quote != Some('\'') {
                escaped = true;
                raw.push(character);
                continue;
            }
            if let Some(active_quote) = quote {
                if character == active_quote {
                    quote = None;
                }
                raw.push(character);
                continue;
            }
            if matches!(character, '\'' | '"') {
                quote = Some(character);
                raw.push(character);
                continue;
            }
            match character {
                '(' => {
                    depth += 1;
                    if depth > 1 {
                        raw.push(character);
                    }
                }
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok((raw, start + offset + 1));
                    }
                    raw.push(character);
                }
                _ if depth > 0 => raw.push(character),
                _ => {}
            }
        }
    }
    Err(ParseError::UnterminatedArray { line: start + 1 })
}

fn capture_function(lines: &[&str], start: usize) -> Result<(String, usize), ParseError> {
    let mut body = String::new();
    let mut depth = 0_i32;
    let mut started = false;
    let mut quote = None;
    let mut escaped = false;

    for (offset, line) in lines[start..].iter().enumerate() {
        let mut line_body = String::new();
        for character in line.chars() {
            if escaped {
                escaped = false;
                if started {
                    line_body.push(character);
                }
                continue;
            }
            if character == '\\' && quote != Some('\'') {
                escaped = true;
                if started {
                    line_body.push(character);
                }
                continue;
            }
            if let Some(active_quote) = quote {
                if character == active_quote {
                    quote = None;
                }
                if started {
                    line_body.push(character);
                }
                continue;
            }
            if matches!(character, '\'' | '"') {
                quote = Some(character);
                if started {
                    line_body.push(character);
                }
                continue;
            }
            if character == '#' {
                break;
            }
            if character == '{' {
                if started {
                    depth += 1;
                    line_body.push(character);
                } else {
                    started = true;
                    depth = 1;
                }
                continue;
            }
            if character == '}' && started {
                depth -= 1;
                if depth == 0 {
                    if !line_body.trim().is_empty() {
                        if !body.is_empty() {
                            body.push('\n');
                        }
                        body.push_str(line_body.trim_end());
                    }
                    return Ok((body.trim().to_string(), start + offset + 1));
                }
                line_body.push(character);
                continue;
            }
            if started {
                line_body.push(character);
            }
        }
        if started && !line_body.trim().is_empty() {
            if !body.is_empty() {
                body.push('\n');
            }
            body.push_str(line_body.trim_end());
        }
    }
    Err(ParseError::UnterminatedFunction { line: start + 1 })
}

fn parse_scalar(raw: &str) -> Result<String, ()> {
    let raw = strip_inline_comment(raw).trim();
    if let (Some(first), Some(last)) = (raw.chars().next(), raw.chars().next_back()) {
        if matches!(first, '\'' | '"') {
            if first != last {
                return Err(());
            }
            return Ok(raw[1..raw.len() - 1].to_string());
        }
    }
    Ok(raw.to_string())
}

fn strip_inline_comment(raw: &str) -> &str {
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in raw.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(active_quote) = quote {
            if character == active_quote {
                quote = None;
            }
            continue;
        }
        if matches!(character, '\'' | '"') {
            quote = Some(character);
        } else if character == '#' {
            return &raw[..index];
        }
    }
    raw
}

fn tokenize_array(raw: &str) -> Result<Vec<String>, ()> {
    let mut values = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut command_depth = 0_usize;
    let mut backtick_depth = 0_usize;
    let characters = raw.chars().collect::<Vec<_>>();
    let mut index = 0;

    while index < characters.len() {
        let character = characters[index];
        if escaped {
            current.push(character);
            escaped = false;
            index += 1;
            continue;
        }
        if character == '\\' && quote != Some('\'') {
            escaped = true;
            index += 1;
            continue;
        }
        if let Some(active_quote) = quote {
            if character == active_quote {
                quote = None;
            } else {
                current.push(character);
            }
            index += 1;
            continue;
        }
        if matches!(character, '\'' | '"') {
            quote = Some(character);
        } else if character == '$' && characters.get(index + 1) == Some(&'(') {
            command_depth += 1;
            current.push('$');
            current.push('(');
            index += 1;
        } else if character == ')' && command_depth > 0 {
            command_depth -= 1;
            current.push(character);
        } else if character == '`' {
            if backtick_depth == 0 {
                backtick_depth += 1;
            } else {
                backtick_depth -= 1;
            }
            current.push(character);
        } else if character.is_whitespace() && command_depth == 0 && backtick_depth == 0 {
            if !current.is_empty() {
                values.push(std::mem::take(&mut current));
            }
        } else if character == '#' && command_depth == 0 && backtick_depth == 0 {
            while index < characters.len() && characters[index] != '\n' {
                index += 1;
            }
            continue;
        } else {
            current.push(character);
        }
        index += 1;
    }
    if quote.is_some() || backtick_depth > 0 {
        return Err(());
    }
    if !current.is_empty() {
        values.push(current);
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scalars_multiline_arrays_and_opaque_functions() {
        let input = r#"pkgname=sample
pkgdesc="A # literal description"
depends=(
  'openssl>=3'
  "python-click"
)

build() {
  make PREFIX="$pkgdir/usr"
  if true; then
    echo "{opaque}"
  fi
}
"#;
        let parsed = parse(input).unwrap();
        assert_eq!(
            parsed.assignments["pkgname"],
            AssignmentValue::Scalar("sample".to_string())
        );
        assert_eq!(
            parsed.assignments["depends"],
            AssignmentValue::Array(vec!["openssl>=3".to_string(), "python-click".to_string()])
        );
        assert!(parsed.functions["build"].contains("make PREFIX"));
        assert!(parsed.functions["build"].contains("{opaque}"));
    }

    #[test]
    fn tokenizes_backtick_command_substitution_as_one_array_element() {
        let parsed = parse("source=(`git describe --tags`.tar.gz)\n").unwrap();
        assert_eq!(
            parsed.assignments["source"],
            AssignmentValue::Array(vec!["`git describe --tags`.tar.gz".to_string()])
        );
        assert!(parsed.assignments["source"].contains_command_substitution());
    }

    #[test]
    fn preserves_command_substitution_without_evaluating_it() {
        let parsed =
            parse("pkgver=$(git describe --tags)\nsource=(\"$(name --flag).tar.gz\")\n").unwrap();
        assert_eq!(
            parsed.assignments["pkgver"],
            AssignmentValue::Scalar("$(git describe --tags)".to_string())
        );
        assert!(parsed.assignments["pkgver"].contains_command_substitution());
        assert!(parsed.assignments["source"].contains_command_substitution());
    }

    #[test]
    fn captures_dynamic_pkgver_and_marks_unknown_top_level_logic() {
        let parsed =
            parse("pkgver=1.0\npkgver() { printf 1.1; }\nif true; then echo x; fi\n").unwrap();
        assert!(parsed.functions.contains_key("pkgver"));
        assert!(parsed.has_additional_logic);
    }

    #[test]
    fn does_not_treat_assignments_inside_top_level_conditionals_as_static_values() {
        let parsed = parse(
            r#"pkgname=static-name
if command -v tool; then
  pkgname=computed-name
fi
pkgver=1.0
"#,
        )
        .unwrap();
        assert_eq!(
            parsed.assignments["pkgname"],
            AssignmentValue::Scalar("static-name".to_string())
        );
        assert!(parsed.has_additional_logic);
    }

    #[test]
    fn skips_single_line_and_nested_top_level_control_blocks() {
        let parsed = parse(
            "if true; then pkgname=ignored; fi\n\
             if true; then\n\
               if false; then\n\
                 pkgver=ignored\n\
               fi\n\
             fi\n\
             pkgname=sample\n\
             pkgver=1.0\n",
        )
        .unwrap();
        assert_eq!(
            parsed.assignments["pkgname"],
            AssignmentValue::Scalar("sample".to_string())
        );
        assert_eq!(
            parsed.assignments["pkgver"],
            AssignmentValue::Scalar("1.0".to_string())
        );
    }

    #[test]
    fn counts_nested_control_openers_after_other_tokens() {
        let parsed = parse(
            "if true; then\n\
               command && if false; then\n\
                 pkgname=ignored\n\
               fi\n\
             fi\n\
             pkgname=sample\n",
        )
        .unwrap();
        assert_eq!(
            parsed.assignments["pkgname"],
            AssignmentValue::Scalar("sample".to_string())
        );
    }

    #[test]
    fn counts_nested_openers_on_the_initial_control_line() {
        let parsed = parse(
            "if true; then if false; then pkgname=ignored; fi\n\
               pkgver=ignored\n\
             fi\n\
             pkgname=sample\n",
        )
        .unwrap();
        assert_eq!(
            parsed.assignments["pkgname"],
            AssignmentValue::Scalar("sample".to_string())
        );
        assert!(!parsed.assignments.contains_key("pkgver"));
    }

    #[test]
    fn ignores_quoted_and_commented_control_terminators() {
        let parsed = parse(
            "if true; then\n\
               echo 'fi' # fi\n\
             fi\n\
             pkgver=1.0\n",
        )
        .unwrap();
        assert_eq!(
            parsed.assignments["pkgver"],
            AssignmentValue::Scalar("1.0".to_string())
        );
    }

    #[test]
    fn function_comments_cannot_close_the_body() {
        let parsed = parse(
            "build() {\n\
               echo '# quoted }'\n\
               echo before # comment closes }\n\
               echo after\n\
             }\n\
             pkgver=1.0\n",
        )
        .unwrap();
        assert!(parsed.functions["build"].contains("# quoted }"));
        assert!(parsed.functions["build"].contains("echo after"));
        assert!(!parsed.functions["build"].contains("comment closes"));
        assert!(parsed.assignments.contains_key("pkgver"));
    }
}
