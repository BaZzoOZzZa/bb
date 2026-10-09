use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftDir {
    root: PathBuf,
}

pub fn default_dir() -> Option<PathBuf> {
    dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .map(|base| base.join("bb-tui").join("drafts"))
}

fn file_stem(thread_id: &str) -> String {
    let mut stem = String::with_capacity(thread_id.len());
    for byte in thread_id.bytes() {
        if byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-' {
            stem.push(char::from(byte));
        } else {
            stem.push_str(&format!("%{byte:02X}"));
        }
    }
    stem
}

impl DraftDir {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path(&self, thread_id: &str) -> PathBuf {
        self.root.join(format!("{}.json", file_stem(thread_id)))
    }

    pub fn load(&self, thread_id: &str) -> Option<String> {
        fs::read_to_string(self.path(thread_id)).ok()
    }

    pub fn save(&self, thread_id: &str, raw: Option<&str>) -> io::Result<()> {
        let path = self.path(thread_id);
        match raw {
            None => match fs::remove_file(&path) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            },
            Some(raw) => {
                fs::create_dir_all(&self.root)?;
                let temp = path.with_extension("json.tmp");
                fs::write(&temp, raw)?;
                fs::rename(&temp, &path)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_loads_and_removes_drafts() {
        let dir = tempfile::tempdir().unwrap();
        let drafts = DraftDir::new(dir.path().join("nested"));
        assert_eq!(drafts.load("thr_1"), None);
        drafts.save("thr_1", None).unwrap();
        drafts.save("thr_1", Some(r#"{"text":"hi"}"#)).unwrap();
        assert_eq!(drafts.load("thr_1").as_deref(), Some(r#"{"text":"hi"}"#));
        drafts.save("thr_1", None).unwrap();
        assert_eq!(drafts.load("thr_1"), None);
    }

    #[test]
    fn thread_ids_cannot_escape_the_directory() {
        let drafts = DraftDir::new("/drafts");
        assert_eq!(
            drafts.path("../etc/passwd"),
            PathBuf::from("/drafts/%2E%2E%2Fetc%2Fpasswd.json")
        );
    }
}
