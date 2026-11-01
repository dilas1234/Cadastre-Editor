//! Task notifications by e-mail (Aumentum's EmailTemplate.xml "UserTask").
//!
//! A minimal SMTP client (plain SMTP, optional AUTH LOGIN), matching what Aumentum's settings
//! describe (smtpServer, smtpPort, smtpUser, smtpPass, FromEmail), and an outbox loop that sends
//! pending notifications. With no SMTP server configured, nothing is sent, as in production.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use sqlx::{PgPool, Row};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

/// SMTP settings, read from `cadastre.app_setting` (mail.*).
#[derive(Clone, Debug, Default)]
pub struct SmtpConfig {
    pub server: String,
    pub port: u16,
    pub from: String,
    pub user: String,
    pub password: String,
}

impl SmtpConfig {
    pub fn configured(&self) -> bool {
        !self.server.trim().is_empty() && !self.from.trim().is_empty()
    }
    pub async fn load(pool: &PgPool) -> Result<Self, sqlx::Error> {
        let rows = sqlx::query("SELECT key, value FROM cadastre.app_setting WHERE key LIKE 'mail.%'").fetch_all(pool).await?;
        let get = |k: &str| rows.iter().find(|r| r.get::<String, _>("key") == k).and_then(|r| r.get::<Option<String>, _>("value")).unwrap_or_default();
        Ok(Self {
            server: get("mail.smtp_server"),
            port: get("mail.smtp_port").parse().unwrap_or(25),
            from: get("mail.from"),
            user: get("mail.smtp_user"),
            password: get("mail.smtp_password"),
        })
    }
}

/// Fill a template's `*Placeholder*` fields.
pub fn render(template: &str, fields: &[(&str, &str)]) -> String {
    fields.iter().fold(template.to_string(), |acc, (k, v)| acc.replace(&format!("*{k}*"), v))
}

async fn expect(r: &mut BufReader<tokio::net::tcp::OwnedReadHalf>, ok: &[u16]) -> Result<(), String> {
    // A reply may span lines ("250-...") and ends with "250 ...".
    loop {
        let mut line = String::new();
        if r.read_line(&mut line).await.map_err(|e| e.to_string())? == 0 {
            return Err("SMTP server closed the connection".into());
        }
        let code: u16 = line.get(..3).and_then(|c| c.parse().ok()).ok_or_else(|| format!("bad SMTP reply: {line:?}"))?;
        if line.as_bytes().get(3) == Some(&b'-') {
            continue;
        }
        return if ok.contains(&code) { Ok(()) } else { Err(format!("SMTP error: {}", line.trim())) };
    }
}

/// Send one HTML message.
pub async fn send(cfg: &SmtpConfig, to: &str, subject: &str, html: &str) -> Result<(), String> {
    let stream = tokio::time::timeout(std::time::Duration::from_secs(20), TcpStream::connect((cfg.server.as_str(), cfg.port)))
        .await
        .map_err(|_| "SMTP connect timed out".to_string())?
        .map_err(|e| e.to_string())?;
    let (rd, mut w) = stream.into_split();
    let mut r = BufReader::new(rd);
    macro_rules! cmd {
        ($line:expr, $ok:expr) => {{
            w.write_all(format!("{}\r\n", $line).as_bytes()).await.map_err(|e| e.to_string())?;
            expect(&mut r, $ok).await?;
        }};
    }
    expect(&mut r, &[220]).await?;
    cmd!("EHLO plagis-cadastre", &[250]);
    if !cfg.user.is_empty() {
        cmd!("AUTH LOGIN", &[334]);
        cmd!(B64.encode(&cfg.user), &[334]);
        cmd!(B64.encode(&cfg.password), &[235]);
    }
    cmd!(format!("MAIL FROM:<{}>", cfg.from), &[250]);
    cmd!(format!("RCPT TO:<{to}>"), &[250, 251]);
    cmd!("DATA", &[354]);
    // Dot-stuff the body (a line starting with '.' gets another '.').
    let body = html.replace("\r\n", "\n").split('\n').map(|l| if l.starts_with('.') { format!(".{l}") } else { l.to_string() }).collect::<Vec<_>>().join("\r\n");
    let msg = format!(
        "From: {from}\r\nTo: {to}\r\nSubject: {subject}\r\nMIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{body}\r\n.",
        from = cfg.from
    );
    cmd!(msg, &[250]);
    let _ = w.write_all(b"QUIT\r\n").await;
    Ok(())
}

/// Send pending notifications every 30 s while an SMTP server is configured.
pub async fn run_outbox(pool: PgPool) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(30));
    loop {
        tick.tick().await;
        let Ok(cfg) = SmtpConfig::load(&pool).await else { continue };
        if !cfg.configured() {
            continue;
        }
        let Ok(rows) = sqlx::query("SELECT id, to_email, subject, body FROM cadastre.wf_notification WHERE status = 'pending' ORDER BY id LIMIT 50")
            .fetch_all(&pool)
            .await
        else {
            continue;
        };
        for row in rows {
            let id: i64 = row.get("id");
            let to: Option<String> = row.get("to_email");
            let result = match to.as_deref().filter(|t| t.contains('@')) {
                Some(to) => send(&cfg, to, row.get("subject"), row.get("body")).await,
                None => Err("the assignee has no e-mail address".into()),
            };
            let (status, detail) = match result {
                Ok(()) => ("sent", None),
                Err(e) => ("failed", Some(e)),
            };
            let _ = sqlx::query("UPDATE cadastre.wf_notification SET status = $2, detail = $3, sent_at = CASE WHEN $2 = 'sent' THEN now() END WHERE id = $1")
                .bind(id)
                .bind(status)
                .bind(detail)
                .execute(&pool)
                .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn templates_fill_aumentum_placeholders() {
        let t = "*FinalUser*, you have been assigned *Task* (*JobInstance*)";
        assert_eq!(render(t, &[("FinalUser", "Ada"), ("Task", "Review Parcel"), ("JobInstance", "Job-PL1")]), "Ada, you have been assigned Review Parcel (Job-PL1)");
    }

    /// The client speaks SMTP correctly to a scripted server (including AUTH LOGIN and dot-stuffing).
    #[tokio::test]
    async fn sends_a_message_over_smtp() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (s, _) = listener.accept().await.unwrap();
            let (rd, mut w) = s.into_split();
            let mut r = BufReader::new(rd);
            let mut seen = Vec::new();
            w.write_all(b"220 test ready\r\n").await.unwrap();
            let mut data = false;
            let mut body = String::new();
            loop {
                let mut line = String::new();
                if r.read_line(&mut line).await.unwrap() == 0 { break; }
                if data {
                    if line == ".\r\n" { data = false; w.write_all(b"250 queued\r\n").await.unwrap(); } else { body.push_str(&line); }
                    continue;
                }
                seen.push(line.trim().to_string());
                let reply: &[u8] = match line.trim() {
                    l if l.starts_with("EHLO") => b"250-test\r\n250 AUTH LOGIN\r\n",
                    "AUTH LOGIN" => b"334 VXNlcm5hbWU6\r\n",
                    l if l == B64.encode("mailer") => b"334 UGFzc3dvcmQ6\r\n",
                    l if l == B64.encode("secret") => b"235 ok\r\n",
                    "DATA" => { data = true; b"354 go\r\n" }
                    "QUIT" => { w.write_all(b"221 bye\r\n").await.unwrap(); break; }
                    _ => b"250 ok\r\n",
                };
                w.write_all(reply).await.unwrap();
            }
            (seen, body)
        });
        let cfg = SmtpConfig { server: "127.0.0.1".into(), port, from: "cadastre@plagis.com".into(), user: "mailer".into(), password: "secret".into() };
        send(&cfg, "officer@plagis.com", "Job-PL1 Assigned", "Hello\n.hidden line").await.unwrap();
        let (seen, body) = server.await.unwrap();
        assert!(seen.contains(&"MAIL FROM:<cadastre@plagis.com>".to_string()));
        assert!(seen.contains(&"RCPT TO:<officer@plagis.com>".to_string()));
        assert!(body.contains("Subject: Job-PL1 Assigned"));
        assert!(body.contains("\r\n..hidden line"), "a leading dot is stuffed");
    }
}
