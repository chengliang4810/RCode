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

pub(crate) fn rotate(path: &Path, writers_finished: bool) -> std::io::Result<bool> {
    // Children own independent append handles, including after a host crash.
    // Copying and then truncating a live file loses every append between the
    // size snapshot and truncate. Only a fully exited process tree is safe.
    if !writers_finished || std::fs::metadata(path)?.len() <= ROTATE_BYTES {
        return Ok(false);
    }
    let mut input = std::fs::File::open(path)?;
    let size = input.metadata()?.len();
    input.seek(SeekFrom::Start(size.saturating_sub(ARCHIVE_BYTES as u64)))?;
    let mut archive = tempfile::NamedTempFile::new_in(path.parent().unwrap_or(Path::new(".")))?;
    std::io::copy(&mut input.take(ARCHIVE_BYTES as u64), &mut archive)?;
    archive.flush()?;
    archive.as_file().sync_all()?;
    archive.persist(path.with_extension("previous.log")).map_err(|error| error.error)?;
    let current = OpenOptions::new().write(true).open(path)?;
    current.set_len(0)?;
    current.sync_all()?;
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
    fn active_rotation_never_truncates_concurrent_appends_and_finished_rotation_archives_the_tail() {
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
        let start = std::sync::Arc::new(std::sync::Barrier::new(2));
        let running = start.clone();
        let child = std::thread::spawn(move || {
            running.wait();
            for index in 0..1024 { writeln!(writer, "append-{index:04}").unwrap(); }
            writer.flush().unwrap();
        });
        start.wait();
        for _ in 0..128 { assert!(!rotate(&path, false).unwrap()); }
        child.join().unwrap();
        let (tail, _) = read(&path, 32 * 1024, true).unwrap();
        for index in 0..1024 { assert!(tail.contains(&format!("append-{index:04}\n"))); }
        assert!(std::fs::metadata(&path).unwrap().len() > ROTATE_BYTES);
        assert!(!path.with_extension("previous.log").exists());
        assert!(preview(&path).unwrap().0.len() <= PREVIEW_BYTES);
        assert!(rotate(&path, true).unwrap());
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
        assert!(preview(&path).unwrap().0.ends_with("append-1023\n"));
        assert_eq!(std::fs::metadata(path.with_extension("previous.log")).unwrap().len(), ARCHIVE_BYTES as u64);
    }
}
