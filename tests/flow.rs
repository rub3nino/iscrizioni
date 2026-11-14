use std::thread;

use chrono::{TimeZone, Utc};
use iscrizioni::store::{tune, Db, Signup};
use iscrizioni::store_ops::{OfferIn, PackageIn, SiteIn};
use rusqlite::Connection;

fn boot(capacity: i64) -> (tempfile::TempDir, Db, String) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(&dir.path().join("t.db")).unwrap();
    db.migrate().unwrap();
    db.seed("owner@example.com", Utc::now()).unwrap();
    db.save_offer(
        OfferIn {
            name: "Prova".into(),
            place: String::new(),
            starts_on: "2027-01-03".into(),
            ends_on: "2027-01-06".into(),
            currency: "EUR".into(),
            capacity,
            deposit_cents: 5000,
            deposit_hours: 48,
            min_age: 10,
            max_age: 70,
            minor_age: 18,
            iban: "IT00X".into(),
            account_holder: "Cassa".into(),
            payment_note: String::new(),
        },
        "owner@example.com",
        Utc::now(),
    )
    .unwrap();
    db.save_site(
        SiteIn {
            door: "open".into(),
            closed_message: "chiuse".into(),
            countdown_at: String::new(),
            show_countdown: false,
            show_spots: true,
            allowed_origins: String::new(),
        },
        "owner@example.com",
        Utc::now(),
    )
    .unwrap();
    db.save_package(
        PackageIn {
            id: String::new(),
            name: "Standard".into(),
            price_cents: 15000,
            capacity: None,
            window_id: None,
            description: String::new(),
            unavailable_label: "Chiuso".into(),
            sort: 0,
            active: true,
        },
        "owner@example.com",
        Utc::now(),
    )
    .unwrap();
    let id = db.packages().unwrap().into_iter().next().unwrap().id;
    (dir, db, id)
}

fn person(pkg: &str, email: &str, birth: &str) -> Signup {
    Signup {
        first_name: "Ada".into(),
        last_name: "Rossi".into(),
        birth_date: birth.into(),
        email: email.into(),
        phone: "+39 333 123 4567".into(),
        package_id: pkg.into(),
        gender: String::new(),
        city: String::new(),
        shirt: String::new(),
        notes: String::new(),
        allergies: String::new(),
        transport: String::new(),
        parent_name: String::new(),
        parent_phone: String::new(),
        parent_consent: false,
        consent_ids: vec![],
    }
}

fn with_consent(db: &Db, mut s: Signup) -> Signup {
    s.consent_ids = db.consents().unwrap().into_iter().map(|c| c.id).collect();
    s
}

#[test]
fn capacity_waitlist_and_release() {
    let (dir, db, pkg) = boot(1);
    let now = Utc.with_ymd_and_hms(2026, 11, 14, 12, 0, 0).unwrap();
    let a = db.signup(&with_consent(&db, person(&pkg, "a@example.com", "2000-01-01")), now).unwrap();
    let b = db.signup(&with_consent(&db, person(&pkg, "b@example.com", "2001-01-01")), now).unwrap();
    assert_eq!(a.status, "ricevuta");
    assert_eq!(b.status, "attesa");
    db.cancel(&a.id, "owner@example.com", now).unwrap();
    assert_eq!(db.reg(&b.id).unwrap().unwrap().status, "attesa");
    let (taken, waiting) = db.occupancy().unwrap();
    assert_eq!((taken, waiting), (0, 1));
    assert_eq!(iscrizioni::domain::spots_left(1, taken, waiting), 0);
    db.arm_deposit(&b.id, "owner@example.com", now).unwrap();
    let held = db.reg(&b.id).unwrap().unwrap();
    assert_eq!(held.status, "attende_acconto");
    let due = held.deposit_due_at.clone();
    db.arm_deposit(&b.id, "owner@example.com", now).unwrap();
    assert_eq!(db.reg(&b.id).unwrap().unwrap().deposit_due_at, due);
    assert!(db.release(&b.id, "owner@example.com", now).is_err());
    let conn = Connection::open(dir.path().join("t.db")).unwrap();
    tune(&conn).unwrap();
    conn.execute(
        "UPDATE registrations SET deposit_due_at='2020-01-01T00:00:00Z' WHERE id=?1",
        [&b.id],
    )
    .unwrap();
    drop(conn);
    db.release(&b.id, "owner@example.com", Utc::now()).unwrap();
    assert_eq!(db.reg(&b.id).unwrap().unwrap().status, "annullata");
    assert!(db.hard_delete(&a.id, "owner@example.com", Utc::now()).is_ok());
    let alive = db.signup(&with_consent(&db, person(&pkg, "c@example.com", "1999-05-05")), Utc::now()).unwrap();
    assert!(db.hard_delete(&alive.id, "owner@example.com", Utc::now()).is_err());
}

#[test]
fn parallel_signups_do_not_overbook() {
    let (dir, db, pkg) = boot(1);
    let consent = db.consents().unwrap()[0].id.clone();
    let path = dir.path().join("t.db");
    let mut handles = Vec::new();
    for i in 0..2 {
        let path = path.clone();
        let pkg = pkg.clone();
        let consent = consent.clone();
        handles.push(thread::spawn(move || {
            let mut c = Connection::open(&path).unwrap();
            tune(&c).unwrap();
            let mut s = person(&pkg, &format!("p{i}@example.com"), "1998-02-02");
            s.consent_ids = vec![consent];
            iscrizioni::store::signup_on(&mut c, &s, Utc::now()).unwrap().status
        }));
    }
    let mut statuses: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    statuses.sort();
    assert_eq!(statuses, vec!["attesa".to_string(), "ricevuta".to_string()]);
    let _ = db;
}

#[test]
fn window_duplicate_and_minor() {
    let (_dir, db, pkg) = boot(10);
    let now = Utc.with_ymd_and_hms(2026, 11, 14, 12, 0, 0).unwrap();
    db.save_window("Passata", "2020-01-01T00:00", "2020-01-02T00:00", false, "owner@example.com", now).unwrap();
    let wid = db.windows().unwrap().into_iter().next().unwrap().id;
    db.save_package(
        PackageIn {
            id: pkg.clone(),
            name: "Standard".into(),
            price_cents: 15000,
            capacity: None,
            window_id: Some(wid),
            description: String::new(),
            unavailable_label: "Chiuso".into(),
            sort: 0,
            active: true,
        },
        "owner@example.com",
        now,
    )
    .unwrap();
    let err = db.signup(&with_consent(&db, person(&pkg, "x@example.com", "2000-01-01")), now).unwrap_err();
    assert_eq!(db.code(&err), "pacchetto_no");

    db.save_package(
        PackageIn {
            id: pkg.clone(),
            name: "Standard".into(),
            price_cents: 15000,
            capacity: None,
            window_id: None,
            description: String::new(),
            unavailable_label: String::new(),
            sort: 0,
            active: true,
        },
        "owner@example.com",
        now,
    )
    .unwrap();
    db.signup(&with_consent(&db, person(&pkg, "same@example.com", "2000-01-01")), now).unwrap();
    let dup = db.signup(&with_consent(&db, person(&pkg, "same@example.com", "2000-01-01")), now).unwrap_err();
    assert_eq!(db.code(&dup), "duplicato");

    let mut minor = person(&pkg, "kid@example.com", "2010-06-01");
    minor.consent_ids = db.consents().unwrap().into_iter().map(|c| c.id).collect();
    let err = db.signup(&minor, now).unwrap_err();
    assert_eq!(db.code(&err), "genitore");
    minor.parent_name = "Lia Neri".into();
    minor.parent_phone = "+39 320 111 2233".into();
    minor.parent_consent = true;
    assert!(db.signup(&minor, now).is_ok());
}
