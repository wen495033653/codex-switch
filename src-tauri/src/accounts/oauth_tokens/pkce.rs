use crate::accounts::{OAUTH_AUTHORIZE_ENDPOINT, OAUTH_CLIENT_ID, OAUTH_SCOPE};
use base64::{engine::general_purpose, Engine as _};
use rand::RngCore;
use sha2::{Digest, Sha256};

pub(crate) fn random_urlsafe(bytes_len: usize) -> String {
    let mut bytes = vec![0_u8; bytes_len];
    rand::thread_rng().fill_bytes(&mut bytes);
    general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub(crate) fn generate_pkce() -> (String, String) {
    let verifier = random_urlsafe(32);
    let digest = Sha256::digest(verifier.as_bytes());
    let challenge = general_purpose::URL_SAFE_NO_PAD.encode(digest);
    (verifier, challenge)
}

pub(crate) fn build_oauth_auth_url(
    port: u16,
    code_challenge: &str,
    state: &str,
) -> Result<String, String> {
    let redirect_uri = format!("http://localhost:{port}/auth/callback");
    let mut url = url::Url::parse(OAUTH_AUTHORIZE_ENDPOINT)
        .map_err(|err| format!("OAuth authorize endpoint 无效: {err}"))?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", OAUTH_CLIENT_ID)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("scope", OAUTH_SCOPE)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("state", state)
        .append_pair("originator", "codex_cli_rs");
    Ok(url.to_string())
}
