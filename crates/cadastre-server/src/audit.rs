//! Hash-chained audit log (NFR-07/11).
//!
//! Each entry's `hash = SHA-256(prev_hash | actor | action | entity | entity_id | created_at |
//! canonical(payload))`, so any tampering with a past entry (or a gap) breaks the chain and is
//! detectable by [`verify_entries`]. The hashing is pure and unit-tested; [`append`] and the
//! verify loader do the DB I/O.

use chrono::{DateTime, SecondsFormat, Timelike, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::Row;

/// A stored audit entry (as needed to re-verify the chain).
#[derive(Clone, Debug)]
pub struct AuditEntry {
    /// Monotonic id.
    pub id: i64,
    /// Who performed the action (subject).
    pub actor: String,
    /// Action name.
    pub action: String,
    /// Entity kind.
    pub entity: String,
    /// Entity id, if any.
    pub entity_id: Option<String>,
    /// Structured payload.
    pub payload: Value,
    /// Hash of the previous entry (None for the first).
    pub prev_hash: Option<String>,
    /// This entry's hash.
    pub hash: String,
    /// When it was recorded (microsecond precision — matches Postgres).
    pub created_at: DateTime<Utc>,
}

/// Deterministic JSON: object keys sorted lexicographically, compact. Stable across the
/// jsonb round-trip, so re-hashing on verify matches the stored hash (keep payloads to
/// integers/strings/bools so Postgres does not renormalise numbers).
pub fn canonical_json(v: &Value) -> String {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .iter()
                .map(|k| {
                    format!(
                        "{}:{}",
                        Value::String((*k).clone()),
                        canonical_json(&map[*k])
                    )
                })
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Array(a) => format!(
            "[{}]",
            a.iter().map(canonical_json).collect::<Vec<_>>().join(",")
        ),
        other => other.to_string(),
    }
}

/// Compute the chain hash for one entry's fields.
#[allow(clippy::too_many_arguments)]
pub fn compute_hash(
    prev_hash: Option<&str>,
    actor: &str,
    action: &str,
    entity: &str,
    entity_id: Option<&str>,
    created_at: &DateTime<Utc>,
    payload: &Value,
) -> String {
    let mut h = Sha256::new();
    for field in [
        prev_hash.unwrap_or(""),
        actor,
        action,
        entity,
        entity_id.unwrap_or(""),
        &created_at.to_rfc3339_opts(SecondsFormat::Micros, true),
        &canonical_json(payload),
    ] {
        h.update(field.as_bytes());
        h.update([0x1f]); // unit separator between fields
    }
    hex(&h.finalize())
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// The outcome of verifying the chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VerifyResult {
    /// Whether the whole chain is intact.
    pub ok: bool,
    /// Number of entries checked.
    pub checked: usize,
    /// Id of the first broken entry, if any.
    pub first_bad: Option<i64>,
}

/// Re-verify an ordered (by id ascending) slice of entries: each `prev_hash` must equal the
/// previous entry's `hash`, and each `hash` must match a recomputation of its fields.
pub fn verify_entries(entries: &[AuditEntry]) -> VerifyResult {
    let mut prev: Option<&str> = None;
    for e in entries {
        let links = e.prev_hash.as_deref() == prev;
        let recomputed = compute_hash(
            prev,
            &e.actor,
            &e.action,
            &e.entity,
            e.entity_id.as_deref(),
            &e.created_at,
            &e.payload,
        );
        if !links || recomputed != e.hash {
            return VerifyResult {
                ok: false,
                checked: entries.len(),
                first_bad: Some(e.id),
            };
        }
        prev = Some(&e.hash);
    }
    VerifyResult {
        ok: true,
        checked: entries.len(),
        first_bad: None,
    }
}

/// Append an entry to the chain within the caller's transaction/connection.
pub async fn append(
    conn: &mut sqlx::PgConnection,
    actor: &str,
    action: &str,
    entity: &str,
    entity_id: Option<&str>,
    payload: Value,
) -> Result<String, sqlx::Error> {
    let prev: Option<String> = sqlx::query("SELECT hash FROM cadastre.audit_log ORDER BY id DESC LIMIT 1")
        .fetch_optional(&mut *conn)
        .await?
        .map(|r| r.get::<String, _>("hash"));

    // Microsecond precision so the stored value equals what we hash (Postgres is µs).
    let now = Utc::now();
    let created = now.with_nanosecond(now.nanosecond() / 1000 * 1000).unwrap_or(now);

    let hash = compute_hash(prev.as_deref(), actor, action, entity, entity_id, &created, &payload);

    sqlx::query(
        "INSERT INTO cadastre.audit_log \
         (actor, action, entity, entity_id, payload, prev_hash, hash, created_at) \
         VALUES ($1, $2, $3, $4, $5::jsonb, $6, $7, $8)",
    )
    .bind(actor)
    .bind(action)
    .bind(entity)
    .bind(entity_id)
    .bind(canonical_json(&payload))
    .bind(&prev)
    .bind(&hash)
    .bind(created)
    .execute(&mut *conn)
    .await?;

    Ok(hash)
}

/// Load the whole chain (ascending) and verify it.
pub async fn verify_chain(pool: &sqlx::PgPool) -> Result<VerifyResult, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, actor, action, entity, entity_id, payload::text AS payload, prev_hash, hash, created_at \
         FROM cadastre.audit_log ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    let entries: Vec<AuditEntry> = rows
        .iter()
        .map(|r| AuditEntry {
            id: r.get("id"),
            actor: r.get("actor"),
            action: r.get("action"),
            entity: r.get("entity"),
            entity_id: r.get("entity_id"),
            payload: serde_json::from_str(&r.get::<String, _>("payload")).unwrap_or(Value::Null),
            prev_hash: r.get("prev_hash"),
            hash: r.get("hash"),
            created_at: r.get("created_at"),
        })
        .collect();

    Ok(verify_entries(&entries))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(id: i64, prev: Option<&str>, payload: Value) -> AuditEntry {
        let created = DateTime::from_timestamp(1_700_000_000 + id, 0).unwrap();
        let hash = compute_hash(prev, "dev", "post", "job", Some("J1"), &created, &payload);
        AuditEntry {
            id,
            actor: "dev".into(),
            action: "post".into(),
            entity: "job".into(),
            entity_id: Some("J1".into()),
            payload,
            prev_hash: prev.map(str::to_string),
            hash,
            created_at: created,
        }
    }

    #[test]
    fn canonical_json_sorts_keys() {
        let a = json!({"b": 1, "a": 2, "c": {"z": 1, "y": 2}});
        assert_eq!(canonical_json(&a), r#"{"a":2,"b":1,"c":{"y":2,"z":1}}"#);
    }

    #[test]
    fn a_well_formed_chain_verifies() {
        let e1 = entry(1, None, json!({"n": 1}));
        let e2 = entry(2, Some(&e1.hash), json!({"n": 2}));
        let e3 = entry(3, Some(&e2.hash), json!({"n": 3}));
        let r = verify_entries(&[e1, e2, e3]);
        assert!(r.ok);
        assert_eq!(r.checked, 3);
        assert_eq!(r.first_bad, None);
    }

    #[test]
    fn tampering_with_a_payload_breaks_the_chain() {
        let e1 = entry(1, None, json!({"n": 1}));
        let e2 = entry(2, Some(&e1.hash), json!({"n": 2}));
        let mut tampered = e2.clone();
        tampered.payload = json!({"n": 999}); // hash no longer matches
        let r = verify_entries(&[e1, tampered]);
        assert!(!r.ok);
        assert_eq!(r.first_bad, Some(2));
    }

    #[test]
    fn a_broken_link_is_detected() {
        let e1 = entry(1, None, json!({"n": 1}));
        // e2 points at the wrong previous hash.
        let e2 = entry(2, Some("deadbeef"), json!({"n": 2}));
        let r = verify_entries(&[e1, e2]);
        assert!(!r.ok);
        assert_eq!(r.first_bad, Some(2));
    }
}
