//! Promote a staged import run into the live cadastre fabric (Phase 8).
//!
//! Reads the `import` schema (populated by `cadastre-import`) and upserts the current fabric:
//! only **active** parcels become live (pending/historic/archived remain in staging as history),
//! and all beacons are promoted. Idempotent (upsert on the natural key) and recorded in the audit
//! log. Aumentum `JobID`s are not cadastre jobs, so promoted parcels have no `job_id`.

use axum::extract::{Path, State};
use axum::Json;
use serde_json::json;

use crate::auth::AuthUser;
use crate::error::{ApiResult, AppError};
use crate::models::PromoteResult;
use crate::state::AppState;
use crate::audit;

/// Promote one import run.
#[utoipa::path(post, path = "/api/v1/import/promote/{run_id}", tag = "import",
    params(("run_id" = i64, Path, description = "import.run id")),
    responses(
        (status = 200, body = PromoteResult),
        (status = 403, description = "Requires admin role"),
        (status = 404, description = "Import run not found"),
    ))]
pub async fn promote_run(
    State(state): State<AppState>,
    user: AuthUser,
    Path(run_id): Path<i64>,
) -> ApiResult<Json<PromoteResult>> {
    user.require_role("admin")?;

    let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM import.run WHERE id = $1")
        .bind(run_id)
        .fetch_optional(&state.pool)
        .await?;
    if exists.is_none() {
        return Err(AppError::NotFound(format!("import run {run_id}")));
    }

    let mut tx = state.pool.begin().await?;

    let beacons = sqlx::query(
        // Imported beacons come from Aumentum's PLAGIS_COORDINATES: the survey control network.
        "INSERT INTO cadastre.beacon (number, geom, is_control) \
         SELECT number, geom, true FROM import.beacon WHERE run_id = $1 \
         ON CONFLICT (number) DO UPDATE SET geom = EXCLUDED.geom, is_control = true, updated_at = now()",
    )
    .bind(run_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    let parcels = sqlx::query(
        "INSERT INTO cadastre.parcel (parcel_number, geom, computed_area, status) \
         SELECT parcel_number, geom, computed_area, 'posted' FROM import.parcel \
         WHERE run_id = $1 AND status = 'active' AND parcel_number IS NOT NULL \
         ON CONFLICT (parcel_number) DO UPDATE \
           SET geom = EXCLUDED.geom, computed_area = EXCLUDED.computed_area, \
               status = 'posted', updated_at = now()",
    )
    .bind(run_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    // Every UPIN in the run is recorded as issued, in every status and including exceptions, so
    // the allocator can never hand one out again. Historic/archived parcels are not live, but
    // registry titles may still name their UPINs.
    let upins_recorded = sqlx::query(
        "INSERT INTO cadastre.issued_upin (upin, source) \
         SELECT DISTINCT ON (parcel_number) parcel_number, 'import:' || run_id || ':' || status \
         FROM import.parcel WHERE run_id = $1 AND parcel_number IS NOT NULL \
         ORDER BY parcel_number \
         ON CONFLICT (upin) DO NOTHING",
    )
    .bind(run_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    sqlx::query(
        "INSERT INTO cadastre.issued_upin (upin, source) \
         SELECT DISTINCT ON (parcel_number) parcel_number, 'import-exception:' || run_id \
         FROM import.exception \
         WHERE run_id = $1 AND parcel_number IS NOT NULL AND btrim(parcel_number) <> '' \
         ORDER BY parcel_number \
         ON CONFLICT (upin) DO NOTHING",
    )
    .bind(run_id)
    .execute(&mut *tx)
    .await?;
    // Every UPIN Aumentum ever issued, in any version or history state (upins-issued.psv).
    sqlx::query(
        "INSERT INTO cadastre.issued_upin (upin, source) \
         SELECT btrim(upin), 'aumentum:' || COALESCE(first_source, 'unknown') FROM import.issued_upin \
         WHERE run_id = $1 AND btrim(upin) NOT IN ('', '0') \
         ON CONFLICT (upin) DO NOTHING",
    )
    .bind(run_id)
    .execute(&mut *tx)
    .await?;

    // Parcel attributes, every field verbatim, including the Registry linkage (LRSID,
    // LinkageStatus, LinkageDate). Upsert on Aumentum's OBJECTID so re-promotion is idempotent.
    let attributes = sqlx::query(
        "INSERT INTO cadastre.parcel_attribute (object_id, upin, dataset_name, region_name, parcel_id, \
           parcel_name, parent, property_number, property_type, plot, block, location, parcel_area, \
           area_unit, plan_number, lga, status, can_rollback, is_strata, mutation_type, creation_date, \
           last_edit_date, consolidation_date, archive_reason, archive_date, lrs_id, linkage_status, \
           linkage_date, surveyor_general, import_run, updated_at) \
         SELECT object_id, NULLIF(btrim(upin), ''), dataset_name, region_name, NULLIF(parcel_id, '')::uuid, \
           parcel_name, parent, property_number, property_type, plot, block, location, \
           NULLIF(parcel_area, '')::double precision, NULLIF(area_unit, '')::smallint, plan_number, lga, status, \
           NULLIF(can_rollback, '')::smallint, NULLIF(is_strata, '')::smallint, NULLIF(mutation_type, '')::smallint, \
           NULLIF(creation_date, '')::timestamp, NULLIF(last_edit_date, '')::timestamp, \
           NULLIF(consolidation_date, '')::timestamp, archive_reason, NULLIF(archive_date, '')::timestamp, \
           NULLIF(btrim(lrs_id), ''), NULLIF(linkage_status, '')::smallint, linkage_date, surveyor_general, \
           run_id, now() \
         FROM import.parcel_attribute WHERE run_id = $1 \
         ON CONFLICT (object_id) DO UPDATE SET \
           upin = EXCLUDED.upin, dataset_name = EXCLUDED.dataset_name, region_name = EXCLUDED.region_name, \
           parcel_id = EXCLUDED.parcel_id, parcel_name = EXCLUDED.parcel_name, parent = EXCLUDED.parent, \
           property_number = EXCLUDED.property_number, property_type = EXCLUDED.property_type, \
           plot = EXCLUDED.plot, block = EXCLUDED.block, location = EXCLUDED.location, \
           parcel_area = EXCLUDED.parcel_area, area_unit = EXCLUDED.area_unit, \
           plan_number = EXCLUDED.plan_number, lga = EXCLUDED.lga, status = EXCLUDED.status, \
           can_rollback = EXCLUDED.can_rollback, is_strata = EXCLUDED.is_strata, \
           mutation_type = EXCLUDED.mutation_type, creation_date = EXCLUDED.creation_date, \
           last_edit_date = EXCLUDED.last_edit_date, consolidation_date = EXCLUDED.consolidation_date, \
           archive_reason = EXCLUDED.archive_reason, archive_date = EXCLUDED.archive_date, \
           lrs_id = EXCLUDED.lrs_id, linkage_status = EXCLUDED.linkage_status, \
           linkage_date = EXCLUDED.linkage_date, surveyor_general = EXCLUDED.surveyor_general, \
           import_run = EXCLUDED.import_run, updated_at = now()",
    )
    .bind(run_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    let next_upin: i64 = sqlx::query_scalar("SELECT cadastre.sync_upin_sequence()")
        .fetch_one(&mut *tx)
        .await?;

    audit::append(
        &mut tx,
        &user.subject,
        "promote",
        "import_run",
        Some(&run_id.to_string()),
        json!({
            "parcels": parcels as i64,
            "beacons": beacons as i64,
            "upins_recorded": upins_recorded as i64,
            "attributes": attributes as i64,
            "next_upin": crate::numbering::format_upin(next_upin),
        }),
    )
    .await?;

    tx.commit().await?;
    Ok(Json(PromoteResult {
        run_id,
        parcels_promoted: parcels as usize,
        beacons_promoted: beacons as usize,
        attributes_promoted: attributes as usize,
    }))
}
