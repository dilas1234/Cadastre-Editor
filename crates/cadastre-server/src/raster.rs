//! Raster overviews for the Catalog: enough to drag a georeferenced `.tif` onto the map.
//!
//! GeoTIFFs here are large (the Plateau_Urban ortho is ~125 GB) and there is no GDAL on the
//! target, so we never decode a full-resolution image. Instead we read the georeferencing from
//! the main file's header (cheap — a few tags), then decode a small level from the GDAL `.ovr`
//! pyramid (or a small main image) into a PNG ground-overlay the browser can place by its corner
//! coordinates. Everything is pure-Rust (`tiff` + `png`); no C/GDAL, so it works in the .exe too.

use std::fs::File;
use std::io::{Error, ErrorKind, Result};
use std::path::{Path, PathBuf};

use tiff::decoder::{Decoder, DecodingResult, Limits};
use tiff::tags::Tag;
use tiff::ColorType;

/// Longest side we are willing to *decode* from a pyramid level. Levels larger than this are
/// skipped; a raster with no level at/under it is reported as not previewable (extent only).
const DECODE_CAP: u32 = 12_000;

/// Georeferencing + previewability for one raster, read from its header.
#[derive(Clone, Debug)]
pub struct RasterInfo {
    pub width: u32,
    pub height: u32,
    /// EPSG code from the GeoKeys, if present (our orthos are 32632).
    pub epsg: Option<u32>,
    /// `[minx, miny, maxx, maxy]` in the raster's own CRS.
    pub bbox: [f64; 4],
    /// Whether a sibling `.ovr` pyramid exists.
    pub has_overview: bool,
    /// Whether we can render an overview PNG within [`DECODE_CAP`].
    pub decodable: bool,
}

fn tiff_err(e: tiff::TiffError) -> Error {
    Error::new(ErrorKind::InvalidData, format!("tiff: {e}"))
}

/// The GDAL external pyramid for `main` (e.g. `sftas.tif` -> `sftas.tif.ovr`).
fn ovr_path(main: &Path) -> PathBuf {
    let mut s = main.as_os_str().to_os_string();
    s.push(".ovr");
    PathBuf::from(s)
}

fn open(path: &Path) -> Result<Decoder<File>> {
    Decoder::new(File::open(path)?)
        .map(|d| d.with_limits(Limits::unlimited()))
        .map_err(tiff_err)
}

/// Pixel dimensions of every IFD (pyramid level), largest first.
fn levels(path: &Path) -> Result<Vec<(u32, u32)>> {
    let mut d = open(path)?;
    let mut out = vec![d.dimensions().map_err(tiff_err)?];
    while d.more_images() {
        d.next_image().map_err(tiff_err)?;
        out.push(d.dimensions().map_err(tiff_err)?);
    }
    Ok(out)
}

/// Read georeferencing from the main file's header and decide if a preview is feasible.
pub fn read_info(main: &Path) -> Result<RasterInfo> {
    let mut d = open(main)?;
    let (width, height) = d.dimensions().map_err(tiff_err)?;

    let f64vec = |d: &mut Decoder<File>, tag: u16| -> Option<Vec<f64>> {
        d.find_tag(Tag::from_u16_exhaustive(tag)).ok().flatten().and_then(|v| v.into_f64_vec().ok())
    };
    // ModelPixelScale (33550): [sx, sy, sz]; ModelTiepoint (33922): [i, j, k, X, Y, Z, ...].
    let scale = f64vec(&mut d, 33550);
    let tie = f64vec(&mut d, 33922);
    let bbox = match (scale, tie) {
        (Some(s), Some(t)) if s.len() >= 2 && t.len() >= 6 => {
            let (sx, sy) = (s[0], s[1]);
            let (i, j, x, y) = (t[0], t[1], t[3], t[4]);
            let minx = x - i * sx;
            let maxy = y - j * sy;
            [minx, maxy - height as f64 * sy, minx + width as f64 * sx, maxy]
        }
        _ => return Err(Error::new(ErrorKind::InvalidData, "not georeferenced (no ModelPixelScale/Tiepoint)")),
    };

    // GeoKeyDirectory (34735): [ver, rev, minor, nkeys] then nkeys quads [key, loc, count, value].
    // A ProjectedCSTypeGeoKey (3072) or GeographicTypeGeoKey (2048) stored inline (loc==0) is the EPSG.
    let epsg = f64vec(&mut d, 34735).and_then(|keys| {
        let k: Vec<u32> = keys.iter().map(|v| *v as u32).collect();
        let nkeys = *k.get(3)? as usize;
        let mut found = None;
        for q in 0..nkeys {
            let b = 4 + q * 4;
            match (k.get(b), k.get(b + 1), k.get(b + 3)) {
                (Some(3072), Some(0), Some(code)) if *code != 0 && *code != 32767 => return Some(*code),
                (Some(2048), Some(0), Some(code)) if *code != 0 && *code != 32767 => found = Some(*code),
                _ => {}
            }
        }
        found
    });

    let has_overview = ovr_path(main).exists();
    let src = if has_overview { ovr_path(main) } else { main.to_path_buf() };
    let decodable = levels(&src).map(|ls| ls.iter().any(|(w, h)| (*w).max(*h) <= DECODE_CAP)).unwrap_or(false);

    Ok(RasterInfo { width, height, epsg, bbox, has_overview, decodable })
}

/// Render an overview PNG (RGBA) whose longest side is about `max_px`.
///
/// Picks the smallest pyramid level whose longest side is still >= `max_px` (so the result is
/// crisp after down-sampling) and decodes only that level. Errors if no level is within
/// [`DECODE_CAP`] (caller should fall back to showing just the extent).
pub fn render_overview(main: &Path, max_px: u32) -> Result<(Vec<u8>, u32, u32)> {
    let src = {
        let o = ovr_path(main);
        if o.exists() { o } else { main.to_path_buf() }
    };
    let dims = levels(&src)?;

    // Candidate levels within the decode cap, smallest first.
    let mut idx: Vec<usize> = (0..dims.len()).filter(|&i| dims[i].0.max(dims[i].1) <= DECODE_CAP).collect();
    idx.sort_by_key(|&i| dims[i].0.max(dims[i].1));
    let chosen = idx
        .iter()
        .find(|&&i| dims[i].0.max(dims[i].1) >= max_px)
        .copied()
        .or_else(|| idx.last().copied())
        .ok_or_else(|| Error::new(ErrorKind::Unsupported, "no overview level within decode budget"))?;

    let mut d = open(&src)?;
    for _ in 0..chosen {
        d.next_image().map_err(tiff_err)?;
    }
    let (w, h) = d.dimensions().map_err(tiff_err)?;
    let rgba = read_level_rgba(&mut d, w, h)?;
    let (small, nw, nh) = downsample(&rgba, w, h, max_px);
    let png = encode_png(&small, nw, nh)?;
    Ok((png, nw, nh))
}

/// First element of an integer tag, if present.
fn tag1(d: &mut Decoder<File>, tag: u16) -> Option<u32> {
    d.find_tag(Tag::from_u16_exhaustive(tag)).ok().flatten()
        .and_then(|v| v.into_f64_vec().ok())
        .and_then(|v| v.first().map(|f| *f as u32))
}

/// Decode the *current* IFD into interleaved RGBA8. Uses the crate's `read_image_to_buffer`, which
/// reconstructs every plane (tiling, edges, deflate, predictor all handled); we then interleave.
/// GDAL `.ovr` overviews here are *planar* (R/G/B as separate planes), which the crate lays out
/// consecutively — the old `read_image` returns only the first plane.
fn read_level_rgba(d: &mut Decoder<File>, w: u32, h: u32) -> Result<Vec<u8>> {
    let color = d.colortype().map_err(tiff_err)?;
    let spp = tag1(d, 277).unwrap_or(match color {
        ColorType::RGB(_) => 3,
        ColorType::RGBA(_) | ColorType::CMYK(_) => 4,
        _ => 1,
    }) as usize;
    let bits = tag1(d, 258).unwrap_or(8);
    if bits != 8 {
        return Err(Error::new(ErrorKind::Unsupported, format!("unsupported bit depth {bits} (only 8-bit rasters preview)")));
    }
    let planar = tag1(d, 284).unwrap_or(1) == 2;

    // `read_image` places all tiles/strips correctly (deflate + predictor handled); for *chunky*
    // images it returns interleaved samples, for *planar* ones only the first plane. We render
    // chunky multi-band as colour and planar/single-band as grayscale — the planar colour path in
    // this `tiff` release is unreliable (see read_image_to_buffer).
    let buf = match d.read_image().map_err(tiff_err)? {
        DecodingResult::U8(v) => v,
        _ => return Err(Error::new(ErrorKind::Unsupported, "unsupported sample type (only 8-bit rasters preview)".to_string())),
    };

    let wh = w as usize * h as usize;
    let mut rgba = vec![0u8; wh * 4];
    if !planar && spp >= 3 && buf.len() >= wh * spp {
        for p in 0..wh {
            let s = p * spp;
            rgba[p * 4] = buf[s];
            rgba[p * 4 + 1] = buf[s + 1];
            rgba[p * 4 + 2] = buf[s + 2];
            rgba[p * 4 + 3] = 255;
        }
    } else if buf.len() >= wh {
        // Grayscale, or the first plane of a planar image.
        for p in 0..wh {
            let g = buf[p];
            rgba[p * 4] = g;
            rgba[p * 4 + 1] = g;
            rgba[p * 4 + 2] = g;
            rgba[p * 4 + 3] = 255;
        }
    } else {
        return Err(Error::new(ErrorKind::Unsupported, format!("decoded buffer too small ({} for {wh}px)", buf.len())));
    }
    Ok(rgba)
}

/// Nearest-neighbour down-sample so the longest side is <= `max_px`. (An overview that is already
/// small is returned unchanged.)
fn downsample(src: &[u8], w: u32, h: u32, max_px: u32) -> (Vec<u8>, u32, u32) {
    let long = w.max(h);
    if long <= max_px {
        return (src.to_vec(), w, h);
    }
    let scale = max_px as f64 / long as f64;
    let nw = ((w as f64 * scale).round() as u32).max(1);
    let nh = ((h as f64 * scale).round() as u32).max(1);
    let mut out = vec![0u8; (nw * nh * 4) as usize];
    for y in 0..nh {
        let sy = (((y as f64) / scale) as u32).min(h - 1);
        for x in 0..nw {
            let sx = (((x as f64) / scale) as u32).min(w - 1);
            let si = ((sy * w + sx) * 4) as usize;
            let di = ((y * nw + x) * 4) as usize;
            out[di..di + 4].copy_from_slice(&src[si..si + 4]);
        }
    }
    (out, nw, nh)
}

fn encode_png(rgba: &[u8], w: u32, h: u32) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut buf, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|e| Error::other(format!("png: {e}")))?;
        writer.write_image_data(rgba).map_err(|e| Error::other(format!("png: {e}")))?;
    }
    Ok(buf)
}
