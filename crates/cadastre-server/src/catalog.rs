//! Catalog: read-only browsing of configured folder connections (e.g. `\\PRODUCTIONMC\Image`),
//! presented the way ArcCatalog presents a folder — GIS datasets only, sidecar files hidden,
//! a shapefile as one item with its geometry type, rasters expandable into their bands.
//!
//! Everything here is read-only and never reads more than a file header (rasters on the share
//! are > 100 GB). Paths are confined to the configured roots.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use utoipa::ToSchema;

/// A configured folder connection: the name shown in the Catalog and where it is read from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogFolder {
    /// Display name, e.g. `\\PRODUCTIONMC\Image`.
    pub name: String,
    /// Filesystem path the server reads (`D:\Image` on the server, a mount point elsewhere).
    pub path: PathBuf,
}

/// Parse `CADASTRE_CATALOG_FOLDERS`: `name=path` pairs separated by `;`
/// (e.g. `\\PRODUCTIONMC\Image=D:\Image`). A bare path uses itself as the name.
pub fn parse_folders(spec: &str) -> Vec<CatalogFolder> {
    spec.split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|item| match item.split_once('=') {
            Some((name, path)) => CatalogFolder { name: name.trim().to_string(), path: PathBuf::from(path.trim()) },
            None => CatalogFolder { name: item.to_string(), path: PathBuf::from(item) },
        })
        .collect()
}

/// What an item is, in ArcCatalog terms. Drives the icon and what the item can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    Folder,
    FileGeodatabase,
    Toolbox,
    Shapefile,
    Raster,
    RasterBand,
    MapDocument,
    LayerFile,
    Kml,
    Table,
    Workbook,
    DatabaseConnection,
}

/// One Catalog entry.
#[derive(Clone, Debug, PartialEq, Serialize, ToSchema)]
pub struct CatalogItem {
    /// Name as ArcCatalog shows it (with extension, e.g. `BOSS.shp`).
    pub name: String,
    pub kind: ItemKind,
    /// Shapefile geometry: `point` | `polyline` | `polygon` | `multipoint`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geometry: Option<String>,
    /// Whether the item has children (folders; rasters whose bands could be read).
    pub expandable: bool,
    /// Size in bytes of the main file (absent for folders).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// Last modified, RFC 3339.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
}

const RASTER_EXT: &[&str] = &["tif", "tiff", "ecw", "jp2", "img", "sid", "jpg", "jpeg", "png", "bmp", "gif", "dem", "bil", "bip", "bsq"];
/// Files ArcCatalog never lists on their own (they belong to another dataset, or are not GIS data).
const SIDECAR_EXT: &[&str] = &[
    "shx", "prj", "sbn", "sbx", "cpg", "fbn", "fbx", "ain", "aih", "atx", "ixs", "mxs", "qix",
    "ovr", "rrd", "aux", "tfw", "tifw", "tiffw", "jgw", "jpgw", "pgw", "pngw", "bpw", "gfw", "eww", "j2w", "sdw", "blw", "lock",
];

fn ext_of(name: &str) -> String {
    Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

/// Classify the entries of one directory the way ArcCatalog displays them. `names` are the
/// directory's entries; `is_dir` says which are folders. Non-GIS files return nothing.
pub fn classify(names: &[(String, bool)]) -> Vec<(String, ItemKind)> {
    let has = |n: &str| names.iter().any(|(x, d)| !*d && x.eq_ignore_ascii_case(n));
    let mut out = Vec::new();
    for (name, is_dir) in names {
        let lower = name.to_ascii_lowercase();
        if *is_dir {
            if lower.ends_with(".gdb") {
                out.push((name.clone(), ItemKind::FileGeodatabase));
            } else if lower != "info" && !lower.starts_with('.') {
                out.push((name.clone(), ItemKind::Folder));
            }
            continue;
        }
        if lower.starts_with("~$") || lower == "thumbs.db" || lower.starts_with('.') {
            continue;
        }
        // Metadata sidecars like `x.shp.xml`, `x.tif.aux.xml` belong to their dataset.
        if lower.ends_with(".xml") && Path::new(&lower).file_stem().is_some_and(|s| !ext_of(&s.to_string_lossy()).is_empty()) {
            continue;
        }
        let ext = ext_of(name);
        if SIDECAR_EXT.contains(&ext.as_str()) {
            continue;
        }
        let stem = &name[..name.len() - ext.len()].trim_end_matches('.');
        let kind = match ext.as_str() {
            "shp" => Some(ItemKind::Shapefile),
            "dbf" if !has(&format!("{stem}.shp")) => Some(ItemKind::Table),
            "dbf" => None,
            "csv" | "txt" | "tab" => Some(ItemKind::Table),
            "xls" | "xlsx" => Some(ItemKind::Workbook),
            "tbx" => Some(ItemKind::Toolbox),
            "mxd" => Some(ItemKind::MapDocument),
            "lyr" => Some(ItemKind::LayerFile),
            "kml" | "kmz" => Some(ItemKind::Kml),
            "sde" => Some(ItemKind::DatabaseConnection),
            e if RASTER_EXT.contains(&e) => Some(ItemKind::Raster),
            _ => None,
        };
        if let Some(k) = kind {
            out.push((name.clone(), k));
        }
    }
    // ArcCatalog order: folders, geodatabases, toolboxes, then datasets — each by name.
    let rank = |k: ItemKind| match k {
        ItemKind::Folder => 0,
        ItemKind::FileGeodatabase => 1,
        ItemKind::Toolbox => 2,
        _ => 3,
    };
    out.sort_by(|a, b| rank(a.1).cmp(&rank(b.1)).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
    out
}

/// Number of bands (samples per pixel) in a TIFF or BigTIFF, reading only the header and the
/// first IFD. `None` if the file is not a readable TIFF.
pub fn tiff_band_count<R: Read + Seek>(r: &mut R) -> Option<u16> {
    let mut head = [0u8; 16];
    r.read_exact(&mut head[..8]).ok()?;
    let le = match &head[..2] {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |b: &[u8]| if le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) };
    let u32_at = |b: &[u8]| if le { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) } else { u32::from_be_bytes([b[0], b[1], b[2], b[3]]) };
    let u64_at = |b: &[u8]| {
        let a: [u8; 8] = b[..8].try_into().unwrap();
        if le { u64::from_le_bytes(a) } else { u64::from_be_bytes(a) }
    };
    let (big, ifd) = match u16_at(&head[2..4]) {
        42 => (false, u64::from(u32_at(&head[4..8]))),
        43 => {
            r.read_exact(&mut head[8..16]).ok()?;
            (true, u64_at(&head[8..16]))
        }
        _ => return None,
    };
    r.seek(SeekFrom::Start(ifd)).ok()?;
    let count = if big {
        let mut b = [0u8; 8];
        r.read_exact(&mut b).ok()?;
        u64_at(&b).min(4096)
    } else {
        let mut b = [0u8; 2];
        r.read_exact(&mut b).ok()?;
        u64::from(u16_at(&b))
    };
    let entry_len = if big { 20 } else { 12 };
    let mut entries = vec![0u8; usize::try_from(count).ok()? * entry_len];
    r.read_exact(&mut entries).ok()?;
    for e in entries.chunks_exact(entry_len) {
        if u16_at(&e[..2]) == 277 {
            // SamplesPerPixel: a SHORT stored inline in the value field.
            let value_at = if big { 12 } else { 8 };
            return Some(u16_at(&e[value_at..value_at + 2]).max(1));
        }
    }
    Some(1) // tag absent: the TIFF default is one sample per pixel
}

/// Shapefile geometry type from the 100-byte main-file header (shape type at byte 32).
pub fn shapefile_geometry<R: Read>(r: &mut R) -> Option<&'static str> {
    let mut head = [0u8; 36];
    r.read_exact(&mut head).ok()?;
    if i32::from_be_bytes(head[..4].try_into().unwrap()) != 9994 {
        return None;
    }
    Some(match i32::from_le_bytes(head[32..36].try_into().unwrap()) {
        1 | 11 | 21 => "point",
        3 | 13 | 23 => "polyline",
        5 | 15 | 25 => "polygon",
        8 | 18 | 28 => "multipoint",
        31 => "multipatch",
        _ => "null",
    })
}

/// Resolve a Catalog-relative path (`Plateau_Urban/488-1086.tif`, `/`-separated) under a root,
/// refusing anything that could escape it (`..`, absolute paths, drive prefixes).
pub fn resolve(root: &Path, rel: &str) -> Option<PathBuf> {
    // Locations are always relative: no leading separator, no drive letters or ADS (`:`).
    if rel.starts_with(['/', '\\']) || rel.contains(':') {
        return None;
    }
    let mut out = root.to_path_buf();
    for part in rel.split(['/', '\\']).filter(|p| !p.is_empty()) {
        match Path::new(part).components().next()? {
            Component::Normal(c) if Path::new(part).components().count() == 1 => out.push(c),
            _ => return None,
        }
    }
    Some(out)
}

fn modified(meta: &fs::Metadata) -> Option<String> {
    let t = meta.modified().ok()?;
    Some(chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339())
}

/// List one Catalog location: a folder's GIS items, or a raster's bands.
pub fn list(path: &Path) -> std::io::Result<Vec<CatalogItem>> {
    let meta = fs::metadata(path)?;
    if meta.is_file() {
        // Expanding a raster lists its bands.
        let bands = File::open(path).ok().and_then(|mut f| tiff_band_count(&mut f)).unwrap_or(0);
        return Ok((1..=bands)
            .map(|i| CatalogItem { name: format!("Band_{i}"), kind: ItemKind::RasterBand, geometry: None, expandable: false, size: None, modified: None })
            .collect());
    }
    let mut names = Vec::new();
    for entry in fs::read_dir(path)?.take(20_000) {
        let entry = entry?;
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        names.push((entry.file_name().to_string_lossy().into_owned(), is_dir));
    }
    Ok(classify(&names)
        .into_iter()
        .map(|(name, kind)| {
            let p = path.join(&name);
            let meta = fs::metadata(&p).ok();
            let is_tiff = matches!(ext_of(&name).as_str(), "tif" | "tiff");
            let geometry = (kind == ItemKind::Shapefile)
                .then(|| File::open(&p).ok().and_then(|mut f| shapefile_geometry(&mut f)).map(str::to_string))
                .flatten();
            CatalogItem {
                expandable: kind == ItemKind::Folder || (kind == ItemKind::Raster && is_tiff),
                size: meta.as_ref().filter(|m| m.is_file()).map(|m| m.len()),
                modified: meta.as_ref().and_then(modified),
                name,
                kind,
                geometry,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn names(v: &[(&str, bool)]) -> Vec<(String, bool)> {
        v.iter().map(|(n, d)| (n.to_string(), *d)).collect()
    }

    #[test]
    fn shows_gis_items_like_arccatalog_and_hides_the_rest() {
        let got = classify(&names(&[
            ("Toolbox.tbx", false), ("488-1086.tif", false), ("488-1086.tfw", false), ("488-1086.tif.aux.xml", false),
            ("BOSS.shp", false), ("BOSS.shx", false), ("BOSS.dbf", false), ("BOSS.prj", false), ("BOSS.shp.xml", false),
            ("New_Shapefile.shp", false), ("Plateau_Urban_Merged.tif", false), ("Plateau_Urban_Merged.tif.ovr", false),
            ("Thumbs.db", false), ("PL10930.pdf", false), ("~$RVEY REPORT.docx", false), ("Capture 1.PNG", false),
            ("weymap 2023", true), ("New File Geodatabase.gdb", true), ("info", true), ("Barkin Ladi", true),
            ("orphan.dbf", false), ("plots.kmz", false), ("Plan.mxd", false),
        ]));
        let shown: Vec<_> = got.iter().map(|(n, k)| format!("{n}:{k:?}")).collect();
        assert_eq!(shown, vec![
            "Barkin Ladi:Folder", "weymap 2023:Folder", "New File Geodatabase.gdb:FileGeodatabase", "Toolbox.tbx:Toolbox",
            "488-1086.tif:Raster", "BOSS.shp:Shapefile", "Capture 1.PNG:Raster", "New_Shapefile.shp:Shapefile",
            "orphan.dbf:Table", "Plan.mxd:MapDocument", "Plateau_Urban_Merged.tif:Raster", "plots.kmz:Kml",
        ]);
    }

    fn tiff(le: bool, big: bool, samples: Option<u16>) -> Vec<u8> {
        let mut v = Vec::new();
        let w16 = |v: &mut Vec<u8>, x: u16| v.extend(if le { x.to_le_bytes() } else { x.to_be_bytes() });
        let w32 = |v: &mut Vec<u8>, x: u32| v.extend(if le { x.to_le_bytes() } else { x.to_be_bytes() });
        let w64 = |v: &mut Vec<u8>, x: u64| v.extend(if le { x.to_le_bytes() } else { x.to_be_bytes() });
        v.extend(if le { b"II" } else { b"MM" });
        let tags: Vec<(u16, u16)> = [(256u16, 100u16)].into_iter().chain(samples.map(|s| (277u16, s))).collect();
        if big {
            w16(&mut v, 43); w16(&mut v, 8); w16(&mut v, 0); w64(&mut v, 16);
            w64(&mut v, tags.len() as u64);
            for (tag, val) in &tags { w16(&mut v, *tag); w16(&mut v, 3); w64(&mut v, 1); w16(&mut v, *val); v.extend([0u8; 6]); }
        } else {
            w16(&mut v, 42); w32(&mut v, 8);
            w16(&mut v, tags.len() as u16);
            for (tag, val) in &tags { w16(&mut v, *tag); w16(&mut v, 3); w32(&mut v, 1); w16(&mut v, *val); v.extend([0u8; 2]); }
        }
        v
    }

    #[test]
    fn reads_band_count_from_tiff_and_bigtiff_in_both_byte_orders() {
        for le in [true, false] {
            for big in [false, true] {
                assert_eq!(tiff_band_count(&mut Cursor::new(tiff(le, big, Some(3)))), Some(3), "le={le} big={big}");
                assert_eq!(tiff_band_count(&mut Cursor::new(tiff(le, big, None))), Some(1), "default 1 band");
            }
        }
        assert_eq!(tiff_band_count(&mut Cursor::new(b"not a tiff".to_vec())), None);
    }

    #[test]
    fn reads_shapefile_geometry_type() {
        let mut h = vec![0u8; 100];
        h[..4].copy_from_slice(&9994i32.to_be_bytes());
        for (code, want) in [(1i32, "point"), (3, "polyline"), (5, "polygon"), (15, "polygon"), (8, "multipoint")] {
            h[32..36].copy_from_slice(&code.to_le_bytes());
            assert_eq!(shapefile_geometry(&mut Cursor::new(h.clone())), Some(want));
        }
        assert_eq!(shapefile_geometry(&mut Cursor::new(vec![0u8; 100])), None);
    }

    #[test]
    fn paths_cannot_escape_the_root() {
        let root = Path::new("/share");
        assert_eq!(resolve(root, "Plateau_Urban/488-1086.tif"), Some(PathBuf::from("/share/Plateau_Urban/488-1086.tif")));
        assert_eq!(resolve(root, ""), Some(PathBuf::from("/share")));
        for bad in ["..", "a/../../etc", "/etc/passwd", "a/./b", "C:\\Windows"] {
            assert_eq!(resolve(root, bad), None, "{bad}");
        }
    }

    #[test]
    fn parses_folder_connections() {
        let f = parse_folders(r"\\PRODUCTIONMC\Image=D:\Image ; /data/maps");
        assert_eq!(f, vec![
            CatalogFolder { name: r"\\PRODUCTIONMC\Image".into(), path: PathBuf::from(r"D:\Image") },
            CatalogFolder { name: "/data/maps".into(), path: PathBuf::from("/data/maps") },
        ]);
        assert!(parse_folders("").is_empty());
    }
}
