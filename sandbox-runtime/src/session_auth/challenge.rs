//! Challenge generation and single-use consumption for the EIP-191 flow.

use super::*;

/// Generate a random challenge nonce for EIP-191 signing.
///
/// Returns an error if the challenge store is at capacity ([`MAX_CHALLENGES`]),
/// preventing memory exhaustion from unauthenticated requests.
pub fn create_challenge() -> Result<Challenge> {
    let mut nonce_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = hex::encode(nonce_bytes);
    let now = now_secs();

    let message = format!(
        "Sign this message to authenticate with Tangle Sandbox.\n\nNonce: {nonce}\nExpires: {}",
        now + CHALLENGE_TTL_SECS,
    );

    let challenge = Challenge {
        nonce: nonce.clone(),
        message,
        expires_at: now + CHALLENGE_TTL_SECS,
    };

    let mut map = CHALLENGES.lock().unwrap_or_else(|e| e.into_inner());
    if map.len() >= MAX_CHALLENGES {
        return Err(SandboxError::Unavailable(
            "Challenge capacity exceeded, try again later".into(),
        ));
    }
    map.insert(nonce, challenge.clone());

    Ok(challenge)
}

/// Consume and validate a challenge nonce. Returns the challenge message if valid.
pub(crate) fn consume_challenge(nonce: &str) -> Result<String> {
    let mut map = CHALLENGES.lock().unwrap_or_else(|e| e.into_inner());
    let challenge = map
        .remove(nonce)
        .ok_or_else(|| SandboxError::Auth("Challenge not found or already consumed".into()))?;

    if now_secs() > challenge.expires_at {
        return Err(SandboxError::Auth("Challenge expired".into()));
    }

    Ok(challenge.message)
}
