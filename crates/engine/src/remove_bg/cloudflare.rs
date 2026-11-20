//! Installing the background remover in the user's Cloudflare account, from a Cloudflare API key
//! the user made ("Edit Cloudflare Workers" template) and pasted into Preferences ▸ AI tools.
//!
//! Only the user does this, from the app's own window: it is not a command, so agents and scripts
//! can't, and the key never reaches the command journal. The key is used for these requests only
//! and is not kept:
//!
//! 1. find the account the key can use (`GET /accounts`);
//! 2. find its `workers.dev` name, choosing one if it has none yet;
//! 3. refuse to replace an existing `figcraft-bg` Worker unless the user said so;
//! 4. upload [`WORKER_JS`](super::WORKER_JS) as `figcraft-bg`, with Cloudflare Images and a new
//!    random access key bound to it;
//! 5. switch on its `workers.dev` address.
//!
//! The app then tests the new remover with [`settle`], which waits while Cloudflare switches the
//! address on.

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::net::{agent, unreachable};
use super::{Connection, WORKER_JS, WORKER_NAME};

/// Cloudflare's API.
pub const API: &str = "https://api.cloudflare.com/client/v4";

/// The date of the Workers runtime behaviour the Worker is written for.
const COMPATIBILITY_DATE: &str = "2025-09-01";

/// How long to keep trying a new remover while Cloudflare switches its address on (a new
/// `workers.dev` name can take a minute or two).
pub const SETTLE: Duration = Duration::from_secs(150);

/// Where an install is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Checking the key and finding the account.
    Checking,
    /// Uploading the Worker with its access key.
    Installing,
    /// Switching on its address.
    Address,
}

/// Why an install stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallError {
    /// Cloudflare didn't accept the key.
    BadKey,
    /// The key works but may not install Workers.
    NoPermission,
    /// The key can't see any account.
    NoAccount,
    /// The account has a `figcraft-bg` Worker already: install again with `replace` to replace it.
    Exists { account: String },
    /// Anything else, as a sentence.
    Other(String),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadKey => f.write_str("Cloudflare didn't accept this key. Make sure you copied the whole key."),
            Self::NoPermission => {
                f.write_str("This key can't install Workers. Make a new one with the \u{201c}Edit Cloudflare Workers\u{201d} template.")
            }
            Self::NoAccount => f.write_str("This key can't see a Cloudflare account. Make it while signed in to your account."),
            Self::Exists { account } => write!(f, "The account {account} already has a background remover named {WORKER_NAME}."),
            Self::Other(s) => f.write_str(s),
        }
    }
}

/// A remover installed and connected.
#[derive(Debug)]
pub struct Installed {
    pub connection: Connection,
    /// The Cloudflare account's name.
    pub account: String,
}

/// A random access key: 24 URL-safe characters (144 bits).
pub fn new_access_key() -> Result<String, String> {
    use base64::Engine as _;
    let mut bytes = [0u8; 18];
    getrandom::fill(&mut bytes).map_err(|e| format!("This computer couldn't make a random key ({e})."))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

/// A Cloudflare API answer: its status and JSON (`Null` when it wasn't JSON).
struct Answer {
    status: u16,
    json: Value,
}

impl Answer {
    fn ok(&self) -> bool {
        (200..300).contains(&self.status) && self.json.get("success").and_then(Value::as_bool).unwrap_or(true)
    }

    /// Cloudflare's error codes in the answer.
    fn codes(&self) -> Vec<u64> {
        self.json
            .get("errors")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|e| e.get("code").and_then(Value::as_u64)).collect())
            .unwrap_or_default()
    }

    /// Cloudflare's first error message, for a sentence.
    fn message(&self) -> String {
        let said = self.json.pointer("/errors/0/message").and_then(Value::as_str).unwrap_or("no details");
        format!("Cloudflare said: {} (error {}).", said.chars().take(200).collect::<String>(), self.status)
    }

    /// The error for a failed answer: the key's problem when it is one.
    fn error(&self) -> InstallError {
        let codes = self.codes();
        // 9106/9109/10000/10001: missing, unknown or unauthorised token.
        if self.status == 401 || codes.iter().any(|c| [9106, 9109, 10001].contains(c)) {
            InstallError::BadKey
        } else if self.status == 403 || codes.contains(&10000) {
            InstallError::NoPermission
        } else {
            InstallError::Other(self.message())
        }
    }
}

/// One request to Cloudflare's API.
struct Api<'a> {
    base: &'a str,
    token: &'a str,
    agent: ureq::Agent,
}

impl Api<'_> {
    fn answer(&self, sent: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Answer, InstallError> {
        let response = sent.map_err(|e| InstallError::Other(unreachable(&e, "api.cloudflare.com").replace("background remover", "Cloudflare")))?;
        let status = response.status().as_u16();
        let text = response.into_body().with_config().limit(4 * 1024 * 1024).read_to_string().unwrap_or_default();
        Ok(Answer { status, json: serde_json::from_str(&text).unwrap_or(Value::Null) })
    }

    fn get(&self, path: &str) -> Result<Answer, InstallError> {
        self.answer(self.agent.get(format!("{}{path}", self.base)).header("authorization", format!("Bearer {}", self.token)).call())
    }

    fn send_json(&self, method: &str, path: &str, body: &Value) -> Result<Answer, InstallError> {
        let url = format!("{}{path}", self.base);
        let req = if method == "PUT" { self.agent.put(url) } else { self.agent.post(url) };
        self.answer(req.header("authorization", format!("Bearer {}", self.token)).header("content-type", "application/json").send(body.to_string()))
    }

    fn put_multipart(&self, path: &str, boundary: &str, body: Vec<u8>) -> Result<Answer, InstallError> {
        self.answer(
            self.agent
                .put(format!("{}{path}", self.base))
                .header("authorization", format!("Bearer {}", self.token))
                .header("content-type", format!("multipart/form-data; boundary={boundary}"))
                .send(body),
        )
    }
}

/// The upload of the Worker: its settings (`metadata`) and its code, as multipart form data.
fn worker_upload(boundary: &str, access_key: &str) -> Vec<u8> {
    let metadata = json!({
        "main_module": "worker.js",
        "compatibility_date": COMPATIBILITY_DATE,
        "bindings": [
            {"type": "images", "name": "IMAGES"},
            {"type": "secret_text", "name": "ACCESS_KEY", "text": access_key},
        ],
    });
    let mut body = String::new();
    body.push_str(&format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"metadata\"\r\nContent-Type: application/json\r\n\r\n{metadata}\r\n"
    ));
    body.push_str(&format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"worker.js\"; filename=\"worker.js\"\r\nContent-Type: application/javascript+module\r\n\r\n{WORKER_JS}\r\n"
    ));
    body.push_str(&format!("--{boundary}--\r\n"));
    body.into_bytes()
}

/// The URL-safe part of a path segment Cloudflare gave (account ids are hex).
fn segment(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(64).collect()
}

/// Install the remover with the Cloudflare API key `token` (`api`: [`API`], or a stand-in in
/// tests), replacing an existing one only when `replace`. `step` hears where it is.
pub fn install(api: &str, token: &str, replace: bool, step: &mut dyn FnMut(Step)) -> Result<Installed, InstallError> {
    let token = token.trim();
    if token.is_empty() || token.len() > 400 || !token.chars().all(|c| c.is_ascii_graphic()) {
        return Err(InstallError::BadKey);
    }
    let api = Api { base: api.trim_end_matches('/'), token, agent: agent(Duration::from_secs(60)) };

    step(Step::Checking);
    let accounts = api.get("/accounts?per_page=50")?;
    if !accounts.ok() {
        return Err(accounts.error());
    }
    let first = accounts.json.get("result").and_then(Value::as_array).and_then(|a| a.first()).ok_or(InstallError::NoAccount)?;
    let account_id = segment(first.get("id").and_then(Value::as_str).unwrap_or(""));
    if account_id.is_empty() {
        return Err(InstallError::NoAccount);
    }
    let account = first.get("name").and_then(Value::as_str).unwrap_or("your Cloudflare account").chars().take(120).collect::<String>();
    let base = format!("/accounts/{account_id}/workers");

    let sub = api.get(&format!("{base}/subdomain"))?;
    let subdomain = match sub.json.pointer("/result/subdomain").and_then(Value::as_str) {
        Some(name) if sub.ok() && !name.is_empty() => segment(name),
        _ if sub.status == 401 || sub.status == 403 => return Err(sub.error()),
        _ => {
            // No workers.dev name yet: choose one ("figcraft-" and eight random letters).
            let mut bytes = [0u8; 8];
            getrandom::fill(&mut bytes).map_err(|e| InstallError::Other(format!("This computer couldn't make a random name ({e}).")))?;
            let name: String = std::iter::once("figcraft-".to_string()).chain(bytes.iter().map(|b| char::from(b'a' + b % 26).to_string())).collect();
            let made = api.send_json("PUT", &format!("{base}/subdomain"), &json!({"subdomain": name}))?;
            if !made.ok() {
                return Err(made.error());
            }
            segment(made.json.pointer("/result/subdomain").and_then(Value::as_str).unwrap_or(&name))
        }
    };

    let script = format!("{base}/scripts/{WORKER_NAME}");
    let existing = api.get(&format!("{script}/settings"))?;
    if existing.ok() && !replace {
        return Err(InstallError::Exists { account });
    }
    if existing.status == 401 || existing.status == 403 {
        return Err(existing.error());
    }

    step(Step::Installing);
    let key = new_access_key().map_err(InstallError::Other)?;
    let boundary = format!("figcraft{}", new_access_key().map_err(InstallError::Other)?.replace(['-', '_'], "x"));
    let uploaded = api.put_multipart(&script, &boundary, worker_upload(&boundary, &key))?;
    if !uploaded.ok() {
        return Err(uploaded.error());
    }

    step(Step::Address);
    let on = api.send_json("POST", &format!("{script}/subdomain"), &json!({"enabled": true, "previews_enabled": false}))?;
    if !on.ok() {
        return Err(on.error());
    }
    let address = format!("https://{WORKER_NAME}.{subdomain}.workers.dev");
    let connection = Connection::new(&address, &key).map_err(InstallError::Other)?;
    Ok(Installed { connection, account })
}

/// How long a key refusal is waited out after an install: a Worker that was just replaced can
/// answer with its old copy, and the old access key, for a few seconds.
pub const KEY_GRACE: Duration = Duration::from_secs(30);

/// Test the remover until it answers or `patience` runs out (a new address takes a while). A key
/// refusal is waited out for [`KEY_GRACE`] at most: past that the key is wrong.
pub fn settle(c: &Connection, patience: Duration) -> Result<Duration, String> {
    let start = Instant::now();
    loop {
        match super::net::test(c) {
            Ok(took) => return Ok(took),
            Err(e) if start.elapsed() + Duration::from_secs(5) >= patience => return Err(e),
            Err(e) if e.contains("access key") && start.elapsed() + Duration::from_secs(5) >= KEY_GRACE => return Err(e),
            Err(_) => std::thread::sleep(Duration::from_secs(5)),
        }
    }
}

#[cfg(test)]
pub(super) fn upload_for_tests(boundary: &str, key: &str) -> Vec<u8> {
    worker_upload(boundary, key)
}
