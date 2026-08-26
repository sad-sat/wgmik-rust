use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String, // user_id as string
    pub sv: i64,     // session_version
    pub exp: i64,    // expiration unix timestamp
}

pub fn create_access_token(user_id: i64, session_version: i64, secret_key: &str, expires_in_days: Option<i64>) -> Result<String, jsonwebtoken::errors::Error> {
    let days = expires_in_days.unwrap_or(7);
    let exp = (Utc::now() + Duration::days(days)).timestamp();
    let claims = Claims {
        sub: user_id.to_string(),
        sv: session_version,
        exp,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret_key.as_bytes()),
    )
}

pub fn verify_token(token: &str, secret_key: &str) -> Option<(i64, i64)> {
    let mut validation = Validation::default();
    validation.validate_exp = true;
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret_key.as_bytes()),
        &validation,
    ).ok()?;

    let user_id = data.claims.sub.parse::<i64>().ok()?;
    Some((user_id, data.claims.sv))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jwt_roundtrip() {
        let secret = "very-secure-secret-key-1234567890-test";
        let token = create_access_token(42, 3, secret, Some(1)).expect("create token");
        let decoded = verify_token(&token, secret);
        assert_eq!(decoded, Some((42, 3)));

        let bad_secret = verify_token(&token, "wrong-secret-key");
        assert_eq!(bad_secret, None);
    }
}
