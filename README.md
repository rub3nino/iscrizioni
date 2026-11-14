# Iscrizioni

Modulo per raccogliere iscrizioni e gestirle da un pannello: posti, prezzi, finestre, avvisi sul modulo, modelli di mail, registro. Un processo Rust, un file SQLite. Si incorpora in un sito già fatto.

L'aspetto usa i ruoli visivi di Quadra (carta, inchiostro, un blu solo per link e selezione). Questo programma non è Quadra e non ne usa il marchio.

## English

A single-binary registration module: public form, capacity, prices, notices, email templates, audit log. SQLite on disk. Embed it in an existing site with one script tag. Admin UI is Italian. Participant-facing copy is edited in the panel.

## Avvio

```bash
cp .env.example .env
# almeno SEED_ADMIN_EMAIL
cargo run
```

Pannello: http://127.0.0.1:8080/admin/login  
Modulo: http://127.0.0.1:8080/modulo

Senza `MAIL_TRANSPORT=smtp` o `resend`, le mail non partono. In sviluppo il codice di accesso compare nel log del processo. Il testo delle iscrizioni no.

## Incorporare il modulo

Sul sito ospite, dove deve comparire il form:

```html
<script src="https://iscrizioni.example/embed.js"></script>
```

Lo script apre un iframe su `/modulo` e ne adatta l'altezza. Nel pannello, in **Sul sito**, elenca le origini consentite (`https://tuosito.example`), una per riga. I posti e i prezzi li decide il server, non la pagina ospite.

`GET /api/offerta` restituisce porta, posti liberi e pacchetti, se il sito vuole mostrarli anche fuori dal modulo.

## Pannello

- **Iscrizioni** — coda, scheda, pagamenti, annullamento, ripristino, eliminazione definitiva (solo proprietario, solo se annullata).
- **Sul sito** — aperta, in pausa, chiusa; avvisi; blocchi di testo; campi; consensi.
- **Offerta** — pacchetti, finestre, posti, acconto.
- **Liste** — taglie, città, minori, allergie, camere, incassi. Senza gli annullati.
- **Mail** — oggetto, HTML, testo, anteprima, invio di prova.
- **Registro** — azioni e invii, solo in aggiunta.
- **Organizzatori** — invito dal pannello. Ruoli: proprietario, organizzatore, sola lettura.

Stati: ricevuta, in lista d'attesa, attende l'acconto, posto assicurato, saldato, annullata. Il termine dell'acconto parte all'invio della mail di pagamento e un reinvio non lo allunga. Un posto liberato non torna visibile come libero finché c'è qualcuno in lista.

## Variabili d'ambiente

Vedi `.env.example`. In produzione servono `APP_ENV=production`, `OTP_SECRET` (almeno 16 caratteri) e `MAIL_TRANSPORT` `smtp` oppure `resend`.

`TRUST_PROXY=true` solo dietro Caddy, nginx o un proxy che scrive `X-Forwarded-For`.

## Ubuntu

```bash
cargo build --release
sudo install -m 755 target/release/iscrizioni /usr/local/bin/iscrizioni
sudo useradd --system --create-home --home-dir /var/lib/iscrizioni iscrizioni
sudo install -m 640 .env /etc/iscrizioni.env
sudo chown root:iscrizioni /etc/iscrizioni.env
sudo install -m 644 deploy/iscrizioni.service /etc/systemd/system/iscrizioni.service
sudo systemctl enable --now iscrizioni
```

Il file del database sta in `DATA_DIR` (nell'unità di sistema, `/var/lib/iscrizioni`). Copia quel volume. Il processo tiene anche le ultime 30 copie giornaliere in `backups/`. Il proprietario può scaricare un backup dal registro.

Davanti metti Caddy o nginx con HTTPS e inoltri verso `127.0.0.1:8080`.

## Immagine

```bash
docker build -t iscrizioni .
docker run --rm -p 8080:8080 -v iscrizioni-data:/data --env-file .env iscrizioni
```

Senza il volume, ogni avvio perde le iscrizioni.

## Prova

```bash
cargo test
```

I test di flusso coprono il tetto dei posti (anche con due invii insieme), la lista d'attesa, il termine dell'acconto, la finestra chiusa, il duplicato e il minore senza responsabile.
