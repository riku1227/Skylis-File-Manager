//! Discover WSL registrations without starting a distribution or probing its UNC path.
use std::{io, path::PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WslDistribution {
    pub name: String,
    pub path: PathBuf,
}

impl WslDistribution {
    /// Build a UNC root from one validated share-name component. No filesystem I/O.
    pub fn from_name(name: &str) -> Option<Self> {
        if name.is_empty()
            || matches!(name, "." | "..")
            || name.trim() != name
            || name.ends_with('.')
            || name.encode_utf16().count() > 255
            || name.chars().any(|c| {
                c.is_control() || matches!(c, '\\' | '/' | ':' | '<' | '>' | '"' | '|' | '?' | '*')
            })
        {
            return None;
        }
        Some(Self {
            name: name.to_owned(),
            path: PathBuf::from(format!(r"\\wsl.localhost\{name}\")),
        })
    }
}

/// Read registered distributions on a background worker. Missing WSL registration
/// is an empty list; a registry access failure is an error. Invalid registrations
/// are ignored. This never runs `wsl.exe`, connects to UNC, or starts a VM.
///
/// The registry layout follows Microsoft's Windows Terminal WslDistroGenerator:
/// HKCU\Software\Microsoft\Windows\CurrentVersion\Lxss\{GUID}\DistributionName.
/// Keep this OS integration isolated because the layout is not a stable SDK API.
#[cfg(windows)]
pub fn installed_distributions() -> io::Result<Vec<WslDistribution>> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::{
        Foundation::{
            ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_ITEMS, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS,
        },
        System::Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_READ, RRF_RT_REG_SZ, RegCloseKey, RegEnumKeyExW,
            RegGetValueW, RegOpenKeyExW,
        },
    };
    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: this handle was successfully opened and has one owner.
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }
    let registry_path: Vec<u16> = r"Software\Microsoft\Windows\CurrentVersion\Lxss"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let value_name: Vec<u16> = "DistributionName".encode_utf16().chain(Some(0)).collect();
    let mut key = null_mut();
    // SAFETY: input strings are NUL terminated and output handle pointer is valid.
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            registry_path.as_ptr(),
            0,
            KEY_READ,
            &mut key,
        )
    };
    if matches!(status, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) {
        return Ok(Vec::new());
    }
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let key = Key(key);
    let mut names = Vec::new();
    for index in 0..u32::MAX {
        // Registry subkeys are at most 255 UTF-16 characters plus NUL.
        let mut subkey = [0u16; 256];
        let mut length = subkey.len() as u32;
        // SAFETY: all output buffers match their stated lengths; optional outputs
        // are null. The RAII key remains valid for the enumeration/read calls.
        let status = unsafe {
            RegEnumKeyExW(
                key.0,
                index,
                subkey.as_mut_ptr(),
                &mut length,
                null(),
                null_mut(),
                null_mut(),
                null_mut(),
            )
        };
        if status == ERROR_NO_MORE_ITEMS {
            break;
        }
        if status != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        let Ok(subkey_name) = String::from_utf16(&subkey[..length as usize]) else {
            continue;
        };
        if !is_distribution_key(&subkey_name) {
            continue;
        }
        let mut value = [0u16; 256];
        let mut bytes = size_of_val(&value) as u32;
        // SAFETY: value is suitably aligned, bytes is its capacity in bytes,
        // subkey and value_name are NUL terminated. REG_SZ restricts the type.
        let status = unsafe {
            RegGetValueW(
                key.0,
                subkey.as_ptr(),
                value_name.as_ptr(),
                RRF_RT_REG_SZ,
                null_mut(),
                value.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if status != ERROR_SUCCESS || bytes % 2 != 0 || bytes as usize > size_of_val(&value) {
            // A distro may be unregistered while this snapshot is being taken.
            continue;
        }
        if let Some(name) = decode_registry_name(&value[..bytes as usize / 2]) {
            names.push(name);
        }
    }
    Ok(distributions_from_names(names))
}

#[cfg(not(windows))]
pub fn installed_distributions() -> io::Result<Vec<WslDistribution>> {
    Ok(Vec::new())
}

#[cfg(any(windows, test))]
fn distributions_from_names(names: impl IntoIterator<Item = String>) -> Vec<WslDistribution> {
    let mut distros: Vec<_> = names
        .into_iter()
        .filter_map(|name| WslDistribution::from_name(&name))
        .map(|distro| (distro.name.to_lowercase(), distro))
        .collect();
    distros.sort_unstable_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));
    distros.dedup_by(|a, b| a.0 == b.0);
    distros.into_iter().map(|(_, distro)| distro).collect()
}

#[cfg(any(windows, test))]
fn decode_registry_name(units: &[u16]) -> Option<String> {
    let name = String::from_utf16(units.strip_suffix(&[0])?).ok()?;
    WslDistribution::from_name(&name).map(|distro| distro.name)
}

#[cfg(any(windows, test))]
fn is_distribution_key(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() == 38
        && bytes[0] == b'{'
        && bytes[37] == b'}'
        && bytes[1..37].iter().enumerate().all(|(i, c)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                *c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_unc_root_without_touching_the_filesystem() {
        let distro = WslDistribution::from_name("Ubuntu-24.04").unwrap();
        assert_eq!(distro.name, "Ubuntu-24.04");
        assert_eq!(distro.path, PathBuf::from(r"\\wsl.localhost\Ubuntu-24.04\"));
        let distro = WslDistribution::from_name("研究 Linux").unwrap();
        assert_eq!(distro.name, "研究 Linux");
        assert_eq!(distro.path, PathBuf::from(r"\\wsl.localhost\研究 Linux\"));
    }

    #[test]
    fn rejects_empty_names_path_traversal_and_windows_path_metacharacters() {
        for name in [
            "",
            " ",
            ".",
            "..",
            "../Ubuntu",
            r"..\Ubuntu",
            "/Ubuntu",
            r"\Ubuntu",
            "C:",
            "a\0b",
            "a\nb",
            "a*b",
            "a?b",
            "a|b",
            "a<b",
            "a>b",
            "a\"b",
            "Ubuntu ",
            "Ubuntu.",
        ] {
            assert!(
                WslDistribution::from_name(name).is_none(),
                "accepted {name:?}"
            );
        }
        assert!(WslDistribution::from_name(&"x".repeat(256)).is_none());
    }

    #[test]
    fn registrations_are_sorted_deduplicated_and_invalid_names_dropped() {
        let names = ["ubuntu", "Debian", "Ubuntu", "../bad", "Alpine", "研究"];
        let distros = distributions_from_names(names.map(str::to_owned));
        assert_eq!(
            distros.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
            ["Alpine", "Debian", "Ubuntu", "研究"]
        );
    }

    #[test]
    fn registry_strings_preserve_unicode_and_reject_malformed_or_embedded_nul() {
        let name: Vec<u16> = "研究🦀\0".encode_utf16().collect();
        assert_eq!(decode_registry_name(&name), Some("研究🦀".into()));
        assert_eq!(decode_registry_name(&[0xd800, 0]), None);
        assert_eq!(decode_registry_name(&[]), None);
        assert_eq!(decode_registry_name(&[0]), None);
        assert_eq!(decode_registry_name(&[65]), None);
        assert_eq!(decode_registry_name(&[65, 0, 66, 0]), None);
        assert_eq!(decode_registry_name(&[65, 0, 0]), None);
    }

    #[test]
    fn only_guid_registration_subkeys_are_distributions() {
        assert!(is_distribution_key(
            "{5dfc1090-1e8e-4960-a42e-05d2cfba6416}"
        ));
        assert!(is_distribution_key(
            "{ABCDEF12-3456-7890-ABCD-123456789ABC}"
        ));
        for name in [
            "AppxInstallerCache",
            "",
            "DistributionName",
            "{zdfc1090-1e8e-4960-a42e-05d2cfba6416}",
            "{5dfc1090_1e8e-4960-a42e-05d2cfba6416}",
        ] {
            assert!(!is_distribution_key(name));
        }
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "read-only smoke test of the current user's registry"]
    fn registry_discovery_smoke() {
        let distributions = installed_distributions().unwrap();
        for distribution in &distributions {
            assert_eq!(
                WslDistribution::from_name(&distribution.name).as_ref(),
                Some(distribution)
            );
        }
        assert!(
            distributions
                .windows(2)
                .all(|pair| { pair[0].name.to_lowercase() < pair[1].name.to_lowercase() })
        );
        eprintln!(
            "Read {} WSL registration(s) without probing UNC paths",
            distributions.len()
        );
    }
}
