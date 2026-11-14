use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use uuid::Uuid;

use crate::domain::{
    age_years, clip, decide_lane, parse_ts, stamp, valid_email, valid_name, valid_phone, Lane,
};

#[derive(Debug, thiserror::Error)]
pub enum Fail {
    #[error("{0}")]
    Msg(String),
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
}

pub fn user_code(e: &Fail) -> String {
    match e {
        Fail::Msg(s) => s.clone(),
        Fail::Sql(err) => {
            tracing::error!(%err, "sql");
            "errore".into()
        }
    }
}

fn msg(code: &str) -> Fail {
    Fail::Msg(code.into())
}

fn nid() -> String {
    Uuid::new_v4().to_string()
}

#[derive(Clone)]
pub struct Db {
    conn: std::sync::Arc<std::sync::Mutex<Connection>>,
    pub path: std::path::PathBuf,
}

impl Db {
    pub fn open(path: &std::path::Path) -> Result<Self, Fail> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(path)?;
        tune(&conn)?;
        Ok(Self {
            conn: std::sync::Arc::new(std::sync::Mutex::new(conn)),
            path: path.to_path_buf(),
        })
    }

    pub(crate) fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn migrate(&self) -> Result<(), Fail> {
        migrate_on(&self.lock())
    }

    pub fn seed(&self, owner: &str, now: DateTime<Utc>) -> Result<(), Fail> {
        seed_on(&self.lock(), owner, now)
    }
}

pub fn tune(c: &Connection) -> Result<(), Fail> {
    c.pragma_update(None, "journal_mode", "WAL")?;
    c.pragma_update(None, "foreign_keys", "ON")?;
    c.busy_timeout(Duration::from_secs(5))?;
    Ok(())
}

pub fn migrate_on(c: &Connection) -> Result<(), Fail> {
    c.execute_batch(include_str!("../migrations/001_init.sql"))?;
    Ok(())
}

pub fn seed_on(c: &Connection, owner: &str, now: DateTime<Utc>) -> Result<(), Fail> {
    let n: i64 = c.query_row("SELECT COUNT(*) FROM event", [], |r| r.get(0))?;
    if n == 0 {
        c.execute(
            "INSERT INTO event (id, name, place, starts_on, ends_on, currency, capacity, deposit_cents, deposit_hours, min_age, max_age, minor_age, door, closed_message, countdown_at, show_countdown, show_spots, iban, account_holder, payment_note, allowed_origins, updated_at)
             VALUES (1, 'Iscrizioni', '', '', '', 'EUR', 0, 0, 48, 16, 99, 18, 'pause', 'Le iscrizioni sono chiuse.', '', 0, 1, '', '', '', '', ?1)",
            params![stamp(now)],
        )?;
    }
    let owners: i64 = c.query_row("SELECT COUNT(*) FROM organizers", [], |r| r.get(0))?;
    if owners == 0 {
        c.execute(
            "INSERT INTO organizers (email, name, role, notify, active, created_at, last_login_at) VALUES (?1, '', 'owner', 1, 1, ?2, '')",
            params![owner.trim().to_lowercase(), stamp(now)],
        )?;
    }
    let counter: i64 = c.query_row(
        "SELECT COUNT(*) FROM counters WHERE name='registration'",
        [],
        |r| r.get(0),
    )?;
    if counter == 0 {
        c.execute("INSERT INTO counters (name, value) VALUES ('registration', 0)", [])?;
    }
    let consents: i64 = c.query_row("SELECT COUNT(*) FROM consents", [], |r| r.get(0))?;
    if consents == 0 {
        c.execute(
            "INSERT INTO consents (id, label, required, sort) VALUES (?1, ?2, 1, 0)",
            params![nid(), "Acconsento al trattamento dei dati per gestire l'iscrizione."],
        )?;
    }
    for (i, (key, label)) in [
        ("gender", "Genere"),
        ("city", "Città"),
        ("shirt", "Taglia"),
        ("notes", "Note"),
        ("allergies", "Allergie o esigenze"),
        ("transport", "Trasporto"),
    ]
    .iter()
    .enumerate()
    {
        c.execute(
            "INSERT INTO form_fields (key, label, enabled, required, sort) VALUES (?1, ?2, 0, 0, ?3) ON CONFLICT(key) DO NOTHING",
            params![key, label, i as i64],
        )?;
    }
    for t in default_templates() {
        c.execute(
            "INSERT INTO mail_templates (key, name, subject, html, text_body, enabled, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6) ON CONFLICT(key) DO NOTHING",
            params![t.0, t.1, t.2, t.3, t.4, stamp(now)],
        )?;
    }
    Ok(())
}

fn mail_shell(inner: &str) -> String {
    format!(
        "<!doctype html><html><body style=\"margin:0;background:#fdfdfb;color:#25231f;font-family:Segoe UI,Helvetica,Arial,sans-serif;\"><table role=\"presentation\" width=\"100%\" cellpadding=\"0\" cellspacing=\"0\"><tr><td align=\"center\" style=\"padding:24px;\"><table role=\"presentation\" width=\"560\" cellpadding=\"0\" cellspacing=\"0\" style=\"max-width:560px;border:1px solid #e3e2de;\"><tr><td style=\"padding:24px;font-size:16px;line-height:1.5;\">{inner}<p style=\"margin:24px 0 0;font-size:13px;color:#6f6d68;\">{{{{evento}}}}</p></td></tr></table></td></tr></table></body></html>"
    )
}

fn default_templates() -> Vec<(&'static str, &'static str, String, String, String)> {
    let mut out = Vec::new();
    let specs = [
        ("iscrizione_ricevuta", "Iscrizione ricevuta", "Abbiamo ricevuto la tua iscrizione, {{nome}}. Il numero è {{numero}}.", "Iscrizione ricevuta, numero {{numero}}"),
        ("lista_attesa", "Sei in lista d'attesa", "{{nome}}, la tua iscrizione numero {{numero}} è in lista d'attesa. Ti scriviamo se si libera un posto.", "Lista d'attesa, numero {{numero}}"),
        ("dati_pagamento", "Dati per l'acconto", "{{nome}}, per il posto {{numero}} versa {{acconto}} entro {{scadenza}}. IBAN {{iban}}, intestato a {{intestatario}}. Causale: {{causale}}.", "Acconto {{acconto}} entro {{scadenza}}"),
        ("posto_assicurato", "Posto assicurato", "{{nome}}, l'acconto del numero {{numero}} è registrato. Il posto è assicurato.", "Posto assicurato, numero {{numero}}"),
        ("termine_scaduto", "Termine scaduto", "{{nome}}, il termine per l'acconto del numero {{numero}} è passato e il posto è stato liberato.", "Termine scaduto, numero {{numero}}"),
        ("saldato", "Saldo registrato", "{{nome}}, il numero {{numero}} risulta saldato. Non resta nulla da pagare.", "Saldo registrato, numero {{numero}}"),
        ("annullata", "Iscrizione annullata", "{{nome}}, l'iscrizione numero {{numero}} è annullata.", "Iscrizione {{numero}} annullata"),
        ("codice_accesso", "Codice di accesso", "Il codice per entrare nel pannello è {{codice}}. Scade tra 10 minuti e si usa una volta sola.", "Codice {{codice}}"),
        ("benvenuto", "Accesso al pannello", "Entra nel pannello con questo indirizzo: a ogni accesso ricevi un codice. {{link}}", "Accesso al pannello"),
        ("avviso_organizzatore", "Nuova iscrizione", "Nuova iscrizione numero {{numero}}: {{nome}} {{cognome}}, pacchetto {{pacchetto}}.", "Nuova iscrizione {{numero}}"),
    ];
    for (key, name, body, subject) in specs {
        let html = mail_shell(&format!(
            "<h1 style=\"margin:0 0 12px;font-size:22px;\">{name}</h1><p style=\"margin:0 0 12px;\">{body}</p>"
        ));
        let text = format!("{name}\n\n{body}\n\n{{{{evento}}}}");
        out.push((key, name, subject.to_string(), html, text));
    }
    out
}

#[derive(Clone, Debug)]
pub struct Event {
    pub name: String,
    pub place: String,
    pub starts_on: String,
    pub ends_on: String,
    pub currency: String,
    pub capacity: i64,
    pub deposit_cents: i64,
    pub deposit_hours: i64,
    pub min_age: i64,
    pub max_age: i64,
    pub minor_age: i64,
    pub door: String,
    pub closed_message: String,
    pub countdown_at: String,
    pub show_countdown: bool,
    pub show_spots: bool,
    pub iban: String,
    pub account_holder: String,
    pub payment_note: String,
    pub allowed_origins: String,
}

fn map_event(r: &rusqlite::Row<'_>) -> rusqlite::Result<Event> {
    Ok(Event {
        name: r.get(0)?,
        place: r.get(1)?,
        starts_on: r.get(2)?,
        ends_on: r.get(3)?,
        currency: r.get(4)?,
        capacity: r.get(5)?,
        deposit_cents: r.get(6)?,
        deposit_hours: r.get(7)?,
        min_age: r.get(8)?,
        max_age: r.get(9)?,
        minor_age: r.get(10)?,
        door: r.get(11)?,
        closed_message: r.get(12)?,
        countdown_at: r.get(13)?,
        show_countdown: r.get::<_, i64>(14)? != 0,
        show_spots: r.get::<_, i64>(15)? != 0,
        iban: r.get(16)?,
        account_holder: r.get(17)?,
        payment_note: r.get(18)?,
        allowed_origins: r.get(19)?,
    })
}

const EVENT_COLS: &str = "name, place, starts_on, ends_on, currency, capacity, deposit_cents, deposit_hours, min_age, max_age, minor_age, door, closed_message, countdown_at, show_countdown, show_spots, iban, account_holder, payment_note, allowed_origins";

pub fn load_event(c: &Connection) -> Result<Event, Fail> {
    Ok(c.query_row(&format!("SELECT {EVENT_COLS} FROM event WHERE id=1"), [], map_event)?)
}

#[derive(Clone, Debug)]
pub struct Window {
    pub id: String,
    pub name: String,
    pub starts_at: String,
    pub ends_at: String,
    pub manually_closed: bool,
    pub sort: i64,
}

#[derive(Clone, Debug)]
pub struct Package {
    pub id: String,
    pub name: String,
    pub price_cents: i64,
    pub capacity: Option<i64>,
    pub window_id: Option<String>,
    pub description: String,
    pub unavailable_label: String,
    pub sort: i64,
    pub active: bool,
}

#[derive(Clone, Debug)]
pub struct Field {
    pub key: String,
    pub label: String,
    pub enabled: bool,
    pub required: bool,
    pub sort: i64,
}

#[derive(Clone, Debug)]
pub struct Consent {
    pub id: String,
    pub label: String,
    pub required: bool,
    pub sort: i64,
}

#[derive(Clone, Debug)]
pub struct Notice {
    pub id: String,
    pub title: String,
    pub body: String,
    pub tone: String,
    pub visible: bool,
    pub starts_at: String,
    pub ends_at: String,
    pub sort: i64,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub id: String,
    pub title: String,
    pub body: String,
    pub placement: String,
    pub visible: bool,
    pub sort: i64,
}

#[derive(Clone, Debug)]
pub struct Reg {
    pub id: String,
    pub number: i64,
    pub status: String,
    pub first_name: String,
    pub last_name: String,
    pub birth_date: String,
    pub email: String,
    pub phone: String,
    pub package_id: String,
    pub gender: String,
    pub city: String,
    pub shirt: String,
    pub notes: String,
    pub allergies: String,
    pub transport: String,
    pub parent_name: String,
    pub parent_phone: String,
    pub parent_consent: bool,
    pub consents: String,
    pub deposit_due_at: String,
    pub cancel_reason: String,
    pub created_at: String,
    pub updated_at: String,
}

fn map_reg(r: &rusqlite::Row<'_>) -> rusqlite::Result<Reg> {
    Ok(Reg {
        id: r.get(0)?,
        number: r.get(1)?,
        status: r.get(2)?,
        first_name: r.get(3)?,
        last_name: r.get(4)?,
        birth_date: r.get(5)?,
        email: r.get(6)?,
        phone: r.get(7)?,
        package_id: r.get(8)?,
        gender: r.get(9)?,
        city: r.get(10)?,
        shirt: r.get(11)?,
        notes: r.get(12)?,
        allergies: r.get(13)?,
        transport: r.get(14)?,
        parent_name: r.get(15)?,
        parent_phone: r.get(16)?,
        parent_consent: r.get::<_, i64>(17)? != 0,
        consents: r.get(18)?,
        deposit_due_at: r.get(19)?,
        cancel_reason: r.get(20)?,
        created_at: r.get(21)?,
        updated_at: r.get(22)?,
    })
}

const REG_COLS: &str = "id, number, status, first_name, last_name, birth_date, email, phone, package_id, gender, city, shirt, notes, allergies, transport, parent_name, parent_phone, parent_consent, consents, deposit_due_at, cancel_reason, created_at, updated_at";

#[derive(Clone, Debug)]
pub struct Organizer {
    pub email: String,
    pub name: String,
    pub role: String,
    pub notify: bool,
    pub active: bool,
    pub created_at: String,
    pub last_login_at: String,
}

#[derive(Clone, Debug)]
pub struct Template {
    pub key: String,
    pub name: String,
    pub subject: String,
    pub html: String,
    pub text_body: String,
    pub enabled: bool,
    pub updated_at: String,
}

#[derive(Clone, Debug)]
pub struct Signup {
    pub first_name: String,
    pub last_name: String,
    pub birth_date: String,
    pub email: String,
    pub phone: String,
    pub package_id: String,
    pub gender: String,
    pub city: String,
    pub shirt: String,
    pub notes: String,
    pub allergies: String,
    pub transport: String,
    pub parent_name: String,
    pub parent_phone: String,
    pub parent_consent: bool,
    pub consent_ids: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Created {
    pub id: String,
    pub number: i64,
    pub status: String,
    pub email: String,
}

pub fn signup_on(c: &mut Connection, s: &Signup, now: DateTime<Utc>) -> Result<Created, Fail> {
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let event = load_event(&tx)?;
    if event.door != "open" {
        return Err(msg("chiuso"));
    }
    let pkg = load_package(&tx, &s.package_id)?.ok_or_else(|| msg("pacchetto_no"))?;
    if !pkg.active || !package_window_open(&tx, &pkg, now)? {
        return Err(msg("pacchetto_no"));
    }
    let birth = NaiveDate::parse_from_str(s.birth_date.trim(), "%Y-%m-%d").map_err(|_| msg("dati"))?;
    let on = NaiveDate::parse_from_str(&event.starts_on, "%Y-%m-%d").unwrap_or_else(|_| now.date_naive());
    let age = age_years(birth, on);
    if age < event.min_age as i32 || age > event.max_age as i32 {
        return Err(msg("eta"));
    }
    if !valid_name(&s.first_name) || !valid_name(&s.last_name) || !valid_email(&s.email) || !valid_phone(&s.phone) {
        return Err(msg("dati"));
    }
    let email = s.email.trim().to_lowercase();
    let dup: i64 = tx.query_row(
        "SELECT COUNT(*) FROM registrations WHERE email=?1 AND birth_date=?2 AND status!='annullata'",
        params![email, s.birth_date.trim()],
        |r| r.get(0),
    )?;
    if dup > 0 {
        return Err(msg("duplicato"));
    }
    if age < event.minor_age as i32 && (!valid_name(&s.parent_name) || !valid_phone(&s.parent_phone) || !s.parent_consent) {
        return Err(msg("genitore"));
    }
    let required: Vec<String> = {
        let mut st = tx.prepare("SELECT id FROM consents WHERE required=1")?;
        let rows = st.query_map([], |r| r.get(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    if required.iter().any(|id| !s.consent_ids.iter().any(|c| c == id)) {
        return Err(msg("consenso"));
    }
    let fields = load_fields(&tx)?;
    check_optional(&fields, s)?;
    let (taken, pkg_taken, waiting) = counts(&tx, &pkg.id)?;
    let lane = decide_lane(event.capacity, taken, pkg.capacity, pkg_taken, waiting);
    let status = if lane == Lane::Waitlist { "attesa" } else { "ricevuta" };
    let number: i64 = tx.query_row(
        "UPDATE counters SET value = value + 1 WHERE name='registration' RETURNING value",
        [],
        |r| r.get(0),
    )?;
    let id = nid();
    let when = stamp(now);
    let consents = serde_json::to_string(&s.consent_ids).unwrap_or_else(|_| "[]".into());
    tx.execute(
        "INSERT INTO registrations (id, number, status, first_name, last_name, birth_date, email, phone, package_id, gender, city, shirt, notes, allergies, transport, parent_name, parent_phone, parent_consent, consents, deposit_due_at, cancel_reason, created_at, updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,'','',?20,?20)",
        params![
            id, number, status,
            clip(&s.first_name, 80), clip(&s.last_name, 80), s.birth_date.trim(), email, clip(&s.phone, 30),
            pkg.id, clip(&s.gender, 16), clip(&s.city, 80), clip(&s.shirt, 16),
            clip(&s.notes, 1000), clip(&s.allergies, 1000), clip(&s.transport, 120),
            clip(&s.parent_name, 80), clip(&s.parent_phone, 30), s.parent_consent as i64, consents, when
        ],
    )?;
    event_row(&tx, &id, "sistema", "creata", &format!("Iscrizione {number}"), &when)?;
    audit(&tx, "sistema", "iscrizione.creata", &id, &format!("numero {number} stato {status}"), &when)?;
    tx.commit()?;
    tracing::info!(id = %id, number, status, "iscrizione creata");
    Ok(Created { id, number, status: status.into(), email })
}

fn check_optional(fields: &[Field], s: &Signup) -> Result<(), Fail> {
    let need = |key: &str, value: &str| -> Result<(), Fail> {
        if fields.iter().any(|f| f.key == key && f.enabled && f.required) && value.trim().is_empty() {
            return Err(msg("dati"));
        }
        Ok(())
    };
    need("gender", &s.gender)?;
    need("city", &s.city)?;
    need("shirt", &s.shirt)?;
    need("notes", &s.notes)?;
    need("allergies", &s.allergies)?;
    need("transport", &s.transport)?;
    Ok(())
}

pub fn counts(c: &Connection, package_id: &str) -> Result<(i64, i64, i64), Fail> {
    let taken: i64 = c.query_row(
        "SELECT COUNT(*) FROM registrations WHERE status NOT IN ('annullata','attesa')",
        [],
        |r| r.get(0),
    )?;
    let pkg: i64 = c.query_row(
        "SELECT COUNT(*) FROM registrations WHERE status NOT IN ('annullata','attesa') AND package_id=?1",
        params![package_id],
        |r| r.get(0),
    )?;
    let waiting: i64 = c.query_row(
        "SELECT COUNT(*) FROM registrations WHERE status='attesa'",
        [],
        |r| r.get(0),
    )?;
    Ok((taken, pkg, waiting))
}

fn package_window_open(c: &Connection, pkg: &Package, now: DateTime<Utc>) -> Result<bool, Fail> {
    let Some(wid) = &pkg.window_id else {
        return Ok(true);
    };
    let Some(w) = load_window(c, wid)? else {
        return Ok(false);
    };
    if w.manually_closed {
        return Ok(false);
    }
    let (Some(start), Some(end)) = (parse_ts(&w.starts_at), parse_ts(&w.ends_at)) else {
        return Ok(false);
    };
    Ok(now >= start && now < end)
}

pub fn event_row(c: &Connection, id: &str, actor: &str, kind: &str, detail: &str, when: &str) -> Result<(), Fail> {
    c.execute(
        "INSERT INTO reg_events (id, registration_id, at, actor, kind, detail) VALUES (?1,?2,?3,?4,?5,?6)",
        params![nid(), id, when, actor, kind, detail],
    )?;
    Ok(())
}

pub fn audit(c: &Connection, actor: &str, action: &str, registration_id: &str, detail: &str, when: &str) -> Result<(), Fail> {
    c.execute(
        "INSERT INTO audit (id, at, actor, action, registration_id, detail) VALUES (?1,?2,?3,?4,?5,?6)",
        params![nid(), when, actor, action, registration_id, detail],
    )?;
    Ok(())
}

pub fn load_package(c: &Connection, id: &str) -> Result<Option<Package>, Fail> {
    Ok(c.query_row(
        "SELECT id, name, price_cents, capacity, window_id, description, unavailable_label, sort, active FROM packages WHERE id=?1",
        params![id],
        map_package,
    ).optional()?)
}

fn map_package(r: &rusqlite::Row<'_>) -> rusqlite::Result<Package> {
    Ok(Package {
        id: r.get(0)?,
        name: r.get(1)?,
        price_cents: r.get(2)?,
        capacity: r.get(3)?,
        window_id: r.get(4)?,
        description: r.get(5)?,
        unavailable_label: r.get(6)?,
        sort: r.get(7)?,
        active: r.get::<_, i64>(8)? != 0,
    })
}

pub fn load_window(c: &Connection, id: &str) -> Result<Option<Window>, Fail> {
    Ok(c.query_row(
        "SELECT id, name, starts_at, ends_at, manually_closed, sort FROM windows WHERE id=?1",
        params![id],
        |r| {
            Ok(Window {
                id: r.get(0)?,
                name: r.get(1)?,
                starts_at: r.get(2)?,
                ends_at: r.get(3)?,
                manually_closed: r.get::<_, i64>(4)? != 0,
                sort: r.get(5)?,
            })
        },
    ).optional()?)
}

pub fn load_fields(c: &Connection) -> Result<Vec<Field>, Fail> {
    let mut s = c.prepare("SELECT key, label, enabled, required, sort FROM form_fields ORDER BY sort, key")?;
    let rows = s.query_map([], |r| {
        Ok(Field {
            key: r.get(0)?,
            label: r.get(1)?,
            enabled: r.get::<_, i64>(2)? != 0,
            required: r.get::<_, i64>(3)? != 0,
            sort: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

impl Db {
    pub fn event(&self) -> Result<Event, Fail> {
        load_event(&self.lock())
    }
    pub fn signup(&self, s: &Signup, now: DateTime<Utc>) -> Result<Created, Fail> {
        let mut c = self.lock();
        signup_on(&mut c, s, now)
    }

    pub fn code(&self, e: &Fail) -> String {
        user_code(e)
    }
    pub fn reg(&self, id: &str) -> Result<Option<Reg>, Fail> {
        Ok(self.lock().query_row(&format!("SELECT {REG_COLS} FROM registrations WHERE id=?1"), params![id], map_reg).optional()?)
    }
    pub fn regs(&self) -> Result<Vec<Reg>, Fail> {
        let c = self.lock();
        let mut s = c.prepare(&format!("SELECT {REG_COLS} FROM registrations ORDER BY number DESC"))?;
        let rows = s.query_map([], map_reg)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
    pub fn packages(&self) -> Result<Vec<Package>, Fail> {
        let c = self.lock();
        let mut s = c.prepare("SELECT id, name, price_cents, capacity, window_id, description, unavailable_label, sort, active FROM packages ORDER BY sort, name")?;
        let rows = s.query_map([], map_package)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
    pub fn windows(&self) -> Result<Vec<Window>, Fail> {
        let c = self.lock();
        let mut s = c.prepare("SELECT id, name, starts_at, ends_at, manually_closed, sort FROM windows ORDER BY sort, starts_at")?;
        let rows = s.query_map([], |r| {
            Ok(Window {
                id: r.get(0)?, name: r.get(1)?, starts_at: r.get(2)?, ends_at: r.get(3)?,
                manually_closed: r.get::<_, i64>(4)? != 0, sort: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
    pub fn fields(&self) -> Result<Vec<Field>, Fail> { load_fields(&self.lock()) }
    pub fn consents(&self) -> Result<Vec<Consent>, Fail> {
        let c = self.lock();
        let mut s = c.prepare("SELECT id, label, required, sort FROM consents ORDER BY sort")?;
        let rows = s.query_map([], |r| Ok(Consent { id: r.get(0)?, label: r.get(1)?, required: r.get::<_, i64>(2)? != 0, sort: r.get(3)? }))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
    pub fn notices(&self) -> Result<Vec<Notice>, Fail> {
        let c = self.lock();
        let mut s = c.prepare("SELECT id, title, body, tone, visible, starts_at, ends_at, sort FROM notices ORDER BY sort, title")?;
        let rows = s.query_map([], |r| Ok(Notice {
            id: r.get(0)?, title: r.get(1)?, body: r.get(2)?, tone: r.get(3)?,
            visible: r.get::<_, i64>(4)? != 0, starts_at: r.get(5)?, ends_at: r.get(6)?, sort: r.get(7)?,
        }))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
    pub fn blocks(&self) -> Result<Vec<Block>, Fail> {
        let c = self.lock();
        let mut s = c.prepare("SELECT id, title, body, placement, visible, sort FROM blocks ORDER BY sort")?;
        let rows = s.query_map([], |r| Ok(Block {
            id: r.get(0)?, title: r.get(1)?, body: r.get(2)?, placement: r.get(3)?,
            visible: r.get::<_, i64>(4)? != 0, sort: r.get(5)?,
        }))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
    pub fn paid_of(&self, id: &str) -> Result<i64, Fail> {
        Ok(self.lock().query_row("SELECT COALESCE(SUM(amount_cents),0) FROM payments WHERE registration_id=?1", params![id], |r| r.get(0))?)
    }
    pub fn occupancy(&self) -> Result<(i64, i64), Fail> {
        let c = self.lock();
        let taken: i64 = c.query_row("SELECT COUNT(*) FROM registrations WHERE status NOT IN ('annullata','attesa')", [], |r| r.get(0))?;
        let waiting: i64 = c.query_row("SELECT COUNT(*) FROM registrations WHERE status='attesa'", [], |r| r.get(0))?;
        Ok((taken, waiting))
    }
    pub fn active_count(&self) -> Result<i64, Fail> {
        Ok(self.lock().query_row("SELECT COUNT(*) FROM registrations WHERE status!='annullata'", [], |r| r.get(0))?)
    }
}
