use rand::RngCore;
use rand::rngs::OsRng;

pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn token_from_request(override_token: &str) -> String {
    if override_token.trim().is_empty() {
        generate_token()
    } else {
        override_token.trim().to_string()
    }
}

pub fn require_sidecar_token(token: &str) -> Result<String, String> {
    if token.trim().is_empty() {
        return Err("sidecar_token is required".to_string());
    }
    Ok(token.trim().to_string())
}
