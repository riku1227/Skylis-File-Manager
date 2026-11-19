//! Launch Microsoft Photos at an image so Photos can browse its parent folder.
use std::{fmt::Write as _, os::windows::ffi::OsStrExt, path::Path, ptr};
use windows_sys::Win32::UI::Shell::{
    ASSOCF_INIT_IGNOREUNKNOWN, ASSOCF_NOTRUNCATE, ASSOCSTR_APPID, ASSOCSTR_PROGID,
    AssocQueryStringW,
};

pub(super) fn viewer_uri_if_default(path: &Path) -> Option<String> {
    let extension = path.extension()?;
    let association: Vec<u16> = std::iter::once(b'.' as u16)
        .chain(extension.encode_wide())
        .chain(std::iter::once(0))
        .collect();
    let photos_is_default = [ASSOCSTR_PROGID, ASSOCSTR_APPID].into_iter().any(|kind| {
        let mut buffer = [0u16; 256];
        let mut size = buffer.len() as u32;
        // Both buffers are valid for the synchronous call. Skip incomplete identifiers.
        let status = unsafe {
            AssocQueryStringW(
                ASSOCF_INIT_IGNOREUNKNOWN | ASSOCF_NOTRUNCATE,
                kind,
                association.as_ptr(),
                ptr::null(),
                buffer.as_mut_ptr(),
                &mut size,
            )
        };
        status == 0
            && buffer
                .iter()
                .position(|&unit| unit == 0)
                .and_then(|end| String::from_utf16(&buffer[..end]).ok())
                .is_some_and(|value| is_photos_association(&value))
    });
    if photos_is_default {
        encode_viewer_uri(path)
    } else {
        None
    }
}

fn is_photos_association(value: &str) -> bool {
    let package_or_id = value.split_once('!').map_or(value, |(package, _)| package);
    package_or_id.eq_ignore_ascii_case("Microsoft.Windows.Photos_8wekyb3d8bbwe")
        || value.eq_ignore_ascii_case("AppX43hnxtbyyps62jhe9sqpdzxn1790zetc")
}

fn encode_viewer_uri(path: &Path) -> Option<String> {
    if !path.is_absolute() {
        return None;
    }
    // Encode the full path as one query parameter, including any ampersands or percent signs.
    let mut uri = String::from("ms-photos:viewer?fileName=");
    for byte in path.to_str()?.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                uri.push(char::from(byte));
            }
            _ => write!(uri, "%{byte:02X}").expect("writing to a String cannot fail"),
        }
    }
    Some(uri)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn photos_uri_preserves_unicode_and_query_delimiters_in_the_path() {
        let path = Path::new(r"C:\写真\A & 50% #1.jpg");
        assert_eq!(
            encode_viewer_uri(path).as_deref(),
            Some(
                "ms-photos:viewer?fileName=C%3A%5C%E5%86%99%E7%9C%9F%5CA%20%26%2050%25%20%231.jpg"
            ),
        );
        assert_eq!(encode_viewer_uri(Path::new("local.jpg")), None);
    }

    #[test]
    fn photos_association_matches_only_known_identifiers() {
        assert!(is_photos_association(
            "appx43hnxtbyyps62jhe9sqpdzxn1790zetc"
        ));
        assert!(is_photos_association(
            "Microsoft.Windows.Photos_8wekyb3d8bbwe!App"
        ));
        assert!(!is_photos_association("Microsoft.Windows.Photos.Legacy"));
        assert!(!is_photos_association("Paint.Picture"));
    }
}
