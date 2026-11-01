//! Delimited-text import: coordinate lists and attribute tables (SRS survey entry, attribute
//! import).
//!
//! Survey coordinate lists arrive as CSV, semicolon, tab, pipe or whitespace-separated text with
//! either a header row or the conventional `id,easting,northing[,elevation][,code]` order.
//! Attribute tables are joined to coordinates on a shared key (beacon/parcel/UPIN number).
//!
//! The parser is deliberately dependency-free and tolerant: a malformed row is reported, never
//! silently dropped.

use std::collections::BTreeMap;
use std::fmt;

use cadastre_core::types::Point;

/// A reported problem with one input row.
#[derive(Clone, Debug, PartialEq)]
pub struct RowIssue {
    /// One-based line number in the input.
    pub line: usize,
    /// What went wrong.
    pub message: String,
}

impl fmt::Display for RowIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

/// A parsed delimited table.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    /// Column names (synthesised as `column1…` when the input has no header).
    pub headers: Vec<String>,
    /// Data rows, aligned with `headers`.
    pub rows: Vec<Vec<String>>,
    /// Whether the first input row was treated as a header.
    pub had_header: bool,
}

/// One imported coordinate.
#[derive(Clone, Debug, PartialEq)]
pub struct CoordinateRow {
    /// Point/beacon identifier.
    pub id: String,
    /// Coordinate in the working grid.
    pub point: Point,
    /// Optional elevation (metres).
    pub elevation: Option<f64>,
    /// Optional feature code or description.
    pub code: Option<String>,
    /// Every other column, kept verbatim for attribute import.
    pub attributes: BTreeMap<String, String>,
    /// One-based input line.
    pub line: usize,
}

/// The result of parsing a coordinate list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CoordinateList {
    /// Successfully parsed coordinates, in input order.
    pub rows: Vec<CoordinateRow>,
    /// Rows that could not be used.
    pub issues: Vec<RowIssue>,
    /// CRS named by an optional `crs`/`srid` column or passed by the caller.
    pub crs: Option<String>,
}

impl CoordinateList {
    /// Whether every row parsed.
    pub fn is_complete(&self) -> bool {
        self.issues.is_empty()
    }
}

/// Detect the most likely delimiter from the first non-empty line.
fn detect_delimiter(contents: &str) -> Option<char> {
    let line = contents
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default();
    let candidates = [',', ';', '\t', '|'];
    let mut best = None;
    let mut best_count = 0;
    for c in candidates {
        let count = line.matches(c).count();
        if count > best_count {
            best_count = count;
            best = Some(c);
        }
    }
    best
}

/// Split one record, honouring double-quoted fields and doubled quotes inside them.
fn split_record(line: &str, delimiter: Option<char>) -> Vec<String> {
    let Some(delimiter) = delimiter else {
        return line.split_whitespace().map(str::to_string).collect();
    };
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if in_quotes => {
                if chars.peek() == Some(&'"') {
                    current.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            }
            '"' => in_quotes = true,
            c if c == delimiter && !in_quotes => {
                fields.push(current.trim().to_string());
                current.clear();
            }
            c => current.push(c),
        }
    }
    fields.push(current.trim().to_string());
    fields
}

/// Normalise a header for matching: lower case ASCII letters and digits only.
fn normalise_header(header: &str) -> String {
    header
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

const ID_HEADERS: &[&str] = &[
    "id", "no", "number", "no.", "point", "pointno", "pointnumber", "beacon", "beaconno",
    "beaconnumber", "name", "station", "alias", "pt", "ptno", "code1",
];
const EASTING_HEADERS: &[&str] = &[
    "easting", "east", "e", "x", "eastings", "eastingm", "xcoord", "xcoordinate", "coordx",
];
const NORTHING_HEADERS: &[&str] = &[
    "northing", "north", "n", "y", "northings", "northingm", "ycoord", "ycoordinate", "coordy",
];
const ELEVATION_HEADERS: &[&str] = &[
    "elevation", "elev", "z", "height", "level", "reducedlevel", "rl",
];
const CODE_HEADERS: &[&str] = &[
    "code", "description", "desc", "feature", "featurecode", "type", "symbol",
];
const CRS_HEADERS: &[&str] = &["crs", "srid", "epsg", "projection", "datum"];

fn find_column(headers: &[String], aliases: &[&str]) -> Option<usize> {
    headers
        .iter()
        .position(|h| aliases.contains(&normalise_header(h).as_str()))
}

fn looks_like_header(row: &[String]) -> bool {
    row.iter().any(|field| {
        let n = normalise_header(field);
        ID_HEADERS.contains(&n.as_str())
            || EASTING_HEADERS.contains(&n.as_str())
            || NORTHING_HEADERS.contains(&n.as_str())
            || ELEVATION_HEADERS.contains(&n.as_str())
    })
}

/// Parse a delimited text table. Returns an empty table for empty input.
pub fn parse_table(contents: &str) -> Table {
    let contents = contents.strip_prefix('\u{feff}').unwrap_or(contents);
    let delimiter = detect_delimiter(contents);
    let mut records: Vec<(usize, Vec<String>)> = contents
        .lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let line = line.trim_end_matches('\r');
            if line.trim().is_empty() || line.trim_start().starts_with('#') {
                return None;
            }
            Some((i + 1, split_record(line, delimiter)))
        })
        .collect();
    if records.is_empty() {
        return Table::default();
    }

    let had_header = looks_like_header(&records[0].1);
    let headers = if had_header {
        let (_, row) = records.remove(0);
        row
    } else {
        let width = records[0].1.len();
        (1..=width).map(|i| format!("column{i}")).collect()
    };

    let rows = records
        .into_iter()
        .map(|(_, mut row)| {
            row.resize(headers.len(), String::new());
            row
        })
        .collect();
    Table {
        headers,
        rows,
        had_header,
    }
}

fn parse_number(text: &str) -> Option<f64> {
    let cleaned: String = text
        .chars()
        .filter(|c| !matches!(c, ',' | '\u{a0}' | ' '))
        .collect();
    cleaned.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Parse a coordinate list. Coordinates are read in metres in the working CRS.
///
/// Header names are matched case-insensitively (`easting`/`east`/`x`, `northing`/`north`/`y`,
/// `id`/`point`/`beacon`, `elevation`/`z`, `code`/`description`). A file without a header must be
/// in `id,easting,northing[,elevation][,code]` order.
pub fn parse_coordinates(contents: &str, default_crs: Option<&str>) -> CoordinateList {
    let table = parse_table(contents);
    let mut result = CoordinateList {
        crs: default_crs.map(str::to_string),
        ..Default::default()
    };
    if table.headers.is_empty() {
        return result;
    }

    let id_col = find_column(&table.headers, ID_HEADERS).unwrap_or(0);
    let east_col = find_column(&table.headers, EASTING_HEADERS).unwrap_or(1);
    let north_col = find_column(&table.headers, NORTHING_HEADERS).unwrap_or(2);
    let elev_col = find_column(&table.headers, ELEVATION_HEADERS);
    let code_col = find_column(&table.headers, CODE_HEADERS);
    let crs_col = find_column(&table.headers, CRS_HEADERS);

    // Headerless positional order is fixed; with a header, fall back only if the named columns
    // are absent altogether.
    if !table.had_header
        && (table.headers.len() < 3
            || find_column(&table.headers, EASTING_HEADERS).is_some()
            || find_column(&table.headers, NORTHING_HEADERS).is_some())
    {
        result.issues.push(RowIssue {
            line: 1,
            message: "a coordinate list needs at least id, easting and northing".to_string(),
        });
        return result;
    }

    for (offset, row) in table.rows.iter().enumerate() {
        let line = offset + 1 + if table.had_header { 1 } else { 0 };
        let get = |index: usize| row.get(index).map(String::as_str).unwrap_or("").trim();
        let id = get(id_col);
        if id.is_empty() {
            result.issues.push(RowIssue {
                line,
                message: "empty point identifier".to_string(),
            });
            continue;
        }
        let (Some(easting), Some(northing)) =
            (parse_number(get(east_col)), parse_number(get(north_col)))
        else {
            result.issues.push(RowIssue {
                line,
                message: format!(
                    "point `{id}` has a non-numeric coordinate (`{}`, `{}`)",
                    get(east_col),
                    get(north_col)
                ),
            });
            continue;
        };
        if easting.abs() > 1e8 || northing.abs() > 1e8 {
            result.issues.push(RowIssue {
                line,
                message: format!("point `{id}` coordinate is out of range"),
            });
            continue;
        }

        let mut attributes = BTreeMap::new();
        for (i, header) in table.headers.iter().enumerate() {
            if i == id_col || i == east_col || i == north_col {
                continue;
            }
            let value = get(i);
            if !value.is_empty() {
                attributes.insert(header.clone(), value.to_string());
            }
        }

        let elevation = elev_col.and_then(|i| parse_number(get(i)));
        let code = code_col
            .map(|i| get(i).to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| attributes.get("Code").cloned());
        if let Some(i) = crs_col {
            let crs = get(i);
            if !crs.is_empty() {
                result.crs = Some(crs.to_string());
            }
        }

        result.rows.push(CoordinateRow {
            id: id.to_string(),
            point: Point::new(easting, northing),
            elevation,
            code,
            attributes,
            line,
        });
    }
    result
}

/// An attribute table keyed by a column.
#[derive(Clone, Debug, PartialEq)]
pub struct AttributeTable {
    /// The original table.
    pub table: Table,
    /// Column used as the join key.
    pub key_column: String,
}

/// The outcome of joining attributes onto coordinates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JoinReport {
    /// Coordinates that received attributes.
    pub matched: usize,
    /// Coordinate ids with no attribute row.
    pub unmatched_points: Vec<String>,
    /// Attribute keys with no coordinate.
    pub unmatched_keys: Vec<String>,
}

/// Parse an attribute table, choosing `key_column` when it exists or the first header otherwise.
pub fn parse_attributes(contents: &str, key_column: Option<&str>) -> AttributeTable {
    let table = parse_table(contents);
    let key_column = key_column
        .map(str::to_string)
        .or_else(|| {
            table
                .headers
                .iter()
                .find(|h| {
                    let n = normalise_header(h);
                    ID_HEADERS.contains(&n.as_str())
                        || n == "upin"
                        || n == "parcelid"
                        || n == "parcelnumber"
                        || n == "key"
                })
                .cloned()
        })
        .or_else(|| table.headers.first().cloned())
        .unwrap_or_default();
    AttributeTable { table, key_column }
}

/// Join attribute rows onto a coordinate list in place.
pub fn join_attributes(
    coordinates: &mut CoordinateList,
    attributes: &AttributeTable,
) -> JoinReport {
    let Some(key_index) = attributes
        .table
        .headers
        .iter()
        .position(|h| h == &attributes.key_column)
    else {
        return JoinReport {
            unmatched_points: coordinates.rows.iter().map(|r| r.id.clone()).collect(),
            ..Default::default()
        };
    };

    let mut by_key: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for row in &attributes.table.rows {
        let Some(key) = row.get(key_index) else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let mut fields = BTreeMap::new();
        for (i, header) in attributes.table.headers.iter().enumerate() {
            if i == key_index {
                continue;
            }
            let value = row.get(i).map(String::as_str).unwrap_or("").trim();
            if !value.is_empty() {
                fields.insert(header.clone(), value.to_string());
            }
        }
        by_key.insert(key.to_string(), fields);
    }

    let mut report = JoinReport::default();
    let mut used: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for row in &mut coordinates.rows {
        match by_key.get(&row.id) {
            Some(fields) => {
                report.matched += 1;
                used.insert(row.id.clone());
                for (k, v) in fields {
                    row.attributes.insert(k.clone(), v.clone());
                }
            }
            None => report.unmatched_points.push(row.id.clone()),
        }
    }
    report.unmatched_keys = by_key
        .keys()
        .filter(|k| !used.contains(*k))
        .cloned()
        .collect();
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_headered_comma_list() {
        let text = "\u{feff}Point,Easting,Northing,Elevation,Code\n\
                    1001,352440.677,1233094.048,512.3,BC\n\
                    1002,352540.677,1233094.048,,\n";
        let list = parse_coordinates(text, Some("EPSG:32632"));
        assert_eq!(list.rows.len(), 2);
        assert_eq!(list.rows[0].id, "1001");
        assert!((list.rows[0].point.x - 352_440.677).abs() < 1e-9);
        assert!((list.rows[0].point.y - 1_233_094.048).abs() < 1e-9);
        assert_eq!(list.rows[0].elevation, Some(512.3));
        assert_eq!(list.rows[0].code.as_deref(), Some("BC"));
        assert_eq!(list.crs.as_deref(), Some("EPSG:32632"));
        assert!(list.issues.is_empty());
    }

    #[test]
    fn parses_headerless_and_quoted_values() {
        let text = "ABUZ,352440.677,1233094.048\n\
                    E2,520719.419,1032578.939\n";
        let list = parse_coordinates(text, None);
        assert_eq!(list.rows.len(), 2);
        assert_eq!(list.rows[0].attributes.len(), 0);

        let text = "id;east;north;description\n\
                    1;\"1,000.5\";2000.25;\"corner, road\"\n";
        let list = parse_coordinates(text, None);
        assert_eq!(list.rows.len(), 1);
        assert!((list.rows[0].point.x - 1000.5).abs() < 1e-9);
        assert_eq!(list.rows[0].code.as_deref(), Some("corner, road"));
    }

    #[test]
    fn reports_bad_rows_instead_of_dropping_them_silently() {
        let text = "id,easting,northing\n1,100,200\n2,abc,300\n3,400,500\n";
        let list = parse_coordinates(text, None);
        assert_eq!(list.rows.len(), 2);
        assert_eq!(list.issues.len(), 1);
        assert_eq!(list.issues[0].line, 3);
    }

    #[test]
    fn whitespace_separated_lists_are_supported() {
        let text = "id easting northing code\n1 100.0 200.0 BC\n2 110.0 200.0 BC\n";
        let list = parse_coordinates(text, None);
        assert_eq!(list.rows.len(), 2);
        assert_eq!(list.rows[1].code.as_deref(), Some("BC"));
    }

    #[test]
    fn attributes_join_on_the_point_number() {
        let mut coords = parse_coordinates(
            "id,easting,northing\n1,100,200\n2,300,400\n3,500,600\n",
            None,
        );
        let attrs = parse_attributes(
            "Beacon,Owner,Plan\n1,Ada,PL1\n2,Bello,PL2\n9,Idris,PL9\n",
            Some("Beacon"),
        );
        let report = join_attributes(&mut coords, &attrs);
        assert_eq!(report.matched, 2);
        assert_eq!(report.unmatched_points, vec!["3"]);
        assert_eq!(report.unmatched_keys, vec!["9"]);
        assert_eq!(
            coords.rows[0].attributes.get("Owner").map(String::as_str),
            Some("Ada")
        );
    }

    #[test]
    fn empty_input_is_empty_not_an_error() {
        let list = parse_coordinates("", None);
        assert!(list.rows.is_empty());
        assert!(list.issues.is_empty());
    }
}
