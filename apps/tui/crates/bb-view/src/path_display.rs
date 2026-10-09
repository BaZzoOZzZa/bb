#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathDisplayMode {
    Compact,
    Full,
}

pub fn file_name_from_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    match normalized.rsplit('/').next() {
        Some(candidate) if !candidate.is_empty() => candidate.to_owned(),
        _ => path.to_owned(),
    }
}

pub fn directory_from_path(path: &str) -> &str {
    match path.rfind('/') {
        Some(index) => &path[..index],
        None => "",
    }
}

pub fn format_timeline_path(path: &str, mode: PathDisplayMode) -> String {
    match mode {
        PathDisplayMode::Compact => file_name_from_path(path),
        PathDisplayMode::Full => path.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_a_portable_file_name() {
        for (path, expected) in [
            ("/tmp/repo/src/index.ts", "index.ts"),
            ("C:\\repo\\docs\\CODE_REVIEW.md", "CODE_REVIEW.md"),
            ("/tmp/repo/", "/tmp/repo/"),
        ] {
            assert_eq!(file_name_from_path(path), expected, "{path}");
        }
    }

    #[test]
    fn splits_directories() {
        assert_eq!(directory_from_path("src/app/main.rs"), "src/app");
        assert_eq!(directory_from_path("main.rs"), "");
        assert_eq!(
            format_timeline_path("src/app/main.rs", PathDisplayMode::Full),
            "src/app/main.rs"
        );
    }
}
