//! ArcSDE versioned-geodatabase access: feature-class registry constants and the DEFAULT-version
//! reconstruction SQL. See `docs/aumentum-import.md` for the derivation.
//!
//! The business table of a multiversioned feature class holds only base rows; the current rows of
//! the DEFAULT version are `base ∪ adds-in-lineage − deletes-in-lineage`, where edits live in the
//! delta tables `a<registration_id>` / `D<registration_id>`.

/// A registered SDE feature class and the pieces needed to reconstruct it.
#[derive(Clone, Copy, Debug)]
pub struct FeatureClass {
    /// Business (base) table name, e.g. `PLATEAU_STATE_PARCELS`.
    pub business_table: &'static str,
    /// `sde.sde_table_registry.registration_id`; names the `a<id>`/`D<id>` delta tables.
    pub registration_id: u32,
    /// Whether the class is registered as multiversioned (needs reconstruction).
    pub versioned: bool,
}

impl FeatureClass {
    /// The adds (inserts) delta table, e.g. `a11`.
    pub fn adds_table(&self) -> String {
        format!("a{}", self.registration_id)
    }

    /// The deletes delta table, e.g. `D11`.
    pub fn deletes_table(&self) -> String {
        format!("D{}", self.registration_id)
    }
}

/// Parcels — `PLATEAU_STATE_PARCELS`, registration_id 11, versioned (286 base vs 21,266 current).
pub const PARCELS: FeatureClass = FeatureClass {
    business_table: "PLATEAU_STATE_PARCELS",
    registration_id: 11,
    versioned: true,
};

/// Survey control coordinates — `PLAGIS_COORDINATES`, registration_id 155, **not** versioned.
pub const COORDINATES: FeatureClass = FeatureClass {
    business_table: "PLAGIS_COORDINATES",
    registration_id: 155,
    versioned: false,
};

/// Build a `SELECT` returning the current rows of `fc` for the DEFAULT version.
///
/// `columns` is the projection (geometry should already be wrapped, e.g. `SHAPE.STAsText() AS wkt`).
/// For a non-versioned class this is a plain select from the business table. For a versioned class
/// it resolves DEFAULT's state dynamically, materialises its lineage, then unions base + adds minus
/// deletes. Caller runs it against the `sde` database.
pub fn current_rows_sql(fc: &FeatureClass, columns: &str) -> String {
    if !fc.versioned {
        return format!("SELECT {columns} FROM sde.{};", fc.business_table);
    }
    let a = fc.adds_table();
    let d = fc.deletes_table();
    let business = fc.business_table;
    format!(
        "SET NOCOUNT ON;\n\
         IF OBJECT_ID('tempdb..#L') IS NOT NULL DROP TABLE #L;\n\
         DECLARE @default_state bigint = (SELECT state_id FROM sde.SDE_versions WHERE name = 'DEFAULT');\n\
         SELECT lineage_id INTO #L FROM sde.SDE_state_lineages\n\
         WHERE lineage_name = (SELECT lineage_name FROM sde.SDE_states WHERE state_id = @default_state);\n\
         CREATE CLUSTERED INDEX ix_L ON #L(lineage_id);\n\
         SELECT {columns} FROM sde.{business} b\n\
         WHERE NOT EXISTS (SELECT 1 FROM sde.{d} d\n\
             WHERE d.SDE_DELETES_ROW_ID = b.OBJECTID AND d.SDE_STATE_ID = 0\n\
               AND d.DELETED_AT IN (SELECT lineage_id FROM #L))\n\
         UNION ALL\n\
         SELECT {columns} FROM sde.{a} a\n\
         WHERE a.SDE_STATE_ID IN (SELECT lineage_id FROM #L)\n\
           AND NOT EXISTS (SELECT 1 FROM sde.{d} d\n\
               WHERE d.SDE_DELETES_ROW_ID = a.OBJECTID AND d.SDE_STATE_ID = a.SDE_STATE_ID\n\
                 AND d.DELETED_AT IN (SELECT lineage_id FROM #L));"
    )
}

/// Projection used for parcels: identifiers + status + job link + geometry as WKT.
pub const PARCEL_COLUMNS: &str =
    "OBJECTID, UPIN, Status, CONVERT(varchar(36), JobID) AS JobID, PlanNo, CadZone, PlotNo, \
     BlockNo, SHAPE.STAsText() AS wkt, SHAPE.STSrid AS srid";

/// Projection used for control coordinates. `X`/`Y` are SQL `numeric`; cast to `float` so the
/// client reads plain `f64` (no decimal dependency).
pub const COORDINATE_COLUMNS: &str =
    "OBJECTID, Alias, CONVERT(float, X) AS X, CONVERT(float, Y) AS Y";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_table_names_follow_registration_id() {
        assert_eq!(PARCELS.adds_table(), "a11");
        assert_eq!(PARCELS.deletes_table(), "D11");
    }

    #[test]
    fn versioned_sql_reconstructs_default_version() {
        let sql = current_rows_sql(&PARCELS, PARCEL_COLUMNS);
        // Resolves DEFAULT dynamically (not a hard-coded state id).
        assert!(sql.contains("name = 'DEFAULT'"));
        assert!(sql.contains("SDE_state_lineages"));
        // Unions base with adds, excluding deletes, via the a11/D11 delta tables.
        assert!(sql.contains("FROM sde.PLATEAU_STATE_PARCELS b"));
        assert!(sql.contains("FROM sde.a11 a"));
        assert!(sql.contains("sde.D11 d"));
        assert!(sql.contains("UNION ALL"));
        assert_eq!(sql.matches("NOT EXISTS").count(), 2);
    }

    #[test]
    fn non_versioned_sql_is_a_plain_select() {
        let sql = current_rows_sql(&COORDINATES, COORDINATE_COLUMNS);
        assert_eq!(
            sql,
            "SELECT OBJECTID, Alias, CONVERT(float, X) AS X, CONVERT(float, Y) AS Y \
             FROM sde.PLAGIS_COORDINATES;"
        );
        assert!(!sql.contains("UNION"));
    }
}
