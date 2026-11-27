use super::{fail, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Cursor, Read},
    path::Path,
    sync::Arc,
};

pub(super) const COVER_FILE: &str = "cover.png";
pub(super) const COVER_LIMIT: u64 = 256 * 1024;
const COVER_WIDTH: u32 = 1920;
const COVER_HEIGHT: u32 = 1080;

#[derive(Clone)]
pub(super) struct CoverPng(Arc<[u8]>);

impl CoverPng {
    pub(super) fn read(path: &Path) -> Result<Self> {
        let file = open_plain_file(path, "cover.png 必须是插件根目录里的普通文件")?;
        let mut bytes = Vec::new();
        file.take(COVER_LIMIT + 1).read_to_end(&mut bytes)?;
        Self::decode(bytes)
    }

    pub(super) fn decode(bytes: Vec<u8>) -> Result<Self> {
        if bytes.len() as u64 > COVER_LIMIT {
            return Err(fail("封面 cover.png 不能超过 256 KiB"));
        }
        if !bytes.ends_with(b"\0\0\0\0IEND\xaeB`\x82") {
            return Err(fail("封面 cover.png 必须是完整的 PNG 图片"));
        }
        let mut reader =
            image::ImageReader::with_format(Cursor::new(&bytes), image::ImageFormat::Png);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(COVER_WIDTH);
        limits.max_image_height = Some(COVER_HEIGHT);
        limits.max_alloc = Some(24 * 1024 * 1024);
        reader.limits(limits);
        reader.decode().map_err(|error| {
            fail(&format!(
                "封面必须是尺寸不超过 {COVER_WIDTH} × {COVER_HEIGHT} 的有效 PNG：{error}"
            ))
        })?;
        Ok(Self(bytes.into()))
    }

    pub(super) fn bytes(&self) -> &[u8] {
        &self.0
    }
}

impl Serialize for CoverPng {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(self.bytes()))
    }
}

impl<'de> Deserialize<'de> for CoverPng {
    fn deserialize<D: serde::Deserializer<'de>>(input: D) -> std::result::Result<Self, D::Error> {
        use serde::de::Error;

        let encoded = String::deserialize(input)?;
        if encoded.len() as u64 > COVER_LIMIT.div_ceil(3) * 4 {
            return Err(Error::custom("插件快照中的封面超过 256 KiB"));
        }
        let bytes = STANDARD.decode(encoded).map_err(Error::custom)?;
        Self::decode(bytes).map_err(Error::custom)
    }
}

pub(super) fn open_plain_file(path: &Path, message: &str) -> Result<fs::File> {
    if !fs::symlink_metadata(path)
        .map_err(|_| fail(message))?
        .is_file()
        || path.canonicalize()?.parent() != path.parent()
    {
        return Err(fail(message));
    }

    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0);
    }
    let file = options.open(path)?;
    if !is_plain_file(&file)? {
        return Err(fail(message));
    }
    Ok(file)
}

#[cfg(windows)]
fn is_plain_file(file: &fs::File) -> Result<bool> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY,
            FILE_ATTRIBUTE_REPARSE_POINT,
        },
    };

    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }?;
    let forbidden = FILE_ATTRIBUTE_DIRECTORY.0 | FILE_ATTRIBUTE_REPARSE_POINT.0;
    Ok(info.dwFileAttributes & forbidden == 0 && info.nNumberOfLinks == 1)
}

#[cfg(unix)]
fn is_plain_file(file: &fs::File) -> Result<bool> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file.metadata()?;
    Ok(metadata.is_file() && metadata.nlink() == 1)
}
