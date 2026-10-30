//! OpenAPI document assembled from the `#[utoipa::path]` annotations on each handler.

use utoipa::OpenApi;

use crate::models::{BeaconDto, NewBeacon, ParcelDto, TolerancesDto};
use crate::routes::{beacons, health, parcels, tolerances};

/// The generated API description, served at `/api-docs/openapi.json`.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "PLAGIS Cadastre Editor API",
        version = "0.1.0",
        description = "Parcel capture and maintenance server (Phase 2 skeleton)."
    ),
    paths(
        health::health,
        health::ready,
        tolerances::get_tolerances,
        tolerances::put_tolerances,
        beacons::list_beacons,
        beacons::get_beacon,
        beacons::create_beacon,
        parcels::list_parcels,
    ),
    components(schemas(TolerancesDto, BeaconDto, NewBeacon, ParcelDto)),
    tags(
        (name = "ops", description = "Liveness and readiness"),
        (name = "config", description = "Configurable tolerances (FR-ADM-03)"),
        (name = "beacons", description = "Survey beacons"),
        (name = "parcels", description = "Parcels"),
    )
)]
pub struct ApiDoc;
