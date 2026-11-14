pub mod auth;
pub mod config;
pub mod domain;
pub mod http;
pub mod mailer;
pub mod store;
pub mod store_ops;

use std::sync::Arc;

use chrono::Utc;

pub async fn run() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "iscrizioni=info".into()),
        )
        .init();
    let cfg = Arc::new(config::Config::from_env()?);
    std::fs::create_dir_all(&cfg.data_dir)?;
    std::fs::create_dir_all(cfg.backup_dir())?;
    let db = store::Db::open(&cfg.db_path())?;
    db.migrate()?;
    db.seed(&cfg.seed_admin_email, Utc::now())?;
    let mailer = mailer::Mailer::from_config(&cfg);
    let state = http::AppState {
        db: db.clone(),
        cfg: cfg.clone(),
        mailer,
        limiter: http::Limiter::default(),
    };
    let backup_db = db.clone();
    let backup_dir = cfg.backup_dir();
    tokio::spawn(async move {
        loop {
            if let Err(e) = backup_db.backup_daily(&backup_dir) {
                tracing::warn!(error = %e, "backup");
            }
            tokio::time::sleep(std::time::Duration::from_secs(6 * 3600)).await;
        }
    });
    let addr = format!("0.0.0.0:{}", cfg.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, "in ascolto");
    axum::serve(
        listener,
        http::router(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    Ok(())
}
