//! Keycloak token verification: a JWKS (JSON Web Key Set) and RS256 signature checks.
//!
//! Pure-Rust crypto (`rsa` + `sha2`), so this does not pull `ring`/C code into the win7
//! server target (NFR-02). The JWKS is loaded at startup (see `jwks::load`); live rotation
//! and HTTPS fetch are finished in Phase 9 alongside the TLS decision.

use std::collections::HashMap;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rsa::{BigUint, Pkcs1v15Sign, RsaPublicKey};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Errors loading a key set.
#[derive(Debug, thiserror::Error)]
pub enum JwksError {
    /// The JWKS JSON could not be parsed.
    #[error("invalid JWKS json: {0}")]
    Json(#[from] serde_json::Error),
    /// A key's modulus/exponent was not valid base64url or a valid RSA key.
    #[error("invalid RSA key for kid {kid:?}")]
    Key {
        /// The offending key id.
        kid: String,
    },
}

/// Errors verifying a token.
#[derive(Debug, thiserror::Error)]
pub enum JwtError {
    /// The token is not three dot-separated segments / not valid base64url / not JSON.
    #[error("malformed token")]
    Malformed,
    /// The algorithm in the header is not RS256.
    #[error("unsupported alg (only RS256)")]
    UnsupportedAlg,
    /// No key in the set matches the token's `kid`.
    #[error("no matching key for kid {0:?}")]
    UnknownKid(String),
    /// The signature did not verify against the matched key.
    #[error("bad signature")]
    BadSignature,
    /// The token's `exp` is in the past.
    #[error("token expired")]
    Expired,
    /// The `iss` or `aud` claim did not match the configured expectation.
    #[error("claim mismatch: {0}")]
    ClaimMismatch(&'static str),
}

/// The verified identity claims we consume.
#[derive(Clone, Debug)]
pub struct VerifiedClaims {
    /// Subject (`sub`).
    pub subject: String,
    /// Username (`preferred_username`), if present.
    pub username: Option<String>,
    /// Realm roles.
    pub roles: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Header {
    alg: String,
    kid: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Claims {
    sub: String,
    preferred_username: Option<String>,
    exp: Option<i64>,
    iss: Option<String>,
    #[serde(default)]
    aud: Aud,
    #[serde(default)]
    realm_access: RealmAccess,
}

/// `aud` may be a single string or an array of strings.
#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum Aud {
    #[default]
    Missing,
    One(String),
    Many(Vec<String>),
}

impl Aud {
    fn contains(&self, expected: &str) -> bool {
        match self {
            Aud::Missing => false,
            Aud::One(a) => a == expected,
            Aud::Many(v) => v.iter().any(|a| a == expected),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct RealmAccess {
    #[serde(default)]
    roles: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RawJwk {
    kty: String,
    kid: String,
    n: String,
    e: String,
}

#[derive(Debug, Deserialize)]
struct RawJwks {
    keys: Vec<RawJwk>,
}

/// A set of RSA public keys keyed by `kid`.
#[derive(Clone, Debug)]
pub struct Jwks {
    keys: HashMap<String, RsaPublicKey>,
}

impl Jwks {
    /// Parse a standard JWKS document (e.g. Keycloak's `.../certs`). Non-RSA keys are skipped.
    pub fn from_json(json: &str) -> Result<Self, JwksError> {
        let raw: RawJwks = serde_json::from_str(json)?;
        let mut keys = HashMap::new();
        for jwk in raw.keys {
            if jwk.kty != "RSA" {
                continue;
            }
            let n = decode_biguint(&jwk.n).ok_or_else(|| JwksError::Key { kid: jwk.kid.clone() })?;
            let e = decode_biguint(&jwk.e).ok_or_else(|| JwksError::Key { kid: jwk.kid.clone() })?;
            let key =
                RsaPublicKey::new(n, e).map_err(|_| JwksError::Key { kid: jwk.kid.clone() })?;
            keys.insert(jwk.kid, key);
        }
        Ok(Self { keys })
    }

    /// Number of keys in the set.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Whether the set is empty. (Paired with `len` for clippy; kept for API completeness.)
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Verify an RS256 token's signature and claims.
    ///
    /// `expected_iss`/`expected_aud`, when provided, must match the token's `iss`/`aud`.
    pub fn verify(
        &self,
        token: &str,
        expected_iss: Option<&str>,
        expected_aud: Option<&str>,
    ) -> Result<VerifiedClaims, JwtError> {
        let mut parts = token.split('.');
        let (h, p, s) = match (parts.next(), parts.next(), parts.next(), parts.next()) {
            (Some(h), Some(p), Some(s), None) => (h, p, s),
            _ => return Err(JwtError::Malformed),
        };

        let header: Header = decode_json(h).ok_or(JwtError::Malformed)?;
        if header.alg != "RS256" {
            return Err(JwtError::UnsupportedAlg);
        }
        let kid = header.kid.unwrap_or_default();
        let key = self
            .keys
            .get(&kid)
            .ok_or_else(|| JwtError::UnknownKid(kid.clone()))?;

        let signature = URL_SAFE_NO_PAD.decode(s).map_err(|_| JwtError::Malformed)?;
        let signing_input = format!("{h}.{p}");
        let digest = Sha256::digest(signing_input.as_bytes());
        key.verify(Pkcs1v15Sign::new::<Sha256>(), &digest, &signature)
            .map_err(|_| JwtError::BadSignature)?;

        let claims: Claims = decode_json(p).ok_or(JwtError::Malformed)?;
        if let Some(exp) = claims.exp {
            // 60s leeway for clock skew.
            if exp + 60 < chrono::Utc::now().timestamp() {
                return Err(JwtError::Expired);
            }
        }
        if let Some(iss) = expected_iss {
            if claims.iss.as_deref() != Some(iss) {
                return Err(JwtError::ClaimMismatch("iss"));
            }
        }
        if let Some(aud) = expected_aud {
            if !claims.aud.contains(aud) {
                return Err(JwtError::ClaimMismatch("aud"));
            }
        }

        Ok(VerifiedClaims {
            subject: claims.sub,
            username: claims.preferred_username,
            roles: claims.realm_access.roles,
        })
    }
}

fn decode_biguint(b64url: &str) -> Option<BigUint> {
    let bytes = URL_SAFE_NO_PAD.decode(b64url).ok()?;
    Some(BigUint::from_bytes_be(&bytes))
}

fn decode_json<T: for<'de> Deserialize<'de>>(segment: &str) -> Option<T> {
    let bytes = URL_SAFE_NO_PAD.decode(segment).ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::pkcs1v15::SigningKey;
    use rsa::signature::{SignatureEncoding, Signer};
    use rsa::traits::PublicKeyParts;
    use rsa::RsaPrivateKey;

    /// Build a JWKS JSON for a public key, plus a helper to mint signed tokens.
    struct TestIdp {
        kid: String,
        signing_key: SigningKey<Sha256>,
        jwks_json: String,
    }

    impl TestIdp {
        fn new(kid: &str) -> Self {
            let mut rng = rand::thread_rng();
            let priv_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
            let pub_key = priv_key.to_public_key();
            let n = URL_SAFE_NO_PAD.encode(pub_key.n().to_bytes_be());
            let e = URL_SAFE_NO_PAD.encode(pub_key.e().to_bytes_be());
            let jwks_json = format!(
                r#"{{"keys":[{{"kty":"RSA","kid":"{kid}","alg":"RS256","use":"sig","n":"{n}","e":"{e}"}}]}}"#
            );
            Self {
                kid: kid.to_string(),
                signing_key: SigningKey::<Sha256>::new(priv_key),
                jwks_json,
            }
        }

        fn mint(&self, claims_json: &str) -> String {
            let header = format!(r#"{{"alg":"RS256","typ":"JWT","kid":"{}"}}"#, self.kid);
            let h = URL_SAFE_NO_PAD.encode(header.as_bytes());
            let p = URL_SAFE_NO_PAD.encode(claims_json.as_bytes());
            let signing_input = format!("{h}.{p}");
            let sig = self.signing_key.sign(signing_input.as_bytes());
            let s = URL_SAFE_NO_PAD.encode(sig.to_bytes());
            format!("{h}.{p}.{s}")
        }
    }

    fn future_exp() -> i64 {
        chrono::Utc::now().timestamp() + 3600
    }

    #[test]
    fn verifies_a_well_formed_token() {
        let idp = TestIdp::new("key-1");
        let jwks = Jwks::from_json(&idp.jwks_json).unwrap();
        assert_eq!(jwks.len(), 1);
        let token = idp.mint(&format!(
            r#"{{"sub":"u1","preferred_username":"surveyor","exp":{},"realm_access":{{"roles":["editor","approver"]}}}}"#,
            future_exp()
        ));
        let claims = jwks.verify(&token, None, None).unwrap();
        assert_eq!(claims.subject, "u1");
        assert_eq!(claims.username.as_deref(), Some("surveyor"));
        assert!(claims.roles.contains(&"approver".to_string()));
    }

    #[test]
    fn rejects_a_tampered_payload() {
        let idp = TestIdp::new("key-1");
        let jwks = Jwks::from_json(&idp.jwks_json).unwrap();
        let token = idp.mint(&format!(r#"{{"sub":"u1","exp":{}}}"#, future_exp()));
        // Swap the payload segment for a different one; signature no longer matches.
        let mut parts: Vec<&str> = token.split('.').collect();
        let forged = URL_SAFE_NO_PAD.encode(br#"{"sub":"admin","exp":9999999999}"#);
        parts[1] = &forged;
        let tampered = parts.join(".");
        assert!(matches!(
            jwks.verify(&tampered, None, None),
            Err(JwtError::BadSignature)
        ));
    }

    #[test]
    fn rejects_unknown_kid() {
        let signer = TestIdp::new("key-1");
        let other = TestIdp::new("key-2");
        // Verify a key-2 token against a set that only knows key-1.
        let jwks = Jwks::from_json(&signer.jwks_json).unwrap();
        let token = other.mint(&format!(r#"{{"sub":"u1","exp":{}}}"#, future_exp()));
        assert!(matches!(
            jwks.verify(&token, None, None),
            Err(JwtError::UnknownKid(_))
        ));
    }

    #[test]
    fn rejects_expired_token() {
        let idp = TestIdp::new("key-1");
        let jwks = Jwks::from_json(&idp.jwks_json).unwrap();
        let past = chrono::Utc::now().timestamp() - 3600;
        let token = idp.mint(&format!(r#"{{"sub":"u1","exp":{past}}}"#));
        assert!(matches!(
            jwks.verify(&token, None, None),
            Err(JwtError::Expired)
        ));
    }

    #[test]
    fn enforces_issuer_and_audience_when_configured() {
        let idp = TestIdp::new("key-1");
        let jwks = Jwks::from_json(&idp.jwks_json).unwrap();
        let token = idp.mint(&format!(
            r#"{{"sub":"u1","exp":{},"iss":"https://kc/realms/cadastre","aud":["cadastre-server","account"]}}"#,
            future_exp()
        ));
        // Correct iss + aud present in the array.
        assert!(jwks
            .verify(&token, Some("https://kc/realms/cadastre"), Some("cadastre-server"))
            .is_ok());
        // Wrong issuer.
        assert!(matches!(
            jwks.verify(&token, Some("https://evil/realms/x"), None),
            Err(JwtError::ClaimMismatch("iss"))
        ));
        // Audience not present.
        assert!(matches!(
            jwks.verify(&token, None, Some("some-other-client")),
            Err(JwtError::ClaimMismatch("aud"))
        ));
    }
}
