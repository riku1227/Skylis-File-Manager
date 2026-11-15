use super::*;
use crate::CancelToken;
use std::{fs, path::Path};

#[test]
fn zip_and_sevenz_roundtrip_preserves_unicode_empty_directories_and_binary_bytes() {
    for format in [ArchiveFormat::Zip, ArchiveFormat::SevenZip] {
        let tmp = tempfile::tempdir().unwrap();
        let input = tmp.path().join("資料");
        fs::create_dir_all(input.join("空のフォルダー")).unwrap();
        fs::write(input.join("文章.txt"), "こんにちは\n").unwrap();
        fs::write(input.join("binary.bin"), (0..=255).collect::<Vec<u8>>()).unwrap();
        let output = tmp.path().join(if format == ArchiveFormat::Zip {
            "資料.zip"
        } else {
            "資料.7z"
        });
        let cancel = CancelToken::new();
        assert_eq!(
            create(&[input], &output, format, &cancel, |_| {}).unwrap(),
            output
        );
        let index = inspect(&output, &cancel).unwrap();
        assert!(
            index
                .entries
                .iter()
                .any(|e| e.path == "資料/空のフォルダー" && e.is_dir)
        );
        let dest = extract(&index, &[], tmp.path(), &cancel, |_| {}).unwrap();
        assert_eq!(
            fs::read(dest.join("資料/文章.txt")).unwrap(),
            "こんにちは\n".as_bytes()
        );
        assert_eq!(
            fs::read(dest.join("資料/binary.bin")).unwrap(),
            (0..=255).collect::<Vec<u8>>()
        );
        assert!(dest.join("資料/空のフォルダー").is_dir());
    }
}

#[test]
fn detect_supports_case_insensitive_extensions_only() {
    assert_eq!(detect_format(Path::new("x.ZIP")), Some(ArchiveFormat::Zip));
    assert_eq!(
        detect_format(Path::new("x.7z")),
        Some(ArchiveFormat::SevenZip)
    );
    assert_eq!(detect_format(Path::new("x.rar")), None);
}

fn entry(path: &str, is_dir: bool, size: u64) -> ArchiveEntry {
    ArchiveEntry {
        path: path.into(),
        is_dir,
        size,
        modified: None,
    }
}

fn assert_no_stage(path: &Path) {
    assert!(fs::read_dir(path).unwrap().all(|e| {
        !e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".skylis-archive-")
    }));
}

#[test]
fn children_synthesizes_directories_without_prefix_leakage() {
    let index = ArchiveIndex {
        source: "a.zip".into(),
        source_len: 0,
        modified: None,
        entries: vec![
            entry("a/b/one", false, 4),
            entry("a/two", false, 2),
            entry("ab/three", false, 3),
        ],
    };
    assert_eq!(
        children(&index, "a"),
        vec![entry("a/b", true, 0), entry("a/two", false, 2)]
    );
    assert_eq!(
        children(&index, ""),
        vec![entry("a", true, 0), entry("ab", true, 0)]
    );
}

#[test]
fn rejects_unsafe_paths_duplicates_and_expansion_limits() {
    for name in [
        "../escape",
        "/root",
        "C:\\x",
        "a/../b",
        "a//b",
        "a/./b",
        "a:stream",
        "CON.txt",
        "LPT²",
        "trailing.",
        "trailing ",
        "a\0b",
        "a//",
    ] {
        assert!(paths::normalize(name).is_err(), "{name}");
    }
    assert_eq!(paths::normalize("a\\b.txt").unwrap(), "a/b.txt");
    assert!(validate_entries(&[entry("A", false, 1), entry("a", false, 2)]).is_err());
    assert!(validate_entries(&[entry("a", false, 1), entry("a/b", false, 2)]).is_err());
    assert!(validate_entries(&[entry("large", false, MAX_MEMBER_BYTES + 1)]).is_err());
    assert!(
        validate_entries(
            &(0..5)
                .map(|i| entry(&format!("{i}"), false, MAX_MEMBER_BYTES))
                .collect::<Vec<_>>()
        )
        .is_err()
    );
}

#[test]
fn no_overwrite_and_parent_child_selection_is_deduplicated() {
    for format in [ArchiveFormat::Zip, ArchiveFormat::SevenZip] {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("folder");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("file"), b"original").unwrap();
        let archive = source.join(if format == ArchiveFormat::Zip {
            "result.zip"
        } else {
            "result.7z"
        });
        let cancel = CancelToken::new();
        create(
            &[source.clone(), source.join("file"), source.clone()],
            &archive,
            format,
            &cancel,
            |_| {},
        )
        .unwrap();
        let before = fs::read(&archive).unwrap();
        assert!(create(&[source.clone()], &archive, format, &cancel, |_| {}).is_err());
        assert_eq!(before, fs::read(&archive).unwrap());
        let index = inspect(&archive, &cancel).unwrap();
        assert_eq!(index.entries.iter().filter(|e| !e.is_dir).count(), 1);
        let first = extract(&index, &[], tmp.path(), &cancel, |_| {}).unwrap();
        fs::write(first.join("sentinel"), b"keep").unwrap();
        let second = extract(&index, &[], tmp.path(), &cancel, |_| {}).unwrap();
        assert_ne!(first, second);
        assert_eq!(fs::read(first.join("sentinel")).unwrap(), b"keep");
        assert_no_stage(tmp.path());
    }
}

#[test]
fn cancellation_before_work_and_during_stream_cleans_owned_output() {
    for format in [ArchiveFormat::Zip, ArchiveFormat::SevenZip] {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("source");
        fs::write(&source, vec![42u8; 2 * COPY_BUFFER]).unwrap();
        let destination = tmp.path().join(if format == ArchiveFormat::Zip {
            "result.zip"
        } else {
            "result.7z"
        });
        let cancel = CancelToken::new();
        cancel.cancel();
        assert!(
            create(
                std::slice::from_ref(&source),
                &destination,
                format,
                &cancel,
                |_| {}
            )
            .is_err()
        );
        assert!(!destination.exists());
        let cancel = CancelToken::new();
        create(
            std::slice::from_ref(&source),
            &destination,
            format,
            &cancel,
            |_| {},
        )
        .unwrap();
        let index = inspect(&destination, &cancel).unwrap();
        cancel.cancel();
        assert!(extract(&index, &[], tmp.path(), &cancel, |_| {}).is_err());
        assert_no_stage(tmp.path());
    }
    // Deterministic cancellation at the first decoded chunk, independent of machine speed.
    let tmp = tempfile::tempdir().unwrap();
    let stage = paths::Staging::directory(tmp.path()).unwrap();
    let cancel = CancelToken::new();
    let token = cancel.clone();
    let mut callback = move |_| token.cancel();
    let mut work = Work::new(&cancel, &mut callback, Some(2 * COPY_BUFFER as u64));
    work.last_emit = Instant::now() - Duration::from_secs(1);
    let mut source = io::Cursor::new(vec![1u8; 2 * COPY_BUFFER]);
    let mut file = paths::prepare_output(
        &stage.path,
        &entry("partial", false, 2 * COPY_BUFFER as u64),
        &mut work.directories,
    )
    .unwrap()
    .unwrap();
    assert!(
        work.copy(&mut source, &mut file, 2 * COPY_BUFFER as u64, true)
            .is_err()
    );
    drop(file);
    drop(stage);
    assert_no_stage(tmp.path());
}

#[test]
fn write_limit_and_truncation_are_enforced_before_publication() {
    let cancel = CancelToken::new();
    let mut callback = |_| {};
    let mut work = Work::new(&cancel, &mut callback, Some(1));
    let mut output = Vec::new();
    assert!(
        work.copy(&mut io::Cursor::new([1, 2]), &mut output, 1, true)
            .is_err()
    );
    assert!(output.is_empty());
    assert!(
        work.copy(&mut io::Cursor::new([1]), &mut output, 2, true)
            .is_err()
    );
}

#[test]
fn changed_archive_fingerprint_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("test.zip");
    fs::write(&path, raw_zip(&[(b"one", b"data", 0, 0, 0)])).unwrap();
    let cancel = CancelToken::new();
    let index = inspect(&path, &cancel).unwrap();
    fs::write(&path, raw_zip(&[(b"two", b"longer data", 0, 0, 0)])).unwrap();
    assert!(extract(&index, &[], tmp.path(), &cancel, |_| {}).is_err());
    assert_no_stage(tmp.path());
}

#[test]
fn selected_solid_sevenz_decodes_preceding_members_without_extracting_them() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("solid.7z");
    let mut writer = sevenz_rust2::ArchiveWriter::new(File::create(&path).unwrap()).unwrap();
    let names = ["first", "nested/selected", "last"];
    let content: [&[u8]; 3] = [b"not selected", b"selected data", b"not selected either"];
    writer
        .push_archive_entries(
            names
                .iter()
                .map(|name| sevenz_rust2::ArchiveEntry::new_file(name))
                .collect(),
            content
                .iter()
                .map(|data| sevenz_rust2::SourceReader::new(io::Cursor::new(data)))
                .collect(),
        )
        .unwrap();
    drop(writer.finish().unwrap());
    let cancel = CancelToken::new();
    let index = inspect(&path, &cancel).unwrap();
    let result = extract(&index, &["nested".into()], tmp.path(), &cancel, |_| {}).unwrap();
    assert_eq!(
        fs::read(result.join("nested/selected")).unwrap(),
        content[1]
    );
    assert!(!result.join("first").exists());
    assert!(!result.join("last").exists());
}

#[test]
fn zip_cp932_and_unicode_names_survive_inspection_and_extraction() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("日本語.zip");
    let (sjis, _, errors) = encoding_rs::SHIFT_JIS.encode("資料.txt");
    assert!(!errors);
    fs::write(
        &path,
        raw_zip(&[
            (&sjis, b"legacy", 0, 0, 0),
            ("現代.txt".as_bytes(), b"utf8", 0x800, 0, 0),
        ]),
    )
    .unwrap();
    let cancel = CancelToken::new();
    let index = inspect(&path, &cancel).unwrap();
    assert_eq!(
        index
            .entries
            .iter()
            .map(|e| e.path.as_str())
            .collect::<Vec<_>>(),
        ["資料.txt", "現代.txt"]
    );
    let result = extract(&index, &[], tmp.path(), &cancel, |_| {}).unwrap();
    assert_eq!(fs::read(result.join("資料.txt")).unwrap(), b"legacy");
    assert_eq!(fs::read(result.join("現代.txt")).unwrap(), b"utf8");
}

#[test]
fn zip_unicode_path_extra_overrides_legacy_name_and_is_validated() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("unicode.zip");
    for name in ["日本語.txt", "../escape"] {
        let raw_name = b"fallback.txt";
        let mut bytes = raw_zip(&[(raw_name, b"text", 0, 0, 0)]);
        let old_end = bytes.len() - 22;
        let start =
            u32::from_le_bytes(bytes[old_end + 16..old_end + 20].try_into().unwrap()) as usize;
        let old_size = u32::from_le_bytes(bytes[old_end + 12..old_end + 16].try_into().unwrap());
        let mut extra = Vec::new();
        extra.extend(0x7075u16.to_le_bytes());
        extra.extend(((5 + name.len()) as u16).to_le_bytes());
        extra.push(1);
        extra.extend(crc32(raw_name).to_le_bytes());
        extra.extend(name.as_bytes());
        bytes[start + 30..start + 32].copy_from_slice(&(extra.len() as u16).to_le_bytes());
        let extra_len = extra.len();
        bytes.splice(
            start + 46 + raw_name.len()..start + 46 + raw_name.len(),
            extra,
        );
        let end = bytes.len() - 22;
        bytes[end + 12..end + 16].copy_from_slice(&(old_size + extra_len as u32).to_le_bytes());
        fs::write(&path, bytes).unwrap();
        let index = inspect(&path, &CancelToken::new());
        if name.starts_with("..") {
            assert!(index.is_err());
        } else {
            assert_eq!(index.unwrap().entries[0].path, name);
        }
    }
}

#[test]
fn publication_never_replaces_an_existing_file() {
    let tmp = tempfile::tempdir().unwrap();
    let destination = tmp.path().join("keep.zip");
    fs::write(&destination, b"original").unwrap();
    let (stage, mut file) = paths::Staging::file(tmp.path()).unwrap();
    file.write_all(b"replacement").unwrap();
    drop(file);
    assert!(stage.publish_file(&destination).is_err());
    assert_eq!(fs::read(destination).unwrap(), b"original");
    assert_no_stage(tmp.path());
}

fn raw_sevenz_header(header: &[u8]) -> Vec<u8> {
    let mut start = Vec::new();
    start.extend(0u64.to_le_bytes()); // next header begins immediately after signature header
    start.extend((header.len() as u64).to_le_bytes());
    start.extend(crc32(header).to_le_bytes());
    let mut archive = vec![b'7', b'z', 0xbc, 0xaf, 0x27, 0x1c, 0, 4];
    archive.extend(crc32(&start).to_le_bytes());
    archive.extend(start);
    archive.extend(header);
    archive
}

#[test]
fn sevenz_declared_counts_are_rejected_before_allocating_file_records() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("count.7z");
    let count = MAX_ENTRIES as u64 + 1;
    let mut header = vec![1, 5, 0xff]; // Header, FilesInfo, full-width u64 count
    header.extend(count.to_le_bytes());
    // Upstream's old check only compared count with header length; padding defeats that.
    header.resize(count as usize + 20, 0);
    fs::write(&path, raw_sevenz_header(&header)).unwrap();
    let failure = inspect(&path, &CancelToken::new()).unwrap_err();
    assert!(failure.contains("100,000"), "{failure}");
}

#[test]
fn sevenz_parser_budget_is_charged_before_entry_allocation_and_resets_after_failure() {
    let mut header = vec![1, 5, 100]; // 100 ArchiveEntry records exceed a 2KiB metadata budget.
    header.resize(200, 0);
    let failure = sevenz_rust2::ArchiveReader::new_with_memory_limit(
        io::Cursor::new(raw_sevenz_header(&header)),
        sevenz_rust2::Password::empty(),
        2,
    )
    .err()
    .unwrap();
    assert!(
        failure.to_string().contains("allocation budget"),
        "{failure}"
    );
    // A failed parser must not leave a depleted budget on this thread.
    let empty = raw_sevenz_header(&[1, 0]);
    assert!(
        sevenz_rust2::ArchiveReader::new_with_memory_limit(
            io::Cursor::new(empty),
            sevenz_rust2::Password::empty(),
            2
        )
        .is_ok()
    );
}

#[test]
fn sevenz_encrypted_header_is_reported_as_unsupported_password() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("encrypted.7z");
    // EncodedHeader with one AES256SHA256 coder; decoding must reject the password
    // requirement before ever interpreting its encrypted bytes as an empty archive.
    let header = [
        0x17, 6, 0, 1, 9, 1, 0, 7, 11, 1, 0, 1, 0x24, 6, 0xf1, 7, 1, 2, 0, 0, 12, 10, 0, 0,
    ];
    fs::write(&path, raw_sevenz_header(&header)).unwrap();
    let failure = inspect(&path, &CancelToken::new()).unwrap_err();
    assert_eq!(failure, PASSWORD_UNSUPPORTED);
}

#[test]
fn extraction_accepts_only_directories_created_under_the_exact_name() {
    let tmp = tempfile::tempdir().unwrap();
    let stage = paths::Staging::directory(tmp.path()).unwrap();
    let mut created = HashSet::new();
    fs::create_dir(stage.path.join("untracked")).unwrap();
    assert!(
        paths::prepare_output(
            &stage.path,
            &entry("untracked/file", false, 0),
            &mut created
        )
        .is_err()
    );
    paths::prepare_output(&stage.path, &entry("tracked/file", false, 0), &mut created).unwrap();
    assert!(
        paths::prepare_output(&stage.path, &entry("tracked/other", false, 0), &mut created).is_ok()
    );
}

#[cfg(windows)]
#[test]
fn reparse_policy_distinguishes_cloud_hydration_from_named_redirects() {
    for tag in [0xa000_000c, 0xa000_0003, 0xa000_0010] {
        assert!(paths::is_name_surrogate(tag));
    }
    for tag in [
        0,
        0x9000_001a,
        0x9000_f01a,
        0x8000_001e,
        0x8000_0021,
        0x8000_0017,
    ] {
        assert!(!paths::is_name_surrogate(tag));
    }
    for tag in [
        0x9000_001a,
        0x9000_101a,
        0x9000_f01a,
        0x8000_0015,
        0x8000_001e,
        0x8000_0021,
    ] {
        assert!(paths::is_cloud_tag(tag));
    }
    assert!(!paths::is_cloud_tag(0xa000_000c));
    assert!(!paths::is_cloud_tag(0x8000_0017));
    assert!(!paths::named_streams_flag(0)); // FAT/exFAT: absence is capability, not a read error.
    assert!(paths::named_streams_flag(0x0004_0000));
}

#[cfg(windows)]
#[test]
fn raw_win32_paths_use_extended_drive_and_unc_spelling() {
    for (path, expected) in [
        (r"C:\directory\file", r"\\?\C:\directory\file"),
        (r"\\server\share\file", r"\\?\UNC\server\share\file"),
        (r"\\?\C:\directory\file", r"\\?\C:\directory\file"),
    ] {
        let value = paths::native_path(Path::new(path)).unwrap();
        assert_eq!(
            String::from_utf16(&value[..value.len() - 1]).unwrap(),
            expected
        );
    }
}

#[cfg(windows)]
#[test]
fn archive_creation_and_extraction_work_beyond_max_path() {
    let tmp = tempfile::tempdir().unwrap();
    let mut parent = tmp.path().to_path_buf();
    for _ in 0..5 {
        parent.push("long-folder-segment-with-more-than-forty-characters");
    }
    fs::create_dir_all(&parent).unwrap();
    assert!(parent.as_os_str().len() > 260);
    let input = parent.join("input.txt");
    let archive = parent.join("result.zip");
    fs::write(&input, b"long path data").unwrap();
    let cancel = CancelToken::new();
    create(&[input], &archive, ArchiveFormat::Zip, &cancel, |_| {}).unwrap();
    let index = inspect(&archive, &cancel).unwrap();
    let output = extract(&index, &[], &parent, &cancel, |_| {}).unwrap();
    assert_eq!(
        fs::read(output.join("input.txt")).unwrap(),
        b"long path data"
    );
}

#[test]
fn zip_rejects_encryption_unsupported_codecs_links_and_traversal() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bad.zip");
    for record in [
        (&b"../escape"[..], &b"data"[..], 0, 0, 0),
        (b"secret", b"data", 1, 0, 0),
        (b"link", b"data", 0, 0o120777 << 16, 0),
        (b"reparse", b"data", 0, 0x400, 0),
        (b"codec", b"data", 0, 0, 99),
    ] {
        fs::write(&path, raw_zip(&[record])).unwrap();
        assert!(
            inspect(&path, &CancelToken::new()).is_err(),
            "{:?}",
            record.0
        );
    }
    assert_no_stage(tmp.path());
}

#[test]
fn corrupt_zip_and_sevenz_never_publish_partial_results() {
    let tmp = tempfile::tempdir().unwrap();
    let cancel = CancelToken::new();
    let zip_path = tmp.path().join("bad.zip");
    let mut bytes = raw_zip(&[(b"name", b"correct contents", 0, 0, 0)]);
    bytes[34] ^= 1; // stored member, after the local header and its name
    fs::write(&zip_path, bytes).unwrap();
    let index = inspect(&zip_path, &cancel).unwrap();
    assert!(extract(&index, &[], tmp.path(), &cancel, |_| {}).is_err());
    assert!(!tmp.path().join("bad").exists());
    assert_no_stage(tmp.path());
    let source = tmp.path().join("source");
    fs::write(&source, vec![99u8; 65_536]).unwrap();
    let seven_path = tmp.path().join("bad.7z");
    create(
        &[source],
        &seven_path,
        ArchiveFormat::SevenZip,
        &cancel,
        |_| {},
    )
    .unwrap();
    let mut bytes = fs::read(&seven_path).unwrap();
    bytes[35] ^= 1;
    fs::write(&seven_path, bytes).unwrap();
    match inspect(&seven_path, &cancel) {
        Err(_) => {}
        Ok(index) => assert!(extract(&index, &[], tmp.path(), &cancel, |_| {}).is_err()),
    }
    assert!(!tmp.path().join("bad").exists());
    assert_no_stage(tmp.path());
}

#[cfg(windows)]
#[test]
fn downloaded_archive_zone_is_preserved_and_oversized_marks_fail_closed() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("download.zip");
    fs::write(
        &path,
        raw_zip(&[(b"nested/one", b"one", 0, 0, 0), (b"two", b"two", 0, 0, 0)]),
    )
    .unwrap();
    let zone = b"[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=https://example.test/download.zip\r\n";
    let mut stream = path.as_os_str().to_os_string();
    stream.push(":Zone.Identifier");
    fs::write(PathBuf::from(&stream), zone).unwrap();
    let cancel = CancelToken::new();
    let index = inspect(&path, &cancel).unwrap();
    let result = extract(&index, &[], tmp.path(), &cancel, |_| {}).unwrap();
    assert_eq!(
        paths::read_zone(&result.join("nested/one"))
            .unwrap()
            .unwrap(),
        zone
    );
    assert_eq!(
        paths::read_zone(&result.join("two")).unwrap().unwrap(),
        zone
    );
    fs::write(PathBuf::from(stream), vec![0; 65_537]).unwrap();
    let index = inspect(&path, &cancel).unwrap();
    assert!(extract(&index, &[], tmp.path(), &cancel, |_| {}).is_err());
    assert_no_stage(tmp.path());
}

// Small independent stored-ZIP fixture builder, permitting deliberately invalid flags and names.
// (name, data, flags, external attributes, compression method)
type RawMember<'a> = (&'a [u8], &'a [u8], u16, u32, u16);
fn raw_zip(members: &[RawMember<'_>]) -> Vec<u8> {
    let mut output = Vec::new();
    let mut central = Vec::new();
    for &(name, data, flags, attrs, method) in members {
        let offset = output.len() as u32;
        let crc = crc32(data);
        output.extend(0x04034b50u32.to_le_bytes());
        for value in [20, flags, method, 0, 0] {
            output.extend(value.to_le_bytes());
        }
        for value in [crc, data.len() as u32, data.len() as u32] {
            output.extend(value.to_le_bytes());
        }
        output.extend((name.len() as u16).to_le_bytes());
        output.extend(0u16.to_le_bytes());
        output.extend(name);
        output.extend(data);
        central.extend(0x02014b50u32.to_le_bytes());
        for value in [0x0314u16, 20, flags, method, 0, 0] {
            central.extend(value.to_le_bytes());
        }
        for value in [crc, data.len() as u32, data.len() as u32] {
            central.extend(value.to_le_bytes());
        }
        for value in [name.len() as u16, 0, 0, 0, 0] {
            central.extend(value.to_le_bytes());
        }
        central.extend(attrs.to_le_bytes());
        central.extend(offset.to_le_bytes());
        central.extend(name);
    }
    let offset = output.len() as u32;
    let length = central.len() as u32;
    output.extend(central);
    output.extend(0x06054b50u32.to_le_bytes());
    for value in [0u16, 0, members.len() as u16, members.len() as u16] {
        output.extend(value.to_le_bytes());
    }
    output.extend(length.to_le_bytes());
    output.extend(offset.to_le_bytes());
    output.extend(0u16.to_le_bytes());
    output
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}
