use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, TimeZone, Utc};

/// Posto in pianta, oppure lista d'attesa.
/// Se c'è già qualcuno in lista, un posto liberato resta riservato a quella lista.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    Seat,
    Waitlist,
}

pub fn decide_lane(
    event_cap: i64,
    event_taken: i64,
    package_cap: Option<i64>,
    package_taken: i64,
    waitlist: i64,
) -> Lane {
    let event_full = event_taken >= event_cap;
    let package_full = package_cap.map(|c| package_taken >= c).unwrap_or(false);
    if event_full || package_full || waitlist > 0 {
        Lane::Waitlist
    } else {
        Lane::Seat
    }
}

pub fn spots_left(event_cap: i64, event_taken: i64, waitlist: i64) -> i64 {
    if waitlist > 0 {
        return 0;
    }
    (event_cap - event_taken).max(0)
}

/// Dopo un incasso. `attesa` e `annullata` non si muovono da qui.
pub fn status_after_money(current: &str, paid: i64, price: i64, deposit: i64) -> String {
    if current == "annullata" || current == "attesa" {
        return current.to_string();
    }
    if price > 0 && paid >= price {
        return "saldato".into();
    }
    if price == 0 && paid > 0 {
        return "saldato".into();
    }
    if paid > 0 && ((deposit > 0 && paid >= deposit) || (deposit == 0 && price > 0 && paid < price))
    {
        return "posto".into();
    }
    current.to_string()
}

pub fn can_release(status: &str, due: &str, paid: i64, now: DateTime<Utc>) -> bool {
    if status != "attende_acconto" || paid != 0 || due.is_empty() {
        return false;
    }
    parse_ts(due).map(|d| d <= now).unwrap_or(false)
}

pub fn status_label(s: &str) -> &'static str {
    match s {
        "ricevuta" => "Ricevuta",
        "attesa" => "In lista d'attesa",
        "attende_acconto" => "Attende l'acconto",
        "posto" => "Posto assicurato",
        "saldato" => "Saldato",
        "annullata" => "Annullata",
        _ => "—",
    }
}

pub fn status_chip(s: &str) -> &'static str {
    match s {
        "attesa" => "chip chip--attention",
        "attende_acconto" => "chip chip--accent",
        "posto" | "saldato" => "chip chip--success",
        "annullata" => "chip chip--danger",
        _ => "chip",
    }
}

pub fn status_mark(s: &str) -> &'static str {
    match s {
        "attesa" => "mark mark--wait",
        "attende_acconto" => "mark mark--half",
        "posto" | "saldato" => "mark mark--full",
        "annullata" => "mark mark--stop",
        _ => "mark",
    }
}

pub fn role_label(s: &str) -> &'static str {
    match s {
        "owner" => "Proprietario",
        "admin" => "Organizzatore",
        "viewer" => "Sola lettura",
        _ => "—",
    }
}

pub fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    if s.is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

pub fn stamp(now: DateTime<Utc>) -> String {
    now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn fmt_rome(s: &str) -> String {
    let Some(dt) = parse_ts(s) else {
        return "—".into();
    };
    dt.with_timezone(&chrono_tz::Europe::Rome)
        .format("%d/%m/%Y %H:%M")
        .to_string()
}

pub fn fmt_rome_date(s: &str) -> String {
    if s.len() == 10 {
        if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
            return d.format("%d/%m/%Y").to_string();
        }
    }
    let Some(dt) = parse_ts(s) else {
        return if s.is_empty() { "—".into() } else { s.to_string() };
    };
    dt.with_timezone(&chrono_tz::Europe::Rome)
        .format("%d/%m/%Y")
        .to_string()
}

pub fn to_local_input(s: &str) -> String {
    let Some(dt) = parse_ts(s) else {
        return String::new();
    };
    dt.with_timezone(&chrono_tz::Europe::Rome)
        .format("%Y-%m-%dT%H:%M")
        .to_string()
}

pub fn parse_local_dt(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let naive = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M")
        .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S"))
        .ok()?;
    chrono_tz::Europe::Rome
        .from_local_datetime(&naive)
        .single()
        .map(|d| d.with_timezone(&Utc))
}

pub fn age_years(birth: NaiveDate, on: NaiveDate) -> i32 {
    let mut age = on.year() - birth.year();
    if (on.month(), on.day()) < (birth.month(), birth.day()) {
        age -= 1;
    }
    age
}

pub fn format_cents(cents: i64) -> String {
    let neg = if cents < 0 { "-" } else { "" };
    let v = cents.unsigned_abs();
    let whole = v / 100;
    let frac = v % 100;
    format!("{neg}{},{frac:02} €", group_thousands(whole))
}

fn group_thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push('.');
        }
        out.push(c);
    }
    out.chars().rev().collect()
}

pub fn clean_html(input: &str) -> String {
    ammonia::Builder::default()
        .add_generic_attributes([
            "style",
            "bgcolor",
            "align",
            "width",
            "height",
            "cellpadding",
            "cellspacing",
            "border",
            "role",
        ])
        .clean(input)
        .to_string()
}

pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn render_vars(src: &str, vars: &[(String, String)], escape_html: bool) -> String {
    let mut out = src.to_string();
    for (k, v) in vars {
        let shown = if escape_html { html_escape(v) } else { v.clone() };
        out = out.replace(&format!("{{{{{k}}}}}"), &shown);
    }
    out
}

pub fn valid_email(s: &str) -> bool {
    let s = s.trim();
    if s.len() < 6 || s.len() > 120 || s.contains(' ') {
        return false;
    }
    let Some((name, host)) = s.split_once('@') else {
        return false;
    };
    !name.is_empty() && host.contains('.') && !host.starts_with('.') && !host.ends_with('.')
}

pub fn valid_name(s: &str) -> bool {
    let t = s.trim();
    let n = t.chars().count();
    (1..=80).contains(&n)
        && t.chars()
            .all(|c| c.is_alphabetic() || matches!(c, ' ' | '\'' | '’' | '-' | '.'))
}

pub fn digit_count(s: &str) -> usize {
    s.chars().filter(|c| c.is_ascii_digit()).count()
}

pub fn valid_phone(s: &str) -> bool {
    let n = digit_count(s);
    (6..=20).contains(&n) && s.trim().chars().count() <= 30
}

pub fn clip(s: &str, max: usize) -> String {
    s.trim().chars().take(max).collect()
}

pub fn explain(code: &str) -> &'static str {
    match code {
        "salvato" => "Modifiche salvate.",
        "invito" => "Invito registrato. Se la posta è attiva, parte la mail di benvenuto.",
        "mail_ok" => "Mail inviata.",
        "mail_ko" => "La mail non è partita. Il resto della modifica è stato salvato.",
        "posto_no" => "Non c'è un posto libero da assegnare.",
        "lettura" => "Questo account può solo leggere.",
        "stato" => "Questa azione non è disponibile per lo stato attuale.",
        "eliminata" => "Iscrizione eliminata. Resta la riga nel registro, senza i dati della persona.",
        "dati" => "Controlla i campi: c'è un valore non valido.",
        "finestra" => "La chiusura deve essere dopo l'apertura.",
        "pacchetto" => "Questo pacchetto è usato da un'iscrizione e non si può togliere.",
        "ultimo" => "Deve restare almeno un proprietario attivo.",
        "codice" => "Codice non valido o scaduto.",
        "limite" => "Troppe richieste. Riprova tra qualche minuto.",
        "origine" => "Richiesta rifiutata.",
        "chiuso" => "Le iscrizioni non accettano invii in questo momento.",
        "duplicato" => "Esiste già un'iscrizione con questa email e questa data di nascita.",
        "eta" => "L'età non rientra nell'intervallo previsto.",
        "genitore" => "Servono nome, telefono e consenso di chi è responsabile.",
        "consenso" => "Manca un consenso obbligatorio.",
        "pacchetto_no" => "Questo pacchetto non è selezionabile adesso.",
        "pagamento" => "Importo o tipo di pagamento non valido.",
        "vuoto" => "Scrivi qualcosa prima di salvare.",
        "prova" => "Mail di prova inviata al tuo indirizzo.",
        _ => "Operazione non riuscita.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn lane_reserves_freed_spot_for_waitlist() {
        assert_eq!(decide_lane(10, 9, None, 0, 1), Lane::Waitlist);
        assert_eq!(decide_lane(10, 9, None, 0, 0), Lane::Seat);
        assert_eq!(decide_lane(10, 10, None, 0, 0), Lane::Waitlist);
        assert_eq!(decide_lane(10, 3, Some(2), 2, 0), Lane::Waitlist);
        assert_eq!(spots_left(10, 9, 1), 0);
        assert_eq!(spots_left(10, 9, 0), 1);
    }

    #[test]
    fn money_moves_status() {
        assert_eq!(status_after_money("attende_acconto", 5000, 15000, 5000), "posto");
        assert_eq!(status_after_money("posto", 15000, 15000, 5000), "saldato");
        assert_eq!(status_after_money("attesa", 5000, 15000, 5000), "attesa");
        assert_eq!(status_after_money("annullata", 15000, 15000, 5000), "annullata");
    }

    #[test]
    fn release_only_after_deadline_without_payment() {
        let due = "2026-01-01T00:00:00Z";
        let now = Utc.with_ymd_and_hms(2026, 1, 2, 0, 0, 0).unwrap();
        assert!(can_release("attende_acconto", due, 0, now));
        assert!(!can_release("attende_acconto", due, 1, now));
        assert!(!can_release("posto", due, 0, now));
        let early = Utc.with_ymd_and_hms(2025, 12, 1, 0, 0, 0).unwrap();
        assert!(!can_release("attende_acconto", due, 0, early));
    }

    #[test]
    fn cents_and_vars_and_html() {
        assert_eq!(format_cents(13500), "135,00 €");
        assert_eq!(format_cents(123456), "1.234,56 €");
        let html = render_vars("<b>{{nome}}</b>", &[("nome".into(), "<x>".into())], true);
        assert_eq!(html, "<b>&lt;x&gt;</b>");
        let cleaned = clean_html("<p>ok</p><script>alert(1)</script>");
        assert!(cleaned.contains("ok"));
        assert!(!cleaned.to_lowercase().contains("script"));
    }

    #[test]
    fn age_and_names() {
        let birth = NaiveDate::from_ymd_opt(2010, 6, 15).unwrap();
        let on = NaiveDate::from_ymd_opt(2027, 1, 3).unwrap();
        assert_eq!(age_years(birth, on), 16);
        assert!(valid_name("Ana-Maria D'Angelo"));
        assert!(valid_email("a.b@example.it"));
        assert!(!valid_email("nope"));
        assert!(valid_phone("+39 333 123 4567"));
    }
}
