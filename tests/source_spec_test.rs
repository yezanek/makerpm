use makerpm::source_spec::{parse_source_entry, SourceEntry};

#[test]
fn local_filename() {
    assert_eq!(
        parse_source_entry("hello-1.0.tar.gz"),
        SourceEntry::Local {
            filename: "hello-1.0.tar.gz".into()
        }
    );
}

#[test]
fn bare_https_url() {
    assert_eq!(
        parse_source_entry("https://example.org/hello-1.0.tar.gz"),
        SourceEntry::Remote {
            filename: "hello-1.0.tar.gz".into(),
            url: "https://example.org/hello-1.0.tar.gz".into()
        }
    );
}

#[test]
fn rename_form() {
    assert_eq!(
        parse_source_entry("hello-1.0.tar.gz::https://example.org/v1.0.tar.gz"),
        SourceEntry::Remote {
            filename: "hello-1.0.tar.gz".into(),
            url: "https://example.org/v1.0.tar.gz".into()
        }
    );
}

#[test]
fn bare_ftp_url_is_remote() {
    assert_eq!(
        parse_source_entry("ftp://example.org/file.tar.gz"),
        SourceEntry::Remote {
            filename: "file.tar.gz".into(),
            url: "ftp://example.org/file.tar.gz".into()
        }
    );
}

#[test]
fn url_trailing_slash_uses_raw_as_filename() {
    assert_eq!(
        parse_source_entry("https://example.org/"),
        SourceEntry::Remote {
            filename: "https://example.org/".into(),
            url: "https://example.org/".into()
        }
    );
}

#[test]
fn non_http_scheme_becomes_local() {
    assert_eq!(
        parse_source_entry("ssh://example.org/file"),
        SourceEntry::Local {
            filename: "ssh://example.org/file".into()
        }
    );
}

#[test]
fn empty_string() {
    assert_eq!(
        parse_source_entry(""),
        SourceEntry::Local {
            filename: "".into()
        }
    );
}

#[test]
fn rename_with_ftp() {
    assert_eq!(
        parse_source_entry("patch::ftp://example.org/p.diff"),
        SourceEntry::Remote {
            filename: "patch".into(),
            url: "ftp://example.org/p.diff".into()
        }
    );
}

#[test]
fn github_archive_url() {
    assert_eq!(
        parse_source_entry("https://github.com/user/repo/archive/refs/tags/v1.0.tar.gz"),
        SourceEntry::Remote {
            filename: "v1.0.tar.gz".into(),
            url: "https://github.com/user/repo/archive/refs/tags/v1.0.tar.gz".into()
        }
    );
}

#[test]
fn bare_http_url() {
    assert_eq!(
        parse_source_entry("http://example.org/data.bin"),
        SourceEntry::Remote {
            filename: "data.bin".into(),
            url: "http://example.org/data.bin".into()
        }
    );
}
