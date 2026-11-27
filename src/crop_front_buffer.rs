use eframe::glow::{self, HasContext};
use windows::Win32::Foundation::RECT;

fn rgba_layout(w: i32, h: i32) -> Option<(usize, usize)> {
    if w <= 0 || h <= 0 {
        return None;
    }
    let stride = (w as usize).checked_mul(4)?;
    Some((stride, stride.checked_mul(h as usize)?))
}

/// 从当前窗口的默认双缓冲读出上一张已交换的画面。
/// 调用方须保证这个窗口的 GL 表面当前已绑定，且其外框尚未调整。
pub(crate) fn read_front(gl: &glow::Context, hw: i32, hh: i32, raw: &mut Vec<u8>) -> bool {
    let Some((stride, len)) = rgba_layout(hw, hh) else {
        return false;
    };
    let version = gl.version();
    if version.is_embedded || version.major < 3 {
        return false;
    }

    unsafe {
        // GL 的错误标记只能读取并清除。遇到已有错误就直接回退，保留其余标记。
        if gl.get_error() != glow::NO_ERROR {
            return false;
        }
        let pack = gl.get_parameter_i32(glow::PACK_ALIGNMENT);
        let source = gl.get_parameter_i32(glow::READ_BUFFER) as u32;
        let compatible = gl.get_parameter_i32(glow::READ_FRAMEBUFFER_BINDING) == 0
            && gl.get_parameter_i32(glow::DOUBLEBUFFER) != 0
            && gl.get_parameter_i32(glow::PIXEL_PACK_BUFFER_BINDING) == 0
            && gl.get_parameter_i32(glow::PACK_ROW_LENGTH) == 0
            && gl.get_parameter_i32(glow::PACK_SKIP_ROWS) == 0
            && gl.get_parameter_i32(glow::PACK_SKIP_PIXELS) == 0
            && matches!(pack, 1 | 2 | 4 | 8)
            && stride % pack as usize == 0;
        let mut viewport = [0; 4];
        gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
        if gl.get_error() != glow::NO_ERROR || !compatible || viewport != [0, 0, hw, hh] {
            return false;
        }

        if raw.try_reserve_exact(len.saturating_sub(raw.len())).is_err() {
            return false;
        }
        raw.resize(len, 0);
        raw.fill(0);
        gl.read_buffer(glow::FRONT);
        if gl.get_error() != glow::NO_ERROR {
            return false;
        }
        // egui 按预乘色绘制，读取 RGBA 字节即可保留原来的混合结果。
        gl.read_pixels(
            0,
            0,
            hw,
            hh,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(raw)),
        );
        let read_ok = gl.get_error() == glow::NO_ERROR;
        gl.read_buffer(source);
        if gl.get_error() != glow::NO_ERROR || !read_ok {
            return false;
        }
    }

    if !usable_transparency(raw) {
        return false;
    }
    flip_rows(raw, stride);
    true
}

fn usable_transparency(raw: &[u8]) -> bool {
    let mut visible = false;
    let mut transparent = false;
    for pixel in raw.chunks_exact(4) {
        let alpha = pixel[3];
        if pixel[..3].iter().any(|&channel| channel > alpha) {
            return false;
        }
        visible |= alpha > 0;
        transparent |= alpha < u8::MAX;
    }
    visible && transparent
}

fn flip_rows(raw: &mut [u8], stride: usize) {
    let half = raw.len() / stride / 2;
    let (top, bottom) = raw.split_at_mut(half * stride);
    for (first, last) in top.chunks_exact_mut(stride).zip(bottom.rchunks_exact_mut(stride)) {
        first.swap_with_slice(last);
    }
}

/// RECT 使用顶左原点的窗口物理像素。返回剪裁后是否还有可见像素。
pub(crate) fn mask_outside_rect(raw: &mut [u8], w: i32, h: i32, clip: RECT) -> bool {
    let Some((stride, len)) = rgba_layout(w, h) else {
        return false;
    };
    if raw.len() != len {
        return false;
    }
    let left = clip.left.clamp(0, w);
    let right = clip.right.clamp(left, w);
    let top = clip.top.clamp(0, h) as usize;
    let bottom = clip.bottom.clamp(top as i32, h) as usize;
    if left == right || top == bottom {
        raw.fill(0);
        return false;
    }
    let start = left as usize * 4;
    let end = right as usize * 4;
    let mut visible = false;
    for (y, row) in raw.chunks_exact_mut(stride).enumerate() {
        if (top..bottom).contains(&y) {
            row[..start].fill(0);
            row[end..].fill(0);
            visible |= row[start + 3..end].iter().step_by(4).any(|&alpha| alpha != 0);
        } else {
            row.fill(0);
        }
    }
    visible
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flip_rows_leaves_an_odd_middle_row_in_place() {
        let mut raw = [1, 2, 3, 4, 5, 6];
        flip_rows(&mut raw, 2);
        assert_eq!(raw, [5, 6, 3, 4, 1, 2]);
    }

    #[test]
    fn requires_visible_and_transparent_premultiplied_data() {
        assert!(usable_transparency(&[4, 8, 3, 16, 0, 0, 0, 0]));
        assert!(!usable_transparency(&[10, 0, 0, 2]));
        assert!(!usable_transparency(&[0, 0, 0, 0]));
        assert!(!usable_transparency(&[0, 0, 0, 255]));
    }

    #[test]
    fn masks_by_client_coordinates_and_clamps_to_the_bitmap() {
        let mut raw = [7; 12 * 4];
        let clip = RECT { left: -2, top: 1, right: 3, bottom: 10 };
        assert!(mask_outside_rect(&mut raw, 4, 3, clip));
        let expected = [0; 16].into_iter()
            .chain([7; 12]).chain([0; 4])
            .chain([7; 12]).chain([0; 4])
            .collect::<Vec<_>>();
        assert_eq!(raw.as_slice(), expected.as_slice());
    }

    #[test]
    fn an_outside_or_empty_region_removes_all_pixels() {
        let clip = RECT { left: 5, top: 0, right: 6, bottom: 3 };
        let mut raw = [7; 12 * 4];
        assert!(!mask_outside_rect(&mut raw, 4, 3, clip));
        assert_eq!(raw, [0; 12 * 4]);
    }
}
