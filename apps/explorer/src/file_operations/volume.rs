//! ドロップの既定動作。ファイルシステムへの問い合わせは転送ワーカー内だけで行う。
use super::TransferKind;
use std::path::Path;

pub(super) fn resolve(kind: TransferKind, source: &Path, destination: &Path) -> TransferKind {
    if kind != TransferKind::Automatic {
        return kind;
    }
    // リンク自身を移動するので、リンク先ではなく元の親フォルダーを調べる。
    let source = source.parent().and_then(identity);
    let destination = identity(destination);
    resolve_identities(source.as_ref(), destination.as_ref())
}

fn resolve_identities<T: Eq>(source: Option<&T>, destination: Option<&T>) -> TransferKind {
    if source.is_some() && source == destination {
        TransferKind::Move
    } else {
        // 不明なボリュームを「同じ」と見なして元を消すことはしない。
        TransferKind::Copy
    }
}

#[cfg(windows)]
fn identity(path: &Path) -> Option<Vec<u16>> {
    use windows_sys::Win32::Storage::FileSystem::{
        GetVolumeNameForVolumeMountPointW, GetVolumePathNameW,
    };
    let resolved = path.canonicalize().ok()?;
    // UNCではGetVolumeNameForVolumeMountPointが使えない。共有ルートを識別子にする。
    if let Some(network) = unc_identity(&resolved) {
        return Some(network.encode_utf16().collect());
    }
    let wide = crate::native_shell::windows_path_wide(&resolved).ok()?;
    let mut root = vec![0u16; 32768];
    if unsafe { GetVolumePathNameW(wide.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
        return None;
    }
    let mut guid = vec![0u16; 64];
    if unsafe {
        GetVolumeNameForVolumeMountPointW(root.as_ptr(), guid.as_mut_ptr(), guid.len() as u32)
    } == 0
    {
        return None;
    }
    guid.truncate(guid.iter().position(|&unit| unit == 0)?);
    Some(guid)
}

#[cfg(windows)]
fn unc_identity(path: &Path) -> Option<String> {
    use std::path::{Component, Prefix};
    match path.components().next()? {
        Component::Prefix(prefix) => match prefix.kind() {
            Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => Some(format!(
                "\\\\{}\\{}",
                server.to_str()?.to_lowercase(),
                share.to_str()?.to_lowercase()
            )),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(unix)]
fn identity(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(std::fs::metadata(path).ok()?.dev())
}

#[cfg(not(any(windows, unix)))]
fn identity(_: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_volumes_resolve_per_source_and_unknown_never_moves() {
        let destination = Some(&1);
        let sources = [Some(&1), Some(&2), None];
        assert_eq!(
            sources.map(|source| resolve_identities(source, destination)),
            [TransferKind::Move, TransferKind::Copy, TransferKind::Copy]
        );
        assert_eq!(resolve_identities::<u8>(None, None), TransferKind::Copy);
    }

    #[test]
    fn release_modifiers_override_volume_default_without_filesystem_reads() {
        assert_eq!(
            TransferKind::from_drop_modifiers(false, false),
            TransferKind::Automatic
        );
        assert_eq!(
            TransferKind::from_drop_modifiers(true, false),
            TransferKind::Copy
        );
        assert_eq!(
            TransferKind::from_drop_modifiers(false, true),
            TransferKind::Move
        );
        assert_eq!(
            TransferKind::from_drop_modifiers(true, true),
            TransferKind::Copy
        );
        let missing = Path::new("does-not-exist");
        assert_eq!(
            resolve(TransferKind::Copy, missing, missing),
            TransferKind::Copy
        );
        assert_eq!(
            resolve(TransferKind::Move, missing, missing),
            TransferKind::Move
        );
    }

    #[cfg(windows)]
    #[test]
    fn unc_identity_distinguishes_shares_and_normalizes_verbatim_prefix() {
        assert_eq!(
            unc_identity(Path::new(r"\\SERVER\Share\folder")),
            unc_identity(Path::new(r"\\?\UNC\server\share\other"))
        );
        assert_ne!(
            unc_identity(Path::new(r"\\server\one\folder")),
            unc_identity(Path::new(r"\\server\two\folder"))
        );
        assert!(unc_identity(Path::new(r"C:\folder")).is_none());
    }

    #[test]
    fn actual_existing_volume_is_recognized() {
        let directory = std::env::temp_dir();
        assert!(identity(&directory).is_some());
        assert_eq!(
            resolve(
                TransferKind::Automatic,
                &directory.join("source.txt"),
                &directory
            ),
            TransferKind::Move
        );
    }
}
