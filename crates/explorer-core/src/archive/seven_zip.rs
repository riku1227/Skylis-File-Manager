use super::*;
use sevenz_rust2::{
    ArchiveReader, ArchiveWriter, EncoderMethod as Method, EntryDecision, NtTime, Password,
    encoder_options::Lzma2Options,
};

fn archive_error(err: sevenz_rust2::Error) -> String {
    match err {
        sevenz_rust2::Error::PasswordRequired | sevenz_rust2::Error::MaybeBadPassword(_) => {
            PASSWORD_UNSUPPORTED.into()
        }
        sevenz_rust2::Error::UnsupportedCompressionMethod(ref method) if method.contains("AES") => {
            PASSWORD_UNSUPPORTED.into()
        }
        _ => error(err),
    }
}

fn open<R: Read + Seek>(reader: R) -> Result<ArchiveReader<R>, String> {
    let mut archive = ArchiveReader::new_with_memory_limit(
        reader,
        Password::empty(),
        (MAX_HEADER_BYTES / 1024) as usize,
    )
    .map_err(archive_error)?;
    archive.set_thread_count(1);
    if archive.archive().files.len() > MAX_ENTRIES {
        return Err("圧縮ファイルの項目数が上限（100,000）を超えています".into());
    }
    // Resolve unsupported/encrypted codecs before creating any extracted files.
    for block in &archive.archive().blocks {
        let mut decoder_memory = 0u64;
        for coder in &block.coders {
            let id = coder.encoder_method_id();
            if id == Method::ID_AES256_SHA256 {
                return Err(PASSWORD_UNSUPPORTED.into());
            }
            if !matches!(
                id,
                Method::ID_COPY
                    | Method::ID_LZMA
                    | Method::ID_LZMA2
                    | Method::ID_BCJ_X86
                    | Method::ID_BCJ_ARM
                    | Method::ID_BCJ_ARM64
                    | Method::ID_BCJ_ARM_THUMB
                    | Method::ID_BCJ_PPC
                    | Method::ID_BCJ_IA64
                    | Method::ID_BCJ_SPARC
                    | Method::ID_BCJ_RISCV
                    | Method::ID_BCJ2
                    | Method::ID_DELTA
            ) {
                return Err(format!("この7zの圧縮方式には対応していません: {id:?}"));
            }
            // The library limit is per decoder, so also bound a complete BCJ2/filter
            // pipeline. Reserve 8MiB per LZMA model (including maximum literal model)
            // and 64KiB per simple filter, in addition to all dictionary allocations.
            let dictionary = match id {
                Method::ID_LZMA => {
                    let bytes: [u8; 4] = coder
                        .properties()
                        .get(1..5)
                        .ok_or("LZMAの設定が壊れています")?
                        .try_into()
                        .map_err(error)?;
                    ((u32::from_le_bytes(bytes) as u64).max(4096) + 15) & !15
                }
                Method::ID_LZMA2 => {
                    let bits = *coder
                        .properties()
                        .first()
                        .ok_or("LZMA2の設定が壊れています")? as u32;
                    if bits > 40 {
                        return Err("LZMA2の辞書設定が壊れています".into());
                    }
                    if bits == 40 {
                        u32::MAX as u64
                    } else {
                        (2 | (bits as u64 & 1)) << (bits / 2 + 11)
                    }
                }
                _ => 0,
            };
            decoder_memory += dictionary
                + if dictionary > 0 {
                    8 * 1024 * 1024
                } else {
                    64 * 1024
                };
            if decoder_memory > MAX_HEADER_BYTES {
                return Err("7zの展開に必要な辞書メモリーが上限（128MiB）を超えています".into());
            }
        }
    }
    Ok(archive)
}

// NtTime's From conversion can panic on untrusted, out-of-range Windows timestamps.
fn system_time(time: NtTime) -> Option<SystemTime> {
    let ticks: u64 = time.into();
    const UNIX_EPOCH_TICKS: u64 = 116_444_736_000_000_000;
    let duration = Duration::from_nanos(ticks.abs_diff(UNIX_EPOCH_TICKS) % 10_000_000 * 100)
        + Duration::from_secs(ticks.abs_diff(UNIX_EPOCH_TICKS) / 10_000_000);
    if ticks >= UNIX_EPOCH_TICKS {
        SystemTime::UNIX_EPOCH.checked_add(duration)
    } else {
        SystemTime::UNIX_EPOCH.checked_sub(duration)
    }
}

pub(super) fn inspect<R: Read + Seek>(
    reader: R,
    cancel: &CancelToken,
) -> Result<Vec<ArchiveEntry>, String> {
    let archive = open(reader)?;
    let mut entries = Vec::with_capacity(archive.archive().files.len());
    let mut names = 0usize;
    for file in &archive.archive().files {
        check_cancel(cancel)?;
        let unix_mode = file.windows_attributes >> 16;
        if file.is_anti_item
            || (file.has_windows_attributes
                && (file.windows_attributes & 0x400 != 0
                    || !matches!(unix_mode & 0o170000, 0 | 0o100000 | 0o040000)))
        {
            return Err("リンク・特殊ファイル・削除項目を含む7zには対応していません".into());
        }
        if file.is_directory && file.has_stream {
            return Err("データを持つフォルダー項目には対応していません".into());
        }
        let path = paths::normalize(&file.name)?;
        names = names.checked_add(path.len()).ok_or("項目名が長すぎます")?;
        if names > MAX_NAMES_BYTES {
            return Err("圧縮ファイルの項目名が上限（32MiB）を超えています".into());
        }
        entries.push(ArchiveEntry {
            path,
            is_dir: file.is_directory,
            size: file.size,
            modified: if file.has_last_modified_date {
                system_time(file.last_modified_date)
            } else {
                None
            },
        });
    }
    Ok(entries)
}

pub(super) fn extract<R: Read + Seek>(
    reader: R,
    index: &ArchiveIndex,
    selected: &[String],
    destination: &Path,
    work: &mut Work<'_>,
) -> Result<(), String> {
    let mut archive = open(reader)?;
    let entries: HashMap<&str, (usize, &ArchiveEntry)> = index
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| (e.path.as_str(), (i, e)))
        .collect();
    let mut seen = vec![false; index.entries.len()];
    archive
        .for_each_entries_with_decision_allowing_surplus(|raw, input| {
            let result = (|| -> Result<(), String> {
                check_cancel(work.cancel)?;
                let name = paths::normalize(&raw.name)?;
                let &(position, entry) = entries
                    .get(name.as_str())
                    .ok_or("7zの内容が変更されました")?;
                if seen[position] || raw.size != entry.size || raw.is_directory != entry.is_dir {
                    return Err("7zの内容が変更されました".into());
                }
                seen[position] = true;
                work.state.current_name.clone_from(&entry.path);
                let selected = selected_path(&entry.path, selected);
                if selected {
                    if let Some(mut output) =
                        paths::prepare_output(destination, entry, &mut work.directories)?
                    {
                        work.copy(input, &mut output, entry.size, true)?;
                        output.flush().map_err(error)?;
                        paths::set_modified(&output, entry.modified)?;
                    } else {
                        work.copy(input, &mut io::sink(), 0, false)?;
                    }
                    work.complete_entry();
                } else {
                    // Solid blocks require decoding earlier members. Explicit draining keeps
                    // cancellation, actual output limits and CRC checks active on skipped members.
                    work.copy(input, &mut io::sink(), entry.size, false)?;
                }
                Ok(())
            })();
            result.map_err(|err| sevenz_rust2::Error::from(io_error(err)))?;
            Ok(EntryDecision::Continue)
        })
        .map_err(archive_error)?;
    if seen.iter().any(|seen| !seen) {
        return Err("7zの項目が途中で終わっています".into());
    }
    Ok(())
}

pub(super) fn create(
    output: File,
    sources: &[SourceEntry],
    work: &mut Work<'_>,
) -> Result<File, String> {
    let mut writer = ArchiveWriter::new(output).map_err(archive_error)?;
    // Each member is its own block: bounded encoder memory and cheap future selected extraction.
    writer.set_content_methods(vec![Lzma2Options::from_level(3).into()]);
    for source in sources {
        check_cancel(work.cancel)?;
        work.state.current_name.clone_from(&source.entry.path);
        work.state.current_source = Some(source.path.clone());
        let mut entry = if source.entry.is_dir {
            sevenz_rust2::ArchiveEntry::new_directory(&source.entry.path)
        } else {
            sevenz_rust2::ArchiveEntry::new_file(&source.entry.path)
        };
        if let Some(time) = source
            .entry
            .modified
            .and_then(|time| NtTime::try_from(time).ok())
        {
            entry.has_last_modified_date = true;
            entry.last_modified_date = time;
        }
        if source.entry.is_dir {
            writer
                .push_archive_entry(entry, None::<io::Empty>)
                .map_err(archive_error)?;
        } else {
            let reader = open_source(source, work)?;
            writer
                .push_archive_entry(entry, Some(reader))
                .map_err(archive_error)?;
        }
        work.complete_entry();
    }
    writer.finish().map_err(error)
}
