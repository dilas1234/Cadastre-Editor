//! Aumentum Registry (LRS43, 10.10.10.5) linkage rules, as pure functions (no database, no
//! network), so each rule is unit-tested. Phase 5.
//!
//! The link: a Registry transaction (`PLS000006909`) holds properties (spatial units); the
//! Cadastre's AUMENTUM REGISTRY extension links a transaction's properties to Cadastre parcels
//! (UPINs). The Registry's `mcintegration` settings (`LRSAdmin.lr_setting`) drive what is copied:
//!
//! * `GISUpdatedFields` - Registry attribute -> Cadastre `ParcelAttributeTable` field, written on
//!   link. On PRODUCTIONMC: `suID:LRSID,1:LinkageStatus,spatialDate1:LinkageDate,
//!   refnum:PropertyNumber,plotNumber:Plot,blockNumber:Block`.
//! * `GISRetrievedFields` - Registry attribute <- Cadastre field (`upin:UPIN`): the Registry
//!   property takes the parcel's UPIN.
//! * `GISSearchFields` - the search form of the Registry's map
//!   (`attr:gis_field:hidden:label[:key]`, comma separated).
//!
//! What a link writes is taken from what Aumentum actually wrote, read from the 6,588 parcel
//! attribute rows with an LRSID imported in Phase 1 (`cadastre.parcel_attribute`), not guessed:
//!
//! * LRSID = the property's SuID (`PLT` + 8 digits, all 6,588 rows); PropertyNumber = its refnum
//!   (the Registry document number, e.g. `PL1049`); LinkageDate = a date, `yyyy-MM-dd`.
//! * LinkageStatus = `1`: the source `1` is not an attribute, so the text itself is written.
//! * Plot = `plotNumber` (6,496 rows) and Block = `blockNumber` (6,445 rows), the attribute NAMES:
//!   Aumentum resolves only some attribute names and writes any other source as text, which is
//!   what makes the constant `1` work. [`resolve`] reproduces this exactly; whether PLAGIS wants
//!   real plot/block numbers instead is an open question for them, not something to change here.
//! * Unlink sets LinkageStatus to 0 and keeps LRSID, LinkageDate and PropertyNumber (the 18
//!   unlinked rows that still carry an LRSID).
//! * A property may be linked to several parcels (e.g. PLT00000136 -> 6 UPINs), so LRSID is not
//!   unique.
//!
//! Link, unlink and sending a plan are refused unless the transaction is Active (lodged): Aumentum
//! refuses Withdrawn, Postponed, Completed and Rejected (decompiled `IsTransactionStatusAllowed`;
//! the message is the one in its log, [`LinkError::TransactionClosed`]).

use chrono::NaiveDateTime;

/// One `source:target` pair of `GISUpdatedFields` / `GISRetrievedFields`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldMap {
    pub source: String,
    pub target: String,
}

/// Parses `a:B,c:D` (whitespace around items ignored; empty items and items without a target are
/// skipped, as an empty setting such as `LRSNonUpdatableKeyFields` yields no pairs).
pub fn parse_field_map(setting: &str) -> Vec<FieldMap> {
    setting
        .split(',')
        .filter_map(|item| {
            let (s, t) = item.trim().split_once(':')?;
            let (s, t) = (s.trim(), t.trim());
            (!s.is_empty() && !t.is_empty()).then(|| FieldMap { source: s.to_string(), target: t.to_string() })
        })
        .collect()
}

/// One field of `GISSearchFields`: `attr:gis_field:hidden:label[:key]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchField {
    /// Registry property attribute (`uid`, `upin`, `district`).
    pub attribute: String,
    /// Cadastre field it matches, if any (`UPIN`).
    pub gis_field: Option<String>,
    /// `true`: the field is not shown on the search form.
    pub hidden: bool,
    pub label: String,
    /// The key field of the search.
    pub key: bool,
}

pub fn parse_search_fields(setting: &str) -> Vec<SearchField> {
    setting
        .split(',')
        .filter_map(|item| {
            let p: Vec<&str> = item.trim().split(':').map(str::trim).collect();
            if p.len() < 4 || p[0].is_empty() {
                return None;
            }
            Some(SearchField {
                attribute: p[0].to_string(),
                gis_field: (!p[1].is_empty()).then(|| p[1].to_string()),
                hidden: p[2].eq_ignore_ascii_case("true"),
                label: p[3].to_string(),
                key: p.get(4).is_some_and(|k| k.eq_ignore_ascii_case("true")),
            })
        })
        .collect()
}

/// The Registry property (`LRSAdmin.lr_spatial_unit`) fields a link reads.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RegistryProperty {
    /// `suid`: the Registry's property id, written to LRSID.
    pub su_id: String,
    /// `refnum`: the Registry document number (`PL1049`), written to PropertyNumber.
    pub refnum: Option<String>,
    /// `spatial_date1`, written to LinkageDate.
    pub spatial_date1: Option<NaiveDateTime>,
    /// `upin`: the Cadastre UPIN the Registry holds for the property.
    pub upin: Option<String>,
}

/// The value a link writes for one `GISUpdatedFields` source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolved {
    /// The property's attribute value (`None`: the attribute is empty, the field is cleared).
    Value(Option<String>),
    /// Not an attribute Aumentum resolves: the source text itself is written.
    Literal(String),
}

impl Resolved {
    pub fn into_value(self) -> Option<String> {
        match self {
            Resolved::Value(v) => v,
            Resolved::Literal(s) => Some(s),
        }
    }
}

/// Aumentum's resolution of a `GISUpdatedFields` source (see the module notes): `suID`, `refnum`
/// and `spatialDate1` are read from the property; anything else is written as text.
pub fn resolve(source: &str, p: &RegistryProperty) -> Resolved {
    match source {
        "suID" => Resolved::Value(Some(p.su_id.clone()).filter(|s| !s.is_empty())),
        "refnum" => Resolved::Value(p.refnum.clone().filter(|s| !s.is_empty())),
        "spatialDate1" => Resolved::Value(p.spatial_date1.map(linkage_date)),
        other => Resolved::Literal(other.to_string()),
    }
}

/// LinkageDate as Aumentum stores it: `yyyy-MM-dd` (10 characters, 6,537 rows).
pub fn linkage_date(d: NaiveDateTime) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// The Cadastre parcel-attribute fields (Aumentum names) a link writes, in setting order.
pub fn link_updates(updated_fields: &[FieldMap], p: &RegistryProperty) -> Vec<(String, Option<String>)> {
    updated_fields.iter().map(|m| (m.target.clone(), resolve(&m.source, p).into_value())).collect()
}

/// The fields an unlink writes: LinkageStatus 0 only; LRSID, LinkageDate and PropertyNumber keep
/// the last link (as in Aumentum).
pub fn unlink_updates() -> Vec<(String, Option<String>)> {
    vec![("LinkageStatus".to_string(), Some("0".to_string()))]
}

/// The Registry property fields a link writes from the parcel (`GISRetrievedFields`, `upin:UPIN`):
/// `(registry attribute, value of the parcel field)`. `parcel_field` gives a parcel field by its
/// Aumentum name.
pub fn registry_updates(
    retrieved_fields: &[FieldMap],
    parcel_field: impl Fn(&str) -> Option<String>,
) -> Vec<(String, Option<String>)> {
    retrieved_fields.iter().map(|m| (m.source.clone(), parcel_field(&m.target))).collect()
}

/// Aumentum's `LinkageFields` setting (`upin:UPIN;LRSID:suid,PropertyNumber:refnum,...`):
/// before `;` the Registry property fields written from the parcel (`lrsField:gisField`), after it
/// the parcel fields written from the property (`gisField:lrsField`). Both as (registry, parcel).
pub fn parse_linkage_fields(setting: &str) -> (Vec<FieldMap>, Vec<FieldMap>) {
    let mut parts = setting.splitn(2, ';');
    let to_registry = parts.next().map(parse_field_map).unwrap_or_default();
    let to_parcel = parts.next().map(|p| parse_field_map(p).into_iter().map(|m| FieldMap { source: m.target, target: m.source }).collect()).unwrap_or_default();
    (to_registry, to_parcel)
}

/// A link made from the Cadastre (Aumentum `LinkageManager.LinkParcel`): the parcel fields it
/// writes from the property. Each Registry name is looked up in the property's columns ignoring
/// case (Aumentum reads a DataTable row); a name the row lacks (`plotNumber`) is skipped, as
/// Aumentum skipped it ("Cannot retrieve the value of 'plotNumber' from Registry"). Empty values are
/// skipped too. Then LinkageStatus 1 and LinkageDate (yyyy-MM-dd, as every stored date is).
pub fn cadastre_link_updates(to_parcel: &[FieldMap], property: &[(&str, Option<String>)], today: chrono::NaiveDate) -> Vec<(String, Option<String>)> {
    let mut out: Vec<(String, Option<String>)> = to_parcel
        .iter()
        .filter_map(|m| {
            let v = property.iter().find(|(k, _)| k.eq_ignore_ascii_case(&m.source))?.1.clone()?;
            (!v.trim().is_empty()).then(|| (m.target.clone(), Some(v)))
        })
        .collect();
    out.push(("LinkageStatus".into(), Some("1".into())));
    out.push(("LinkageDate".into(), Some(today.format("%Y-%m-%d").to_string())));
    out
}

/// An unlink made from the Cadastre: every linked parcel field cleared, LinkageStatus 0,
/// LinkageDate cleared (Aumentum writes empty values).
pub fn cadastre_unlink_updates(to_parcel: &[FieldMap]) -> Vec<(String, Option<String>)> {
    let mut out: Vec<(String, Option<String>)> = to_parcel.iter().map(|m| (m.target.clone(), None)).collect();
    out.push(("LinkageStatus".into(), Some("0".into())));
    out.push(("LinkageDate".into(), None));
    out
}

/// A Registry transaction status (`LRSAdmin.lr_dictionary`, category of `transaction_status`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransactionStatus {
    pub code: String,
    pub label: String,
}

/// Statuses that close a transaction to link, unlink and sending a plan: Aumentum's
/// `LinkageManager.IsTransactionStatusAllowed` list, verbatim (only Active / lodged passes).
pub const CLOSED_STATUS_CODES: &[&str] = &["transaction_withdrawn", "transaction_postponed", "transaction_completed", "transaction_rejected"];

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LinkError {
    /// Aumentum's text (RegistryExtension log 2017-05-10).
    #[error("Link/Unlink cannot be performed because the property belongs to a Transaction of status '{0}'.")]
    TransactionClosed(String),
    /// Aumentum's text (RegistryExtension log 2017-04-11).
    #[error("Cannot retrieve information of the Transaction. Transaction number:'{0}'.")]
    TransactionNotFound(String),
}

/// Whether a property of a transaction in `status` may be linked or unlinked.
pub fn check_link_allowed(transaction_number: &str, status: Option<&TransactionStatus>) -> Result<(), LinkError> {
    match status {
        None => Err(LinkError::TransactionNotFound(transaction_number.to_string())),
        Some(s) if CLOSED_STATUS_CODES.iter().any(|c| c.eq_ignore_ascii_case(s.code.trim())) => {
            Err(LinkError::TransactionClosed(s.label.clone()))
        }
        Some(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    /// PRODUCTIONMC's settings, verbatim from the Registry inventory (lr_setting ids 45-47).
    const UPDATED: &str = "suID:LRSID,1:LinkageStatus,spatialDate1:LinkageDate,refnum:PropertyNumber,plotNumber:Plot,blockNumber:Block";
    const RETRIEVED: &str = "upin:UPIN";
    const SEARCH: &str = "uid::false:Property UID,upin:UPIN:false:Upin:true,district::false:Country";

    fn prop() -> RegistryProperty {
        RegistryProperty {
            su_id: "PLT00000857".into(),
            refnum: Some("PL1049".into()),
            spatial_date1: NaiveDate::from_ymd_opt(2016, 5, 26).and_then(|d| d.and_hms_opt(10, 11, 12)),
            upin: None,
        }
    }

    #[test]
    fn parses_productionmc_field_maps() {
        let m = parse_field_map(UPDATED);
        assert_eq!(m.len(), 6);
        assert_eq!(m[0], FieldMap { source: "suID".into(), target: "LRSID".into() });
        assert_eq!(m[1], FieldMap { source: "1".into(), target: "LinkageStatus".into() });
        assert_eq!(parse_field_map(RETRIEVED), vec![FieldMap { source: "upin".into(), target: "UPIN".into() }]);
        assert!(parse_field_map("").is_empty());
        assert!(parse_field_map(" , a: ,:b").is_empty());
    }

    #[test]
    fn link_writes_what_aumentum_wrote() {
        // Matches the imported row for UPIN PL000000003002: LRSID PLT00000857, PropertyNumber
        // PL1049, Plot 'plotNumber', Block 'blockNumber', LinkageStatus 1, a yyyy-MM-dd date.
        let u = link_updates(&parse_field_map(UPDATED), &prop());
        let s = |v: &str| Some(v.to_string());
        assert_eq!(
            u,
            vec![
                ("LRSID".into(), s("PLT00000857")),
                ("LinkageStatus".into(), s("1")),
                ("LinkageDate".into(), s("2016-05-26")),
                ("PropertyNumber".into(), s("PL1049")),
                ("Plot".into(), s("plotNumber")),
                ("Block".into(), s("blockNumber")),
            ]
        );
    }

    #[test]
    fn empty_attributes_clear_the_field() {
        let p = RegistryProperty { su_id: "PLT1".into(), ..Default::default() };
        assert_eq!(resolve("refnum", &p), Resolved::Value(None));
        assert_eq!(resolve("spatialDate1", &p), Resolved::Value(None));
        assert_eq!(resolve("refnum", &RegistryProperty { refnum: Some(String::new()), ..p.clone() }), Resolved::Value(None));
    }

    #[test]
    fn cadastre_side_link_and_unlink_follow_linkage_fields() {
        let (to_reg, to_parcel) = parse_linkage_fields("upin:UPIN;LRSID:suid,PropertyNumber:refnum,Plot:plotNumber,Block:blockNumber");
        assert_eq!(to_reg, vec![FieldMap { source: "upin".into(), target: "UPIN".into() }], "Registry upin <- parcel UPIN");
        assert_eq!(to_parcel.iter().map(|m| (m.source.as_str(), m.target.as_str())).collect::<Vec<_>>(),
            vec![("suid", "LRSID"), ("refnum", "PropertyNumber"), ("plotNumber", "Plot"), ("blockNumber", "Block")]);
        let prop = [("SuID", Some("PLT1".to_string())), ("Refnum", Some("PL9".to_string())), ("LotNumber", Some("12".to_string())), ("BlockNumber", Some("B".to_string()))];
        let u = cadastre_link_updates(&to_parcel, &prop, chrono::NaiveDate::from_ymd_opt(2026, 11, 2).unwrap());
        let s = |v: &str| Some(v.to_string());
        assert_eq!(u, vec![("LRSID".into(), s("PLT1")), ("PropertyNumber".into(), s("PL9")), ("Block".into(), s("B")),
            ("LinkageStatus".into(), s("1")), ("LinkageDate".into(), s("2026-11-02"))], "plotNumber is not a property column: skipped");
        let un = cadastre_unlink_updates(&to_parcel);
        assert_eq!(un.len(), 6);
        assert!(un.iter().any(|(k, v)| k == "LinkageStatus" && v.as_deref() == Some("0")));
        assert!(un.iter().any(|(k, v)| k == "LRSID" && v.is_none()));
    }

    #[test]
    fn unlink_only_clears_the_status() {
        assert_eq!(unlink_updates(), vec![("LinkageStatus".to_string(), Some("0".to_string()))]);
    }

    #[test]
    fn registry_takes_the_parcels_upin() {
        let u = registry_updates(&parse_field_map(RETRIEVED), |f| (f == "UPIN").then(|| "PL000000003002".to_string()));
        assert_eq!(u, vec![("upin".to_string(), Some("PL000000003002".to_string()))]);
    }

    #[test]
    fn parses_search_fields() {
        let f = parse_search_fields(SEARCH);
        assert_eq!(f.len(), 3);
        assert_eq!(f[0], SearchField { attribute: "uid".into(), gis_field: None, hidden: false, label: "Property UID".into(), key: false });
        assert_eq!(f[1], SearchField { attribute: "upin".into(), gis_field: Some("UPIN".into()), hidden: false, label: "Upin".into(), key: true });
        assert_eq!(f[2].label, "Country");
    }

    #[test]
    fn closed_transactions_refuse_link_and_unlink() {
        let st = |c: &str, l: &str| TransactionStatus { code: c.into(), label: l.into() };
        assert_eq!(check_link_allowed("PLS000006910", Some(&st("transaction_lodged", "Lodged"))), Ok(()));
        let e = check_link_allowed("PLS000006909", Some(&st("transaction_completed", "Completed"))).unwrap_err();
        assert_eq!(e.to_string(), "Link/Unlink cannot be performed because the property belongs to a Transaction of status 'Completed'.");
        assert!(check_link_allowed("X", Some(&st("TRANSACTION_WITHDRAWN ", "Withdrawn"))).is_err());
        assert!(check_link_allowed("X", Some(&st("transaction_postponed", "Postponed"))).is_err());
        assert!(check_link_allowed("X", Some(&st("transaction_rejected", "Rejected"))).is_err());
        let e = check_link_allowed("PLS000006909", None).unwrap_err();
        assert_eq!(e.to_string(), "Cannot retrieve information of the Transaction. Transaction number:'PLS000006909'.");
    }
}
