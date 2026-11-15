use super::*;
use ::zip::{
    CompressionMethod, ZipArchive, ZipWriter,
    read::{ArchiveOffset, Config},
    write::SimpleFileOptions,
};

fn open<R: Read + Seek>(mut reader: R) -> Result<ZipArchive<R>, String> {
    let count = preflight(&mut reader)?;
    reader.rewind().map_err(error)?;
    let archive = ZipArchive::with_config(
        Config {
            archive_offset: ArchiveOffset::Known(0),
        },
        reader,
    )
    .map_err(error)?;
    if archive.len() != count {
        return Err("ZIPの項目数が一致しません。重複または破損した項目があります".into());
    }
    Ok(archive)
}

/// Bound ZIP metadata BEFORE ZipArchive builds its central-directory allocation.
/// Accept a single-disk, canonical end record. ZIP64 is supported, not multi-volume ZIPs.
fn preflight(reader: &mut (impl Read + Seek)) -> Result<usize, String> {
    let length = reader.seek(SeekFrom::End(0)).map_err(error)?;
    if length < 22 {
        return Err("ZIPの終端情報がありません".into());
    }
    let tail_len = length.min(65_557) as usize;
    reader
        .seek(SeekFrom::End(-(tail_len as i64)))
        .map_err(error)?;
    let mut tail = vec![0; tail_len];
    reader.read_exact(&mut tail).map_err(error)?;
    let position = (0..=tail_len - 22)
        .rev()
        .find(|&i| {
            tail[i..i + 4] == [0x50, 0x4b, 5, 6]
                && i + 22 + u16_at(&tail, i + 20) as usize == tail_len
        })
        .ok_or("ZIPの終端情報が壊れています")?;
    let end = length - tail_len as u64 + position as u64;
    if u16_at(&tail, position + 4) != 0
        || u16_at(&tail, position + 6) != 0
        || u16_at(&tail, position + 8) != u16_at(&tail, position + 10)
    {
        return Err("分割ZIPには対応していません".into());
    }
    let mut count = u16_at(&tail, position + 10) as u64;
    let mut bytes = u32_at(&tail, position + 12) as u64;
    let mut start = u32_at(&tail, position + 16) as u64;
    let mut directory_end = end;
    if count == u16::MAX as u64 || bytes == u32::MAX as u64 || start == u32::MAX as u64 {
        if end < 20 {
            return Err("ZIP64の終端情報が壊れています".into());
        }
        reader.seek(SeekFrom::Start(end - 20)).map_err(error)?;
        let mut locator = [0; 20];
        reader.read_exact(&mut locator).map_err(error)?;
        if locator[..4] != [0x50, 0x4b, 6, 7]
            || u32_at(&locator, 4) != 0
            || u32_at(&locator, 16) != 1
        {
            return Err("分割・破損したZIP64には対応していません".into());
        }
        let offset = u64_at(&locator, 8);
        if offset.checked_add(56).is_none_or(|n| n > end - 20) {
            return Err("ZIP64の位置が不正です".into());
        }
        reader.seek(SeekFrom::Start(offset)).map_err(error)?;
        let mut record = [0; 56];
        reader.read_exact(&mut record).map_err(error)?;
        let record_len = u64_at(&record, 4);
        if record[..4] != [0x50, 0x4b, 6, 6]
            || record_len < 44
            || record_len > 1024 * 1024
            || offset
                .checked_add(record_len)
                .and_then(|n| n.checked_add(12))
                != Some(end - 20)
            || u32_at(&record, 16) != 0
            || u32_at(&record, 20) != 0
            || u64_at(&record, 24) != u64_at(&record, 32)
        {
            return Err("ZIP64の終端情報が不正です".into());
        }
        count = u64_at(&record, 32);
        bytes = u64_at(&record, 40);
        start = u64_at(&record, 48);
        directory_end = offset;
    }
    if count > MAX_ENTRIES as u64 || bytes > MAX_HEADER_BYTES {
        return Err("ZIPの項目数またはヘッダーが上限を超えています".into());
    }
    if start.checked_add(bytes) != Some(directory_end)
        || count.checked_mul(46).is_none_or(|n| n > bytes)
    {
        return Err("ZIPの一覧情報の範囲が不正です".into());
    }
    reader.seek(SeekFrom::Start(start)).map_err(error)?;
    let mut consumed = 0u64;
    let mut names = 0usize;
    for _ in 0..count {
        if consumed.checked_add(46).is_none_or(|n| n > bytes) {
            return Err("ZIPの一覧が途中で終わっています".into());
        }
        let mut header = [0; 46];
        reader.read_exact(&mut header).map_err(error)?;
        if header[..4] != [0x50, 0x4b, 1, 2] {
            return Err("ZIPの一覧情報が壊れています".into());
        }
        names += u16_at(&header, 28) as usize;
        if names > MAX_NAMES_BYTES {
            return Err("ZIPの項目名が上限（32MiB）を超えています".into());
        }
        let extra =
            u16_at(&header, 28) as u64 + u16_at(&header, 30) as u64 + u16_at(&header, 32) as u64;
        consumed += 46 + extra;
        if consumed > bytes {
            return Err("ZIPの一覧情報が範囲外です".into());
        }
        reader
            .seek(SeekFrom::Current(extra as i64))
            .map_err(error)?;
    }
    if consumed != bytes {
        return Err("ZIPの一覧サイズが一致しません".into());
    }
    Ok(count as usize)
}
fn u16_at(bytes: &[u8], index: usize) -> u16 {
    u16::from_le_bytes(bytes[index..index + 2].try_into().unwrap())
}
fn u32_at(bytes: &[u8], index: usize) -> u32 {
    u32::from_le_bytes(bytes[index..index + 4].try_into().unwrap())
}
fn u64_at(bytes: &[u8], index: usize) -> u64 {
    u64::from_le_bytes(bytes[index..index + 8].try_into().unwrap())
}

pub(super) fn inspect<R: Read + Seek>(
    reader: R,
    cancel: &CancelToken,
) -> Result<Vec<ArchiveEntry>, String> {
    let archive = open(reader)?;
    let mut entries = Vec::with_capacity(archive.len());
    let mut names = 0usize;
    for index in 0..archive.len() {
        check_cancel(cancel)?;
        let file = archive.by_index_data(index).map_err(error)?;
        if file.encrypted() {
            return Err(PASSWORD_UNSUPPORTED.into());
        }
        if !matches!(
            file.compression(),
            CompressionMethod::Stored | CompressionMethod::Deflated | CompressionMethod::Deflate64
        ) {
            return Err(format!(
                "このZIPの圧縮方式には対応していません: {:?}",
                file.compression()
            ));
        }
        if file.is_symlink()
            || file.external_attributes() & 0x400 != 0
            || file
                .unix_mode()
                .is_some_and(|mode| !matches!(mode & 0o170000, 0 | 0o100000 | 0o040000))
        {
            return Err("リンク・特殊ファイルを含むZIPには対応していません".into());
        }
        // zip applies a valid Info-ZIP Unicode Path override before exposing name_raw().
        let raw = file.name_raw();
        let name = if let Ok(name) = std::str::from_utf8(raw) {
            name.to_owned()
        } else if file.flags().as_u16() & 0x800 != 0 {
            return Err("UTF-8の項目名が壊れています".into());
        } else if let Some(name) =
            encoding_rs::SHIFT_JIS.decode_without_bom_handling_and_without_replacement(raw)
        {
            name.into_owned()
        } else {
            file.name().map_err(error)?.into_owned()
        };
        let modified = file.last_modified().and_then(|date| {
            use chrono::{Local, TimeZone};
            Local
                .with_ymd_and_hms(
                    date.year() as i32,
                    date.month() as u32,
                    date.day() as u32,
                    date.hour() as u32,
                    date.minute() as u32,
                    date.second() as u32,
                )
                .single()
                .map(SystemTime::from)
        });
        let path = paths::normalize(&name)?;
        names = names.checked_add(path.len()).ok_or("項目名が長すぎます")?;
        if names > MAX_NAMES_BYTES {
            return Err("圧縮ファイルの項目名が上限（32MiB）を超えています".into());
        }
        entries.push(ArchiveEntry {
            path,
            is_dir: file.is_dir(),
            size: file.size(),
            modified,
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
    if archive.len() != index.entries.len() {
        return Err("ZIPの内容が変更されました".into());
    }
    for (number, entry) in index.entries.iter().enumerate() {
        check_cancel(work.cancel)?;
        if !selected_path(&entry.path, selected) {
            continue;
        }
        work.state.current_name.clone_from(&entry.path);
        let mut reader = archive.by_index(number).map_err(error)?;
        if let Some(mut file) = paths::prepare_output(destination, entry, &mut work.directories)? {
            work.copy(&mut reader, &mut file, entry.size, true)?;
            file.flush().map_err(error)?;
            paths::set_modified(&file, entry.modified)?;
        }
        work.complete_entry();
    }
    Ok(())
}

pub(super) fn create(
    output: File,
    sources: &[SourceEntry],
    work: &mut Work<'_>,
) -> Result<File, String> {
    let mut writer = ZipWriter::new(output);
    for source in sources {
        check_cancel(work.cancel)?;
        work.state.current_name.clone_from(&source.entry.path);
        let mut options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .compression_level(Some(6))
            .large_file(source.entry.size >= u32::MAX as u64);
        if let Some(modified) = source.entry.modified {
            use chrono::{Datelike, Timelike};
            let date: chrono::DateTime<chrono::Local> = modified.into();
            if let Ok(date) = ::zip::DateTime::from_date_and_time(
                date.year().try_into().unwrap_or(0),
                date.month() as u8,
                date.day() as u8,
                date.hour() as u8,
                date.minute() as u8,
                date.second() as u8,
            ) {
                options = options.last_modified_time(date);
            }
        }
        if source.entry.is_dir {
            writer
                .add_directory(format!("{}/", source.entry.path), options)
                .map_err(error)?;
        } else {
            writer
                .start_file(&source.entry.path, options)
                .map_err(error)?;
            let mut reader = open_source(source, work)?;
            io::copy(&mut reader, &mut writer).map_err(error)?;
        }
        work.complete_entry();
    }
    writer.finish().map_err(error)
}
