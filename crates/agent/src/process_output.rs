//! File-backed command output. Children own append handles so logging survives
//! a lost desktop/pipe reader; previews are bounded independently of disk logs.
use std::{
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};

pub const PREVIEW_BYTES: usize = 8192;
const ROTATE_BYTES: u64 = 64 * 1024 * 1024;
const ARCHIVE_BYTES: usize = 8 * 1024 * 1024;

pub(crate) fn read(path: &Path, limit: usize, tail: bool) -> std::io::Result<(String, bool)> {
    let mut file = std::fs::File::open(path)?;
    let size = file.metadata()?.len();
    let length = size.min(limit as u64) as usize;
    let offset = if tail { size - length as u64 } else { 0 };
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::with_capacity(length);
    file.take(length as u64).read_to_end(&mut bytes)?;
    // A tail can begin inside a UTF-8 code point. Skip continuation bytes only
    // at this artificial boundary; preserve replacement for genuinely bad data.
    let skip = if offset > 0 {
        bytes.iter().take(3).take_while(|b| **b & 0xc0 == 0x80).count()
    } else {
        0
    };
    Ok((
        String::from_utf8_lossy(&bytes[skip..]).into_owned(),
        size > length as u64,
    ))
}

pub(crate) fn preview(path: &Path) -> std::io::Result<(String, bool)> {
    let (current, truncated) = read(path, PREVIEW_BYTES, true)?;
    if truncated || current.len() >= PREVIEW_BYTES { return Ok((current, truncated)); }
    match read(&path.with_extension("previous.log"), PREVIEW_BYTES - current.len(), true) {
        Ok((previous, truncated)) => Ok((previous + &current, truncated)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok((current, false)),
        Err(error) => Err(error),
    }
}

pub(crate) fn rotate(path: &Path) -> std::io::Result<bool> {
    if std::fs::metadata(path)?.len() <= ROTATE_BYTES {
        return Ok(false);
    }
    let mut input = std::fs::File::open(path)?;
    let size = input.metadata()?.len();
    input.seek(SeekFrom::Start(size.saturating_sub(ARCHIVE_BYTES as u64)))?;
    let mut archive = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path.with_extension("previous.log"))?;
    std::io::copy(&mut input.take(ARCHIVE_BYTES as u64), &mut archive)?;
    archive.flush()?;
    // Do not rename an active file: a surviving child must keep writing to the
    // same inode/Windows file object through its existing append handle.
    OpenOptions::new().write(true).open(path)?.set_len(0)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tail_is_bounded_and_skips_split_unicode() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("out.log");
        std::fs::write(&path, "旧的输出\n新的输出").unwrap();
        let (text, truncated) = read(&path, 10, true).unwrap();
        assert!(truncated);
        assert_eq!(text, "的输出");
        assert!(!text.contains('\u{fffd}'));
    }
    #[test]
    fn rotation_keeps_append_handles_and_archives_recent_output() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("out.log");
        let mut writer = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&path)
            .unwrap();
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(ROTATE_BYTES + 1)
            .unwrap();
        writer.write_all(b"old tail").unwrap();
        assert!(rotate(&path).unwrap());
        writer.write_all(b"new output").unwrap();
        assert_eq!(read(&path, 100, true).unwrap().0, "new output");
        assert!(preview(&path).unwrap().0.ends_with("old tailnew output"));
        assert!(
            read(&path.with_extension("previous.log"), 100, true)
                .unwrap()
                .0
                .ends_with("old tail")
        );
    }
}
