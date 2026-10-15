//! Zip archive members as libzip gives them to ImageMagick (`zip_open`,
//! `zip_fopen`, `zip_fread`): the end of central directory record found
//! from the end of the file, a member found by its exact name in the
//! central directory, its data after the local header, stored or deflated,
//! and checked against the directory's CRC.

use anyhow::{bail, Result};

const END_SIGNATURE: u32 = 0x0605_4b50;
const CENTRAL_SIGNATURE: u32 = 0x0201_4b50;
const LOCAL_SIGNATURE: u32 = 0x0403_4b50;

fn u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
    bytes.get(offset..offset + 2).map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    bytes.get(offset..offset + 4).map(|quad| u32::from_le_bytes([quad[0], quad[1], quad[2], quad[3]]))
}

/// The member named `name`, uncompressed; `None` when the archive has no
/// such member.
pub(crate) fn read_member(archive: &[u8], name: &[u8]) -> Result<Option<Vec<u8>>> {
    // The end record is within the last 64 KiB (its comment) and 22 bytes.
    let lowest = archive.len().saturating_sub(22 + 0xffff);
    let mut end = None;
    let mut position = archive.len().saturating_sub(22) as isize;
    while position >= lowest as isize {
        if u32_at(archive, position as usize) == Some(END_SIGNATURE) {
            end = Some(position as usize);
            break;
        }
        position -= 1;
    }
    let Some(end) = end else {
        bail!("UnableToOpenFile");
    };
    let entries = u16_at(archive, end + 10).unwrap_or(0) as usize;
    let directory_size = u32_at(archive, end + 12).unwrap_or(0) as usize;
    let directory_offset = u32_at(archive, end + 16).unwrap_or(0) as usize;
    if directory_offset.saturating_add(directory_size) > end {
        bail!("UnableToOpenFile");
    }
    let mut cursor = directory_offset;
    for _ in 0..entries {
        if u32_at(archive, cursor) != Some(CENTRAL_SIGNATURE) {
            bail!("UnableToOpenFile");
        }
        let method = u16_at(archive, cursor + 10).unwrap_or(0);
        let crc = u32_at(archive, cursor + 16).unwrap_or(0);
        let compressed_size = u32_at(archive, cursor + 20).unwrap_or(0) as usize;
        let size = u32_at(archive, cursor + 24).unwrap_or(0) as usize;
        let name_length = u16_at(archive, cursor + 28).unwrap_or(0) as usize;
        let extra_length = u16_at(archive, cursor + 30).unwrap_or(0) as usize;
        let comment_length = u16_at(archive, cursor + 32).unwrap_or(0) as usize;
        let local_offset = u32_at(archive, cursor + 42).unwrap_or(0) as usize;
        let entry_name = archive.get(cursor + 46..cursor + 46 + name_length).unwrap_or(&[]);
        cursor += 46 + name_length + extra_length + comment_length;
        if entry_name != name {
            continue;
        }
        if u32_at(archive, local_offset) != Some(LOCAL_SIGNATURE) {
            bail!("UnableToOpenFile");
        }
        let local_name = u16_at(archive, local_offset + 26).unwrap_or(0) as usize;
        let local_extra = u16_at(archive, local_offset + 28).unwrap_or(0) as usize;
        let data_start = local_offset + 30 + local_name + local_extra;
        let Some(data) = archive.get(data_start..data_start + compressed_size) else {
            bail!("UnableToReadImageData");
        };
        let member = match method {
            0 => data.to_vec(),
            8 => {
                let mut inflater = flate2::Decompress::new(false);
                let mut output = Vec::with_capacity(size);
                loop {
                    let consumed = inflater.total_in() as usize;
                    output.reserve(size.max(4096));
                    let status = inflater.decompress_vec(&data[consumed..], &mut output, flate2::FlushDecompress::Finish);
                    match status {
                        Ok(flate2::Status::StreamEnd) => break,
                        Ok(_) if inflater.total_in() as usize == consumed && output.len() == output.capacity() => continue,
                        Ok(_) if inflater.total_in() as usize == consumed => bail!("UnableToReadImageData"),
                        Ok(_) => {}
                        Err(_) => bail!("UnableToReadImageData"),
                    }
                }
                output
            }
            _ => bail!("UnableToReadImageData"),
        };
        if member.len() != size || crc32fast::hash(&member) != crc {
            bail!("UnableToReadImageData");
        }
        return Ok(Some(member));
    }
    Ok(None)
}
