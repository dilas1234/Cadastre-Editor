//! A surveyor's scanned plan (TIFF) for the Checking and Data Verification unit: decoded to a PNG
//! the browser can show, with where the file says it lies (GeoTIFF tags). Nothing is kept.
//!
//! Unlike the Catalog's overviews ([`crate::raster`]), scans come as bytes, in any usual bit depth
//! (1-bit Group 4 plans included), and are averaged down so thin plan lines survive.

use std::io::Cursor;

use tiff::decoder::{ChunkType, Decoder, DecodingResult};
use tiff::tags::Tag;

/// Longest side of the PNG.
const MAX_SIDE: u32 = 4096;
/// Pixels we are willing to decode (an A0 sheet at 400 dpi is about 250 million).
const MAX_PIXELS: u64 = 400_000_000;

/// A decoded scan.
pub struct Picture {
    /// Size of the scan itself, in pixels.
    pub width: u32,
    pub height: u32,
    /// The PNG, scaled down to at most [`MAX_SIDE`].
    pub png: Vec<u8>,
    /// `[a, b, c, d, e, f]`: `X = a·col + b·row + c`, `Y = d·col + e·row + f`, with col/row counted
    /// from the outer corner of the top-left pixel.
    pub transform: Option<[f64; 6]>,
    pub epsg: Option<u32>,
    pub warnings: Vec<String>,
}

/// Set the first page's PhotometricInterpretation to 1 (BlackIsZero) and return what it was. The
/// decoder then hands palette and white-is-zero samples over as stored, and we read them
/// ourselves: it refuses palettes, and inverting 1-bit samples. Classic TIFF only; a BigTIFF is
/// left alone.
fn neutral_photometric(b: &mut [u8]) -> Option<u16> {
    let le = match b.get(..4)? {
        [b'I', b'I', 42, 0] => true,
        [b'M', b'M', 0, 42] => false,
        _ => return None,
    };
    let u16_at = |b: &[u8], i: usize| b.get(i..i + 2).map(|x| if le { u16::from_le_bytes([x[0], x[1]]) } else { u16::from_be_bytes([x[0], x[1]]) });
    let off = b.get(4..8).map(|x| if le { u32::from_le_bytes([x[0], x[1], x[2], x[3]]) } else { u32::from_be_bytes([x[0], x[1], x[2], x[3]]) })? as usize;
    let n = u16_at(b, off)? as usize;
    for k in 0..n {
        let at = off + 2 + k * 12;
        if u16_at(b, at)? == 262 {
            let old = u16_at(b, at + 8)?;
            if matches!(old, 0 | 3) {
                let one = if le { 1u16.to_le_bytes() } else { 1u16.to_be_bytes() };
                b.get_mut(at + 8..at + 10)?.copy_from_slice(&one);
            }
            return Some(old);
        }
    }
    None
}

fn georeference(d: &mut Decoder<Cursor<Vec<u8>>>) -> (Option<[f64; 6]>, Option<u32>) {
    let mut f64s = |tag: u16| d.find_tag(Tag::from_u16_exhaustive(tag)).ok().flatten().and_then(|v| v.into_f64_vec().ok());
    let (scale, tie, matrix) = (f64s(33550), f64s(33922), f64s(34264));
    let keys = d.find_tag(Tag::from_u16_exhaustive(34735)).ok().flatten().and_then(|v| v.into_u16_vec().ok()).unwrap_or_default();
    let key = |id: u16| keys.get(4..).unwrap_or(&[]).chunks_exact(4).find(|q| q[0] == id && q[1] == 0).map(|q| q[3]);
    let mut t = match (scale.as_deref(), tie.as_deref(), matrix.as_deref()) {
        (Some([sx, sy, ..]), Some([i, j, _, x, y, ..]), _) => Some([*sx, 0.0, x - i * sx, 0.0, -sy, y + j * sy]),
        (_, _, Some(m)) if m.len() >= 8 => Some([m[0], m[1], m[3], m[4], m[5], m[7]]),
        _ => None,
    };
    if key(1025) == Some(2) {
        // PixelIsPoint: the tie is the pixel's centre.
        if let Some([a, b, c, dd, e, f]) = t.as_mut() {
            *c -= (*a + *b) / 2.0;
            *f -= (*dd + *e) / 2.0;
        }
    }
    let t = t.filter(|t| t.iter().all(|v| v.is_finite()) && (t[0] * t[4] - t[1] * t[3]).abs() > 0.0);
    let epsg = [3072, 2048].into_iter().filter_map(key).find(|c| *c != 0 && *c != 32767).map(u32::from);
    (t, epsg)
}

/// Running averages for the scaled-down picture.
struct Shrink {
    w: u32,
    h: u32,
    factor: f64,
    sum: Vec<[u32; 4]>, // R, G, B weighted by coverage, and the number of pixels
}

impl Shrink {
    fn new(width: u32, height: u32) -> Self {
        let factor = (f64::from(width.max(height)) / f64::from(MAX_SIDE)).max(1.0);
        let side = |v: u32| ((f64::from(v) / factor).round() as u32).max(1);
        let (w, h) = (side(width), side(height));
        Shrink { w, h, factor, sum: vec![[0; 4]; w as usize * h as usize] }
    }
    fn put(&mut self, x: u32, y: u32, rgb: [u8; 3]) {
        let sx = ((f64::from(x) / self.factor) as u32).min(self.w - 1);
        let sy = ((f64::from(y) / self.factor) as u32).min(self.h - 1);
        let c = &mut self.sum[sy as usize * self.w as usize + sx as usize];
        for k in 0..3 {
            c[k] += u32::from(rgb[k]);
        }
        c[3] += 1;
    }
    fn png(&self) -> Result<Vec<u8>, String> {
        let rgb: Vec<u8> = self.sum.iter().flat_map(|c| [0, 1, 2].map(|k| (c[k] / c[3].max(1)) as u8)).collect();
        let mut out = Vec::new();
        let mut e = png::Encoder::new(&mut out, self.w, self.h);
        e.set_color(png::ColorType::Rgb);
        e.set_depth(png::BitDepth::Eight);
        e.write_header().and_then(|mut w| w.write_image_data(&rgb)).map_err(|e| e.to_string())?;
        Ok(out)
    }
}

/// Decode a TIFF scan (its first page).
pub fn read_tiff(mut bytes: Vec<u8>) -> Result<Picture, String> {
    let photometric = neutral_photometric(&mut bytes);
    let plain = |e: tiff::TiffError| format!("the TIFF could not be read ({e}); save it as an uncompressed, LZW or Group 4 TIFF");
    let mut d = Decoder::new(Cursor::new(bytes)).map_err(plain)?;
    let (width, height) = d.dimensions().map_err(plain)?;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(format!("the TIFF is {width} × {height} pixels, too large to show; scan it at a lower resolution"));
    }
    let (transform, epsg) = georeference(&mut d);
    let mut tag = |t: u16, or: usize| d.find_tag(Tag::from_u16_exhaustive(t)).ok().flatten()
        .and_then(|v| v.into_u64_vec().ok()).and_then(|v| v.first().copied()).map_or(or, |v| v as usize);
    let (bits, spp, planar) = (tag(258, 1), tag(277, 1), tag(284, 1) == 2);
    let photometric = photometric.unwrap_or(tag(262, 1) as u16);
    if !matches!(bits, 1 | 2 | 4 | 8 | 16) || spp == 0 || spp > 16 {
        return Err(format!("the TIFF's pixels ({bits} bits, {spp} samples) are not read; save it as 8-bit colour, grey or black and white"));
    }
    if matches!(photometric, 8..=10) {
        return Err("the TIFF's colours (Lab) are not read; save it as RGB".into());
    }
    let map: Vec<u16> = if photometric == 3 { d.find_tag(Tag::ColorMap).ok().flatten().and_then(|v| v.into_u16_vec().ok()).unwrap_or_default() } else { Vec::new() };
    let colours = 1usize << bits.min(8);
    if photometric == 3 && (bits > 8 || map.len() < 3 * colours) {
        return Err("the TIFF's colour table is missing".into());
    }
    let cmyk = photometric == 5 && spp >= 4;
    let per_pixel = if planar { 1 } else { spp };
    let tiled = matches!(d.get_chunk_type(), ChunkType::Tile);
    let (cw, ch) = d.chunk_dimensions();
    if cw == 0 || ch == 0 {
        return Err("the TIFF is damaged".into());
    }
    let (across, down) = (if tiled { width.div_ceil(cw) } else { 1 }, height.div_ceil(ch));
    let max = (1u32 << bits) - 1;
    let colour = |s: &[u8]| -> [u8; 3] {
        // `s`: the pixel's samples as 8-bit values, except palette indexes.
        match photometric {
            3 => { let i = s[0] as usize; [0, 1, 2].map(|k| (map[k * colours + i] >> 8) as u8) }
            0 => [255 - s[0]; 3],
            _ if cmyk => { let k = 255 - u32::from(s[3]); [0, 1, 2].map(|c| ((255 - u32::from(s[c])) * k / 255) as u8) }
            _ if s.len() >= 3 => [s[0], s[1], s[2]],
            _ => [s[0]; 3],
        }
    };
    let mut shrink = Shrink::new(width, height);
    // Planar colour: one sample of every pixel of a chunk at a time.
    let planes = if planar { spp.min(4) } else { 1 };
    for i in 0..across * down {
        let (tx, ty) = ((i % across) * cw, (i / across) * ch);
        let (vw, vh) = (cw.min(width - tx), ch.min(height - ty));
        let mut rows: Vec<Vec<u8>> = Vec::with_capacity(planes);
        for p in 0..planes as u32 {
            rows.push(match d.read_chunk(p * across * down + i).map_err(plain)? {
                DecodingResult::U8(v) => v,
                DecodingResult::U16(v) => v.iter().map(|x| (x >> 8) as u8).collect(),
                _ => return Err("the TIFF's pixels are not whole numbers; save it as 8-bit".into()),
            });
        }
        // The decoder trims padding on the right; a row ends on a whole byte.
        let depth = if bits == 16 { 8 } else { bits };
        let stride = (vw as usize * per_pixel * depth).div_ceil(8);
        if rows.iter().any(|r| r.len() < stride * vh as usize) {
            return Err("the TIFF is damaged (a part of the picture is short)".into());
        }
        let mut s = vec![0u8; if planar { planes } else { spp }];
        for y in 0..vh as usize {
            for x in 0..vw as usize {
                for (k, slot) in s.iter_mut().enumerate() {
                    let (row, col) = if planar { (&rows[k], x) } else { (&rows[0], x * spp + k) };
                    *slot = if depth == 8 {
                        row[y * stride + col]
                    } else {
                        let bit = col * depth;
                        let v = (u32::from(row[y * stride + bit / 8]) >> (8 - depth - bit % 8)) & max;
                        if photometric == 3 { v as u8 } else { (v * 255 / max) as u8 }
                    };
                }
                shrink.put(tx + x as u32, ty + y as u32, colour(&s));
            }
        }
    }
    let mut warnings = Vec::new();
    if d.more_images() {
        warnings.push("the TIFF has more than one page; the first is shown".into());
    }
    if shrink.factor > 1.0 {
        warnings.push(format!("shown at 1/{:.1} of the scan's resolution", shrink.factor));
    }
    Ok(Picture { width, height, png: shrink.png()?, transform, epsg, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One-strip little-endian TIFF from raw tags.
    fn tiff(w: u32, h: u32, photometric: u32, bits: u32, spp: u32, map: &[u16], geo: bool, data: &[u8]) -> Vec<u8> {
        let mut f = b"II*\0\0\0\0\0".to_vec();
        let mut place = |b: &[u8]| { let at = f.len() as u32; f.extend(b); at };
        let strip = place(data);
        let u16s = |v: &[u16]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<_>>();
        let f64s = |v: &[f64]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<_>>();
        let bps = place(&u16s(&vec![bits as u16; spp.max(3) as usize]));
        let m = place(&u16s(map));
        let sc = place(&f64s(&[0.5, 0.5, 0.0]));
        let ti = place(&f64s(&[0.0, 0.0, 0.0, 493_000.0, 1_083_100.0, 0.0]));
        let ke = place(&u16s(&[1, 1, 0, 2, 1025, 0, 1, 1, 3072, 0, 1, 32632]));
        let mut tags: Vec<(u16, u16, u32, u32)> = vec![(256, 4, 1, w), (257, 4, 1, h),
            if spp > 2 { (258, 3, spp, bps) } else { (258, 3, 1, bits) }, (259, 3, 1, 1), (262, 3, 1, photometric),
            (273, 4, 1, strip), (277, 3, 1, spp), (278, 4, 1, h), (279, 4, 1, data.len() as u32)];
        if !map.is_empty() { tags.push((320, 3, map.len() as u32, m)); }
        if geo { tags.extend([(33550, 12, 3, sc), (33922, 12, 6, ti), (34735, 3, 12, ke)]); }
        let at = f.len() as u32;
        f[4..8].copy_from_slice(&at.to_le_bytes());
        f.extend((tags.len() as u16).to_le_bytes());
        for (t, k, n, v) in tags {
            f.extend(t.to_le_bytes()); f.extend(k.to_le_bytes()); f.extend(n.to_le_bytes()); f.extend(v.to_le_bytes());
        }
        f.extend([0; 4]);
        f
    }
    fn pixels(p: &Picture) -> Vec<[u8; 3]> {
        let mut r = png::Decoder::new(Cursor::new(&p.png)).read_info().unwrap();
        let mut b = vec![0; r.output_buffer_size()];
        r.next_frame(&mut b).unwrap();
        b.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect()
    }
    const W: [u8; 3] = [255; 3];
    const K: [u8; 3] = [0; 3];

    #[test]
    fn black_and_white_grey_palette_and_colour() {
        // 10 pixels wide: rows end on a whole byte.
        let p = read_tiff(tiff(10, 2, 1, 1, 1, &[], false, &[0b1000_0000, 0b0100_0000, 0, 0])).unwrap();
        let px = pixels(&p);
        assert_eq!((px[0], px[1], px[9], px[10]), (W, K, W, K));
        assert_eq!((p.transform, p.epsg), (None, None));
        let px = pixels(&read_tiff(tiff(10, 1, 0, 1, 1, &[], false, &[0b1000_0000, 0b0100_0000])).unwrap());
        assert_eq!((px[0], px[1], px[9]), (K, W, K), "white is zero");
        let mut map = vec![0u16; 768];
        (map[7], map[256 + 7], map[512 + 7]) = (0xFF00, 0x8000, 0x1000);
        assert_eq!(pixels(&read_tiff(tiff(2, 1, 3, 8, 1, &map, false, &[7, 0])).unwrap()), vec![[255, 128, 16], K]);
        assert_eq!(pixels(&read_tiff(tiff(1, 1, 2, 8, 3, &[], false, &[1, 2, 3])).unwrap()), vec![[1, 2, 3]]);
        assert_eq!(pixels(&read_tiff(tiff(2, 1, 1, 4, 1, &[], false, &[0xF0])).unwrap()), vec![W, K]);
    }

    #[test]
    fn a_geotiff_says_where_it_lies() {
        let p = read_tiff(tiff(2, 1, 1, 8, 1, &[], true, &[0, 255])).unwrap();
        assert_eq!(p.transform, Some([0.5, 0.0, 493_000.0, 0.0, -0.5, 1_083_100.0]));
        assert_eq!(p.epsg, Some(32632));
    }

    #[test]
    fn large_scans_are_averaged_down() {
        let (w, h) = (MAX_SIDE * 2, 4);
        let data: Vec<u8> = (0..w * h).map(|i| if i % 2 == 0 { 0 } else { 254 }).collect();
        let p = read_tiff(tiff(w, h, 1, 8, 1, &[], false, &data)).unwrap();
        assert!(pixels(&p).iter().all(|c| *c == [127; 3]));
        assert_eq!((p.width, p.warnings.len()), (w, 1));
    }

    #[test]
    fn damaged_files_are_refused() {
        assert!(read_tiff(b"II*\0".to_vec()).is_err());
        let whole = tiff(10, 2, 1, 1, 1, &[], true, &[1, 2, 3, 4]);
        for n in 0..whole.len() {
            let _ = read_tiff(whole[..n].to_vec());
        }
        assert!(read_tiff(tiff(60_000, 60_000, 1, 1, 1, &[], false, &[0])).err().unwrap().contains("too large"));
    }
}
