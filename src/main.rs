#[tokio::main]
async fn main() {
    if let Err(e) = iscrizioni::run().await {
        eprintln!("iscrizioni: {e:#}");
        std::process::exit(1);
    }
}
