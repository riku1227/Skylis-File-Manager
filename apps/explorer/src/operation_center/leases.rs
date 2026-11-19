use super::JobKind;
use std::path::{Component, Path, PathBuf};

#[derive(Default)]
pub(super) struct PathLease {
    reads: Vec<PathBuf>,
    writes: Vec<PathBuf>,
}

impl PathLease {
    pub(super) fn archive(sources: &[PathBuf], destination: Option<&Path>) -> Self {
        Self {
            reads: sources.iter().map(|path| key(path)).collect(),
            writes: destination.into_iter().map(key).collect(),
        }
    }

    pub(super) fn transfer(kind: JobKind, sources: &[PathBuf], destination: Option<&Path>) -> Self {
        let mut lease = Self::archive(sources, destination);
        if matches!(
            kind,
            JobKind::Move | JobKind::CopyOrMove | JobKind::Trash | JobKind::PermanentDelete
        ) {
            lease.writes.append(&mut lease.reads);
        }
        lease
    }

    pub(super) fn conflicts(&self, other: &Self) -> bool {
        fn overlaps(writes: &[PathBuf], paths: impl Iterator<Item = impl AsRef<Path>>) -> bool {
            paths.into_iter().any(|path| {
                writes.iter().any(|write| {
                    let path = path.as_ref();
                    write.starts_with(path) || path.starts_with(write)
                })
            })
        }
        overlaps(&self.writes, other.writes.iter().chain(&other.reads))
            || overlaps(&other.writes, self.reads.iter())
    }
}

// Lexical keys only. Resolving filesystem aliases here would block job admission on I/O.
fn key(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        if component == Component::ParentDir && normalized.file_name().is_some() {
            normalized.pop();
        } else {
            normalized.push(component.as_os_str());
        }
    }
    #[cfg(windows)]
    {
        use std::path::Prefix;
        let mut parts = normalized.components();
        if let Some(Component::Prefix(prefix)) = parts.next() {
            // Compare the ordinary and extended spellings of drive and UNC paths alike.
            let mut ordinary = match prefix.kind() {
                Prefix::VerbatimDisk(letter) => PathBuf::from(format!("{}:", char::from(letter))),
                Prefix::VerbatimUNC(server, share) => {
                    let mut unc = PathBuf::from(r"\\");
                    unc.push(server);
                    unc.push(share);
                    unc
                }
                _ => PathBuf::from(prefix.as_os_str()),
            };
            ordinary.extend(parts);
            normalized = ordinary;
        }
        normalized = PathBuf::from(normalized.to_string_lossy().to_uppercase());
    }
    normalized
}
