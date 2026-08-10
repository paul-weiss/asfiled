//! The asfiled web server: static app + published data + accounts API.
//!
//! One container, mirroring news-river and ask-puzzler: axum + SQLite for
//! users (replicated by Litestream in production), passwordless magic-link
//! sign-in, HttpOnly session cookies. With no SMTP configured the magic link
//! is logged instead of sent — local development needs no mail credentials.
//!
//! Accounts exist ahead of need: every user carries a `tier` (default
//! `free`), which is the hook later entitlement checks and a paywall attach
//! to. Nothing is gated today.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Redirect};
use axum::routing::{get, post};
use axum::Router;
use chrono::Utc;
use rand::RngCore;
use rusqlite::{params, Connection};
use serde::Deserialize;
use serde_json::json;
use tower_http::services::ServeDir;

const SESSION_COOKIE: &str = "af_session";
const LOGIN_TOKEN_TTL_MIN: i64 = 15;
const SESSION_TTL_DAYS: i64 = 30;
/// Sign-in emails per address per hour. Generous for humans, hostile to loops.
const REQUEST_LIMIT_PER_HOUR: usize = 5;

struct AppState {
    db: Mutex<Connection>,
    mailer: Mailer,
    app_url: String,
    secure_cookies: bool,
    recent_requests: Mutex<HashMap<String, Vec<Instant>>>,
}

enum Mailer {
    /// SMTP relay (SES in production).
    Smtp {
        transport: lettre::AsyncSmtpTransport<lettre::Tokio1Executor>,
        from: String,
    },
    /// No SMTP configured: log the link. Local development only.
    LogOnly,
}

impl Mailer {
    fn from_env() -> Self {
        let host = std::env::var("ASFILED_SMTP_HOST").ok();
        let user = std::env::var("ASFILED_SMTP_USER").ok();
        let pass = std::env::var("ASFILED_SMTP_PASS").ok();
        let from = std::env::var("ASFILED_MAIL_FROM")
            .unwrap_or_else(|_| "asfiled <signin@asfiled.io>".into());
        match (host, user, pass) {
            (Some(host), Some(user), Some(pass)) => {
                let transport = lettre::AsyncSmtpTransport::<lettre::Tokio1Executor>::relay(&host)
                    .expect("valid SMTP host")
                    .credentials(lettre::transport::smtp::authentication::Credentials::new(
                        user, pass,
                    ))
                    .build();
                Mailer::Smtp { transport, from }
            }
            _ => Mailer::LogOnly,
        }
    }

    async fn send_magic_link(&self, email: &str, link: &str) -> Result<(), String> {
        match self {
            Mailer::LogOnly => {
                eprintln!("SMTP not configured — magic link for {email}: {link}");
                Ok(())
            }
            Mailer::Smtp { transport, from } => {
                use lettre::AsyncTransport;
                let message = lettre::Message::builder()
                    .from(from.parse().map_err(|e| format!("from: {e}"))?)
                    .to(email.parse().map_err(|e| format!("to: {e}"))?)
                    .subject("Sign in to asfiled")
                    .body(format!(
                        "Sign in to asfiled:\n\n{link}\n\nThis link expires in \
                         {LOGIN_TOKEN_TTL_MIN} minutes. If you didn't request it, ignore this email."
                    ))
                    .map_err(|e| format!("build: {e}"))?;
                transport
                    .send(message)
                    .await
                    .map(|_| ())
                    .map_err(|e| format!("send: {e}"))
            }
        }
    }
}

const USERS_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS users (
    id         INTEGER PRIMARY KEY,
    email      TEXT NOT NULL UNIQUE,
    tier       TEXT NOT NULL DEFAULT 'free',
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS login_tokens (
    token      TEXT PRIMARY KEY,
    email      TEXT NOT NULL,
    expires_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS sessions (
    token      TEXT PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES users(id),
    expires_at TEXT NOT NULL
);
";

fn new_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn plausible_email(email: &str) -> bool {
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    !local.is_empty() && domain.contains('.') && !domain.ends_with('.') && email.len() <= 254
}

fn session_cookie(token: &str, max_age_secs: i64, secure: bool) -> String {
    let secure_attr = if secure { "; Secure" } else { "" };
    format!("{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age_secs}{secure_attr}")
}

fn session_token_from_headers(headers: &HeaderMap) -> Option<String> {
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    cookies.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name == SESSION_COOKIE).then(|| value.to_string())
    })
}

#[tokio::main]
async fn main() {
    let addr: SocketAddr = std::env::var("ASFILED_SERVER_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".into())
        .parse()
        .expect("valid ASFILED_SERVER_ADDR");
    let app_url = std::env::var("ASFILED_APP_URL").unwrap_or_else(|_| format!("http://{addr}"));
    let users_db =
        PathBuf::from(std::env::var("ASFILED_USERS_DB").unwrap_or_else(|_| "data/users.db".into()));
    let static_dir = std::env::var("ASFILED_STATIC_DIR").unwrap_or_else(|_| "web".into());
    let data_dir = std::env::var("ASFILED_DATA_DIR").unwrap_or_else(|_| "data/publish".into());

    if let Some(parent) = users_db.parent() {
        std::fs::create_dir_all(parent).expect("users db directory");
    }
    let db = Connection::open(&users_db).expect("open users db");
    db.execute_batch(USERS_SCHEMA).expect("users schema");

    let state = Arc::new(AppState {
        db: Mutex::new(db),
        mailer: Mailer::from_env(),
        secure_cookies: app_url.starts_with("https://"),
        app_url,
        recent_requests: Mutex::new(HashMap::new()),
    });

    let app = Router::new()
        .route("/", get(|| async { Redirect::permanent("/app/") }))
        .route("/api/health", get(health))
        .route("/api/auth/request", post(auth_request))
        .route("/api/auth/verify", get(auth_verify))
        .route("/api/auth/logout", post(auth_logout))
        .route("/api/me", get(me))
        .nest_service("/app", ServeDir::new(static_dir))
        .nest_service("/data", ServeDir::new(data_dir))
        .with_state(state);

    eprintln!("asfiled server listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .expect("server");
}

/// Verifies the database is readable, not just that the process is alive.
async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let ok = state
        .db
        .lock()
        .unwrap()
        .query_row("SELECT count(*) FROM users", [], |r| r.get::<_, i64>(0))
        .is_ok();
    if ok {
        (StatusCode::OK, Json(json!({"ok": true})))
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"ok": false})))
    }
}

#[derive(Deserialize)]
struct AuthRequest {
    email: String,
}

async fn auth_request(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AuthRequest>,
) -> impl IntoResponse {
    let email = body.email.trim().to_lowercase();
    if !plausible_email(&email) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "that doesn't look like an email address"})),
        );
    }

    {
        let mut recent = state.recent_requests.lock().unwrap();
        let hits = recent.entry(email.clone()).or_default();
        hits.retain(|t| t.elapsed() < Duration::from_secs(3600));
        if hits.len() >= REQUEST_LIMIT_PER_HOUR {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({"error": "too many sign-in emails — try again later"})),
            );
        }
        hits.push(Instant::now());
    }

    let token = new_token();
    let expires = (Utc::now() + chrono::Duration::minutes(LOGIN_TOKEN_TTL_MIN)).to_rfc3339();
    state
        .db
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO login_tokens (token, email, expires_at) VALUES (?, ?, ?)",
            params![token, email, expires],
        )
        .expect("insert login token");

    let link = format!("{}/api/auth/verify?token={}", state.app_url, token);
    if let Err(err) = state.mailer.send_magic_link(&email, &link).await {
        eprintln!("magic link send failed for {email}: {err}");
        return (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": "couldn't send the email — try again shortly"})),
        );
    }
    // Same response whether or not the address has an account: sign-in
    // requests must not disclose who is registered.
    (StatusCode::OK, Json(json!({"sent": true})))
}

#[derive(Deserialize)]
struct VerifyQuery {
    token: String,
}

async fn auth_verify(
    State(state): State<Arc<AppState>>,
    Query(query): Query<VerifyQuery>,
) -> impl IntoResponse {
    let now = Utc::now().to_rfc3339();
    let session = {
        let db = state.db.lock().unwrap();
        let email: Option<String> = db
            .query_row(
                "DELETE FROM login_tokens WHERE token = ? AND expires_at > ? RETURNING email",
                params![query.token, now],
                |r| r.get(0),
            )
            .ok();
        let Some(email) = email else {
            return (
                StatusCode::UNAUTHORIZED,
                [(header::SET_COOKIE, String::new())],
                "This sign-in link is invalid or expired. Request a fresh one from the app.",
            )
                .into_response();
        };

        db.execute(
            "INSERT INTO users (email, created_at) VALUES (?, ?)
             ON CONFLICT (email) DO NOTHING",
            params![email, Utc::now().to_rfc3339()],
        )
        .expect("upsert user");
        let user_id: i64 = db
            .query_row(
                "SELECT id FROM users WHERE email = ?",
                params![email],
                |r| r.get(0),
            )
            .expect("user id");

        let session = new_token();
        let expires = (Utc::now() + chrono::Duration::days(SESSION_TTL_DAYS)).to_rfc3339();
        db.execute(
            "INSERT INTO sessions (token, user_id, expires_at) VALUES (?, ?, ?)",
            params![session, user_id, expires],
        )
        .expect("insert session");
        session
    };

    let cookie = session_cookie(&session, SESSION_TTL_DAYS * 24 * 3600, state.secure_cookies);
    ([(header::SET_COOKIE, cookie)], Redirect::to("/app/")).into_response()
}

async fn me(State(state): State<Arc<AppState>>, headers: HeaderMap) -> impl IntoResponse {
    let Some(token) = session_token_from_headers(&headers) else {
        return (StatusCode::UNAUTHORIZED, Json(json!({"signed_in": false})));
    };
    let row = state.db.lock().unwrap().query_row(
        "SELECT u.email, u.tier FROM sessions s JOIN users u ON u.id = s.user_id
         WHERE s.token = ? AND s.expires_at > ?",
        params![token, Utc::now().to_rfc3339()],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    );
    match row {
        Ok((email, tier)) => (
            StatusCode::OK,
            Json(json!({"signed_in": true, "email": email, "tier": tier})),
        ),
        Err(_) => (StatusCode::UNAUTHORIZED, Json(json!({"signed_in": false}))),
    }
}

async fn auth_logout(State(state): State<Arc<AppState>>, headers: HeaderMap) -> impl IntoResponse {
    if let Some(token) = session_token_from_headers(&headers) {
        let _ = state
            .db
            .lock()
            .unwrap()
            .execute("DELETE FROM sessions WHERE token = ?", params![token]);
    }
    (
        [(
            header::SET_COOKIE,
            session_cookie("", 0, state.secure_cookies),
        )],
        Json(json!({"signed_out": true})),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_long_and_unique() {
        let a = new_token();
        let b = new_token();
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
    }

    #[test]
    fn email_plausibility() {
        assert!(plausible_email("a@b.co"));
        assert!(!plausible_email("nope"));
        assert!(!plausible_email("@b.co"));
        assert!(!plausible_email("a@nodot"));
        assert!(!plausible_email("a@dot."));
    }

    #[test]
    fn cookie_roundtrip() {
        let cookie = session_cookie("abc123", 60, true);
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("Secure"));
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            format!("other=1; {SESSION_COOKIE}=abc123").parse().unwrap(),
        );
        assert_eq!(
            session_token_from_headers(&headers).as_deref(),
            Some("abc123")
        );
    }
}
