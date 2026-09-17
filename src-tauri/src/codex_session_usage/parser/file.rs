use super::normalize::{newer_usage_info, usage_info_from_line};
use serde_json::Value;
use std::{
    fs,
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom},
    path::Path,
};

const RESUME_CHECK_BYTES: u64 = 64;

/// How far one rollout file has been folded: every complete line before `offset` is already
/// merged into `latest`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FileUsageProgress {
    offset: u64,
    // The bytes right before `offset`. Rollout files only grow, except when session sync
    // rewrites one in place; a rewritten file no longer matches and is folded from the start.
    resume_check: Vec<u8>,
    latest: Option<Value>,
}

/// Folds the lines appended since `previous` and returns the new progress together with the
/// file's latest usage info. A trailing line without a newline counts towards the result but
/// not towards the progress, so a half-written line is read again once it is complete.
pub(crate) fn fold_usage_info_from_file(
    path: &Path,
    previous: Option<FileUsageProgress>,
) -> Result<(FileUsageProgress, Option<Value>), String> {
    fold_usage_info(path, previous)
        .map_err(|err| format!("读取 Codex session 文件失败 {}: {err}", path.display()))
}

fn fold_usage_info(
    path: &Path,
    previous: Option<FileUsageProgress>,
) -> io::Result<(FileUsageProgress, Option<Value>)> {
    let mut file = fs::File::open(path)?;
    let mut progress = match previous {
        Some(previous) if resumes_at(&mut file, &previous)? => previous,
        _ => FileUsageProgress::default(),
    };
    file.seek(SeekFrom::Start(progress.offset))?;

    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    let mut unterminated = None;
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        if line.last() != Some(&b'\n') {
            // May be cut in the middle of a multi-byte character, so invalid UTF-8 here only
            // means the line is incomplete.
            unterminated = std::str::from_utf8(&line)
                .ok()
                .and_then(usage_info_from_line);
            break;
        }
        let text = std::str::from_utf8(&line)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        if let Some(usage_info) = usage_info_from_line(text) {
            progress.latest = newer_usage_info(progress.latest.take(), usage_info);
        }
        progress.offset += line.len() as u64;
    }

    let mut file = reader.into_inner();
    let check_len = progress.offset.min(RESUME_CHECK_BYTES);
    progress.resume_check = vec![0; check_len as usize];
    file.seek(SeekFrom::Start(progress.offset - check_len))?;
    file.read_exact(&mut progress.resume_check)?;

    let latest = match unterminated {
        Some(usage_info) => newer_usage_info(progress.latest.clone(), usage_info),
        None => progress.latest.clone(),
    };
    Ok((progress, latest))
}

fn resumes_at(file: &mut fs::File, previous: &FileUsageProgress) -> io::Result<bool> {
    if file.metadata()?.len() < previous.offset {
        return Ok(false);
    }
    let mut check = vec![0; previous.resume_check.len()];
    file.seek(SeekFrom::Start(previous.offset - check.len() as u64))?;
    file.read_exact(&mut check)?;
    Ok(check == previous.resume_check)
}
