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

/// The extent from a world file beside the raster (`.tfw`, `.tifw`, `.wld`): the old cadastral
/// sheets are plain TIFFs placed this way. Lines: x size, two rotations, y size (negative), then
/// the map position of the centre of the top-left pixel. A rotated sheet is refused, because an
/// overview is placed by its extent and would land in the wrong spot.
fn world_file_bbox(main: &Path, width: u32, height: u32) -> Result<[f64; 4]> {
    let none = || Error::new(ErrorKind::InvalidData, "not georeferenced (no ModelPixelScale/Tiepoint, no world file)");
    let text = ["tfw", "tifw", "wld", "TFW", "TIFW", "WLD"].iter()
        .find_map(|e| std::fs::read_to_string(main.with_extension(e)).ok()).ok_or_else(none)?;
    let v: Vec<f64> = text.split_whitespace().filter_map(|t| t.parse().ok()).collect();
    let [a, d, b, e, c, f] = v[..] else { return Err(none()) };
    if d != 0.0 || b != 0.0 {
        return Err(Error::new(ErrorKind::InvalidData, "rotated world file: not supported"));
    }
    if a <= 0.0 || e >= 0.0 {
        return Err(none());
    }
    let (minx, maxy) = (c - a / 2.0, f - e / 2.0);
    Ok([minx, maxy + height as f64 * e, minx + width as f64 * a, maxy])
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
        _ => world_file_bbox(main, width, height)?,
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
/// `background`: an 8-bit value to show as transparent in every band (ArcMap's "Display Background
/// Value"); when `None`, the file's own NoData value is used, if it has a usable one.
pub fn render_overview(main: &Path, max_px: u32, background: Option<u8>) -> Result<(Vec<u8>, u32, u32)> {
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
    let rgba = read_level_rgba(&mut d, w, h, background.or_else(|| nodata_u8(main)))?;
    let (small, nw, nh) = downsample(&rgba, w, h, max_px);
    let png = encode_png(&small, nw, nh)?;
    Ok((png, nw, nh))
}

/// First element of an integer tag, if present. Uses the integer accessor: `into_f64_vec` fails on
/// a single SHORT value, which made PlanarConfiguration (284) read as absent, so planar colour
/// overviews were decoded as their first band only (grey).
fn tag1(d: &mut Decoder<File>, tag: u16) -> Option<u32> {
    d.find_tag(Tag::from_u16_exhaustive(tag)).ok().flatten()
        .and_then(|v| v.into_u64_vec().ok())
        .and_then(|v| v.first().and_then(|n| u32::try_from(*n).ok()))
}

/// Decode the *current* IFD into interleaved RGBA8.
///
/// * Chunky (interleaved) colour: decoded whole with `read_image`.
/// * Planar colour (GDAL `.ovr` pyramids store R, G and B as separate planes): each band is
///   assembled chunk by chunk with `read_chunk`, which returns one plane per chunk (chunks are
///   ordered plane by plane). Edge chunks are clipped to the image. This avoids `read_image`,
///   which for planar images yields only the first plane, so the ortho previewed as grey.
/// * Single band: grey.
///
/// Pixels equal to `nodata` in every band become transparent (GDAL NoData collars).
fn read_level_rgba(d: &mut Decoder<File>, w: u32, h: u32, nodata: Option<u8>) -> Result<Vec<u8>> {
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
    let wh = w as usize * h as usize;
    let mut rgba = vec![0u8; wh * 4];
    for px in rgba.chunks_exact_mut(4) {
        px[3] = 255;
    }

    if planar && spp >= 3 {
        // Chunk grid of one plane.
        let (cw, chh) = d.chunk_dimensions();
        let (across, per_plane) = match d.get_chunk_type() {
            tiff::decoder::ChunkType::Tile => {
                let across = w.div_ceil(cw);
                (across, across * h.div_ceil(chh))
            }
            tiff::decoder::ChunkType::Strip => (1, h.div_ceil(chh)),
        };
        // R, G, B, and alpha when there is a fourth band.
        for plane in 0..spp.min(4) {
            for i in 0..per_plane {
                let idx = plane as u32 * per_plane + i;
                let data = match d.read_chunk(idx).map_err(tiff_err)? {
                    DecodingResult::U8(v) => v,
                    _ => return Err(Error::new(ErrorKind::Unsupported, "unsupported sample type (only 8-bit rasters preview)")),
                };
                // Row width is the tile width, or what remains of the image in the last column
                // (the library trims right padding for every plane). Row count follows from the
                // data: the library trims bottom padding only for the first plane's chunks, so
                // other planes' bottom tiles arrive full height; place_chunk clips at the edge.
                let (tx, ty) = ((i % across) * cw, (i / across) * chh);
                let stride = match d.get_chunk_type() {
                    tiff::decoder::ChunkType::Tile => cw.min(w - tx),
                    tiff::decoder::ChunkType::Strip => w,
                };
                place_chunk(&mut rgba, (w, h), plane, (tx, ty), stride, &data);
            }
        }
    } else {
        let buf = match d.read_image().map_err(tiff_err)? {
            DecodingResult::U8(v) => v,
            _ => return Err(Error::new(ErrorKind::Unsupported, "unsupported sample type (only 8-bit rasters preview)".to_string())),
        };
        if !planar && spp >= 3 && buf.len() >= wh * spp {
            for p in 0..wh {
                let s = p * spp;
                rgba[p * 4..p * 4 + 3].copy_from_slice(&buf[s..s + 3]);
                if spp >= 4 {
                    rgba[p * 4 + 3] = buf[s + 3];
                }
            }
        } else if buf.len() >= wh {
            // Grayscale (or a planar image with fewer than three bands: its first band).
            for p in 0..wh {
                rgba[p * 4..p * 4 + 3].fill(buf[p]);
            }
        } else {
            return Err(Error::new(ErrorKind::Unsupported, format!("decoded buffer too small ({} for {wh}px)", buf.len())));
        }
    }
    if let Some(nd) = nodata {
        for px in rgba.chunks_exact_mut(4) {
            if px[0] == nd && px[1] == nd && px[2] == nd {
                px[3] = 0;
            }
        }
    }
    Ok(rgba)
}

/// Copy one decoded single-band chunk (row stride `stride`, origin `(tx, ty)`) into band `plane` of
/// an interleaved RGBA image of size `(w, h)`, clipping whatever lies past the image edge.
fn place_chunk(rgba: &mut [u8], (w, h): (u32, u32), plane: usize, (tx, ty): (u32, u32), stride: u32, data: &[u8]) {
    let rows = data.len() as u32 / stride.max(1);
    for y in 0..rows {
        let gy = ty + y;
        if gy >= h {
            break;
        }
        for x in 0..stride {
            let gx = tx + x;
            if gx >= w {
                break;
            }
            rgba[((gy * w + gx) * 4) as usize + plane] = data[(y * stride + x) as usize];
        }
    }
}

/// GDAL's NoData value (TIFF tag 42113, ASCII), when it is an 8-bit value. Read from the main
/// file, else its `.ovr`.
fn nodata_u8(main: &Path) -> Option<u8> {
    let read = |p: &Path| -> Option<u8> {
        let mut d = open(p).ok()?;
        let v = d.find_tag(Tag::Unknown(42113)).ok().flatten()?;
        let s = v.into_string().ok()?;
        let f: f64 = s.trim_matches(char::from(0)).trim().parse().ok()?;
        (f.fract() == 0.0 && (0.0..=255.0).contains(&f)).then_some(f as u8)
    };
    read(main).or_else(|| read(&ovr_path(main)))
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

#[cfg(test)]
mod tests {
    use super::place_chunk;

    /// A 3x3 RGBA image assembled from 2x2 tiles, one band at a time, as a planar `.ovr` stores it.
    #[test]
    fn planar_tiles_land_in_their_band_and_are_clipped_at_the_edges() {
        let (w, h) = (3u32, 3u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        // Tile grid 2x2 of 2x2 tiles; values encode band*100 + tile*10 + position.
        for plane in 0..3usize {
            for (i, (tx, ty)) in [(0, 0), (2, 0), (0, 2), (2, 2)].into_iter().enumerate() {
                let data: Vec<u8> = (0..4).map(|p| (plane * 100 + i * 10 + p) as u8).collect();
                place_chunk(&mut rgba, (w, h), plane, (tx, ty), 2, &data);
            }
        }
        let px = |x: u32, y: u32| &rgba[((y * w + x) * 4) as usize..((y * w + x) * 4 + 4) as usize];
        assert_eq!(px(0, 0)[..3], [0, 100, 200]); // tile 0, position 0, in R, G, B
        assert_eq!(px(1, 1)[..3], [3, 103, 203]); // tile 0, position 3
        assert_eq!(px(2, 0)[..3], [10, 110, 210]); // tile 1 starts at x = 2
        assert_eq!(px(2, 2)[..3], [30, 130, 230]); // bottom-right tile, only its first pixel fits
        assert_eq!(px(0, 2)[..3], [20, 120, 220]);
        assert!(rgba.chunks_exact(4).all(|p| p[3] == 0), "place_chunk never writes alpha unless asked");
    }
}
