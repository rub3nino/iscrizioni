use askama::Template;
use axum::response::{IntoResponse, Response};

use crate::domain::explain;

#[derive(Clone)]
pub struct Shell {
    pub event_name: String,
    pub me_name: String,
    pub me_email: String,
    pub role_label: String,
    pub page: String,
    pub count: i64,
    pub flash: String,
    pub flash_err: bool,
    pub can_edit: bool,
    pub is_owner: bool,
    pub mail_on: bool,
    pub saved: bool,
}

pub fn flash_of(code: Option<&String>, ok: bool) -> (String, bool, bool) {
    let Some(code) = code else { return (String::new(), false, false); };
    if code.is_empty() { return (String::new(), false, false); }
    (explain(code).to_string(), !ok, ok)
}

pub fn render<T: Template>(t: T) -> Response {
    match t.render() {
        Ok(body) => (
            [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
            body,
        ).into_response(),
        Err(e) => {
            tracing::error!(%e, "template");
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Pagina non disponibile").into_response()
        }
    }
}

#[derive(Template)]
#[template(path = "home.html")]
pub struct HomePage { pub name: String }

#[derive(Template)]
#[template(path = "login.html")]
pub struct LoginPage { pub step: String, pub email: String, pub error: String, pub name: String }

#[derive(Template)]
#[template(path = "form.html")]
pub struct FormPage {
    pub name: String,
    pub door: String,
    pub closed_message: String,
    pub show_spots: bool,
    pub spots: String,
    pub show_countdown: bool,
    pub countdown_label: String,
    pub countdown_iso: String,
    pub notices: Vec<Note>,
    pub blocks_before: Vec<Blk>,
    pub blocks_mid: Vec<Blk>,
    pub blocks_end: Vec<Blk>,
    pub packages: Vec<PkgChoice>,
    pub fields: Vec<Fld>,
    pub consents: Vec<Cns>,
    pub minor_age: i64,
    pub error: String,
    pub frame: String,
}

#[derive(Clone)]
pub struct Note { pub title: String, pub body: String, pub tone: String }
#[derive(Clone)]
pub struct Blk { pub title: String, pub body: String }
#[derive(Clone)]
pub struct PkgChoice { pub id: String, pub name: String, pub price: String, pub description: String, pub enabled: bool, pub label: String }
#[derive(Clone)]
pub struct Fld { pub key: String, pub label: String, pub required: bool }
#[derive(Clone)]
pub struct Cns { pub id: String, pub label: String, pub required: bool }

#[derive(Template)]
#[template(path = "grazie.html")]
pub struct GraziePage { pub name: String, pub number: String, pub wait: bool, pub mail_off: bool }

#[derive(Template)]
#[template(path = "iscrizioni.html")]
pub struct IscrizioniPage {
    pub s: Shell,
    pub title: String,
    pub da_fare: String,
    pub acconto: String,
    pub posti: String,
    pub cap: String,
    pub da_incassare: String,
    pub q: String,
    pub vista: String,
    pub tabs: Vec<Tab>,
    pub rows: Vec<RegRow>,
    pub open: bool,
    pub d: Detail,
}

#[derive(Clone)]
pub struct Tab { pub id: String, pub label: String, pub n: i64, pub on: bool }
#[derive(Clone)]
pub struct RegRow {
    pub href: String, pub number: String, pub name: String, pub email: String,
    pub package: String, pub status: String, pub chip: String, pub mark: String, pub paid: String,
}
#[derive(Clone, Default)]
pub struct Detail {
    pub id: String, pub number: String, pub name: String, pub email: String, pub phone: String,
    pub birth: String, pub package: String, pub status: String, pub chip: String, pub mark: String,
    pub paid: String, pub price: String, pub rest: String, pub due: String,
    pub gender: String, pub city: String, pub shirt: String, pub notes: String, pub allergies: String, pub transport: String,
    pub parent_name: String, pub parent_phone: String, pub parent_ok: bool,
    pub first: String, pub last: String, pub birth_raw: String, pub phone_raw: String, pub package_id: String,
    pub can_mail_pay: bool, pub can_pay: bool, pub can_cancel: bool, pub can_restore: bool, pub can_release: bool, pub can_delete: bool,
    pub hist: Vec<Line>, pub pays: Vec<Line>, pub notes_i: Vec<Line>, pub mails: Vec<Line>,
    pub pkgs: Vec<Opt>,
}
#[derive(Clone, Default)]
pub struct Line { pub a: String, pub b: String, pub c: String }
#[derive(Clone)]
pub struct Opt { pub id: String, pub name: String, pub on: bool }

#[derive(Template)]
#[template(path = "sito.html")]
pub struct SitoPage {
    pub s: Shell, pub title: String, pub door: String, pub closed_message: String,
    pub countdown: String, pub show_countdown: bool, pub show_spots: bool, pub origins: String,
    pub notices: Vec<NEdit>, pub blocks: Vec<BEdit>, pub fields: Vec<FEdit>, pub consents: Vec<CEdit>,
}
#[derive(Clone)]
pub struct NEdit { pub id: String, pub title: String, pub body: String, pub tone: String, pub visible: bool, pub starts: String, pub ends: String }
#[derive(Clone)]
pub struct BEdit { pub id: String, pub title: String, pub body: String, pub placement: String, pub visible: bool }
#[derive(Clone)]
pub struct FEdit { pub key: String, pub label: String, pub enabled: bool, pub required: bool }
#[derive(Clone)]
pub struct CEdit { pub id: String, pub label: String, pub required: bool }

#[derive(Template)]
#[template(path = "offerta.html")]
pub struct OffertaPage {
    pub s: Shell, pub title: String,
    pub name: String, pub place: String, pub starts_on: String, pub ends_on: String, pub currency: String,
    pub capacity: i64, pub deposit: String, pub hours: i64, pub min_age: i64, pub max_age: i64, pub minor_age: i64,
    pub iban: String, pub holder: String, pub pay_note: String,
    pub windows: Vec<WRow>, pub packages: Vec<PRow>, pub window_opts: Vec<Opt>,
}
#[derive(Clone)]
pub struct WRow { pub id: String, pub name: String, pub start: String, pub end: String, pub closed: bool }
#[derive(Clone)]
pub struct PRow { pub id: String, pub name: String, pub price: String, pub cap: String, pub window: String, pub active: bool, pub description: String, pub label: String, pub price_cents: i64, pub cap_raw: String, pub window_id: String, pub sort: i64 }

#[derive(Template)]
#[template(path = "liste.html")]
pub struct ListePage { pub s: Shell, pub title: String, pub lists: Vec<Lista> }
#[derive(Clone)]
pub struct Lista { pub title: String, pub lead: String, pub warn: String, pub headers: Vec<String>, pub rows: Vec<Vec<String>>, pub copy: String }

#[derive(Template)]
#[template(path = "mail.html")]
pub struct MailListPage { pub s: Shell, pub title: String, pub rows: Vec<MRow> }
#[derive(Clone)]
pub struct MRow { pub key: String, pub name: String, pub subject: String, pub on: bool }

#[derive(Template)]
#[template(path = "mail_edit.html")]
pub struct MailEditPage {
    pub s: Shell, pub title: String, pub key: String, pub name: String,
    pub subject: String, pub html: String, pub text_body: String, pub enabled: bool,
    pub preview: String, pub vars: String,
}

#[derive(Template)]
#[template(path = "registro.html")]
pub struct RegistroPage { pub s: Shell, pub title: String, pub rows: Vec<Line>, pub mails: Vec<Line> }

#[derive(Template)]
#[template(path = "organizzatori.html")]
pub struct OrgPage { pub s: Shell, pub title: String, pub rows: Vec<OrgRow> }
#[derive(Clone)]
pub struct OrgRow { pub email: String, pub name: String, pub role: String, pub role_label: String, pub notify: bool, pub active: bool, pub last: String }
