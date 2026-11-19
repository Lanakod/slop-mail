use super::*;
use anyhow::{Context, Result};
use lettre::{SmtpTransport, Transport};
use mailparse::MailHeaderMap;
use rusqlite::Connection;
use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::Duration,
};

const LETTER:&str="From: Alice <alice@example.org>\r\nTo: me@example.org\r\nReply-To: support@example.org\r\nSubject: Invoice\r\nMessage-ID: <invoice@example.org>\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nПривет invoice\r\n";
fn account(c: &mut Connection) -> Result<store::Account> {
    let mut a: store::Account = serde_json::from_value(
        serde_json::json!({"name":"Me","address":"me@example.org","login":"me","smtp_login":"relay","signature":"Работа","incoming":{"proto":"imap","host":"mail.example.org","port":993,"tls":"tls"},"outgoing":{"proto":"smtp","host":"relay.example.org","port":587,"tls":"starttls"},"keep":true}),
    )?;
    store::save_account(c, &mut a)?;
    Ok(a)
}
fn database() -> Result<Connection> {
    let c = Connection::open_in_memory()?;
    c.execute_batch(store::SCHEMA)?;
    Ok(c)
}
fn fixture<F, T>(f: F) -> Result<(u16, thread::JoinHandle<Result<T>>)>
where
    F: FnOnce(BufReader<TcpStream>) -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    let l = TcpListener::bind(("127.0.0.1", 0))?;
    let port = l.local_addr()?.port();
    Ok((
        port,
        thread::spawn(move || {
            let (s, _) = l.accept()?;
            s.set_read_timeout(Some(Duration::from_secs(5)))?;
            f(BufReader::new(s))
        }),
    ))
}
fn line(s: &mut BufReader<TcpStream>) -> Result<String> {
    let mut out = String::new();
    anyhow::ensure!(s.read_line(&mut out)? > 0, "End of stream");
    Ok(out.trim_end_matches(['\r', '\n']).into())
}
fn imap_round(c: &mut Connection, a: &store::Account, validity: u32) -> Result<()> {
    let (port, server) = fixture(move |mut s| {
        write!(s.get_mut(), "* OK IMAP ready\r\n")?;
        loop {
            let command = line(&mut s)?;
            let (tag, cmd) = command.split_once(' ').context("IMAP tag")?;
            let reply = if cmd.starts_with("LOGIN") {
                "".into()
            } else if cmd.starts_with("LIST") {
                "* LIST () \"/\" \"INBOX\"\r\n".into()
            } else if cmd.starts_with("SELECT") {
                format!("* 1 EXISTS\r\n* OK [UIDVALIDITY {validity}] ready\r\n")
            } else if cmd.contains("BODY.PEEK") {
                format!(
                    "* 1 FETCH (UID 7 BODY[] {{{}}}\r\n{})\r\n",
                    LETTER.len(),
                    LETTER
                )
            } else if cmd.starts_with("UID FETCH") {
                "* 1 FETCH (UID 7 FLAGS ())\r\n".into()
            } else if cmd.starts_with("LOGOUT") {
                write!(s.get_mut(), "* BYE closing\r\n{tag} OK done\r\n")?;
                break;
            } else {
                anyhow::bail!("Unexpected {cmd}")
            };
            write!(s.get_mut(), "{reply}{tag} OK done\r\n")?;
        }
        Ok(())
    })?;
    let mut client = imap::Client::new(TcpStream::connect(("127.0.0.1", port))?);
    client.read_greeting()?;
    let mut session = client.login("test", "fixture").map_err(|e| e.0)?;
    network::imap_sync(c, a, &mut session)?;
    session.logout()?;
    server.join().unwrap()?;
    Ok(())
}
#[derive(Default)]
struct Captured {
    to: Vec<String>,
    raw: String,
}
fn smtp_server(
    reject: bool,
    disconnect: bool,
) -> Result<(u16, thread::JoinHandle<Result<Captured>>)> {
    fixture(move |mut s| {
        let mut mail = Captured::default();
        write!(s.get_mut(), "220 localhost ESMTP\r\n")?;
        loop {
            let Ok(cmd) = line(&mut s) else { break };
            if cmd.starts_with("EHLO") {
                write!(s.get_mut(), "250 localhost\r\n")?
            } else if cmd.starts_with("MAIL FROM:") {
                write!(s.get_mut(), "250 ok\r\n")?
            } else if cmd.starts_with("RCPT TO:") {
                mail.to.push(cmd);
                if reject {
                    write!(s.get_mut(), "550 rejected\r\n")?;
                    break;
                }
                write!(s.get_mut(), "250 ok\r\n")?
            } else if cmd == "DATA" {
                write!(s.get_mut(), "354 go\r\n")?;
                loop {
                    let row = line(&mut s)?;
                    if row == "." {
                        break;
                    }
                    mail.raw += &row;
                    mail.raw += "\r\n";
                }
                if disconnect {
                    break;
                }
                write!(s.get_mut(), "250 queued\r\n")?;
            } else if cmd == "QUIT" {
                write!(s.get_mut(), "221 bye\r\n")?;
                break;
            } else {
                anyhow::bail!("Unexpected {cmd}")
            }
        }
        Ok(mail)
    })
}
fn compose(a: &store::Account, to: &str, personal: bool, parent: Option<i64>) -> delivery::Compose {
    delivery::Compose {
        account: a.id.clone(),
        to: to.into(),
        subject: "Re: Invoice".into(),
        body: "Ответ".into(),
        signature: a.signature.clone(),
        personal,
        parent,
        files: vec![],
        slot: "test".into(),
    }
}
fn smtp_round(c: &mut Connection, id: &str, reject: bool, disconnect: bool) -> Result<Captured> {
    let (port, server) = smtp_server(reject, disconnect)?;
    delivery::send_one(c, id, |_, env, raw| {
        let t = SmtpTransport::builder_dangerous("127.0.0.1")
            .port(port)
            .build();
        t.send_raw(env, raw)
            .map(|_| ())
            .map_err(delivery::smtp_failure)
    })?;
    server.join().unwrap()
}
fn pending(c: &Connection) -> Result<Vec<String>> {
    Ok(
        c.prepare("SELECT id FROM queue WHERE state='pending' ORDER BY rowid")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?,
    )
}
#[test]
fn receive_open_reply_and_search_offline() -> Result<()> {
    let mut c = database()?;
    let a = account(&mut c)?;
    imap_round(&mut c, &a, 1)?;
    imap_round(&mut c, &a, 1)?;
    imap_round(&mut c, &a, 2)?;
    assert_eq!(
        c.query_row("SELECT count(*) FROM messages", [], |r| r
            .get::<_, usize>(0))?,
        1
    );
    let (id, reply, thread): (i64, String, String) =
        c.query_row("SELECT id,reply_to,thread FROM messages", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
    delivery::enqueue(&mut c, compose(&a, &reply, false, Some(id)))?;
    let job = pending(&c)?.remove(0);
    let captured = smtp_round(&mut c, &job, false, false)?;
    assert_eq!(captured.to, ["RCPT TO:<support@example.org>"]);
    let parsed = mailparse::parse_mail(captured.raw.as_bytes())?;
    assert_eq!(
        parsed.headers.get_first_value("In-Reply-To").as_deref(),
        Some("<invoice@example.org>")
    );
    assert!(parsed.get_body()?.contains("Работа"));
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM messages WHERE thread=?",
            [thread],
            |r| r.get::<_, usize>(0)
        )?,
        2
    );
    for q in ["привет", "invoice", "alice"] {
        assert!(!api::list(&c, &serde_json::json!({"q":q}))?
            .as_array()
            .unwrap()
            .is_empty())
    }
    Ok(())
}
