//! Passkeys end to end over HTTP, with a simulated authenticator (a real
//! P-256 key, real CBOR and signatures).

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ciborium::Value as Cbor;
use common::TestApp;
use http_body_util::BodyExt as _;
use p256::ecdsa::signature::Signer as _;
use p256::ecdsa::{Signature, SigningKey};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt as _;

const ORIGIN: &str = "http://localhost:7070";

struct Reply {
    status: StatusCode,
    cookie: Option<String>,
    body: Value,
}

async fn send(
    app: &TestApp,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    bearer: Option<&str>,
    body: Option<Value>,
) -> Reply {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::ORIGIN, ORIGIN);
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, format!("board_session={c}"));
    }
    if let Some(t) = bearer {
        b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let req = match body {
        Some(v) => b
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(v.to_string())),
        None => b.body(Body::empty()),
    }
    .unwrap();
    let res = app.router.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let cookie = res
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .and_then(|kv| kv.strip_prefix("board_session="))
        .map(str::to_string);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    Reply {
        status,
        cookie,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

/// A session the old way: a one-time login code.
async fn session(app: &TestApp) -> String {
    let now = app.state.clock.now();
    let code = board::auth::create_login_code(&app.state.pool, now)
        .await
        .unwrap();
    board::auth::redeem_login_code(&app.state.pool, &code, now)
        .await
        .unwrap()
        .unwrap()
}

fn b64(b: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(b)
}

struct Authenticator {
    key: SigningKey,
    id: Vec<u8>,
}

impl Authenticator {
    fn new(seed: u8) -> Self {
        Self {
            key: SigningKey::from_slice(&[seed; 32]).unwrap(),
            id: vec![seed; 16],
        }
    }

    fn auth_data(&self, attested: bool) -> Vec<u8> {
        let mut d = Sha256::digest(b"localhost").to_vec();
        d.push(0x01 | 0x04 | if attested { 0x40 } else { 0 });
        d.extend_from_slice(&0u32.to_be_bytes());
        if attested {
            let point = self.key.verifying_key().to_sec1_point(false);
            let xy = point.as_bytes();
            let cose = Cbor::Map(vec![
                (Cbor::from(1), Cbor::from(2)),
                (Cbor::from(3), Cbor::from(-7)),
                (Cbor::from(-1), Cbor::from(1)),
                (Cbor::from(-2), Cbor::Bytes(xy[1..33].to_vec())),
                (Cbor::from(-3), Cbor::Bytes(xy[33..65].to_vec())),
            ]);
            d.extend_from_slice(&[0u8; 16]);
            d.extend_from_slice(&(self.id.len() as u16).to_be_bytes());
            d.extend_from_slice(&self.id);
            ciborium::into_writer(&cose, &mut d).unwrap();
        }
        d
    }

    fn client(kind: &str, challenge: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({ "type": kind, "challenge": challenge, "origin": ORIGIN }))
            .unwrap()
    }

    fn create(&self, options: &Value) -> Value {
        let att = Cbor::Map(vec![
            (Cbor::Text("fmt".into()), Cbor::Text("none".into())),
            (Cbor::Text("attStmt".into()), Cbor::Map(vec![])),
            (
                Cbor::Text("authData".into()),
                Cbor::Bytes(self.auth_data(true)),
            ),
        ]);
        let mut att_bytes = Vec::new();
        ciborium::into_writer(&att, &mut att_bytes).unwrap();
        let client = Self::client("webauthn.create", options["challenge"].as_str().unwrap());
        json!({ "name": "Test phone", "clientDataJSON": b64(&client), "attestationObject": b64(&att_bytes) })
    }

    fn get(&self, options: &Value) -> Value {
        let data = self.auth_data(false);
        let client = Self::client("webauthn.get", options["challenge"].as_str().unwrap());
        let mut msg = data.clone();
        msg.extend_from_slice(&Sha256::digest(&client));
        let sig: Signature = self.key.sign(&msg);
        json!({ "id": b64(&self.id), "clientDataJSON": b64(&client), "authenticatorData": b64(&data),
                "signature": b64(sig.to_der().as_bytes()) })
    }
}

#[tokio::test]
async fn a_passkey_signs_you_in() {
    let app = TestApp::new().await;
    let cookie = session(&app).await;
    let phone = Authenticator::new(3);

    // Register, from a signed-in browser.
    let options = send(
        &app,
        "POST",
        "/api/passkeys/options",
        Some(&cookie),
        None,
        None,
    )
    .await;
    assert_eq!(options.status, StatusCode::OK, "{}", options.body);
    assert_eq!(options.body["rp"]["id"], "localhost");
    assert_eq!(
        options.body["authenticatorSelection"]["residentKey"],
        "required"
    );
    assert_eq!(options.body["pubKeyCredParams"][0]["alg"], -7);
    let added = send(
        &app,
        "POST",
        "/api/passkeys",
        Some(&cookie),
        None,
        Some(phone.create(&options.body)),
    )
    .await;
    assert_eq!(added.status, StatusCode::CREATED, "{}", added.body);
    let list = send(&app, "GET", "/api/passkeys", Some(&cookie), None, None).await;
    assert_eq!(list.body["passkeys"][0]["name"], "Test phone");

    // Registering again excludes it.
    let again = send(
        &app,
        "POST",
        "/api/passkeys/options",
        Some(&cookie),
        None,
        None,
    )
    .await;
    assert_eq!(again.body["excludeCredentials"][0]["id"], b64(&phone.id));

    // Sign in from somewhere with no session.
    let options = send(&app, "POST", "/auth/passkey/options", None, None, None).await;
    assert_eq!(options.status, StatusCode::OK);
    let assertion = phone.get(&options.body);
    let signed_in = send(
        &app,
        "POST",
        "/auth/passkey/login",
        None,
        None,
        Some(assertion.clone()),
    )
    .await;
    assert_eq!(signed_in.status, StatusCode::OK, "{}", signed_in.body);
    let new_cookie = signed_in.cookie.expect("a session cookie");
    let me = send(&app, "GET", "/api/me", Some(&new_cookie), None, None).await;
    assert_eq!(me.status, StatusCode::OK);
    assert_eq!(me.body["actor"], "user");

    // The same signed response can't be used twice.
    let replay = send(
        &app,
        "POST",
        "/auth/passkey/login",
        None,
        None,
        Some(assertion),
    )
    .await;
    assert_eq!(replay.status, StatusCode::UNAUTHORIZED);

    // A different key claiming that credential id fails.
    let options = send(&app, "POST", "/auth/passkey/options", None, None, None).await;
    let mut forged = Authenticator::new(4).get(&options.body);
    forged["id"] = json!(b64(&phone.id));
    let res = send(
        &app,
        "POST",
        "/auth/passkey/login",
        None,
        None,
        Some(forged),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);

    // Removed passkeys can't sign in.
    let id = list.body["passkeys"][0]["id"].as_str().unwrap();
    let removed = send(
        &app,
        "DELETE",
        &format!("/api/passkeys/{id}"),
        Some(&cookie),
        None,
        None,
    )
    .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    let options = send(&app, "POST", "/auth/passkey/options", None, None, None).await;
    let res = send(
        &app,
        "POST",
        "/auth/passkey/login",
        None,
        None,
        Some(phone.get(&options.body)),
    )
    .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["error"], "unknown_passkey");
}

#[tokio::test]
async fn tokens_and_other_sites_cant_use_passkeys() {
    let app = TestApp::new().await;
    // An API token can't add a passkey (it would turn a token into a login).
    let res = send(
        &app,
        "POST",
        "/api/passkeys/options",
        None,
        Some(&app.admin),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    let res = send(&app, "GET", "/api/passkeys", None, None, None).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);

    // Sign-in requests must come from Board's own pages.
    let req = Request::builder()
        .method("POST")
        .uri("/auth/passkey/options")
        .header(header::ORIGIN, "https://evil.example")
        .body(Body::empty())
        .unwrap();
    let res = app.router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // A registration signed for a challenge Board never issued is refused.
    let cookie = session(&app).await;
    let fake = Authenticator::new(5).create(&json!({ "challenge": "made-up" }));
    let res = send(
        &app,
        "POST",
        "/api/passkeys",
        Some(&cookie),
        None,
        Some(fake),
    )
    .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST, "{}", res.body);
}
