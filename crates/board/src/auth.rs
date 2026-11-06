//! Authentication (spec section 3).
//!
//! - The web UI uses a session cookie, set by a one-time link that
//!   `board login` prints.
//! - Machine clients (Claude, scripts, Design) use bearer tokens with a scope.
//!
//! Secrets are stored only as SHA-256 hashes. Lookups go by hash, so a timing
//! difference can reveal nothing useful: an attacker can't steer the hash of
//! their guess, which is what makes the usual constant-time compare
//! unnecessary here.

use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, header};
use axum::middleware::Next;
use axum::response::Response;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::TimeDelta;
use sha2::{Digest as _, Sha256};
use sqlx::SqlitePool;

use crate::app::AppState;
use crate::clock::Timestamp;
use crate::error::{AppError, AppResult};
use crate::types::{Actor, Caller, Scope, new_id};

pub const SESSION_COOKIE: &str = "board_session";
pub const SESSION_DAYS: i64 = 30;
const LOGIN_CODE_MINUTES: i64 = 10;
const TOKEN_PREFIX: &str = "brd_";

/// Don't write `last_used_at` on every request; once an hour is plenty.
const TOUCH_EVERY: TimeDelta = TimeDelta::hours(1);

pub(crate) fn random_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn hash(secret: &str) -> String {
    hex(&Sha256::digest(secret.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------- Tokens ----------

pub struct NewToken {
    pub id: String,
    /// The token itself. Shown once; only its hash is stored.
    pub token: String,
}

pub struct TokenInfo {
    pub id: String,
    pub name: String,
    pub scope: String,
    pub actor: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

pub async fn create_token(
    pool: &SqlitePool,
    name: &str,
    scope: Scope,
    actor: Actor,
    now: Timestamp,
) -> AppResult<NewToken> {
    if actor == Actor::User {
        return Err(AppError::BadRequest(
            "tokens act as `claude` or `api`; `user` is the web UI".into(),
        ));
    }
    let id = new_id();
    let token = format!("{TOKEN_PREFIX}{}", random_secret());
    sqlx::query(
        "INSERT INTO api_tokens (id, name, token_hash, scope, actor, created_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(name)
    .bind(hash(&token))
    .bind(scope.as_str())
    .bind(actor.as_str())
    .bind(now.to_db())
    .execute(pool)
    .await?;
    Ok(NewToken { id, token })
}

pub async fn list_tokens(pool: &SqlitePool) -> AppResult<Vec<TokenInfo>> {
    let rows: Vec<(String, String, String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, name, scope, actor, created_at, last_used_at FROM api_tokens ORDER BY created_at",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(id, name, scope, actor, created_at, last_used_at)| TokenInfo {
                id,
                name,
                scope,
                actor,
                created_at,
                last_used_at,
            },
        )
        .collect())
}

/// Returns false if there was no such token.
pub async fn revoke_token(pool: &SqlitePool, id: &str) -> AppResult<bool> {
    let result = sqlx::query("DELETE FROM api_tokens WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

async fn caller_from_token(
    pool: &SqlitePool,
    token: &str,
    now: Timestamp,
) -> AppResult<Option<Caller>> {
    let token_hash = hash(token);
    let row: Option<(String, String, Option<String>)> =
        sqlx::query_as("SELECT scope, actor, last_used_at FROM api_tokens WHERE token_hash = ?")
            .bind(&token_hash)
            .fetch_optional(pool)
            .await?;
    let Some((scope, actor, last_used_at)) = row else {
        return Ok(None);
    };

    if needs_touch(last_used_at.as_deref(), now) {
        sqlx::query("UPDATE api_tokens SET last_used_at = ? WHERE token_hash = ?")
            .bind(now.to_db())
            .bind(&token_hash)
            .execute(pool)
            .await?;
    }

    let scope = Scope::parse(&scope).ok_or_else(|| anyhow::anyhow!("bad scope in db"))?;
    let actor = Actor::parse(&actor).ok_or_else(|| anyhow::anyhow!("bad actor in db"))?;
    Ok(Some(Caller { actor, scope }))
}

fn needs_touch(last_used_at: Option<&str>, now: Timestamp) -> bool {
    last_used_at.is_none_or(|t| t < now.minus(TOUCH_EVERY).to_db().as_str())
}

// ---------- Login links and sessions ----------

/// Create a one-time login code, valid for 10 minutes.
pub async fn create_login_code(pool: &SqlitePool, now: Timestamp) -> AppResult<String> {
    let code = random_secret();
    sqlx::query("INSERT INTO login_codes (code_hash, created_at, expires_at) VALUES (?, ?, ?)")
        .bind(hash(&code))
        .bind(now.to_db())
        .bind(now.plus(TimeDelta::minutes(LOGIN_CODE_MINUTES)).to_db())
        .execute(pool)
        .await?;
    Ok(code)
}

/// Use a login code and start a session. Returns the session cookie value,
/// or `None` if the code is unknown, expired or already used.
pub async fn redeem_login_code(
    pool: &SqlitePool,
    code: &str,
    now: Timestamp,
) -> AppResult<Option<String>> {
    let mut tx = crate::db::write_tx(pool).await?;
    let used = sqlx::query(
        "UPDATE login_codes SET used_at = ? WHERE code_hash = ? AND used_at IS NULL AND expires_at > ?",
    )
    .bind(now.to_db())
    .bind(hash(code))
    .bind(now.to_db())
    .execute(&mut *tx)
    .await?;
    if used.rows_affected() == 0 {
        return Ok(None);
    }
    let session = start_session(&mut tx, now).await?;
    tx.commit().await?;
    Ok(Some(session))
}

/// Start a UI session (30 days). Returns the cookie value.
pub async fn start_session(conn: &mut sqlx::SqliteConnection, now: Timestamp) -> AppResult<String> {
    let session = random_secret();
    sqlx::query("INSERT INTO sessions (id_hash, created_at, expires_at) VALUES (?, ?, ?)")
        .bind(hash(&session))
        .bind(now.to_db())
        .bind(now.plus(TimeDelta::days(SESSION_DAYS)).to_db())
        .execute(conn)
        .await?;
    Ok(session)
}

pub async fn end_session(pool: &SqlitePool, session: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM sessions WHERE id_hash = ?")
        .bind(hash(session))
        .execute(pool)
        .await?;
    Ok(())
}

async fn caller_from_session(
    pool: &SqlitePool,
    session: &str,
    now: Timestamp,
) -> AppResult<Option<Caller>> {
    let id_hash = hash(session);
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT last_used_at FROM sessions WHERE id_hash = ? AND expires_at > ?")
            .bind(&id_hash)
            .bind(now.to_db())
            .fetch_optional(pool)
            .await?;
    let Some((last_used_at,)) = row else {
        return Ok(None);
    };
    if needs_touch(last_used_at.as_deref(), now) {
        sqlx::query("UPDATE sessions SET last_used_at = ? WHERE id_hash = ?")
            .bind(now.to_db())
            .bind(&id_hash)
            .execute(pool)
            .await?;
    }
    Ok(Some(Caller {
        actor: Actor::User,
        scope: Scope::Admin,
    }))
}

/// Delete expired sessions and login codes. Run by the daily housekeeping task.
pub async fn purge_expired(pool: &SqlitePool, now: Timestamp) -> AppResult<()> {
    sqlx::query("DELETE FROM sessions WHERE expires_at <= ?")
        .bind(now.to_db())
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM login_codes WHERE expires_at <= ?")
        .bind(now.to_db())
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM passkey_challenges WHERE expires_at <= ?")
        .bind(now.to_db())
        .execute(pool)
        .await?;
    Ok(())
}

/// The `Set-Cookie` value for a new session. `Secure` is set whenever Board
/// is served over https (and left off for plain-http local development).
pub fn session_cookie(value: &str, public_origin: &str) -> String {
    let secure = if public_origin.starts_with("https://") {
        "; Secure"
    } else {
        ""
    };
    format!(
        "{SESSION_COOKIE}={value}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{secure}",
        SESSION_DAYS * 24 * 60 * 60
    )
}

pub fn clear_session_cookie() -> String {
    format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0")
}

pub fn session_from_headers(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == SESSION_COOKIE)
        .map(|(_, value)| value)
        .filter(|value| !value.is_empty())
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

// ---------- Middleware ----------

/// Authenticate every `/api` request and attach a [`Caller`].
///
/// - A bearer token wins over a cookie.
/// - `read` tokens may only make GET/HEAD requests.
/// - Cookie sessions must send an `Origin` matching `public_origin` on
///   anything that isn't GET/HEAD. Browsers always send `Origin` on those
///   requests, so this blocks cross-site writes on top of `SameSite=Strict`.
pub async fn require_caller(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let now = state.clock.now();
    let headers = request.headers();
    let safe_method = matches!(*request.method(), Method::GET | Method::HEAD);

    let caller = if let Some(token) = bearer(headers) {
        caller_from_token(&state.pool, token, now)
            .await?
            .ok_or(AppError::Unauthorized)?
    } else if let Some(session) = session_from_headers(headers) {
        let caller = caller_from_session(&state.pool, session, now)
            .await?
            .ok_or(AppError::Unauthorized)?;
        if !safe_method {
            let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
            if origin != Some(state.config.public_origin.as_str()) {
                return Err(AppError::Forbidden(
                    "Origin doesn't match this Board's public_origin",
                ));
            }
        }
        caller
    } else {
        return Err(AppError::Unauthorized);
    };

    if caller.scope == Scope::Read && !safe_method {
        return Err(AppError::Forbidden("this token is read-only"));
    }

    request.extensions_mut().insert(caller);
    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_session_cookie_among_others() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "theme=dark; board_session=abc123; other=1".parse().unwrap(),
        );
        assert_eq!(session_from_headers(&headers), Some("abc123"));

        headers.insert(header::COOKIE, "board_session=".parse().unwrap());
        assert_eq!(session_from_headers(&headers), None);
    }

    #[test]
    fn cookie_is_secure_only_over_https() {
        assert!(session_cookie("v", "https://board.example.com").contains("; Secure"));
        assert!(!session_cookie("v", "http://localhost:7070").contains("Secure"));
        let cookie = session_cookie("v", "https://board.example.com");
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));
    }
}
