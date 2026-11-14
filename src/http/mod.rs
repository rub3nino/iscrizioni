mod view;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Form, Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{middleware, Router};
use chrono::{Duration as ChronoDuration, Utc};
use serde::Deserialize;

use crate::auth::{code6, otp_hash};
use crate::config::Config;
use crate::domain::{
    clean_html, explain, fmt_rome, fmt_rome_date, format_cents, render_vars, role_label, spots_left,
    stamp, status_chip, status_label, status_mark, to_local_input, valid_email,
};
use crate::mailer::{Mailer, Outgoing};
use crate::store::{Event, Package, Reg, Signup, Db};
use crate::store_ops::{age_of, BlockIn, CorrectIn, NoticeIn, OfferIn, PackageIn, SiteIn};
use view::*;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub cfg: Arc<Config>,
    pub mailer: Mailer,
    pub limiter: Limiter,
}

#[derive(Clone, Default)]
pub struct Limiter {
    hits: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
}

impl Limiter {
    pub fn allow(&self, key: &str, limit: usize, window: Duration) -> bool {
        let mut map = self.hits.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let entry = map.entry(key.to_string()).or_default();
        entry.retain(|t| now.saturating_duration_since(*t) < window);
        if entry.len() >= limit { return false; }
        entry.push(now);
        true
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(home))
        .route("/api/health", get(health))
        .route("/api/offerta", get(api_offerta))
        .route("/modulo", get(form_page).post(form_post))
        .route("/modulo/grazie", get(grazie))
        .route("/embed.js", get(embed_js))
        .route("/static/{*path}", get(asset))
        .route("/admin/login", get(login_get).post(login_post))
        .route("/admin/login/codice", post(login_code))
        .route("/admin/logout", post(logout))
        .route("/admin", get(|| async { redir("/admin/iscrizioni") }))
        .route("/admin/iscrizioni", get(page_iscrizioni))
        .route("/admin/iscrizioni.csv", get(csv))
        .route("/admin/iscrizioni/{id}/scheda.json", get(export_one))
        .route("/admin/iscrizioni/{id}/pagamento", post(post_pay))
        .route("/admin/iscrizioni/{id}/nota", post(post_note))
        .route("/admin/iscrizioni/{id}/correggi", post(post_correct))
        .route("/admin/iscrizioni/{id}/annulla", post(post_cancel))
        .route("/admin/iscrizioni/{id}/ripristina", post(post_restore))
        .route("/admin/iscrizioni/{id}/libera", post(post_release))
        .route("/admin/iscrizioni/{id}/elimina", post(post_delete))
        .route("/admin/iscrizioni/{id}/mail", post(post_mail))
        .route("/admin/sito", get(page_sito).post(post_sito))
        .route("/admin/sito/avviso", post(post_notice))
        .route("/admin/sito/avviso/muovi", post(post_notice_move))
        .route("/admin/sito/avviso/elimina", post(post_notice_del))
        .route("/admin/sito/blocco", post(post_block))
        .route("/admin/sito/blocco/elimina", post(post_block_del))
        .route("/admin/sito/campo", post(post_field))
        .route("/admin/sito/consenso", post(post_consent))
        .route("/admin/sito/consenso/elimina", post(post_consent_del))
        .route("/admin/offerta", get(page_offerta).post(post_offerta))
        .route("/admin/offerta/finestra", post(post_window))
        .route("/admin/offerta/finestra/elimina", post(post_window_del))
        .route("/admin/offerta/pacchetto", post(post_package))
        .route("/admin/offerta/pacchetto/elimina", post(post_package_del))
        .route("/admin/liste", get(page_liste))
        .route("/admin/mail", get(page_mail))
        .route("/admin/mail/{key}/prova", post(post_mail_test))
        .route("/admin/mail/{key}", get(page_mail_edit).post(post_mail_save))
        .route("/admin/registro", get(page_registro))
        .route("/admin/organizzatori", get(page_org).post(post_invite))
        .route("/admin/organizzatori/aggiorna", post(post_org))
        .route("/admin/backup", get(backup))
        .layer(middleware::from_fn(secure_headers))
        .with_state(state)
}

async fn secure_headers(req: axum::extract::Request, next: middleware::Next) -> Response {
    let admin = req.uri().path().starts_with("/admin");
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("same-origin"));
    if admin {
        h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    res
}

fn redir(to: &str) -> Response { Redirect::to(to).into_response() }
fn back(path: &str, code: &str) -> Response {
    let join = if path.contains('?') { '&' } else { '?' };
    redir(&format!("{path}{join}e={code}"))
}
fn okgo(path: &str) -> Response {
    let join = if path.contains('?') { '&' } else { '?' };
    redir(&format!("{path}{join}ok=salvato"))
}
fn qcode(q: &HashMap<String, String>) -> (String, bool, bool) {
    if let Some(e) = q.get("e") { return flash_of(Some(e), false); }
    if let Some(o) = q.get("ok") { return flash_of(Some(o), true); }
    (String::new(), false, false)
}
fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        let mut it = part.trim().splitn(2, '=');
        if it.next() == Some(name) { return it.next().map(|s| s.to_string()); }
    }
    None
}
fn set_cookie(id: &str, secure: bool) -> HeaderValue {
    let sec = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!("session={id}; HttpOnly; SameSite=Strict; Path=/; Max-Age=28800{sec}")).unwrap()
}
fn clear_cookie(secure: bool) -> HeaderValue {
    let sec = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!("session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0{sec}")).unwrap()
}
fn secure_flag(headers: &HeaderMap, cfg: &Config) -> bool {
    cfg.public_url.starts_with("https://") || headers.get("x-forwarded-proto").and_then(|v| v.to_str().ok()) == Some("https")
}
fn same_origin(headers: &HeaderMap) -> bool {
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    if host.is_empty() { return false; }
    headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()).or_else(|| headers.get(header::REFERER).and_then(|v| v.to_str().ok())).map(|o| o.contains(host)).unwrap_or(false)
}
fn client_ip(headers: &HeaderMap, addr: SocketAddr, trust: bool) -> String {
    if trust {
        if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
            if let Some(first) = xff.split(',').next() {
                let ip = first.trim();
                if !ip.is_empty() { return ip.to_string(); }
            }
        }
    }
    addr.ip().to_string()
}
fn actor(state: &AppState, headers: &HeaderMap) -> Option<crate::store::Organizer> {
    let id = cookie(headers, "session")?;
    let email = state.db.session_email(&id, Utc::now()).ok()??;
    let org = state.db.organizer(&email).ok()??;
    if org.active { Some(org) } else { None }
}
fn shell(state: &AppState, org: &crate::store::Organizer, page: &str, q: &HashMap<String, String>) -> Shell {
    let (flash, flash_err, saved) = qcode(q);
    Shell {
        event_name: state.db.event().map(|e| e.name).unwrap_or_else(|_| "Iscrizioni".into()),
        me_name: if org.name.is_empty() { org.email.clone() } else { org.name.clone() },
        me_email: org.email.clone(),
        role_label: role_label(&org.role).into(),
        page: page.into(),
        count: state.db.active_count().unwrap_or(0),
        flash, flash_err,
        can_edit: org.role != "viewer",
        is_owner: org.role == "owner",
        mail_on: state.mailer.configured(),
        saved,
    }
}
fn need_user(state: &AppState, headers: &HeaderMap) -> Result<crate::store::Organizer, Response> {
    actor(state, headers).ok_or_else(|| redir("/admin/login"))
}
fn gate(state: &AppState, headers: &HeaderMap, back_path: &str) -> Result<crate::store::Organizer, Response> {
    let org = need_user(state, headers)?;
    if org.role == "viewer" { return Err(back(back_path, "lettura")); }
    if !same_origin(headers) { return Err(back(back_path, "origine")); }
    Ok(org)
}
fn on(s: &str) -> bool { s == "1" || s == "on" }
fn dash(s: &str) -> String { if s.trim().is_empty() { "—".into() } else { s.to_string() } }
fn urlencoding_min(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'@' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
fn euro_to_cents(s: &str) -> Option<i64> {
    let t = s.trim().replace(['€', ' '], "");
    if t.is_empty() { return None; }
    let norm = if t.contains(',') { t.replace('.', "").replace(',', ".") } else { t };
    let v: f64 = norm.parse().ok()?;
    if !v.is_finite() || !(0.0..=100_000.0).contains(&v) { return None; }
    Some((v * 100.0).round() as i64)
}

#[derive(rust_embed::Embed)]
#[folder = "static/"]
struct Assets;

async fn asset(Path(path): Path<String>) -> Response {
    match Assets::get(&path) {
        Some(file) => {
            let mime = if path.ends_with(".css") { "text/css; charset=utf-8" }
                else if path.ends_with(".js") { "text/javascript; charset=utf-8" }
                else if path.ends_with(".svg") { "image/svg+xml" }
                else { "application/octet-stream" };
            ([(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, "public, max-age=3600")], file.data.into_owned()).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
async fn embed_js() -> Response { asset(Path("embed.js".into())).await }
async fn health() -> impl IntoResponse { ([(header::CONTENT_TYPE, "application/json")], r#"{"ok":true}"#) }
async fn home(State(state): State<AppState>) -> Response {
    render(HomePage { name: state.db.event().map(|e| e.name).unwrap_or_else(|_| "Iscrizioni".into()) })
}
async fn api_offerta(State(state): State<AppState>) -> Response {
    let event = match state.db.event() { Ok(e) => e, Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response() };
    let pkgs = state.db.packages().unwrap_or_default();
    let (taken, waiting) = state.db.occupancy().unwrap_or((0, 0));
    let body = serde_json::json!({
        "name": event.name, "door": event.door, "spots": spots_left(event.capacity, taken, waiting), "capacity": event.capacity,
        "packages": pkgs.iter().map(|p| serde_json::json!({"id": p.id, "name": p.name, "price_cents": p.price_cents, "active": p.active})).collect::<Vec<_>>(),
    });
    ([(header::CONTENT_TYPE, "application/json")], body.to_string()).into_response()
}

fn visible_now(starts: &str, ends: &str, now: chrono::DateTime<Utc>) -> bool {
    let after = if starts.is_empty() { true } else { crate::domain::parse_ts(starts).map(|t| now >= t).unwrap_or(false) };
    let before = if ends.is_empty() { true } else { crate::domain::parse_ts(ends).map(|t| now < t).unwrap_or(false) };
    after && before
}
fn pkg_selectable(p: &Package, windows: &[crate::store::Window], now: chrono::DateTime<Utc>) -> bool {
    if !p.active { return false; }
    let Some(wid) = &p.window_id else { return true };
    let Some(w) = windows.iter().find(|w| &w.id == wid) else { return false };
    if w.manually_closed { return false; }
    matches!((crate::domain::parse_ts(&w.starts_at), crate::domain::parse_ts(&w.ends_at)), (Some(a), Some(b)) if now >= a && now < b)
}
fn fallback_event() -> Event {
    Event { name: "Iscrizioni".into(), place: String::new(), starts_on: String::new(), ends_on: String::new(), currency: "EUR".into(), capacity: 0, deposit_cents: 0, deposit_hours: 48, min_age: 16, max_age: 99, minor_age: 18, door: "pause".into(), closed_message: "Le iscrizioni sono chiuse.".into(), countdown_at: String::new(), show_countdown: false, show_spots: true, iban: String::new(), account_holder: String::new(), payment_note: String::new(), allowed_origins: String::new() }
}
fn frame_csp(origins: &str) -> String {
    let mut parts = vec!["'self'".to_string()];
    for raw in origins.split(|c: char| c.is_whitespace() || c == ',') {
        let o = raw.trim().trim_end_matches('/');
        if o.starts_with("https://") || o.starts_with("http://") { parts.push(o.to_string()); }
    }
    format!("frame-ancestors {}", parts.join(" "))
}
fn form_model(state: &AppState, error: String) -> FormPage {
    let now = Utc::now();
    let event = state.db.event().unwrap_or_else(|_| fallback_event());
    let (taken, waiting) = state.db.occupancy().unwrap_or((0, 0));
    let windows = state.db.windows().unwrap_or_default();
    let packages = state.db.packages().unwrap_or_default().into_iter().filter(|p| p.active).map(|p| {
        let open = pkg_selectable(&p, &windows, now);
        PkgChoice { id: p.id.clone(), name: p.name.clone(), price: format_cents(p.price_cents), description: p.description.clone(), enabled: open, label: if open { String::new() } else if p.unavailable_label.is_empty() { "Non selezionabile".into() } else { p.unavailable_label.clone() } }
    }).collect();
    let notices = state.db.notices().unwrap_or_default().into_iter().filter(|n| n.visible && visible_now(&n.starts_at, &n.ends_at, now)).map(|n| Note { title: n.title, body: n.body, tone: if matches!(n.tone.as_str(), "info" | "attention" | "danger") { n.tone } else { "info".into() } }).collect();
    let blocks = state.db.blocks().unwrap_or_default();
    let take = |place: &str| blocks.iter().filter(|b| b.visible && b.placement == place).map(|b| Blk { title: b.title.clone(), body: b.body.clone() }).collect();
    FormPage {
        name: event.name.clone(), door: event.door.clone(),
        closed_message: if event.closed_message.is_empty() { "Le iscrizioni non sono aperte.".into() } else { event.closed_message },
        show_spots: event.show_spots, spots: spots_left(event.capacity, taken, waiting).to_string(),
        show_countdown: event.show_countdown && !event.countdown_at.is_empty(), countdown_label: fmt_rome(&event.countdown_at), countdown_iso: event.countdown_at,
        notices, blocks_before: take("before_form"), blocks_mid: take("after_packages"), blocks_end: take("before_submit"),
        packages, fields: state.db.fields().unwrap_or_default().into_iter().filter(|f| f.enabled).map(|f| Fld { key: f.key, label: f.label, required: f.required }).collect(),
        consents: state.db.consents().unwrap_or_default().into_iter().map(|c| Cns { id: c.id, label: c.label, required: c.required }).collect(),
        minor_age: event.minor_age, error, frame: frame_csp(&event.allowed_origins),
    }
}
async fn form_page(State(state): State<AppState>, Query(q): Query<HashMap<String, String>>) -> Response {
    let err = q.get("e").map(|c| explain(c).to_string()).unwrap_or_default();
    let page = form_model(&state, err);
    let mut res = render(page);
    if let Ok(ev) = state.db.event() {
        if let Ok(v) = HeaderValue::from_str(&frame_csp(&ev.allowed_origins)) { res.headers_mut().insert(header::CONTENT_SECURITY_POLICY, v); }
    }
    res
}

#[derive(Deserialize)]
struct SignupForm {
    #[serde(default)] first_name: String, #[serde(default)] last_name: String, #[serde(default)] birth_date: String,
    #[serde(default)] email: String, #[serde(default)] phone: String, #[serde(default)] package_id: String,
    #[serde(default)] gender: String, #[serde(default)] city: String, #[serde(default)] shirt: String,
    #[serde(default)] notes: String, #[serde(default)] allergies: String, #[serde(default)] transport: String,
    #[serde(default)] parent_name: String, #[serde(default)] parent_phone: String, #[serde(default)] parent_consent: String,
    #[serde(default)] consents: Vec<String>, #[serde(default)] sito_web: String,
}
async fn form_post(State(state): State<AppState>, ConnectInfo(addr): ConnectInfo<SocketAddr>, headers: HeaderMap, Form(f): Form<SignupForm>) -> Response {
    if !same_origin(&headers) { return back("/modulo", "origine"); }
    let ip = client_ip(&headers, addr, state.cfg.trust_proxy);
    if !state.limiter.allow(&format!("form:{ip}"), 5, Duration::from_secs(600)) { return back("/modulo", "limite"); }
    if !f.sito_web.trim().is_empty() { return redir("/modulo/grazie"); }
    let signup = Signup {
        first_name: f.first_name, last_name: f.last_name, birth_date: f.birth_date, email: f.email, phone: f.phone, package_id: f.package_id,
        gender: f.gender, city: f.city, shirt: f.shirt, notes: f.notes, allergies: f.allergies, transport: f.transport,
        parent_name: f.parent_name, parent_phone: f.parent_phone, parent_consent: on(&f.parent_consent), consent_ids: f.consents,
    };
    match state.db.signup(&signup, Utc::now()) {
        Ok(created) => {
            let key = if created.status == "attesa" { "lista_attesa" } else { "iscrizione_ricevuta" };
            let sent = dispatch(&state, key, &created.id, "sistema", None).await;
            notify_organizers(&state, &created.id).await;
            redir(&format!("/modulo/grazie?n={}&w={}&m={}", created.number, i32::from(created.status == "attesa"), i32::from(sent)))
        }
        Err(e) => back("/modulo", &state.db.code(&e)),
    }
}
async fn grazie(State(state): State<AppState>, Query(q): Query<HashMap<String, String>>) -> Response {
    render(GraziePage {
        name: state.db.event().map(|e| e.name).unwrap_or_else(|_| "Iscrizioni".into()),
        number: q.get("n").cloned().unwrap_or_default(),
        wait: q.get("w").map(|s| s == "1").unwrap_or(false),
        mail_off: q.get("m").map(|s| s == "0").unwrap_or(false),
    })
}

async fn login_get(State(state): State<AppState>, Query(q): Query<HashMap<String, String>>) -> Response {
    render(LoginPage {
        step: q.get("step").cloned().unwrap_or_else(|| "email".into()),
        email: q.get("email").cloned().unwrap_or_default(),
        error: q.get("e").map(|c| explain(c).to_string()).unwrap_or_default(),
        name: state.db.event().map(|e| e.name).unwrap_or_else(|_| "Iscrizioni".into()),
    })
}
#[derive(Deserialize)] struct EmailForm { email: String }
async fn login_post(State(state): State<AppState>, ConnectInfo(addr): ConnectInfo<SocketAddr>, headers: HeaderMap, Form(f): Form<EmailForm>) -> Response {
    if !same_origin(&headers) { return back("/admin/login", "origine"); }
    let ip = client_ip(&headers, addr, state.cfg.trust_proxy);
    let email = f.email.trim().to_lowercase();
    if !state.limiter.allow(&format!("otp-ip:{ip}"), 8, Duration::from_secs(900)) || !state.limiter.allow(&format!("otp-mail:{email}"), 3, Duration::from_secs(600)) {
        return back("/admin/login", "limite");
    }
    if valid_email(&email) {
        let code = code6();
        let hash = otp_hash(&state.cfg.otp_secret, &email, &code);
        if state.db.issue_otp(&email, &hash, Utc::now()).unwrap_or(false) {
            let _ = state.mailer.send(Outgoing { to: email.clone(), subject: "Codice di accesso".into(), html: format!("<p>Il codice è {code}. Scade tra 10 minuti.</p>"), text: format!("Il codice è {code}. Scade tra 10 minuti."), dev_code: Some(code) }).await;
        }
    }
    redir(&format!("/admin/login?step=code&email={}", urlencoding_min(&email)))
}
#[derive(Deserialize)] struct CodeForm { email: String, code: String }
async fn login_code(State(state): State<AppState>, ConnectInfo(addr): ConnectInfo<SocketAddr>, headers: HeaderMap, Form(f): Form<CodeForm>) -> Response {
    if !same_origin(&headers) { return back("/admin/login", "origine"); }
    let ip = client_ip(&headers, addr, state.cfg.trust_proxy);
    if !state.limiter.allow(&format!("ver-ip:{ip}"), 10, Duration::from_secs(900)) { return back("/admin/login", "limite"); }
    let email = f.email.trim().to_lowercase();
    let hash = otp_hash(&state.cfg.otp_secret, &email, f.code.trim());
    if state.db.verify_otp(&email, &hash, Utc::now()).unwrap_or(false) {
        if let Ok(id) = state.db.create_session(&email, Utc::now()) {
            let mut res = redir("/admin/iscrizioni");
            res.headers_mut().insert(header::SET_COOKIE, set_cookie(&id, secure_flag(&headers, &state.cfg)));
            return res;
        }
    }
    redir(&format!("/admin/login?step=code&email={}&e=codice", urlencoding_min(&email)))
}
async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(id) = cookie(&headers, "session") { let _ = state.db.drop_session(&id); }
    let mut res = redir("/admin/login");
    res.headers_mut().insert(header::SET_COOKIE, clear_cookie(secure_flag(&headers, &state.cfg)));
    res
}

fn reg_vars(state: &AppState, id: &str) -> Vec<(String, String)> {
    let event = state.db.event().unwrap_or_else(|_| fallback_event());
    let Some(reg) = state.db.reg(id).ok().flatten() else { return sample_vars(&event) };
    let pkg = state.db.packages().ok().and_then(|ps| ps.into_iter().find(|p| p.id == reg.package_id));
    let price = pkg.as_ref().map(|p| p.price_cents).unwrap_or(0);
    let pname = pkg.map(|p| p.name).unwrap_or_else(|| "—".into());
    let due = if reg.deposit_due_at.is_empty() { stamp(Utc::now() + ChronoDuration::hours(event.deposit_hours)) } else { reg.deposit_due_at.clone() };
    let causale = if event.payment_note.is_empty() { format!("Iscrizione {}", reg.number) } else { event.payment_note.clone() };
    vec![
        ("nome".into(), reg.first_name), ("cognome".into(), reg.last_name), ("numero".into(), reg.number.to_string()),
        ("pacchetto".into(), pname), ("prezzo".into(), format_cents(price)), ("acconto".into(), format_cents(event.deposit_cents)),
        ("scadenza".into(), fmt_rome(&due)), ("iban".into(), dash(&event.iban)), ("intestatario".into(), dash(&event.account_holder)),
        ("causale".into(), causale), ("evento".into(), event.name), ("email".into(), reg.email), ("telefono".into(), reg.phone),
        ("link".into(), format!("{}/admin/login", state.cfg.public_url.trim_end_matches('/'))), ("codice".into(), String::new()),
    ]
}
fn sample_vars(event: &Event) -> Vec<(String, String)> {
    vec![
        ("nome".into(), "Mario".into()), ("cognome".into(), "Rossi".into()), ("numero".into(), "12".into()),
        ("pacchetto".into(), "Standard".into()), ("prezzo".into(), format_cents(15000)), ("acconto".into(), format_cents(event.deposit_cents.max(5000))),
        ("scadenza".into(), "01/12/2026 18:00".into()), ("iban".into(), dash(&event.iban)), ("intestatario".into(), dash(&event.account_holder)),
        ("causale".into(), "Iscrizione 12".into()), ("evento".into(), event.name.clone()), ("email".into(), "mario.rossi@example.com".into()),
        ("telefono".into(), "+39 333 0000000".into()), ("link".into(), "https://example.test/admin/login".into()), ("codice".into(), "123456".into()),
    ]
}
async fn dispatch(state: &AppState, key: &str, reg_id: &str, actor_email: &str, dev_code: Option<String>) -> bool {
    let Some(tpl) = state.db.template(key).ok().flatten() else { return false };
    if !tpl.enabled { let _ = state.db.log_mail(key, reg_id, "", &tpl.subject, false, "disattivato", actor_email, Utc::now()); return false; }
    let mut vars = if reg_id.is_empty() { sample_vars(&state.db.event().unwrap_or_else(|_| fallback_event())) } else { reg_vars(state, reg_id) };
    if let Some(code) = &dev_code { if let Some(slot) = vars.iter_mut().find(|(k, _)| k == "codice") { slot.1.clone_from(code); } }
    let to = state.db.reg(reg_id).ok().flatten().map(|r| r.email).unwrap_or_default();
    if to.is_empty() { return false; }
    let subject = render_vars(&tpl.subject, &vars, false);
    let html = render_vars(&tpl.html, &vars, true);
    let text = render_vars(&tpl.text_body, &vars, false);
    let res = state.mailer.send(Outgoing { to: to.clone(), subject: subject.clone(), html, text, dev_code }).await;
    let ok = res.is_ok();
    let err = res.err().unwrap_or_default();
    let _ = state.db.log_mail(key, reg_id, &to, &subject, ok, &err, actor_email, Utc::now());
    ok
}
async fn notify_organizers(state: &AppState, reg_id: &str) {
    for to in state.db.notify_emails().unwrap_or_default() {
        let Some(tpl) = state.db.template("avviso_organizzatore").ok().flatten() else { continue };
        let vars = reg_vars(state, reg_id);
        let subject = render_vars(&tpl.subject, &vars, false);
        let html = render_vars(&tpl.html, &vars, true);
        let text = render_vars(&tpl.text_body, &vars, false);
        let res = state.mailer.send(Outgoing { to: to.clone(), subject: subject.clone(), html, text, dev_code: None }).await;
        let ok = res.is_ok();
        let err = res.err().unwrap_or_default();
        let _ = state.db.log_mail("avviso_organizzatore", reg_id, &to, &subject, ok, &err, "sistema", Utc::now());
    }
}

async fn page_iscrizioni(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let org = match need_user(&state, &headers) { Ok(o) => o, Err(r) => return r };
    let s = shell(&state, &org, "iscrizioni", &q);
    let regs = state.db.regs().unwrap_or_default();
    let pkgs = state.db.packages().unwrap_or_default();
    let vista = q.get("vista").cloned().unwrap_or_else(|| "tutte".into());
    let query = q.get("q").cloned().unwrap_or_default();
    let ql = query.to_lowercase();
    let keep = |r: &Reg| {
        let hit = ql.is_empty() || format!("{} {} {} {} {}", r.first_name, r.last_name, r.email, r.city, r.phone).to_lowercase().contains(&ql);
        let view_ok = match vista.as_str() {
            "fare" => r.status == "ricevuta" || r.status == "attende_acconto",
            "acconto" => r.status == "attende_acconto",
            "lista" => r.status == "attesa",
            "posti" => r.status == "posto" || r.status == "saldato",
            "annullate" => r.status == "annullata",
            _ => true,
        };
        hit && view_ok
    };
    let n = |pred: fn(&Reg) -> bool| regs.iter().filter(|r| pred(r)).count() as i64;
    let tabs = vec![
        Tab { id: "tutte".into(), label: "Tutte".into(), n: regs.len() as i64, on: vista == "tutte" },
        Tab { id: "fare".into(), label: "Da fare".into(), n: n(|r| r.status == "ricevuta" || r.status == "attende_acconto"), on: vista == "fare" },
        Tab { id: "acconto".into(), label: "Acconto".into(), n: n(|r| r.status == "attende_acconto"), on: vista == "acconto" },
        Tab { id: "lista".into(), label: "Lista".into(), n: n(|r| r.status == "attesa"), on: vista == "lista" },
        Tab { id: "posti".into(), label: "Posti".into(), n: n(|r| r.status == "posto" || r.status == "saldato"), on: vista == "posti" },
        Tab { id: "annullate".into(), label: "Annullate".into(), n: n(|r| r.status == "annullata"), on: vista == "annullate" },
    ];
    let event = state.db.event().unwrap_or_else(|_| fallback_event());
    let (taken, _) = state.db.occupancy().unwrap_or((0, 0));
    let mut da_incassare = 0i64;
    let rows = regs.iter().filter(|r| keep(r)).map(|r| {
        let paid = state.db.paid_of(&r.id).unwrap_or(0);
        let price = pkgs.iter().find(|p| p.id == r.package_id).map(|p| p.price_cents).unwrap_or(0);
        if r.status != "annullata" && r.status != "attesa" { da_incassare += (price - paid).max(0); }
        RegRow { href: format!("/admin/iscrizioni?vista={vista}&q={}&id={}", urlencoding_min(&query), r.id), number: r.number.to_string(), name: format!("{} {}", r.first_name, r.last_name), email: r.email.clone(), package: pkgs.iter().find(|p| p.id == r.package_id).map(|p| p.name.clone()).unwrap_or_else(|| "—".into()), status: status_label(&r.status).into(), chip: status_chip(&r.status).into(), mark: status_mark(&r.status).into(), paid: format_cents(paid) }
    }).collect();
    let open_id = q.get("id").cloned().unwrap_or_default();
    let (open, d) = if let Some(r) = regs.iter().find(|r| r.id == open_id) { (true, detail_of(&state, r, &pkgs)) } else { (false, Detail::default()) };
    render(IscrizioniPage { s, title: "Iscrizioni".into(), da_fare: n(|r| r.status == "ricevuta" || r.status == "attende_acconto").to_string(), acconto: n(|r| r.status == "attende_acconto").to_string(), posti: taken.to_string(), cap: event.capacity.to_string(), da_incassare: format_cents(da_incassare), q: query, vista, tabs, rows, open, d })
}
fn detail_of(state: &AppState, r: &Reg, pkgs: &[Package]) -> Detail {
    let paid = state.db.paid_of(&r.id).unwrap_or(0);
    let price = pkgs.iter().find(|p| p.id == r.package_id).map(|p| p.price_cents).unwrap_or(0);
    Detail {
        id: r.id.clone(), number: r.number.to_string(), name: format!("{} {}", r.first_name, r.last_name), email: r.email.clone(), phone: dash(&r.phone), birth: fmt_rome_date(&r.birth_date),
        package: pkgs.iter().find(|p| p.id == r.package_id).map(|p| p.name.clone()).unwrap_or_else(|| "—".into()), status: status_label(&r.status).into(), chip: status_chip(&r.status).into(), mark: status_mark(&r.status).into(),
        paid: format_cents(paid), price: format_cents(price), rest: format_cents((price - paid).max(0)), due: fmt_rome(&r.deposit_due_at),
        gender: dash(&r.gender), city: dash(&r.city), shirt: dash(&r.shirt), notes: if r.notes.is_empty() { String::new() } else { r.notes.clone() }, allergies: if r.allergies.is_empty() { String::new() } else { r.allergies.clone() }, transport: dash(&r.transport),
        parent_name: if r.parent_name.is_empty() { String::new() } else { r.parent_name.clone() }, parent_phone: if r.parent_phone.is_empty() { String::new() } else { r.parent_phone.clone() }, parent_ok: r.parent_consent,
        first: r.first_name.clone(), last: r.last_name.clone(), birth_raw: r.birth_date.clone(), phone_raw: r.phone.clone(), package_id: r.package_id.clone(),
        can_mail_pay: matches!(r.status.as_str(), "ricevuta" | "attesa" | "attende_acconto"), can_pay: !matches!(r.status.as_str(), "annullata" | "attesa"), can_cancel: r.status != "annullata", can_restore: r.status == "annullata", can_release: crate::domain::can_release(&r.status, &r.deposit_due_at, paid, Utc::now()), can_delete: r.status == "annullata",
        hist: state.db.history(&r.id).unwrap_or_default().into_iter().map(|(a, b, c)| Line { a: fmt_rome(&a), b, c }).collect(),
        pays: state.db.payments(&r.id).unwrap_or_default().into_iter().map(|(cents, method, kind, _, at)| Line { a: format_cents(cents), b: format!("{kind} · {method}"), c: fmt_rome(&at) }).collect(),
        notes_i: state.db.notes(&r.id).unwrap_or_default().into_iter().map(|(body, author, at)| Line { a: fmt_rome(&at), b: author, c: body }).collect(),
        mails: state.db.mails_of(&r.id).unwrap_or_default().into_iter().map(|(key, sub, ok, at)| Line { a: fmt_rome(&at), b: key, c: format!("{sub} · {}", if ok == 1 { "inviata" } else { "non partita" }) }).collect(),
        pkgs: pkgs.iter().map(|p| Opt { id: p.id.clone(), name: p.name.clone(), on: p.id == r.package_id }).collect(),
    }
}

#[derive(Deserialize)] struct KeyForm { key: String }
async fn post_mail(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<KeyForm>) -> Response {
    let path = format!("/admin/iscrizioni?id={id}");
    let org = match gate(&state, &headers, &path) { Ok(o) => o, Err(r) => return r };
    if f.key == "dati_pagamento" { if let Err(e) = state.db.deposit_possible(&id) { return back(&path, &state.db.code(&e)); } }
    let sent = dispatch(&state, &f.key, &id, &org.email, None).await;
    if sent && f.key == "dati_pagamento" { if let Err(e) = state.db.arm_deposit(&id, &org.email, Utc::now()) { return back(&path, &state.db.code(&e)); } }
    back(&path, if sent { "mail_ok" } else { "mail_ko" })
}
#[derive(Deserialize)] struct PayForm { euro: String, kind: String, method: String, #[serde(default)] reference: String }
async fn post_pay(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<PayForm>) -> Response {
    let path = format!("/admin/iscrizioni?id={id}");
    let org = match gate(&state, &headers, &path) { Ok(o) => o, Err(r) => return r };
    let Some(cents) = euro_to_cents(&f.euro) else { return back(&path, "pagamento"); };
    let before = state.db.reg(&id).ok().flatten().map(|r| r.status);
    match state.db.add_payment(&id, cents, &f.method, &f.kind, &f.reference, &org.email, Utc::now()) {
        Ok(after) => {
            if before.as_deref() != Some(after.as_str()) {
                let key = if after == "saldato" { "saldato" } else if after == "posto" { "posto_assicurato" } else { "" };
                if !key.is_empty() { let _ = dispatch(&state, key, &id, &org.email, None).await; }
            }
            okgo(&path)
        }
        Err(e) => back(&path, &state.db.code(&e)),
    }
}
#[derive(Deserialize)] struct NoteForm { body: String }
async fn post_note(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<NoteForm>) -> Response {
    let path = format!("/admin/iscrizioni?id={id}");
    let org = match gate(&state, &headers, &path) { Ok(o) => o, Err(r) => return r };
    match state.db.add_note(&id, &f.body, &org.email, Utc::now()) { Ok(()) => okgo(&path), Err(e) => back(&path, &state.db.code(&e)) }
}
#[derive(Deserialize)]
struct CorrectForm {
    first_name: String, last_name: String, birth_date: String, email: String, phone: String, package_id: String,
    #[serde(default)] gender: String, #[serde(default)] city: String, #[serde(default)] shirt: String,
    #[serde(default)] notes: String, #[serde(default)] allergies: String, #[serde(default)] transport: String,
    #[serde(default)] parent_name: String, #[serde(default)] parent_phone: String, #[serde(default)] parent_consent: String,
}
async fn post_correct(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Form(f): Form<CorrectForm>) -> Response {
    let path = format!("/admin/iscrizioni?id={id}");
    let org = match gate(&state, &headers, &path) { Ok(o) => o, Err(r) => return r };
    let input = CorrectIn { first_name: f.first_name, last_name: f.last_name, birth_date: f.birth_date, email: f.email, phone: f.phone, package_id: f.package_id, gender: f.gender, city: f.city, shirt: f.shirt, notes: f.notes, allergies: f.allergies, transport: f.transport, parent_name: f.parent_name, parent_phone: f.parent_phone, parent_consent: on(&f.parent_consent) };
    match state.db.correct(&id, input, &org.email, Utc::now()) { Ok(()) => okgo(&path), Err(e) => back(&path, &state.db.code(&e)) }
}
async fn post_cancel(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response { status_op(&state, &headers, &id, "annullata", |db, who| db.cancel(&id, who, Utc::now())).await }
async fn post_restore(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let path = format!("/admin/iscrizioni?id={id}");
    let org = match gate(&state, &headers, &path) { Ok(o) => o, Err(r) => return r };
    match state.db.restore(&id, &org.email, Utc::now()) { Ok(_) => okgo(&path), Err(e) => back(&path, &state.db.code(&e)) }
}
async fn post_release(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response { status_op(&state, &headers, &id, "termine_scaduto", |db, who| db.release(&id, who, Utc::now())).await }
async fn post_delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let path = format!("/admin/iscrizioni?id={id}");
    let org = match gate(&state, &headers, &path) { Ok(o) => o, Err(r) => return r };
    if org.role != "owner" { return back(&path, "lettura"); }
    match state.db.hard_delete(&id, &org.email, Utc::now()) { Ok(()) => redir("/admin/iscrizioni?ok=eliminata"), Err(e) => back(&path, &state.db.code(&e)) }
}
async fn status_op(state: &AppState, headers: &HeaderMap, id: &str, mail_key: &str, op: impl FnOnce(&Db, &str) -> Result<(), crate::store::Fail>) -> Response {
    let path = format!("/admin/iscrizioni?id={id}");
    let org = match gate(state, headers, &path) { Ok(o) => o, Err(r) => return r };
    match op(&state.db, &org.email) {
        Ok(()) => { let _ = dispatch(state, mail_key, id, &org.email, None).await; okgo(&path) }
        Err(e) => back(&path, &state.db.code(&e)),
    }
}
async fn csv(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if need_user(&state, &headers).is_err() { return redir("/admin/login"); }
    let pkgs = state.db.packages().unwrap_or_default();
    let mut out = String::from("\u{feff}numero,stato,nome,cognome,email,telefono,nascita,pacchetto,incassato_centesimi,citta,taglia,genere,allergie\n");
    for r in state.db.regs().unwrap_or_default() {
        let paid = state.db.paid_of(&r.id).unwrap_or(0);
        let pkg = pkgs.iter().find(|p| p.id == r.package_id).map(|p| p.name.as_str()).unwrap_or("");
        out.push_str(&format!("{},{},{},{},{},{},{},{},{},{},{},{},{}\n", r.number, cell(&r.status), cell(&r.first_name), cell(&r.last_name), cell(&r.email), cell(&r.phone), r.birth_date, cell(pkg), paid, cell(&r.city), cell(&r.shirt), cell(&r.gender), cell(&r.allergies)));
    }
    ([(header::CONTENT_TYPE, "text/csv; charset=utf-8"), (header::CONTENT_DISPOSITION, "attachment; filename=\"iscrizioni.csv\"")], out).into_response()
}
fn cell(s: &str) -> String { if s.contains([',', '"', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() } }
async fn export_one(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if need_user(&state, &headers).is_err() { return redir("/admin/login"); }
    let Some(r) = state.db.reg(&id).ok().flatten() else { return StatusCode::NOT_FOUND.into_response() };
    let body = serde_json::json!({ "id": r.id, "number": r.number, "status": r.status, "first_name": r.first_name, "last_name": r.last_name, "birth_date": r.birth_date, "email": r.email, "phone": r.phone, "paid_cents": state.db.paid_of(&r.id).unwrap_or(0) });
    ([(header::CONTENT_TYPE, "application/json"), (header::CONTENT_DISPOSITION, "attachment; filename=\"iscrizione.json\"")], body.to_string()).into_response()
}

async fn page_sito(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let org = match need_user(&state, &headers) { Ok(o) => o, Err(r) => return r };
    let e = state.db.event().unwrap_or_else(|_| fallback_event());
    render(SitoPage {
        s: shell(&state, &org, "sito", &q), title: "Sul sito".into(), door: e.door, closed_message: e.closed_message, countdown: to_local_input(&e.countdown_at), show_countdown: e.show_countdown, show_spots: e.show_spots, origins: e.allowed_origins,
        notices: state.db.notices().unwrap_or_default().into_iter().map(|n| NEdit { id: n.id, title: n.title, body: n.body, tone: n.tone, visible: n.visible, starts: to_local_input(&n.starts_at), ends: to_local_input(&n.ends_at) }).collect(),
        blocks: state.db.blocks().unwrap_or_default().into_iter().map(|b| BEdit { id: b.id, title: b.title, body: b.body, placement: b.placement, visible: b.visible }).collect(),
        fields: state.db.fields().unwrap_or_default().into_iter().map(|f| FEdit { key: f.key, label: f.label, enabled: f.enabled, required: f.required }).collect(),
        consents: state.db.consents().unwrap_or_default().into_iter().map(|c| CEdit { id: c.id, label: c.label, required: c.required }).collect(),
    })
}
#[derive(Deserialize)] struct SiteForm { door: String, #[serde(default)] closed_message: String, #[serde(default)] countdown: String, #[serde(default)] show_countdown: String, #[serde(default)] show_spots: String, #[serde(default)] origins: String }
async fn post_sito(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<SiteForm>) -> Response {
    let org = match gate(&state, &headers, "/admin/sito") { Ok(o) => o, Err(r) => return r };
    let input = SiteIn { door: f.door, closed_message: f.closed_message, countdown_at: crate::domain::parse_local_dt(&f.countdown).map(|d| stamp(d)).unwrap_or_default(), show_countdown: on(&f.show_countdown), show_spots: on(&f.show_spots), allowed_origins: f.origins };
    match state.db.save_site(input, &org.email, Utc::now()) { Ok(()) => okgo("/admin/sito"), Err(e) => back("/admin/sito", &state.db.code(&e)) }
}
#[derive(Deserialize)] struct NoticeForm { #[serde(default)] id: String, title: String, #[serde(default)] body: String, #[serde(default)] tone: String, #[serde(default)] visible: String, #[serde(default)] starts: String, #[serde(default)] ends: String }
async fn post_notice(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<NoticeForm>) -> Response {
    let org = match gate(&state, &headers, "/admin/sito") { Ok(o) => o, Err(r) => return r };
    let n = NoticeIn { id: f.id, title: f.title, body: f.body, tone: f.tone, visible: on(&f.visible), starts_at: crate::domain::parse_local_dt(&f.starts).map(|d| stamp(d)).unwrap_or_default(), ends_at: crate::domain::parse_local_dt(&f.ends).map(|d| stamp(d)).unwrap_or_default() };
    match state.db.save_notice(n, &org.email, Utc::now()) { Ok(()) => okgo("/admin/sito"), Err(e) => back("/admin/sito", &state.db.code(&e)) }
}
#[derive(Deserialize)] struct MoveForm { id: String, dir: i64 }
#[derive(Deserialize)] struct IdForm { id: String }
async fn post_notice_move(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<MoveForm>) -> Response {
    if gate(&state, &headers, "/admin/sito").is_err() { return back("/admin/sito", "lettura"); }
    let _ = state.db.move_notice(&f.id, f.dir); redir("/admin/sito")
}
async fn post_notice_del(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<IdForm>) -> Response {
    if gate(&state, &headers, "/admin/sito").is_err() { return back("/admin/sito", "lettura"); }
    let _ = state.db.delete_notice(&f.id); redir("/admin/sito")
}
#[derive(Deserialize)] struct BlockForm { #[serde(default)] id: String, #[serde(default)] title: String, #[serde(default)] body: String, #[serde(default)] placement: String, #[serde(default)] visible: String }
async fn post_block(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<BlockForm>) -> Response {
    let org = match gate(&state, &headers, "/admin/sito") { Ok(o) => o, Err(r) => return r };
    match state.db.save_block(BlockIn { id: f.id, title: f.title, body: f.body, placement: f.placement, visible: on(&f.visible) }, &org.email, Utc::now()) { Ok(()) => okgo("/admin/sito"), Err(e) => back("/admin/sito", &state.db.code(&e)) }
}
async fn post_block_del(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<IdForm>) -> Response {
    if gate(&state, &headers, "/admin/sito").is_err() { return back("/admin/sito", "lettura"); }
    let _ = state.db.delete_block(&f.id); redir("/admin/sito")
}
#[derive(Deserialize)] struct FieldForm { key: String, label: String, #[serde(default)] enabled: String, #[serde(default)] required: String }
async fn post_field(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<FieldForm>) -> Response {
    if gate(&state, &headers, "/admin/sito").is_err() { return back("/admin/sito", "lettura"); }
    let _ = state.db.save_field(&f.key, &f.label, on(&f.enabled), on(&f.required)); okgo("/admin/sito")
}
#[derive(Deserialize)] struct ConsentForm { label: String, #[serde(default)] required: String }
async fn post_consent(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<ConsentForm>) -> Response {
    let org = match gate(&state, &headers, "/admin/sito") { Ok(o) => o, Err(r) => return r };
    match state.db.save_consent(&f.label, on(&f.required), &org.email, Utc::now()) { Ok(()) => okgo("/admin/sito"), Err(e) => back("/admin/sito", &state.db.code(&e)) }
}
async fn post_consent_del(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<IdForm>) -> Response {
    if gate(&state, &headers, "/admin/sito").is_err() { return back("/admin/sito", "lettura"); }
    match state.db.delete_consent(&f.id) { Ok(()) => redir("/admin/sito"), Err(e) => back("/admin/sito", &state.db.code(&e)) }
}

async fn page_offerta(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let org = match need_user(&state, &headers) { Ok(o) => o, Err(r) => return r };
    let e = state.db.event().unwrap_or_else(|_| fallback_event());
    let windows = state.db.windows().unwrap_or_default();
    let euros = |c: i64| format!("{}.{:02}", c / 100, (c.abs() % 100));
    render(OffertaPage {
        s: shell(&state, &org, "offerta", &q), title: "Offerta".into(), name: e.name, place: e.place, starts_on: e.starts_on, ends_on: e.ends_on, currency: e.currency, capacity: e.capacity, deposit: euros(e.deposit_cents), hours: e.deposit_hours, min_age: e.min_age, max_age: e.max_age, minor_age: e.minor_age, iban: e.iban, holder: e.account_holder, pay_note: e.payment_note,
        windows: windows.iter().map(|w| WRow { id: w.id.clone(), name: w.name.clone(), start: fmt_rome(&w.starts_at), end: fmt_rome(&w.ends_at), closed: w.manually_closed }).collect(),
        packages: state.db.packages().unwrap_or_default().into_iter().map(|p| PRow { price: euros(p.price_cents), cap: p.capacity.map(|c| c.to_string()).unwrap_or_else(|| "—".into()), window: windows.iter().find(|w| Some(&w.id) == p.window_id.as_ref()).map(|w| w.name.clone()).unwrap_or_else(|| "—".into()), cap_raw: p.capacity.map(|c| c.to_string()).unwrap_or_default(), window_id: p.window_id.clone().unwrap_or_default(), id: p.id, name: p.name, active: p.active, description: p.description, label: p.unavailable_label, price_cents: p.price_cents, sort: p.sort }).collect(),
        window_opts: windows.iter().map(|w| Opt { id: w.id.clone(), name: w.name.clone(), on: false }).collect(),
    })
}
#[derive(Deserialize)] struct OfferForm { name: String, #[serde(default)] place: String, #[serde(default)] starts_on: String, #[serde(default)] ends_on: String, #[serde(default)] currency: String, capacity: i64, deposit: String, hours: i64, min_age: i64, max_age: i64, minor_age: i64, #[serde(default)] iban: String, #[serde(default)] holder: String, #[serde(default)] pay_note: String }
async fn post_offerta(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<OfferForm>) -> Response {
    let org = match gate(&state, &headers, "/admin/offerta") { Ok(o) => o, Err(r) => return r };
    let Some(deposit_cents) = euro_to_cents(&f.deposit) else { return back("/admin/offerta", "dati"); };
    let input = OfferIn { name: f.name, place: f.place, starts_on: f.starts_on, ends_on: f.ends_on, currency: if f.currency.is_empty() { "EUR".into() } else { f.currency }, capacity: f.capacity, deposit_cents, deposit_hours: f.hours, min_age: f.min_age, max_age: f.max_age, minor_age: f.minor_age, iban: f.iban, account_holder: f.holder, payment_note: f.pay_note };
    match state.db.save_offer(input, &org.email, Utc::now()) { Ok(()) => okgo("/admin/offerta"), Err(e) => back("/admin/offerta", &state.db.code(&e)) }
}
#[derive(Deserialize)] struct WindowForm { name: String, starts: String, ends: String, #[serde(default)] closed: String }
async fn post_window(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<WindowForm>) -> Response {
    let org = match gate(&state, &headers, "/admin/offerta") { Ok(o) => o, Err(r) => return r };
    match state.db.save_window(&f.name, &f.starts, &f.ends, on(&f.closed), &org.email, Utc::now()) { Ok(()) => okgo("/admin/offerta"), Err(e) => back("/admin/offerta", &state.db.code(&e)) }
}
async fn post_window_del(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<IdForm>) -> Response {
    let org = match gate(&state, &headers, "/admin/offerta") { Ok(o) => o, Err(r) => return r };
    match state.db.delete_window(&f.id, &org.email, Utc::now()) { Ok(()) => redir("/admin/offerta"), Err(e) => back("/admin/offerta", &state.db.code(&e)) }
}
#[derive(Deserialize)] struct PkgForm { #[serde(default)] id: String, name: String, euro: String, #[serde(default)] cap: String, #[serde(default)] window_id: String, #[serde(default)] description: String, #[serde(default)] label: String, #[serde(default)] sort: i64, #[serde(default)] active: String }
async fn post_package(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<PkgForm>) -> Response {
    let org = match gate(&state, &headers, "/admin/offerta") { Ok(o) => o, Err(r) => return r };
    let Some(price_cents) = euro_to_cents(&f.euro) else { return back("/admin/offerta", "dati"); };
    let capacity = if f.cap.trim().is_empty() { None } else { f.cap.trim().parse().ok() };
    let window_id = if f.window_id.is_empty() { None } else { Some(f.window_id) };
    let active = on(&f.active);
    match state.db.save_package(PackageIn { id: f.id, name: f.name, price_cents, capacity, window_id, description: f.description, unavailable_label: f.label, sort: f.sort, active }, &org.email, Utc::now()) { Ok(()) => okgo("/admin/offerta"), Err(e) => back("/admin/offerta", &state.db.code(&e)) }
}
async fn post_package_del(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<IdForm>) -> Response {
    let org = match gate(&state, &headers, "/admin/offerta") { Ok(o) => o, Err(r) => return r };
    match state.db.delete_package(&f.id, &org.email, Utc::now()) { Ok(()) => redir("/admin/offerta"), Err(e) => back("/admin/offerta", &state.db.code(&e)) }
}

async fn page_liste(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let org = match need_user(&state, &headers) { Ok(o) => o, Err(r) => return r };
    let regs: Vec<Reg> = state.db.regs().unwrap_or_default().into_iter().filter(|r| r.status != "annullata").collect();
    let fields = state.db.fields().unwrap_or_default();
    let enabled = |k: &str| fields.iter().any(|f| f.key == k && f.enabled);
    let warn = |k: &str, label: &str| if enabled(k) { String::new() } else { format!("Il campo {label} non è nel modulo. Attivalo da Sul sito.") };
    let event = state.db.event().unwrap_or_else(|_| fallback_event());
    let pkgs = state.db.packages().unwrap_or_default();
    let mut lists = vec![
        bucket_list("Taglie", "Quante persone per taglia.", &warn("shirt", "Taglia"), &regs, |r| if r.shirt.is_empty() { "—".into() } else { r.shirt.clone() }),
        bucket_list("Città", "Quante persone per città.", &warn("city", "Città"), &regs, |r| if r.city.is_empty() { "—".into() } else { r.city.clone() }),
    ];
    let minori = regs.iter().filter(|r| age_of(&event.starts_on, &r.birth_date, Utc::now().date_naive()).unwrap_or(99) < event.minor_age as i32).map(|r| vec![format!("{} {}", r.first_name, r.last_name), fmt_rome_date(&r.birth_date), dash(&r.parent_name), dash(&r.parent_phone)]).collect();
    lists.push(table_list("Minori", "Sotto la soglia, con chi ne è responsabile.", "", &["Persona", "Nascita", "Responsabile", "Telefono"], minori));
    let allergie = regs.iter().filter(|r| !r.allergies.trim().is_empty() || !r.notes.trim().is_empty()).map(|r| vec![format!("{} {}", r.first_name, r.last_name), if r.allergies.is_empty() { r.notes.clone() } else { r.allergies.clone() }]).collect();
    lists.push(table_list("Allergie", "Chi ha scritto un'allergia o una nota.", &warn("allergies", "Allergie"), &["Persona", "Nota"], allergie));
    let rooms = vec![vec!["Donna".into(), regs.iter().filter(|r| r.gender == "f").count().to_string()], vec!["Uomo".into(), regs.iter().filter(|r| r.gender == "m").count().to_string()], vec!["Non indicato".into(), regs.iter().filter(|r| r.gender != "f" && r.gender != "m").count().to_string()]];
    lists.push(table_list("Camere", "Per genere.", &warn("gender", "Genere"), &["Genere", "Persone"], rooms));
    let mut money = Vec::new();
    for p in &pkgs {
        let mine: Vec<&Reg> = regs.iter().filter(|r| r.package_id == p.id).collect();
        let inc: i64 = mine.iter().map(|r| state.db.paid_of(&r.id).unwrap_or(0)).sum();
        let due: i64 = mine.iter().map(|r| (p.price_cents - state.db.paid_of(&r.id).unwrap_or(0)).max(0)).sum();
        money.push(vec![p.name.clone(), mine.len().to_string(), format_cents(inc), format_cents(due)]);
    }
    lists.push(table_list("Incassi", "Per pacchetto.", "", &["Pacchetto", "Persone", "Incassato", "Da incassare"], money));
    let tutti = regs.iter().map(|r| vec![format!("{} {}", r.first_name, r.last_name), status_label(&r.status).into(), format_cents(state.db.paid_of(&r.id).unwrap_or(0))]).collect();
    lists.push(table_list("Tutti", "Elenco di chi non è annullato.", "", &["Persona", "Stato", "Incassato"], tutti));
    render(ListePage { s: shell(&state, &org, "liste", &q), title: "Liste".into(), lists })
}
fn bucket_list(title: &str, lead: &str, warn: &str, regs: &[Reg], key: impl Fn(&Reg) -> String) -> Lista {
    let mut map: HashMap<String, i64> = HashMap::new();
    for r in regs { *map.entry(key(r)).or_default() += 1; }
    let mut rows: Vec<_> = map.into_iter().map(|(k, n)| vec![k, n.to_string()]).collect();
    rows.sort_by(|a, b| a[0].cmp(&b[0]));
    table_list(title, lead, warn, &["Voce", "Persone"], rows)
}
fn table_list(title: &str, lead: &str, warn: &str, headers: &[&str], rows: Vec<Vec<String>>) -> Lista {
    let copy = rows.iter().map(|r| r.join("\t")).collect::<Vec<_>>().join("\n");
    Lista { title: title.into(), lead: lead.into(), warn: warn.into(), headers: headers.iter().map(|s| (*s).to_string()).collect(), rows, copy }
}

async fn page_mail(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let org = match need_user(&state, &headers) { Ok(o) => o, Err(r) => return r };
    let rows = state.db.templates().unwrap_or_default().into_iter().map(|t| MRow { key: t.key, name: t.name, subject: t.subject, on: t.enabled }).collect();
    render(MailListPage { s: shell(&state, &org, "mail", &q), title: "Mail".into(), rows })
}
async fn page_mail_edit(State(state): State<AppState>, headers: HeaderMap, Path(key): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let org = match need_user(&state, &headers) { Ok(o) => o, Err(r) => return r };
    let Some(t) = state.db.template(&key).ok().flatten() else { return StatusCode::NOT_FOUND.into_response() };
    let event = state.db.event().unwrap_or_else(|_| fallback_event());
    let preview = clean_html(&render_vars(&t.html, &sample_vars(&event), true));
    render(MailEditPage { s: shell(&state, &org, "mail", &q), title: t.name.clone(), key: t.key, name: t.name, subject: t.subject, html: t.html, text_body: t.text_body, enabled: t.enabled, preview, vars: "{{nome}} {{cognome}} {{numero}} {{pacchetto}} {{prezzo}} {{acconto}} {{scadenza}} {{iban}} {{intestatario}} {{causale}} {{evento}} {{codice}} {{link}}".into() })
}
#[derive(Deserialize)] struct MailSave { subject: String, #[serde(default)] html: String, #[serde(default)] text_body: String, #[serde(default)] enabled: String }
async fn post_mail_save(State(state): State<AppState>, headers: HeaderMap, Path(key): Path<String>, Form(f): Form<MailSave>) -> Response {
    let path = format!("/admin/mail/{key}");
    let org = match gate(&state, &headers, &path) { Ok(o) => o, Err(r) => return r };
    match state.db.save_template(&key, &f.subject, &clean_html(&f.html), &f.text_body, on(&f.enabled), &org.email, Utc::now()) { Ok(()) => okgo(&path), Err(e) => back(&path, &state.db.code(&e)) }
}
async fn post_mail_test(State(state): State<AppState>, headers: HeaderMap, Path(key): Path<String>) -> Response {
    let path = format!("/admin/mail/{key}");
    let org = match gate(&state, &headers, &path) { Ok(o) => o, Err(r) => return r };
    let Some(t) = state.db.template(&key).ok().flatten() else { return back(&path, "dati"); };
    let vars = sample_vars(&state.db.event().unwrap_or_else(|_| fallback_event()));
    let subject = format!("[Prova] {}", render_vars(&t.subject, &vars, false));
    let res = state.mailer.send(Outgoing { to: org.email.clone(), subject: subject.clone(), html: render_vars(&t.html, &vars, true), text: render_vars(&t.text_body, &vars, false), dev_code: None }).await;
    let ok = res.is_ok();
    let err = res.err().unwrap_or_default();
    let _ = state.db.log_mail(&key, "", &org.email, &subject, ok, &err, &org.email, Utc::now());
    back(&path, if ok { "prova" } else { "mail_ko" })
}
async fn page_registro(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let org = match need_user(&state, &headers) { Ok(o) => o, Err(r) => return r };
    let rows = state.db.registro().unwrap_or_default().into_iter().map(|(at, who, action, _, detail)| Line { a: fmt_rome(&at), b: who, c: format!("{action} · {detail}") }).collect();
    let mails = state.db.mail_log_all().unwrap_or_default().into_iter().map(|(at, key, _, ok, _)| Line { a: fmt_rome(&at), b: key, c: if ok == 1 { "inviata".into() } else { "non partita".into() } }).collect();
    render(RegistroPage { s: shell(&state, &org, "registro", &q), title: "Registro".into(), rows, mails })
}
async fn page_org(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let org = match need_user(&state, &headers) { Ok(o) => o, Err(r) => return r };
    if org.role != "owner" { return back("/admin/iscrizioni", "lettura"); }
    let rows = state.db.organizers().unwrap_or_default().into_iter().map(|o| OrgRow { email: o.email, name: o.name, role_label: role_label(&o.role).into(), role: o.role, notify: o.notify, active: o.active, last: fmt_rome(&o.last_login_at) }).collect();
    render(OrgPage { s: shell(&state, &org, "organizzatori", &q), title: "Organizzatori".into(), rows })
}
#[derive(Deserialize)] struct InviteForm { email: String, #[serde(default)] name: String, role: String, #[serde(default)] notify: String }
async fn post_invite(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<InviteForm>) -> Response {
    let org = match need_user(&state, &headers) { Ok(o) => o, Err(r) => return r };
    if org.role != "owner" || !same_origin(&headers) { return back("/admin/organizzatori", "lettura"); }
    match state.db.invite(&f.email, &f.name, &f.role, on(&f.notify), &org.email, Utc::now()) {
        Ok(()) => { let _ = welcome(&state, &f.email, &f.name, &org.email).await; redir("/admin/organizzatori?ok=invito") }
        Err(e) => back("/admin/organizzatori", &state.db.code(&e)),
    }
}
async fn welcome(state: &AppState, to: &str, name: &str, actor_email: &str) -> bool {
    let Some(tpl) = state.db.template("benvenuto").ok().flatten() else { return false };
    let mut vars = sample_vars(&state.db.event().unwrap_or_else(|_| fallback_event()));
    if let Some(slot) = vars.iter_mut().find(|(k, _)| k == "nome") { slot.1 = if name.is_empty() { to.to_string() } else { name.to_string() }; }
    let subject = render_vars(&tpl.subject, &vars, false);
    let res = state.mailer.send(Outgoing { to: to.into(), subject: subject.clone(), html: render_vars(&tpl.html, &vars, true), text: render_vars(&tpl.text_body, &vars, false), dev_code: None }).await;
    let ok = res.is_ok();
    let err = res.err().unwrap_or_default();
    let _ = state.db.log_mail("benvenuto", "", to, &subject, ok, &err, actor_email, Utc::now());
    ok
}
#[derive(Deserialize)] struct OrgUpdate { email: String, role: String, #[serde(default)] notify: String, #[serde(default)] active: String }
async fn post_org(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<OrgUpdate>) -> Response {
    let org = match need_user(&state, &headers) { Ok(o) => o, Err(r) => return r };
    if org.role != "owner" || !same_origin(&headers) { return back("/admin/organizzatori", "lettura"); }
    match state.db.update_organizer(&f.email, &f.role, on(&f.notify), on(&f.active), &org.email, Utc::now()) { Ok(()) => okgo("/admin/organizzatori"), Err(e) => back("/admin/organizzatori", &state.db.code(&e)) }
}
async fn backup(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let org = match need_user(&state, &headers) { Ok(o) => o, Err(r) => return r };
    if org.role != "owner" { return back("/admin/registro", "lettura"); }
    let _ = state.db.backup_daily(&state.cfg.backup_dir());
    let tmp = std::env::temp_dir().join(format!("iscrizioni-{}.db", Utc::now().timestamp()));
    if let (Ok(src), Ok(mut dst)) = (rusqlite::Connection::open(&state.db.path), rusqlite::Connection::open(&tmp)) {
        if let Ok(b) = rusqlite::backup::Backup::new(&src, &mut dst) { let _ = b.step(-1); }
    }
    match std::fs::read(&tmp) {
        Ok(bytes) => { let _ = std::fs::remove_file(&tmp); ([(header::CONTENT_TYPE, "application/octet-stream"), (header::CONTENT_DISPOSITION, "attachment; filename=\"iscrizioni.db\"")], bytes).into_response() }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
