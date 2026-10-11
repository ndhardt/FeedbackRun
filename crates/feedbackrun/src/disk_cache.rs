//! On-disk, versioned RGBA frame cache. Files are written atomically and read
//! directly by Smart Render; they are never imported as footage.

use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::UNIX_EPOCH,
};

use feedbackrun_engine::{FrameBuffer, Rgba};

const MAGIC: &[u8; 8] = b"FBRUN001";
const KEY_SIZE: usize = 13 * 4;
const HEADER_SIZE: usize = MAGIC.len() + KEY_SIZE + 8 + 8 + 4;
const MAX_FRAME_BYTES: usize = 1024 * 1024 * 1024;
static IO_LOCK: Mutex<()> = Mutex::new(());
static TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug)]
pub struct FrameKey {
    pub frame: i32,
    pub revision: i32,
    pub width: usize,
    pub height: usize,
    pub origin_x: i32,
    pub origin_y: i32,
    pub depth: i16,
    pub time_step: i32,
    pub time_scale: u32,
    pub downsample_x_num: i32,
    pub downsample_x_den: u32,
    pub downsample_y_num: i32,
    pub downsample_y_den: u32,
}

impl FrameKey {
    fn pixel_bytes(self) -> io::Result<usize> {
        self.width
            .checked_mul(self.height)
            .and_then(|pixels| pixels.checked_mul(16))
            .filter(|&bytes| bytes <= MAX_FRAME_BYTES)
            .ok_or_else(|| invalid("frame dimensions exceed cache limit"))
    }

    fn bytes(self) -> io::Result<Vec<u8>> {
        self.pixel_bytes()?;
        let width = u32::try_from(self.width).map_err(|_| invalid("invalid width"))?;
        let height = u32::try_from(self.height).map_err(|_| invalid("invalid height"))?;
        let mut bytes = Vec::with_capacity(KEY_SIZE);
        bytes.extend_from_slice(&self.frame.to_le_bytes());
        bytes.extend_from_slice(&self.revision.to_le_bytes());
        bytes.extend_from_slice(&width.to_le_bytes());
        bytes.extend_from_slice(&height.to_le_bytes());
        bytes.extend_from_slice(&self.origin_x.to_le_bytes());
        bytes.extend_from_slice(&self.origin_y.to_le_bytes());
        bytes.extend_from_slice(&i32::from(self.depth).to_le_bytes());
        bytes.extend_from_slice(&self.time_step.to_le_bytes());
        bytes.extend_from_slice(&self.time_scale.to_le_bytes());
        bytes.extend_from_slice(&self.downsample_x_num.to_le_bytes());
        bytes.extend_from_slice(&self.downsample_x_den.to_le_bytes());
        bytes.extend_from_slice(&self.downsample_y_num.to_le_bytes());
        bytes.extend_from_slice(&self.downsample_y_den.to_le_bytes());
        Ok(bytes)
    }

    pub fn path(self, folder: &Path) -> PathBuf {
        folder.join(format!(
            "frame_{}_{}_{}_{}_{}_{}_{}_{}_{}_{}_{}_{}_{}.feedbackrun",
            self.frame,
            self.revision,
            self.width,
            self.height,
            self.origin_x,
            self.origin_y,
            self.depth,
            self.time_step,
            self.time_scale,
            self.downsample_x_num,
            self.downsample_x_den,
            self.downsample_y_num,
            self.downsample_y_den,
        ))
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub fn fingerprint(folder: &Path, key: FrameKey) -> u64 {
    let Ok(metadata) = fs::metadata(key.path(folder)) else {
        return 0;
    };
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok());
    let seconds = modified.map_or(0, |time| time.as_secs());
    let nanos = modified.map_or(0, |time| time.subsec_nanos());
    metadata.len() ^ seconds.rotate_left(17) ^ u64::from(nanos).rotate_left(43)
}

pub fn write(folder: &Path, key: FrameKey, frame: &FrameBuffer) -> io::Result<()> {
    if !folder.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "cache folder is unavailable",
        ));
    }
    let raw_len = key.pixel_bytes()?;
    if frame.width() != key.width || frame.height() != key.height {
        return Err(invalid("cache frame size mismatch"));
    }
    let mut raw = Vec::with_capacity(raw_len);
    for pixel in frame.pixels() {
        for channel in [pixel.r, pixel.g, pixel.b, pixel.a] {
            raw.extend_from_slice(&channel.to_le_bytes());
        }
    }
    let compressed = lz4_flex::block::compress(&raw);
    if compressed.len() > MAX_FRAME_BYTES {
        return Err(invalid("compressed cache frame exceeds limit"));
    }
    let mut content = Vec::with_capacity(HEADER_SIZE + compressed.len());
    content.extend_from_slice(MAGIC);
    content.extend_from_slice(&key.bytes()?);
    content.extend_from_slice(&(raw.len() as u64).to_le_bytes());
    content.extend_from_slice(&(compressed.len() as u64).to_le_bytes());
    content.extend_from_slice(&crc32fast::hash(&raw).to_le_bytes());
    content.extend_from_slice(&compressed);

    let path = key.path(folder);
    let temp = path.with_extension(format!(
        "feedbackrun.tmp.{}.{}",
        std::process::id(),
        TEMP_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let _guard = IO_LOCK
        .lock()
        .map_err(|_| invalid("cache I/O lock poisoned"))?;
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(&content)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, &path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub fn read(folder: &Path, key: FrameKey) -> io::Result<Option<FrameBuffer>> {
    let bytes = {
        let _guard = IO_LOCK
            .lock()
            .map_err(|_| invalid("cache I/O lock poisoned"))?;
        let path = key.path(folder);
        match fs::metadata(&path) {
            Ok(metadata) if metadata.len() > (HEADER_SIZE + MAX_FRAME_BYTES) as u64 => {
                return Err(invalid("cache file exceeds size limit"));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
            _ => {}
        }
        match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        }
    };
    if bytes.len() < HEADER_SIZE || &bytes[..MAGIC.len()] != MAGIC {
        return Err(invalid("invalid FeedbackRun cache header"));
    }
    if bytes[MAGIC.len()..MAGIC.len() + KEY_SIZE] != key.bytes()? {
        return Err(invalid("cache frame settings do not match"));
    }
    let mut cursor = MAGIC.len() + KEY_SIZE;
    let raw_len = u64::from_le_bytes(bytes[cursor..cursor + 8].try_into().unwrap());
    cursor += 8;
    let compressed_len = u64::from_le_bytes(bytes[cursor..cursor + 8].try_into().unwrap());
    cursor += 8;
    let checksum = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap());
    cursor += 4;
    if raw_len != key.pixel_bytes()? as u64
        || compressed_len > MAX_FRAME_BYTES as u64
        || compressed_len != (bytes.len() - cursor) as u64
    {
        return Err(invalid("invalid cache payload length"));
    }
    let raw = lz4_flex::block::decompress(&bytes[cursor..], raw_len as usize)
        .map_err(|_| invalid("cache decompression failed"))?;
    if raw.len() != raw_len as usize || crc32fast::hash(&raw) != checksum {
        return Err(invalid("cache checksum mismatch"));
    }
    let pixels = raw
        .chunks_exact(16)
        .map(|bytes| {
            Rgba::new(
                f32::from_le_bytes(bytes[0..4].try_into().unwrap()),
                f32::from_le_bytes(bytes[4..8].try_into().unwrap()),
                f32::from_le_bytes(bytes[8..12].try_into().unwrap()),
                f32::from_le_bytes(bytes[12..16].try_into().unwrap()),
            )
        })
        .collect();
    FrameBuffer::from_pixels(key.width, key.height, pixels)
        .map(Some)
        .ok_or_else(|| invalid("cache pixel count mismatch"))
}

pub fn clear(folder: &Path) -> io::Result<usize> {
    let _guard = IO_LOCK
        .lock()
        .map_err(|_| invalid("cache I/O lock poisoned"))?;
    let mut removed = 0;
    for entry in fs::read_dir(folder)? {
        let path = entry?.path();
        if path.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension == "feedbackrun")
            && path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("frame_"))
        {
            fs::remove_file(path)?;
            removed += 1;
        }
    }
    Ok(removed)
}

pub fn latest_revision(folder: &Path) -> io::Result<Option<i32>> {
    let mut latest = None;
    for entry in fs::read_dir(folder)? {
        let path = entry?.path();
        if !path.is_file()
            || path
                .extension()
                .is_none_or(|extension| extension != "feedbackrun")
        {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let mut parts = stem.split('_');
        if parts.next() != Some("frame")
            || parts
                .next()
                .and_then(|frame| frame.parse::<i32>().ok())
                .is_none()
        {
            continue;
        }
        let Some(revision) = parts
            .next()
            .and_then(|revision| revision.parse::<i32>().ok())
        else {
            continue;
        };
        if (0..=1_000_000).contains(&revision) {
            latest = Some(latest.map_or(revision, |old: i32| old.max(revision)));
        }
    }
    Ok(latest)
}
