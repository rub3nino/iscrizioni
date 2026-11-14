use crate::config::Config;

#[derive(Clone, Debug)]
pub struct Outgoing {
    pub to: String,
    pub subject: String,
    pub html: String,
    pub text: String,
    pub dev_code: Option<String>,
}

#[derive(Clone)]
pub enum Mailer {
    Log { show_code: bool },
    Smtp {
        host: String,
        port: u16,
        user: String,
        password: String,
        from: String,
        reply_to: String,
    },
    Resend {
        key: String,
        from: String,
        reply_to: String,
        client: reqwest::Client,
    },
}

impl Mailer {
    pub fn from_config(cfg: &Config) -> Self {
        match cfg.transport.as_str() {
            "smtp" => Self::Smtp {
                host: cfg.smtp_host.clone(),
                port: cfg.smtp_port,
                user: cfg.smtp_user.clone(),
                password: cfg.smtp_password.clone(),
                from: cfg.mail_from.clone(),
                reply_to: cfg.reply_to.clone(),
            },
            "resend" => Self::Resend {
                key: cfg.resend_key.clone(),
                from: cfg.mail_from.clone(),
                reply_to: cfg.reply_to.clone(),
                client: reqwest::Client::new(),
            },
            _ => Self::Log { show_code: !cfg.production },
        }
    }

    pub fn configured(&self) -> bool {
        !matches!(self, Self::Log { .. })
    }

    pub async fn send(&self, msg: Outgoing) -> Result<(), String> {
        match self {
            Self::Log { show_code } => {
                if *show_code {
                    if let Some(code) = &msg.dev_code {
                        tracing::info!(email = %msg.to, codice = %code, "codice di accesso, solo sviluppo");
                    }
                }
                if msg.dev_code.is_none() {
                    tracing::info!(to = %msg.to, subject = %msg.subject, "mail non inviata, trasporto di sviluppo");
                }
                Err("posta non configurata".into())
            }
            Self::Smtp { host, port, user, password, from, reply_to } => {
                let host = host.clone();
                let port = *port;
                let user = user.clone();
                let password = password.clone();
                let from = from.clone();
                let reply_to = reply_to.clone();
                tokio::task::spawn_blocking(move || smtp_send(&host, port, &user, &password, &from, &reply_to, &msg))
                    .await
                    .map_err(|e| e.to_string())?
            }
            Self::Resend { key, from, reply_to, client } => {
                let body = serde_json::json!({
                    "from": from,
                    "to": [msg.to],
                    "subject": msg.subject,
                    "html": msg.html,
                    "text": msg.text,
                    "reply_to": if reply_to.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(reply_to.clone()) },
                });
                let res = client
                    .post("https://api.resend.com/emails")
                    .bearer_auth(key)
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                if res.status().is_success() {
                    Ok(())
                } else {
                    Err(format!("resend {}", res.status()))
                }
            }
        }
    }
}

fn smtp_send(host: &str, port: u16, user: &str, password: &str, from: &str, reply_to: &str, msg: &Outgoing) -> Result<(), String> {
    use lettre::message::{header::ContentType, MultiPart, SinglePart};
    use lettre::transport::smtp::authentication::Credentials;
    use lettre::{Message, SmtpTransport, Transport};

    let mut b = Message::builder()
        .from(from.parse().map_err(|e: lettre::address::AddressError| e.to_string())?)
        .to(msg.to.parse().map_err(|e: lettre::address::AddressError| e.to_string())?)
        .subject(&msg.subject);
    if !reply_to.is_empty() {
        b = b.reply_to(reply_to.parse().map_err(|e: lettre::address::AddressError| e.to_string())?);
    }
    let email = b
        .multipart(
            MultiPart::alternative()
                .singlepart(SinglePart::builder().header(ContentType::TEXT_PLAIN).body(msg.text.clone()))
                .singlepart(SinglePart::builder().header(ContentType::TEXT_HTML).body(msg.html.clone())),
        )
        .map_err(|e| e.to_string())?;
    let mut builder = if port == 465 {
        SmtpTransport::relay(host).map_err(|e| e.to_string())?.port(port)
    } else {
        SmtpTransport::starttls_relay(host).map_err(|e| e.to_string())?.port(port)
    };
    if !user.is_empty() {
        builder = builder.credentials(Credentials::new(user.into(), password.into()));
    }
    builder.build().send(&email).map(|_| ()).map_err(|e| e.to_string())
}
