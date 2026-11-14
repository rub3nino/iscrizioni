use chrono::{DateTime, Duration, NaiveDate, Utc};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::domain::{
    can_release, clip, decide_lane, parse_local_dt, stamp, status_after_money, valid_email, valid_name,
    valid_phone, Lane,
};
use crate::store::{
    audit, counts, event_row, load_event, load_package, user_code, Db, Fail, Organizer, Reg, Template,
};

fn msg(code: &str) -> Fail {
    Fail::Msg(code.into())
}
fn nid() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn now_s(now: DateTime<Utc>) -> String {
    stamp(now)
}

impl Db {
    pub fn save_offer(&self, e: OfferIn, actor: &str, now: DateTime<Utc>) -> Result<(), Fail> {
        if e.capacity < 0 || e.capacity > 100_000 || e.deposit_cents < 0 || e.deposit_cents > 10_000_000 {
            return Err(msg("dati"));
        }
        if !(1..=720).contains(&e.deposit_hours) || e.min_age < 0 || e.max_age < e.min_age || e.minor_age < 0 {
            return Err(msg("dati"));
        }
        if e.name.trim().is_empty() {
            return Err(msg("dati"));
        }
        let c = self.lock();
        let when = now_s(now);
        c.execute(
            "UPDATE event SET name=?1, place=?2, starts_on=?3, ends_on=?4, currency=?5, capacity=?6, deposit_cents=?7, deposit_hours=?8, min_age=?9, max_age=?10, minor_age=?11, iban=?12, account_holder=?13, payment_note=?14, updated_at=?15 WHERE id=1",
            params![clip(&e.name, 80), clip(&e.place, 120), e.starts_on, e.ends_on, clip(&e.currency, 8), e.capacity, e.deposit_cents, e.deposit_hours, e.min_age, e.max_age, e.minor_age, clip(&e.iban, 42), clip(&e.account_holder, 80), clip(&e.payment_note, 160), when],
        )?;
        audit(&c, actor, "offerta.salvata", "", "dati evento e acconto", &when)?;
        Ok(())
    }

    pub fn save_site(&self, s: SiteIn, actor: &str, now: DateTime<Utc>) -> Result<(), Fail> {
        if !matches!(s.door.as_str(), "open" | "pause" | "closed") {
            return Err(msg("dati"));
        }
        let c = self.lock();
        let when = now_s(now);
        c.execute(
            "UPDATE event SET door=?1, closed_message=?2, countdown_at=?3, show_countdown=?4, show_spots=?5, allowed_origins=?6, updated_at=?7 WHERE id=1",
            params![s.door, clip(&s.closed_message, 2000), s.countdown_at, s.show_countdown as i64, s.show_spots as i64, clip(&s.allowed_origins, 1000), when],
        )?;
        audit(&c, actor, "sito.salvato", "", "porta e origini", &when)?;
        Ok(())
    }

    pub fn save_window(&self, name: &str, starts: &str, ends: &str, closed: bool, actor: &str, now: DateTime<Utc>) -> Result<(), Fail> {
        let (Some(a), Some(b)) = (parse_local_dt(starts), parse_local_dt(ends)) else { return Err(msg("dati")); };
        if b <= a || name.trim().is_empty() { return Err(msg("finestra")); }
        let c = self.lock();
        let when = now_s(now);
        c.execute(
            "INSERT INTO windows (id, name, starts_at, ends_at, manually_closed, sort) VALUES (?1,?2,?3,?4,?5, (SELECT COALESCE(MAX(sort),0)+1 FROM windows))",
            params![nid(), clip(name, 80), stamp(a), stamp(b), closed as i64],
        )?;
        audit(&c, actor, "offerta.finestra", "", "finestra salvata", &when)?;
        Ok(())
    }

    pub fn delete_window(&self, id: &str, actor: &str, now: DateTime<Utc>) -> Result<(), Fail> {
        let c = self.lock();
        let used: i64 = c.query_row("SELECT COUNT(*) FROM packages WHERE window_id=?1", params![id], |r| r.get(0))?;
        if used > 0 { return Err(msg("pacchetto")); }
        c.execute("DELETE FROM windows WHERE id=?1", params![id])?;
        audit(&c, actor, "offerta.finestra", "", "finestra tolta", &now_s(now))?;
        Ok(())
    }

    pub fn save_package(&self, p: PackageIn, actor: &str, now: DateTime<Utc>) -> Result<(), Fail> {
        if p.name.trim().is_empty() || p.price_cents < 0 || p.price_cents > 10_000_000 { return Err(msg("dati")); }
        if let Some(cap) = p.capacity { if !(0..=100_000).contains(&cap) { return Err(msg("dati")); } }
        let c = self.lock();
        let when = now_s(now);
        let id = if p.id.is_empty() { nid() } else { p.id };
        c.execute(
            "INSERT INTO packages (id, name, price_cents, capacity, window_id, description, unavailable_label, sort, active) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
             ON CONFLICT(id) DO UPDATE SET name=excluded.name, price_cents=excluded.price_cents, capacity=excluded.capacity, window_id=excluded.window_id, description=excluded.description, unavailable_label=excluded.unavailable_label, sort=excluded.sort, active=excluded.active",
            params![id, clip(&p.name, 80), p.price_cents, p.capacity, p.window_id, clip(&p.description, 500), clip(&p.unavailable_label, 80), p.sort, p.active as i64],
        )?;
        audit(&c, actor, "offerta.pacchetto", "", "pacchetto salvato", &when)?;
        Ok(())
    }

    pub fn delete_package(&self, id: &str, actor: &str, now: DateTime<Utc>) -> Result<(), Fail> {
        let c = self.lock();
        let used: i64 = c.query_row("SELECT COUNT(*) FROM registrations WHERE package_id=?1", params![id], |r| r.get(0))?;
        if used > 0 { return Err(msg("pacchetto")); }
        c.execute("DELETE FROM packages WHERE id=?1", params![id])?;
        audit(&c, actor, "offerta.pacchetto", "", "pacchetto tolto", &now_s(now))?;
        Ok(())
    }

    pub fn save_notice(&self, n: NoticeIn, actor: &str, now: DateTime<Utc>) -> Result<(), Fail> {
        if n.title.trim().is_empty() { return Err(msg("vuoto")); }
        let tone = if matches!(n.tone.as_str(), "info" | "attention" | "danger") { n.tone } else { "info".into() };
        let c = self.lock();
        let id = if n.id.is_empty() { nid() } else { n.id };
        let sort: i64 = c.query_row("SELECT COALESCE(MAX(sort),0)+1 FROM notices", [], |r| r.get(0))?;
        c.execute(
            "INSERT INTO notices (id, title, body, tone, visible, starts_at, ends_at, sort) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
             ON CONFLICT(id) DO UPDATE SET title=excluded.title, body=excluded.body, tone=excluded.tone, visible=excluded.visible, starts_at=excluded.starts_at, ends_at=excluded.ends_at",
            params![id, clip(&n.title, 120), clip(&n.body, 2000), tone, n.visible as i64, n.starts_at, n.ends_at, sort],
        )?;
        audit(&c, actor, "sito.avviso", "", "avviso salvato", &now_s(now))?;
        Ok(())
    }

    pub fn delete_notice(&self, id: &str) -> Result<(), Fail> {
        self.lock().execute("DELETE FROM notices WHERE id=?1", params![id])?;
        Ok(())
    }

    pub fn move_notice(&self, id: &str, dir: i64) -> Result<(), Fail> {
        let c = self.lock();
        let sort: i64 = c.query_row("SELECT sort FROM notices WHERE id=?1", params![id], |r| r.get(0))?;
        let other: Option<(String, i64)> = if dir < 0 {
            c.query_row("SELECT id, sort FROM notices WHERE sort < ?1 ORDER BY sort DESC LIMIT 1", params![sort], |r| Ok((r.get(0)?, r.get(1)?))).optional()?
        } else {
            c.query_row("SELECT id, sort FROM notices WHERE sort > ?1 ORDER BY sort ASC LIMIT 1", params![sort], |r| Ok((r.get(0)?, r.get(1)?))).optional()?
        };
        if let Some((oid, os)) = other {
            c.execute("UPDATE notices SET sort=?1 WHERE id=?2", params![os, id])?;
            c.execute("UPDATE notices SET sort=?1 WHERE id=?2", params![sort, oid])?;
        }
        Ok(())
    }

    pub fn save_block(&self, b: BlockIn, actor: &str, now: DateTime<Utc>) -> Result<(), Fail> {
        if b.body.trim().is_empty() { return Err(msg("vuoto")); }
        let place = if matches!(b.placement.as_str(), "before_form" | "after_packages" | "before_submit") { b.placement } else { "before_form".into() };
        let c = self.lock();
        let id = if b.id.is_empty() { nid() } else { b.id };
        let sort: i64 = c.query_row("SELECT COALESCE(MAX(sort),0)+1 FROM blocks", [], |r| r.get(0))?;
        c.execute(
            "INSERT INTO blocks (id, title, body, placement, visible, sort) VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(id) DO UPDATE SET title=excluded.title, body=excluded.body, placement=excluded.placement, visible=excluded.visible",
            params![id, clip(&b.title, 120), clip(&b.body, 4000), place, b.visible as i64, sort],
        )?;
        audit(&c, actor, "sito.blocco", "", "blocco salvato", &now_s(now))?;
        Ok(())
    }

    pub fn delete_block(&self, id: &str) -> Result<(), Fail> {
        self.lock().execute("DELETE FROM blocks WHERE id=?1", params![id])?;
        Ok(())
    }

    pub fn save_field(&self, key: &str, label: &str, enabled: bool, required: bool) -> Result<(), Fail> {
        self.lock().execute(
            "UPDATE form_fields SET label=?2, enabled=?3, required=?4 WHERE key=?1",
            params![key, clip(label, 80), enabled as i64, required as i64],
        )?;
        Ok(())
    }

    pub fn save_consent(&self, label: &str, required: bool, actor: &str, now: DateTime<Utc>) -> Result<(), Fail> {
        if label.trim().is_empty() { return Err(msg("vuoto")); }
        let c = self.lock();
        let sort: i64 = c.query_row("SELECT COALESCE(MAX(sort),0)+1 FROM consents", [], |r| r.get(0))?;
        c.execute(
            "INSERT INTO consents (id, label, required, sort) VALUES (?1,?2,?3,?4)",
            params![nid(), clip(label, 300), required as i64, sort],
        )?;
        audit(&c, actor, "sito.consenso", "", "consenso salvato", &now_s(now))?;
        Ok(())
    }

    pub fn delete_consent(&self, id: &str) -> Result<(), Fail> {
        let c = self.lock();
        let n: i64 = c.query_row("SELECT COUNT(*) FROM consents", [], |r| r.get(0))?;
        if n <= 1 { return Err(msg("ultimo")); }
        c.execute("DELETE FROM consents WHERE id=?1", params![id])?;
        Ok(())
    }

    pub fn add_payment(&self, id: &str, cents: i64, method: &str, kind: &str, reference: &str, actor: &str, now: DateTime<Utc>) -> Result<String, Fail> {
        if cents <= 0 || cents > 10_000_000 { return Err(msg("pagamento")); }
        if !matches!(method, "bonifico" | "contanti" | "altro") || !matches!(kind, "acconto" | "saldo") {
            return Err(msg("pagamento"));
        }
        let mut c = self.lock();
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let reg = must_reg(&tx, id)?;
        if reg.status == "annullata" || reg.status == "attesa" { return Err(msg("stato")); }
        let pkg = load_package(&tx, &reg.package_id)?.ok_or_else(|| msg("pacchetto_no"))?;
        let event = load_event(&tx)?;
        tx.execute(
            "INSERT INTO payments (id, registration_id, amount_cents, method, kind, reference, created_at, created_by) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![nid(), id, cents, method, kind, clip(reference, 80), now_s(now), actor],
        )?;
        let paid: i64 = tx.query_row("SELECT COALESCE(SUM(amount_cents),0) FROM payments WHERE registration_id=?1", params![id], |r| r.get(0))?;
        let next = status_after_money(&reg.status, paid, pkg.price_cents, event.deposit_cents);
        let when = now_s(now);
        if next != reg.status {
            tx.execute("UPDATE registrations SET status=?1, updated_at=?2 WHERE id=?3", params![next, when, id])?;
            event_row(&tx, id, actor, "stato", &format!("{} → {next}", reg.status), &when)?;
        }
        event_row(&tx, id, actor, "pagamento", kind, &when)?;
        audit(&tx, actor, "iscrizione.pagamento", id, kind, &when)?;
        tx.commit()?;
        Ok(next)
    }

    pub fn add_note(&self, id: &str, body: &str, actor: &str, now: DateTime<Utc>) -> Result<(), Fail> {
        let body = clip(body, 2000);
        if body.is_empty() { return Err(msg("vuoto")); }
        let c = self.lock();
        let when = now_s(now);
        c.execute("INSERT INTO notes (id, registration_id, body, author, created_at) VALUES (?1,?2,?3,?4,?5)", params![nid(), id, body, actor, when])?;
        event_row(&c, id, actor, "nota", "nota interna", &when)?;
        Ok(())
    }

    pub fn correct(&self, id: &str, s: CorrectIn, actor: &str, now: DateTime<Utc>) -> Result<(), Fail> {
        if !valid_name(&s.first_name) || !valid_name(&s.last_name) || !valid_email(&s.email) || !valid_phone(&s.phone) {
            return Err(msg("dati"));
        }
        let mut c = self.lock();
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let reg = must_reg(&tx, id)?;
        let email = s.email.trim().to_lowercase();
        let dup: i64 = tx.query_row(
            "SELECT COUNT(*) FROM registrations WHERE email=?1 AND birth_date=?2 AND status!='annullata' AND id!=?3",
            params![email, s.birth_date, id], |r| r.get(0))?;
        if dup > 0 { return Err(msg("duplicato")); }
        let when = now_s(now);
        tx.execute(
            "UPDATE registrations SET first_name=?1, last_name=?2, birth_date=?3, email=?4, phone=?5, package_id=?6, gender=?7, city=?8, shirt=?9, notes=?10, allergies=?11, transport=?12, parent_name=?13, parent_phone=?14, parent_consent=?15, updated_at=?16 WHERE id=?17",
            params![clip(&s.first_name, 80), clip(&s.last_name, 80), s.birth_date, email, clip(&s.phone, 30), s.package_id, clip(&s.gender, 16), clip(&s.city, 80), clip(&s.shirt, 16), clip(&s.notes, 1000), clip(&s.allergies, 1000), clip(&s.transport, 120), clip(&s.parent_name, 80), clip(&s.parent_phone, 30), s.parent_consent as i64, when, id],
        )?;
        event_row(&tx, id, actor, "correzione", "dati aggiornati", &when)?;
        audit(&tx, actor, "iscrizione.correzione", id, &reg.number.to_string(), &when)?;
        tx.commit()?;
        Ok(())
    }

    pub fn cancel(&self, id: &str, actor: &str, now: DateTime<Utc>) -> Result<(), Fail> {
        self.set_cancelled(id, actor, "manuale", "iscrizione.annullata", now)
    }

    pub fn release(&self, id: &str, actor: &str, now: DateTime<Utc>) -> Result<(), Fail> {
        let c = self.lock();
        let reg = must_reg(&c, id)?;
        let paid: i64 = c.query_row("SELECT COALESCE(SUM(amount_cents),0) FROM payments WHERE registration_id=?1", params![id], |r| r.get(0))?;
        if !can_release(&reg.status, &reg.deposit_due_at, paid, now) { return Err(msg("stato")); }
        drop(c);
        self.set_cancelled(id, actor, "termine", "posto.liberato", now)
    }

    fn set_cancelled(&self, id: &str, actor: &str, reason: &str, action: &str, now: DateTime<Utc>) -> Result<(), Fail> {
        let c = self.lock();
        let reg = must_reg(&c, id)?;
        if reg.status == "annullata" { return Err(msg("stato")); }
        let when = now_s(now);
        c.execute("UPDATE registrations SET status='annullata', cancel_reason=?1, updated_at=?2 WHERE id=?3", params![reason, when, id])?;
        event_row(&c, id, actor, "stato", &format!("{} → annullata", reg.status), &when)?;
        audit(&c, actor, action, id, &reg.number.to_string(), &when)?;
        Ok(())
    }

    pub fn restore(&self, id: &str, actor: &str, now: DateTime<Utc>) -> Result<String, Fail> {
        let mut c = self.lock();
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let reg = must_reg(&tx, id)?;
        if reg.status != "annullata" { return Err(msg("stato")); }
        let event = load_event(&tx)?;
        let pkg = load_package(&tx, &reg.package_id)?.ok_or_else(|| msg("pacchetto_no"))?;
        let (taken, pkg_taken, waiting) = counts(&tx, &pkg.id)?;
        let lane = decide_lane(event.capacity, taken, pkg.capacity, pkg_taken, waiting);
        let status = if lane == Lane::Waitlist { "attesa" } else { "ricevuta" };
        let when = now_s(now);
        tx.execute("UPDATE registrations SET status=?1, cancel_reason='', deposit_due_at='', updated_at=?2 WHERE id=?3", params![status, when, id])?;
        event_row(&tx, id, actor, "stato", &format!("annullata → {status}"), &when)?;
        audit(&tx, actor, "iscrizione.ripristinata", id, status, &when)?;
        tx.commit()?;
        Ok(status.into())
    }

    pub fn hard_delete(&self, id: &str, actor: &str, now: DateTime<Utc>) -> Result<(), Fail> {
        let c = self.lock();
        let reg = must_reg(&c, id)?;
        if reg.status != "annullata" { return Err(msg("stato")); }
        let when = now_s(now);
        audit(&c, actor, "iscrizione.eliminata", id, &format!("numero {}", reg.number), &when)?;
        c.execute("DELETE FROM mail_log WHERE registration_id=?1", params![id])?;
        c.execute("DELETE FROM registrations WHERE id=?1 AND status='annullata'", params![id])?;
        Ok(())
    }

    pub fn deposit_possible(&self, id: &str) -> Result<(), Fail> {
        let c = self.lock();
        let reg = must_reg(&c, id)?;
        if reg.status == "attesa" {
            let event = load_event(&c)?;
            let (taken, _, _) = counts(&c, &reg.package_id)?;
            if taken >= event.capacity { return Err(msg("posto_no")); }
        } else if reg.status != "ricevuta" && reg.status != "attende_acconto" {
            return Err(msg("stato"));
        }
        Ok(())
    }

    pub fn arm_deposit(&self, id: &str, actor: &str, now: DateTime<Utc>) -> Result<(), Fail> {
        let mut c = self.lock();
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let reg = must_reg(&tx, id)?;
        let event = load_event(&tx)?;
        let when = now_s(now);
        if reg.status == "attesa" {
            let (taken, _, _) = counts(&tx, &reg.package_id)?;
            if taken >= event.capacity { return Err(msg("posto_no")); }
            let due = if reg.deposit_due_at.is_empty() { stamp(now + Duration::hours(event.deposit_hours)) } else { reg.deposit_due_at.clone() };
            tx.execute("UPDATE registrations SET status='attende_acconto', deposit_due_at=?1, updated_at=?2 WHERE id=?3", params![due, when, id])?;
            event_row(&tx, id, actor, "stato", "attesa → attende_acconto", &when)?;
        } else if reg.status == "ricevuta" {
            let due = if reg.deposit_due_at.is_empty() { stamp(now + Duration::hours(event.deposit_hours)) } else { reg.deposit_due_at.clone() };
            tx.execute("UPDATE registrations SET status='attende_acconto', deposit_due_at=?1, updated_at=?2 WHERE id=?3", params![due, when, id])?;
            event_row(&tx, id, actor, "stato", "ricevuta → attende_acconto", &when)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn notes(&self, id: &str) -> Result<Vec<(String, String, String)>, Fail> {
        let c = self.lock();
        let mut s = c.prepare("SELECT body, author, created_at FROM notes WHERE registration_id=?1 ORDER BY created_at")?;
        let rows = s.query_map(params![id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
    pub fn payments(&self, id: &str) -> Result<Vec<(i64, String, String, String, String)>, Fail> {
        let c = self.lock();
        let mut s = c.prepare("SELECT amount_cents, method, kind, reference, created_at FROM payments WHERE registration_id=?1 ORDER BY created_at")?;
        let rows = s.query_map(params![id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
    pub fn history(&self, id: &str) -> Result<Vec<(String, String, String)>, Fail> {
        let c = self.lock();
        let mut s = c.prepare("SELECT at, actor, detail FROM reg_events WHERE registration_id=?1 ORDER BY at")?;
        let rows = s.query_map(params![id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
    pub fn mails_of(&self, id: &str) -> Result<Vec<(String, String, i64, String)>, Fail> {
        let c = self.lock();
        let mut s = c.prepare("SELECT template_key, subject, ok, created_at FROM mail_log WHERE registration_id=?1 ORDER BY created_at DESC")?;
        let rows = s.query_map(params![id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn templates(&self) -> Result<Vec<Template>, Fail> {
        let c = self.lock();
        let mut s = c.prepare("SELECT key, name, subject, html, text_body, enabled, updated_at FROM mail_templates ORDER BY name")?;
        let rows = s.query_map([], map_tpl)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
    pub fn template(&self, key: &str) -> Result<Option<Template>, Fail> {
        Ok(self.lock().query_row("SELECT key, name, subject, html, text_body, enabled, updated_at FROM mail_templates WHERE key=?1", params![key], map_tpl).optional()?)
    }
    pub fn save_template(&self, key: &str, subject: &str, html: &str, text_body: &str, enabled: bool, actor: &str, now: DateTime<Utc>) -> Result<(), Fail> {
        if subject.trim().is_empty() { return Err(msg("vuoto")); }
        let c = self.lock();
        let when = now_s(now);
        let n = c.execute(
            "UPDATE mail_templates SET subject=?2, html=?3, text_body=?4, enabled=?5, updated_at=?6 WHERE key=?1",
            params![key, clip(subject, 200), html, text_body, enabled as i64, when],
        )?;
        if n == 0 { return Err(msg("dati")); }
        audit(&c, actor, "mail.modello", "", key, &when)?;
        Ok(())
    }
    pub fn log_mail(&self, key: &str, reg_id: &str, to: &str, subject: &str, ok: bool, error: &str, actor: &str, now: DateTime<Utc>) -> Result<(), Fail> {
        let c = self.lock();
        let when = now_s(now);
        c.execute(
            "INSERT INTO mail_log (id, template_key, registration_id, to_email, subject, ok, error, actor, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![nid(), key, reg_id, to, clip(subject, 200), ok as i64, clip(error, 300), actor, when],
        )?;
        if !reg_id.is_empty() {
            event_row(&c, reg_id, actor, "email", key, &when)?;
        }
        Ok(())
    }

    pub fn organizers(&self) -> Result<Vec<Organizer>, Fail> {
        let c = self.lock();
        let mut s = c.prepare("SELECT email, name, role, notify, active, created_at, last_login_at FROM organizers ORDER BY email")?;
        let rows = s.query_map([], map_org)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
    pub fn organizer(&self, email: &str) -> Result<Option<Organizer>, Fail> {
        Ok(self.lock().query_row(
            "SELECT email, name, role, notify, active, created_at, last_login_at FROM organizers WHERE email=?1",
            params![email.trim().to_lowercase()], map_org,
        ).optional()?)
    }
    pub fn invite(&self, email: &str, name: &str, role: &str, notify: bool, actor: &str, now: DateTime<Utc>) -> Result<(), Fail> {
        if !valid_email(email) || !matches!(role, "owner" | "admin" | "viewer") { return Err(msg("dati")); }
        let c = self.lock();
        let email = email.trim().to_lowercase();
        let when = now_s(now);
        c.execute(
            "INSERT INTO organizers (email, name, role, notify, active, created_at, last_login_at) VALUES (?1,?2,?3,?4,1,?5,'')
             ON CONFLICT(email) DO UPDATE SET name=excluded.name, role=excluded.role, notify=excluded.notify, active=1",
            params![email, clip(name, 80), role, notify as i64, when],
        )?;
        audit(&c, actor, "organizzatore.aggiunto", "", &email, &when)?;
        Ok(())
    }
    pub fn update_organizer(&self, email: &str, role: &str, notify: bool, active: bool, actor: &str, now: DateTime<Utc>) -> Result<(), Fail> {
        if !matches!(role, "owner" | "admin" | "viewer") { return Err(msg("dati")); }
        let c = self.lock();
        if !active || role != "owner" {
            let owners: i64 = c.query_row("SELECT COUNT(*) FROM organizers WHERE role='owner' AND active=1 AND email!=?1", params![email], |r| r.get(0))?;
            let current: Option<String> = c.query_row("SELECT role FROM organizers WHERE email=?1", params![email], |r| r.get(0)).optional()?;
            if current.as_deref() == Some("owner") && owners == 0 { return Err(msg("ultimo")); }
        }
        let when = now_s(now);
        c.execute("UPDATE organizers SET role=?2, notify=?3, active=?4 WHERE email=?1", params![email, role, notify as i64, active as i64])?;
        audit(&c, actor, "organizzatore.aggiornato", "", email, &when)?;
        Ok(())
    }

    pub fn issue_otp(&self, email: &str, code_hash: &str, now: DateTime<Utc>) -> Result<bool, Fail> {
        let c = self.lock();
        let email = email.trim().to_lowercase();
        let org = c.query_row("SELECT active FROM organizers WHERE email=?1", params![email], |r| r.get::<_, i64>(0)).optional()?;
        let Some(active) = org else { return Ok(false); };
        if active == 0 { return Ok(false); }
        let prev: Option<String> = c.query_row("SELECT sent_at FROM otp WHERE email=?1", params![email], |r| r.get(0)).optional()?;
        if let Some(sent) = prev {
            if let Some(t) = crate::domain::parse_ts(&sent) {
                if now.signed_duration_since(t) < Duration::seconds(60) { return Ok(false); }
            }
        }
        let exp = stamp(now + Duration::minutes(10));
        c.execute(
            "INSERT INTO otp (email, code_hash, expires_at, attempts, sent_at) VALUES (?1,?2,?3,0,?4)
             ON CONFLICT(email) DO UPDATE SET code_hash=excluded.code_hash, expires_at=excluded.expires_at, attempts=0, sent_at=excluded.sent_at",
            params![email, code_hash, exp, now_s(now)],
        )?;
        Ok(true)
    }

    pub fn verify_otp(&self, email: &str, code_hash: &str, now: DateTime<Utc>) -> Result<bool, Fail> {
        let c = self.lock();
        let email = email.trim().to_lowercase();
        let row: Option<(String, String, i64)> = c.query_row(
            "SELECT code_hash, expires_at, attempts FROM otp WHERE email=?1",
            params![email], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).optional()?;
        let Some((hash, exp, attempts)) = row else { return Ok(false); };
        if attempts >= 5 { return Ok(false); }
        let expired = crate::domain::parse_ts(&exp).map(|t| t < now).unwrap_or(true);
        let ok = !expired && crate::auth::ct_eq(&hash, code_hash);
        if !ok {
            c.execute("UPDATE otp SET attempts=attempts+1 WHERE email=?1", params![email])?;
            return Ok(false);
        }
        c.execute("DELETE FROM otp WHERE email=?1", params![email])?;
        c.execute("UPDATE organizers SET last_login_at=?1 WHERE email=?2", params![now_s(now), email])?;
        Ok(true)
    }

    pub fn create_session(&self, email: &str, now: DateTime<Utc>) -> Result<String, Fail> {
        let id = nid();
        let c = self.lock();
        c.execute(
            "INSERT INTO sessions (id, email, created_at, expires_at) VALUES (?1,?2,?3,?4)",
            params![id, email.trim().to_lowercase(), now_s(now), stamp(now + Duration::hours(8))],
        )?;
        Ok(id)
    }
    pub fn session_email(&self, id: &str, now: DateTime<Utc>) -> Result<Option<String>, Fail> {
        let c = self.lock();
        let row: Option<(String, String)> = c.query_row(
            "SELECT email, expires_at FROM sessions WHERE id=?1", params![id], |r| Ok((r.get(0)?, r.get(1)?)),
        ).optional()?;
        let Some((email, exp)) = row else { return Ok(None); };
        if crate::domain::parse_ts(&exp).map(|t| t < now).unwrap_or(true) {
            c.execute("DELETE FROM sessions WHERE id=?1", params![id])?;
            return Ok(None);
        }
        Ok(Some(email))
    }
    pub fn drop_session(&self, id: &str) -> Result<(), Fail> {
        self.lock().execute("DELETE FROM sessions WHERE id=?1", params![id])?;
        Ok(())
    }

    pub fn registro(&self) -> Result<Vec<(String, String, String, String, String)>, Fail> {
        let c = self.lock();
        let mut s = c.prepare("SELECT at, actor, action, registration_id, detail FROM audit ORDER BY at DESC LIMIT 300")?;
        let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
    pub fn mail_log_all(&self) -> Result<Vec<(String, String, String, i64, String)>, Fail> {
        let c = self.lock();
        let mut s = c.prepare("SELECT created_at, template_key, to_email, ok, registration_id FROM mail_log ORDER BY created_at DESC LIMIT 200")?;
        let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
    pub fn notify_emails(&self) -> Result<Vec<String>, Fail> {
        let c = self.lock();
        let mut s = c.prepare("SELECT email FROM organizers WHERE notify=1 AND active=1")?;
        let rows = s.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn backup_daily(&self, dir: &std::path::Path) -> Result<(), Fail> {
        std::fs::create_dir_all(dir).ok();
        let day = Utc::now().format("%Y-%m-%d").to_string();
        let dest = dir.join(format!("iscrizioni-{day}.db"));
        if !dest.exists() {
            let src = Connection::open(&self.path)?;
            crate::store::tune(&src)?;
            let mut dst = Connection::open(&dest)?;
            let backup = rusqlite::backup::Backup::new(&src, &mut dst)?;
            backup.step(-1)?;
        }
        let mut files: Vec<_> = std::fs::read_dir(dir).map(|rd| {
            rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "db")).collect::<Vec<_>>()
        }).unwrap_or_default();
        files.sort();
        while files.len() > 30 {
            if let Some(old) = files.first() { let _ = std::fs::remove_file(old); }
            files.remove(0);
        }
        Ok(())
    }
}

fn map_tpl(r: &rusqlite::Row<'_>) -> rusqlite::Result<Template> {
    Ok(Template { key: r.get(0)?, name: r.get(1)?, subject: r.get(2)?, html: r.get(3)?, text_body: r.get(4)?, enabled: r.get::<_, i64>(5)? != 0, updated_at: r.get(6)? })
}
fn map_org(r: &rusqlite::Row<'_>) -> rusqlite::Result<Organizer> {
    Ok(Organizer { email: r.get(0)?, name: r.get(1)?, role: r.get(2)?, notify: r.get::<_, i64>(3)? != 0, active: r.get::<_, i64>(4)? != 0, created_at: r.get(5)?, last_login_at: r.get(6)? })
}
fn must_reg(c: &Connection, id: &str) -> Result<Reg, Fail> {
    c.query_row(
        "SELECT id, number, status, first_name, last_name, birth_date, email, phone, package_id, gender, city, shirt, notes, allergies, transport, parent_name, parent_phone, parent_consent, consents, deposit_due_at, cancel_reason, created_at, updated_at FROM registrations WHERE id=?1",
        params![id],
        |r| Ok(Reg {
            id: r.get(0)?, number: r.get(1)?, status: r.get(2)?, first_name: r.get(3)?, last_name: r.get(4)?,
            birth_date: r.get(5)?, email: r.get(6)?, phone: r.get(7)?, package_id: r.get(8)?, gender: r.get(9)?,
            city: r.get(10)?, shirt: r.get(11)?, notes: r.get(12)?, allergies: r.get(13)?, transport: r.get(14)?,
            parent_name: r.get(15)?, parent_phone: r.get(16)?, parent_consent: r.get::<_, i64>(17)? != 0,
            consents: r.get(18)?, deposit_due_at: r.get(19)?, cancel_reason: r.get(20)?, created_at: r.get(21)?, updated_at: r.get(22)?,
        }),
    ).optional()?.ok_or_else(|| msg("dati"))
}

pub struct OfferIn {
    pub name: String, pub place: String, pub starts_on: String, pub ends_on: String, pub currency: String,
    pub capacity: i64, pub deposit_cents: i64, pub deposit_hours: i64, pub min_age: i64, pub max_age: i64, pub minor_age: i64,
    pub iban: String, pub account_holder: String, pub payment_note: String,
}
pub struct SiteIn {
    pub door: String, pub closed_message: String, pub countdown_at: String, pub show_countdown: bool, pub show_spots: bool, pub allowed_origins: String,
}
pub struct PackageIn {
    pub id: String, pub name: String, pub price_cents: i64, pub capacity: Option<i64>, pub window_id: Option<String>,
    pub description: String, pub unavailable_label: String, pub sort: i64, pub active: bool,
}
pub struct NoticeIn { pub id: String, pub title: String, pub body: String, pub tone: String, pub visible: bool, pub starts_at: String, pub ends_at: String }
pub struct BlockIn { pub id: String, pub title: String, pub body: String, pub placement: String, pub visible: bool }
pub struct CorrectIn {
    pub first_name: String, pub last_name: String, pub birth_date: String, pub email: String, pub phone: String, pub package_id: String,
    pub gender: String, pub city: String, pub shirt: String, pub notes: String, pub allergies: String, pub transport: String,
    pub parent_name: String, pub parent_phone: String, pub parent_consent: bool,
}

pub fn age_of(event_start: &str, birth: &str, today: NaiveDate) -> Option<i32> {
    let b = NaiveDate::parse_from_str(birth, "%Y-%m-%d").ok()?;
    let on = NaiveDate::parse_from_str(event_start, "%Y-%m-%d").unwrap_or(today);
    Some(crate::domain::age_years(b, on))
}
