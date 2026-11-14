use std::path::PathBuf;

use crate::domain::valid_email;

#[derive(Clone, Debug)]
pub struct Config {
    pub port: u16,
    pub data_dir: PathBuf,
    pub seed_admin_email: String,
    pub otp_secret: String,
    pub public_url: String,
    pub mail_from: String,
    pub reply_to: String,
    pub transport: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_user: String,
    pub smtp_password: String,
    pub resend_key: String,
    pub production: bool,
    pub trust_proxy: bool,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let production = std::env::var("APP_ENV").unwrap_or_default() == "production";
        let seed = std::env::var("SEED_ADMIN_EMAIL").unwrap_or_default().trim().to_lowercase();
        if !valid_email(&seed) {
            anyhow::bail!("SEED_ADMIN_EMAIL manca o non è un indirizzo valido");
        }
        let mut otp_secret = std::env::var("OTP_SECRET").unwrap_or_default();
        if otp_secret.len() < 16 {
            if production {
                anyhow::bail!("OTP_SECRET è obbligatorio in produzione (almeno 16 caratteri)");
            }
            otp_secret = ephemeral_secret();
            tracing::warn!("OTP_SECRET assente: i codici di accesso scadono al riavvio");
        }
        let transport = std::env::var("MAIL_TRANSPORT").unwrap_or_else(|_| "log".into());
        if production && transport == "log" {
            anyhow::bail!("In produzione MAIL_TRANSPORT non può essere log");
        }
        if transport == "smtp" && std::env::var("SMTP_HOST").unwrap_or_default().is_empty() {
            anyhow::bail!("SMTP_HOST manca");
        }
        if transport == "resend" && std::env::var("RESEND_API_KEY").unwrap_or_default().is_empty() {
            anyhow::bail!("RESEND_API_KEY manca");
        }
        let mail_from = std::env::var("MAIL_FROM").unwrap_or_else(|_| "Iscrizioni <noreply@localhost>".into());
        if production && transport != "log" && !mail_from.contains('@') {
            anyhow::bail!("MAIL_FROM manca");
        }
        Ok(Self {
            port: std::env::var("PORT").ok().and_then(|s| s.parse().ok()).unwrap_or(8080),
            data_dir: PathBuf::from(std::env::var("DATA_DIR").unwrap_or_else(|_| "./data".into())),
            seed_admin_email: seed,
            otp_secret,
            public_url: std::env::var("PUBLIC_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".into()),
            mail_from,
            reply_to: std::env::var("REPLY_TO").unwrap_or_default(),
            transport,
            smtp_host: std::env::var("SMTP_HOST").unwrap_or_default(),
            smtp_port: std::env::var("SMTP_PORT").ok().and_then(|s| s.parse().ok()).unwrap_or(587),
            smtp_user: std::env::var("SMTP_USER").unwrap_or_default(),
            smtp_password: std::env::var("SMTP_PASSWORD").unwrap_or_default(),
            resend_key: std::env::var("RESEND_API_KEY").unwrap_or_default(),
            production,
            trust_proxy: std::env::var("TRUST_PROXY").unwrap_or_default() == "true",
        })
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("iscrizioni.db")
    }

    pub fn backup_dir(&self) -> PathBuf {
        self.data_dir.join("backups")
    }
}

fn ephemeral_secret() -> String {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).expect("generatore casuale");
    hex::encode(b)
}
