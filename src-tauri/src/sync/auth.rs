//! Signing in to Entra.
//!
//! Authorization code with PKCE, through the system browser, with a loopback
//! redirect. The app is a **public client**: it ships to end users, so anything
//! embedded in the binary is public and it can hold no secret. PKCE is what
//! replaces one — the authorization code is worthless without the verifier,
//! which never leaves this process.
//!
//! The pure parts live here and are tested. Opening a browser and listening on
//! a socket are thin wrappers around them, because the mistakes worth catching
//! are in what gets built and what gets checked, not in the plumbing.

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Characters allowed unescaped in a URL query value.
///
/// An allowlist, because the alternative is a list of characters that need
/// escaping and a bet that it is complete. A scope value like
/// `api://<guid>/Sync.Access` contains `:` and `/`, and unescaped those turn
/// the parameter into something Entra reads differently.
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// base64url without padding, as the PKCE and JWT specs use.
fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        let chars = [
            ALPHABET[(n >> 18) as usize & 63],
            ALPHABET[(n >> 12) as usize & 63],
            ALPHABET[(n >> 6) as usize & 63],
            ALPHABET[n as usize & 63],
        ];
        // Padding is omitted rather than trimmed to '=' — the spec says
        // unpadded, and Entra rejects a padded challenge.
        let keep = chunk.len() + 1;
        for c in chars.iter().take(keep) {
            out.push(*c as char);
        }
    }
    out
}

/// 32 random bytes, from the OS.
fn random_bytes() -> [u8; 32] {
    // Two v4 UUIDs rather than another dependency. Both come from the platform
    // CSPRNG; six bits carry version and variant, which leaves 244 bits of
    // entropy — far past what a code verifier needs.
    let mut out = [0u8; 32];
    out[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    out[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    out
}

/// A PKCE verifier and the challenge derived from it.
///
/// The verifier is the secret. It is sent only in the token request, over TLS,
/// after the code comes back — which is what makes an intercepted code useless.
#[derive(Debug, Clone)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn generate() -> Self {
        let verifier = base64url(&random_bytes());
        let challenge = base64url(Sha256::digest(verifier.as_bytes()).as_slice());
        Self {
            verifier,
            challenge,
        }
    }
}

/// What the app needs to talk to Entra. None of it is secret — it all ships in
/// the binary, which is precisely why the flow cannot rely on a secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthConfig {
    pub tenant_id: String,
    /// The desktop app's own registration.
    pub client_id: String,
    /// The API's registration, which the scope is expressed against.
    pub api_client_id: String,
}

impl AuthConfig {
    /// The Note67 registrations, as created in the CyberFitz tenant.
    pub fn default_registrations() -> Self {
        Self {
            tenant_id: "aaa2d5a9-514c-4c48-9836-31bd0e93b619".into(),
            client_id: "919ea6b8-6e30-4978-8f0d-295542a0b9e0".into(),
            api_client_id: "ac2463be-ae20-4347-bda9-8e7e65ecb42c".into(),
        }
    }

    /// What the service requires in every token, plus a refresh token.
    ///
    /// `offline_access` is what makes signing in last longer than an hour.
    /// Without it Entra issues no refresh token and the user is sent back to a
    /// browser every time an access token expires.
    pub fn scope(&self) -> String {
        format!(
            "api://{}/Sync.Access offline_access openid profile",
            self.api_client_id
        )
    }

    pub fn authorize_endpoint(&self) -> String {
        format!(
            "https://login.microsoftonline.com/{}/oauth2/v2.0/authorize",
            self.tenant_id
        )
    }

    pub fn token_endpoint(&self) -> String {
        format!(
            "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
            self.tenant_id
        )
    }
}

/// One sign-in attempt, held while the browser is away.
#[derive(Debug, Clone)]
pub struct PendingSignIn {
    pub pkce: Pkce,
    /// Random, and checked when the browser comes back. Without it, another
    /// page could drive this app's loopback listener with a code of its
    /// choosing.
    pub state: String,
    pub redirect_uri: String,
}

impl PendingSignIn {
    pub fn start(port: u16) -> Self {
        Self {
            pkce: Pkce::generate(),
            state: base64url(&random_bytes()),
            // Entra permits any port under http://localhost for a public
            // client, so the listener can take whatever the OS gives it rather
            // than fighting over a fixed one that another app may hold.
            redirect_uri: format!("http://localhost:{port}"),
        }
    }

    pub fn authorize_url(&self, config: &AuthConfig) -> String {
        format!(
            "{}?client_id={}&response_type=code&redirect_uri={}&response_mode=query\
             &scope={}&state={}&code_challenge={}&code_challenge_method=S256",
            config.authorize_endpoint(),
            escape(&config.client_id),
            escape(&self.redirect_uri),
            escape(&config.scope()),
            escape(&self.state),
            escape(&self.pkce.challenge),
        )
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum SignInError {
    #[error("the sign-in response carried no authorization code")]
    NoCode,
    #[error("the sign-in response did not match this attempt")]
    StateMismatch,
    #[error("sign-in was refused: {0}")]
    Refused(String),
}

/// Read the code out of the loopback redirect.
///
/// Checks `state` before anything else. A mismatch means this response belongs
/// to some other request — possibly one a malicious page started — and the code
/// in it must not be exchanged.
pub fn code_from_redirect(query: &str, expected_state: &str) -> Result<String, SignInError> {
    let mut code = None;
    let mut state = None;
    let mut error = None;
    let mut description = None;

    for pair in query.trim_start_matches('?').split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        let value = unescape(value);
        match key {
            "code" => code = Some(value),
            "state" => state = Some(value),
            "error" => error = Some(value),
            "error_description" => description = Some(value),
            _ => {}
        }
    }

    // Checked first, and against a response that may be an error: a forged
    // error would otherwise be reported to the user as Entra's words.
    if state.as_deref() != Some(expected_state) {
        return Err(SignInError::StateMismatch);
    }
    if let Some(err) = error {
        return Err(SignInError::Refused(description.unwrap_or(err)));
    }
    code.ok_or(SignInError::NoCode)
}

fn unescape(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                match u8::from_str_radix(&value[i + 1..i + 3], 16) {
                    Ok(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Entra's token response.
#[derive(Debug, Clone, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    /// Absent when `offline_access` was not granted, which would mean sending
    /// the user back to a browser every hour.
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Seconds.
    pub expires_in: i64,
}

/// The form body that exchanges a code for tokens.
///
/// No client secret: this is a public client, and Entra rejects one here. The
/// verifier is what proves the exchange belongs to the request that started it.
pub fn token_request_body(
    config: &AuthConfig,
    pending: &PendingSignIn,
    code: &str,
) -> Vec<(String, String)> {
    vec![
        ("client_id".into(), config.client_id.clone()),
        ("grant_type".into(), "authorization_code".into()),
        ("code".into(), code.to_string()),
        ("redirect_uri".into(), pending.redirect_uri.clone()),
        ("code_verifier".into(), pending.pkce.verifier.clone()),
        ("scope".into(), config.scope()),
    ]
}

/// The form body that trades a refresh token for a fresh access token.
pub fn refresh_request_body(config: &AuthConfig, refresh_token: &str) -> Vec<(String, String)> {
    vec![
        ("client_id".into(), config.client_id.clone()),
        ("grant_type".into(), "refresh_token".into()),
        ("refresh_token".into(), refresh_token.to_string()),
        ("scope".into(), config.scope()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> AuthConfig {
        AuthConfig::default_registrations()
    }

    #[test]
    fn a_verifier_is_the_length_the_spec_allows() {
        // RFC 7636 requires 43 to 128 characters. 32 bytes of base64url is 43.
        let p = Pkce::generate();
        assert_eq!(p.verifier.len(), 43);
        assert!(p.verifier.len() >= 43 && p.verifier.len() <= 128);
    }

    #[test]
    fn a_verifier_uses_only_unreserved_characters() {
        let p = Pkce::generate();
        assert!(
            p.verifier
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-._~".contains(c)),
            "{}",
            p.verifier
        );
    }

    #[test]
    fn two_sign_ins_do_not_share_a_verifier() {
        // The verifier is the secret that replaces a client secret. A
        // predictable one would make an intercepted code exchangeable.
        assert_ne!(Pkce::generate().verifier, Pkce::generate().verifier);
    }

    #[test]
    fn the_challenge_is_the_sha256_of_the_verifier_unpadded() {
        // Known answer from RFC 7636 appendix B.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = base64url(Sha256::digest(verifier.as_bytes()).as_slice());
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        assert!(!challenge.contains('='), "padding would be rejected");
    }

    #[test]
    fn base64url_uses_the_url_alphabet() {
        // '+' and '/' would be mangled in a query string.
        let encoded = base64url(&[0xfb, 0xff, 0xbe]);
        assert!(!encoded.contains('+') && !encoded.contains('/'));
    }

    #[test]
    fn base64url_handles_lengths_that_are_not_multiples_of_three() {
        assert_eq!(base64url(b"a"), "YQ");
        assert_eq!(base64url(b"ab"), "YWI");
        assert_eq!(base64url(b"abc"), "YWJj");
    }

    #[test]
    fn the_scope_asks_for_a_refresh_token() {
        // Without offline_access Entra issues none, and the user is sent back
        // to a browser every time the access token expires.
        assert!(config().scope().contains("offline_access"));
    }

    #[test]
    fn the_scope_names_the_api_not_the_app() {
        // A scope expressed against the desktop registration would produce a
        // token whose audience the service rejects — and the failure would
        // look like a server bug.
        let c = config();
        assert!(c.scope().starts_with(&format!("api://{}/Sync.Access", c.api_client_id)));
        assert!(!c.scope().contains(&c.client_id));
    }

    #[test]
    fn the_authorize_url_escapes_the_scope() {
        // `api://guid/Sync.Access` contains `:` and `/`; unescaped, Entra reads
        // a different parameter than the one intended.
        let pending = PendingSignIn::start(53123);
        let url = pending.authorize_url(&config());
        assert!(url.contains("scope=api%3A%2F%2F"), "{url}");
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A53123"), "{url}");
    }

    #[test]
    fn the_authorize_url_asks_for_s256() {
        // `plain` would send the verifier itself, which is the whole thing PKCE
        // exists to avoid.
        let url = PendingSignIn::start(1).authorize_url(&config());
        assert!(url.contains("code_challenge_method=S256"));
        assert!(!url.contains("code_challenge_method=plain"));
    }

    #[test]
    fn a_redirect_yields_its_code() {
        let pending = PendingSignIn::start(1);
        let query = format!("?code=abc123&state={}", pending.state);
        assert_eq!(
            code_from_redirect(&query, &pending.state).unwrap(),
            "abc123"
        );
    }

    #[test]
    fn a_redirect_with_the_wrong_state_is_refused() {
        // Otherwise any page could drive this app's loopback listener with a
        // code of its choosing.
        assert_eq!(
            code_from_redirect("?code=abc&state=somebody-elses", "mine"),
            Err(SignInError::StateMismatch)
        );
    }

    #[test]
    fn a_redirect_with_no_state_is_refused() {
        assert_eq!(
            code_from_redirect("?code=abc", "mine"),
            Err(SignInError::StateMismatch)
        );
    }

    #[test]
    fn state_is_checked_before_an_error_is_believed() {
        // A forged error would otherwise be shown to the user as Entra's words.
        assert_eq!(
            code_from_redirect("?error=access_denied&state=forged", "mine"),
            Err(SignInError::StateMismatch)
        );
    }

    #[test]
    fn a_refusal_reports_what_entra_said() {
        let err = code_from_redirect(
            "?error=access_denied&error_description=The+user+cancelled&state=mine",
            "mine",
        )
        .unwrap_err();
        assert_eq!(err, SignInError::Refused("The user cancelled".into()));
    }

    #[test]
    fn a_percent_escaped_code_is_decoded() {
        assert_eq!(
            code_from_redirect("?code=a%2Fb%3Dc&state=mine", "mine").unwrap(),
            "a/b=c"
        );
    }

    #[test]
    fn the_token_request_carries_no_client_secret() {
        // A public client has none, and Entra rejects the exchange if one is
        // sent. The verifier is what proves this exchange belongs to the
        // request that started it.
        let pending = PendingSignIn::start(1);
        let body = token_request_body(&config(), &pending, "code123");
        let keys: Vec<&str> = body.iter().map(|(k, _)| k.as_str()).collect();
        assert!(!keys.contains(&"client_secret"));
        assert!(keys.contains(&"code_verifier"));
    }

    #[test]
    fn the_token_request_sends_the_verifier_not_the_challenge() {
        let pending = PendingSignIn::start(1);
        let body = token_request_body(&config(), &pending, "code123");
        let verifier = body
            .iter()
            .find(|(k, _)| k == "code_verifier")
            .map(|(_, v)| v.clone())
            .unwrap();
        assert_eq!(verifier, pending.pkce.verifier);
        assert_ne!(verifier, pending.pkce.challenge);
    }

    #[test]
    fn the_redirect_uri_matches_the_one_authorization_used() {
        // Entra compares them, and a mismatch fails the exchange after the user
        // has already consented — which reads as a broken app.
        let pending = PendingSignIn::start(49152);
        let body = token_request_body(&config(), &pending, "c");
        let sent = body.iter().find(|(k, _)| k == "redirect_uri").unwrap();
        assert_eq!(sent.1, pending.redirect_uri);
        assert!(pending.authorize_url(&config()).contains(&escape(&sent.1)));
    }

    #[test]
    fn a_refresh_reuses_the_same_scope() {
        // A narrower scope on refresh silently drops offline_access, and the
        // next refresh has nothing to use.
        let body = refresh_request_body(&config(), "rt");
        let scope = body.iter().find(|(k, _)| k == "scope").unwrap();
        assert_eq!(scope.1, config().scope());
    }

    #[test]
    fn a_token_response_without_a_refresh_token_still_parses() {
        // It is a degraded sign-in, not a malformed one — worth surfacing
        // rather than failing on.
        let r: TokenResponse =
            serde_json::from_str(r#"{"access_token":"at","expires_in":3599}"#).unwrap();
        assert!(r.refresh_token.is_none());
        assert_eq!(r.expires_in, 3599);
    }
}
