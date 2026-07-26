#[derive(Debug, PartialEq, Eq)]
pub enum SourceEntry {
    Local { filename: String },
    Remote { filename: String, url: String },
}

pub fn parse_source_entry(raw: &str) -> SourceEntry {
    if let Some((filename, url)) = raw.split_once("::") {
        return SourceEntry::Remote {
            filename: filename.to_string(),
            url: url.to_string(),
        };
    }
    if let Ok(url) = url::Url::parse(raw) {
        if matches!(url.scheme(), "http" | "https") {
            let filename = url
                .path_segments()
                .and_then(|mut segs| segs.next_back())
                .filter(|s| !s.is_empty())
                .unwrap_or(raw)
                .to_string();
            return SourceEntry::Remote {
                filename,
                url: raw.to_string(),
            };
        }
    }
    SourceEntry::Local {
        filename: raw.to_string(),
    }
}
