//! File source: parse the pipe-joined exports produced by a read-only `sqlcmd` dump on the
//! Aumentum box into the same [`RawParcel`]/[`RawCoordinate`] types the live reader yields.
//!
//! This lets the DEFAULT-version reconstruction be run read-only on the server (no tooling
//! installed, no binary deployed) and the staging/promotion done elsewhere, from the dump.
//!
//! Parcel line: `OBJECTID|UPIN|Status|JobID|PlanNo|CadZone|PlotNo|BlockNo|SRID|WKT`
//! Beacon line: `Alias|X|Y`
//! Empty fields are NULL; WKT (last field) may contain spaces/commas but no `|`.

use crate::model::{RawCoordinate, RawParcel};

fn opt(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// Parse the parcels export. Malformed lines (wrong field count / unparseable ids) are skipped.
pub fn parse_parcels(contents: &str) -> Vec<RawParcel> {
    contents.lines().filter_map(parse_parcel_line).collect()
}

fn parse_parcel_line(line: &str) -> Option<RawParcel> {
    if line.trim().is_empty() {
        return None;
    }
    let f: Vec<&str> = line.splitn(10, '|').collect();
    if f.len() < 10 {
        return None;
    }
    Some(RawParcel {
        object_id: f[0].trim().parse().ok()?,
        upin: opt(f[1]),
        status_code: f[2].trim().parse().ok()?,
        job_id: opt(f[3]),
        plan_no: opt(f[4]),
        cad_zone: opt(f[5]),
        plot_no: opt(f[6]),
        block_no: opt(f[7]),
        srid: f[8].trim().parse().ok()?,
        wkt: opt(f[9]),
    })
}

/// Parse the beacons export. Malformed lines are skipped.
pub fn parse_beacons(contents: &str) -> Vec<RawCoordinate> {
    contents
        .lines()
        .filter_map(|line| {
            if line.trim().is_empty() {
                return None;
            }
            let f: Vec<&str> = line.splitn(3, '|').collect();
            if f.len() < 3 {
                return None;
            }
            Some(RawCoordinate {
                alias: opt(f[0]),
                x: f[1].trim().parse().ok()?,
                y: f[2].trim().parse().ok()?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_parcel_lines_with_attributes_and_nulls() {
        let data = "112534|PL000000005621|2|9D79072B-D46A|||||32632|POLYGON ((1 1, 2 1, 2 2, 1 1))\n\
                    7||1|||ZONE-A||B3|32632|POINT EMPTY\n\
                    \n";
        let ps = parse_parcels(data);
        assert_eq!(ps.len(), 2);
        assert_eq!(ps[0].object_id, 112534);
        assert_eq!(ps[0].upin.as_deref(), Some("PL000000005621"));
        assert_eq!(ps[0].status_code, 2);
        assert_eq!(ps[0].job_id.as_deref(), Some("9D79072B-D46A"));
        assert_eq!(ps[0].srid, 32632);
        assert!(ps[0].wkt.as_deref().unwrap().starts_with("POLYGON"));
        // second row: empty UPIN/job, cad_zone set, block set
        assert!(ps[1].upin.is_none());
        assert_eq!(ps[1].cad_zone.as_deref(), Some("ZONE-A"));
        assert_eq!(ps[1].block_no.as_deref(), Some("B3"));
    }

    #[test]
    fn skips_malformed_parcel_lines() {
        assert_eq!(parse_parcels("too|few|fields").len(), 0);
        assert_eq!(parse_parcels("notanint|PL1|2|||||| 32632|POLYGON EMPTY").len(), 0);
    }

    #[test]
    fn parses_beacons_ignoring_trailing_pad() {
        let data = "ABUZ|352440.677|1233094.048          \nE2|520719.419|1032578.939\n";
        let bs = parse_beacons(data);
        assert_eq!(bs.len(), 2);
        assert_eq!(bs[0].alias.as_deref(), Some("ABUZ"));
        assert!((bs[0].x - 352440.677).abs() < 1e-3);
        assert!((bs[1].y - 1032578.939).abs() < 1e-3);
    }
}
