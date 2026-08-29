use std::fmt;

use tray_icon::Icon;

const ICON_BYTES: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/mcp.ico"));

#[derive(Debug)]
pub(crate) struct IconError(&'static str);

impl IconError {
    pub(crate) fn backend_rejected() -> Self {
        Self("the tray backend rejected the embedded MCP Panel icon")
    }
}

impl fmt::Display for IconError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for IconError {}

pub(crate) fn load() -> Result<Icon, IconError> {
    let (rgba, width, height) = decode_ico(ICON_BYTES)?;
    Icon::from_rgba(rgba, width, height)
        .map_err(|_| IconError("the embedded MCP Panel icon was rejected by the tray backend"))
}

fn decode_ico(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), IconError> {
    if bytes.len() < 22 || read_u16(bytes, 0)? != 0 || read_u16(bytes, 2)? != 1 {
        return Err(IconError(
            "the embedded MCP Panel icon has an invalid ICO header",
        ));
    }
    if read_u16(bytes, 4)? == 0 {
        return Err(IconError("the embedded MCP Panel icon has no image"));
    }

    let entry = 6;
    let width = u32::from(bytes[entry]).max(1);
    let height = u32::from(bytes[entry + 1]).max(1);
    let bits_per_pixel = read_u16(bytes, entry + 6)?;
    let image_size = usize::try_from(read_u32(bytes, entry + 8)?)
        .map_err(|_| IconError("the embedded MCP Panel icon is too large"))?;
    let image_offset = usize::try_from(read_u32(bytes, entry + 12)?)
        .map_err(|_| IconError("the embedded MCP Panel icon has an invalid offset"))?;

    if bits_per_pixel != 32 || image_size < 40 {
        return Err(IconError(
            "the embedded MCP Panel icon is not a 32-bit bitmap",
        ));
    }
    let image_end = image_offset
        .checked_add(image_size)
        .ok_or(IconError("the embedded MCP Panel icon size overflowed"))?;
    if image_end > bytes.len() || image_offset + 40 > bytes.len() {
        return Err(IconError("the embedded MCP Panel icon is truncated"));
    }

    let dib_width = read_i32(bytes, image_offset + 4)?;
    let dib_height = read_i32(bytes, image_offset + 8)?;
    if dib_width != i32::try_from(width).unwrap_or_default()
        || dib_height != i32::try_from(height.saturating_mul(2)).unwrap_or_default()
    {
        return Err(IconError(
            "the embedded MCP Panel icon dimensions are invalid",
        ));
    }

    let width_usize = usize::try_from(width).map_err(|_| IconError("icon width overflowed"))?;
    let height_usize = usize::try_from(height).map_err(|_| IconError("icon height overflowed"))?;
    let xor_row_bytes = width_usize
        .checked_mul(4)
        .ok_or(IconError("the embedded MCP Panel icon row size overflowed"))?;
    let and_row_bytes = width_usize.checked_add(31).ok_or(IconError(
        "the embedded MCP Panel icon mask size overflowed",
    ))? / 32
        * 4;
    let xor_bytes = xor_row_bytes.checked_mul(height_usize).ok_or(IconError(
        "the embedded MCP Panel icon pixel size overflowed",
    ))?;
    let and_bytes = and_row_bytes
        .checked_mul(height_usize)
        .ok_or(IconError("the embedded MCP Panel icon mask overflowed"))?;
    let required_size = 40usize
        .checked_add(xor_bytes)
        .and_then(|size| size.checked_add(and_bytes))
        .ok_or(IconError("the embedded MCP Panel icon size overflowed"))?;
    if required_size > image_size {
        return Err(IconError("the embedded MCP Panel icon is truncated"));
    }

    let mut rgba = vec![0; xor_bytes];
    let pixel_data = image_offset + 40;
    for source_row in 0..height_usize {
        let destination_row = height_usize - source_row - 1;
        for x in 0..width_usize {
            let source = pixel_data + source_row * xor_row_bytes + x * 4;
            let destination = destination_row * xor_row_bytes + x * 4;
            rgba[destination] = bytes[source + 2];
            rgba[destination + 1] = bytes[source + 1];
            rgba[destination + 2] = bytes[source];
            rgba[destination + 3] = bytes[source + 3];
        }
    }

    let mask_data = pixel_data + xor_bytes;
    for row in 0..height_usize {
        let destination_row = height_usize - row - 1;
        for x in 0..width_usize {
            let mask_byte = bytes[mask_data + row * and_row_bytes + x / 8];
            if mask_byte & (0x80 >> (x % 8)) != 0 {
                rgba[destination_row * xor_row_bytes + x * 4 + 3] = 0;
            }
        }
    }

    Ok((rgba, width, height))
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, IconError> {
    let end = offset
        .checked_add(2)
        .ok_or(IconError("the embedded MCP Panel icon offset overflowed"))?;
    let value = bytes
        .get(offset..end)
        .ok_or(IconError("the embedded MCP Panel icon is truncated"))?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, IconError> {
    let end = offset
        .checked_add(4)
        .ok_or(IconError("the embedded MCP Panel icon offset overflowed"))?;
    let value = bytes
        .get(offset..end)
        .ok_or(IconError("the embedded MCP Panel icon is truncated"))?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn read_i32(bytes: &[u8], offset: usize) -> Result<i32, IconError> {
    Ok(i32::from_le_bytes(read_u32(bytes, offset)?.to_le_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_icon_is_a_valid_32_bit_ico() {
        let (rgba, width, height) = decode_ico(ICON_BYTES).expect("embedded icon should decode");

        assert_eq!((width, height), (32, 32));
        assert_eq!(rgba.len(), 32 * 32 * 4);
        assert!(rgba.chunks_exact(4).any(|pixel| pixel[3] != 0));
    }
}
