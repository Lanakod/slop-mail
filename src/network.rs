use crate::{
    mime,
    store::{Account, Server},
};
use anyhow::{bail, ensure, Context, Result};
use lettre::{transport::smtp::authentication::Credentials, SmtpTransport};
use native_tls::{TlsConnector, TlsStream};
use rusqlite::{params, Connection, OptionalExtension};
use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    time::Duration,
};
static SESSION: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
fn credential_key(profile: &str, a: &Account, side: &str) -> Result<String> {
    let (server, login) = if side == "in" {
        (&a.incoming, &a.login)
    } else {
        (&a.outgoing, &a.smtp_login)
    };
    Ok(serde_json::to_string(&(
        profile,
        &a.id,
        side,
        &server.proto,
        &server.host,
        server.port,
        &server.tls,
        login,
    ))?)
}
pub fn secret(profile: &str, a: &Account, side: &str) -> Result<String> {
    let key = credential_key(profile, a, side)?;
    if let Some(value) = SESSION.lock().unwrap().get(&key) {
        return Ok(value.clone());
    }
    keyring::Entry::new("org.slopmail.desktop", &key)?
        .get_password()
        .context("Нужен пароль. Откройте настройки ящика")
}
pub fn save_secret(
    profile: &str,
    a: &Account,
    side: &str,
    input: &str,
    remember: bool,
) -> Result<()> {
    let key = credential_key(profile, a, side)?;
    let entry = keyring::Entry::new("org.slopmail.desktop", &key)?;
    let value = if input.is_empty() {
        if let Some(v) = SESSION.lock().unwrap().get(&key).cloned() {
            v
        } else {
            match entry.get_password() {
                Ok(v) => v,
                Err(keyring::Error::NoEntry) => return Ok(()),
                Err(e) => return Err(e.into()),
            }
        }
    } else {
        input.to_owned()
    };
    if remember {
        entry.set_password(&value)?;
        SESSION.lock().unwrap().remove(&key);
    } else {
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => (),
            Err(e) => return Err(e.into()),
        };
        SESSION.lock().unwrap().insert(key, value);
    }
    Ok(())
}
fn socket(s: &Server) -> Result<TcpStream> {
    let mut last = None;
    for addr in (s.host.as_str(), s.port).to_socket_addrs()? {
        match TcpStream::connect_timeout(&addr, Duration::from_secs(12)) {
            Ok(t) => {
                t.set_read_timeout(Some(Duration::from_secs(60)))?;
                t.set_write_timeout(Some(Duration::from_secs(30)))?;
                return Ok(t);
            }
            Err(e) => last = Some(e),
        }
    }
    bail!("Нет соединения с {}: {:?}", s.host, last)
}
type Imap = imap::Session<TlsStream<TcpStream>>;
fn imap_login(a: &Account, pw: &str) -> Result<Imap> {
    let s = &a.incoming;
    let t = socket(s)?;
    let tls = TlsConnector::new()?;
    let c = if s.tls == "tls" {
        let mut c = imap::Client::new(tls.connect(&s.host, t)?);
        c.read_greeting()?;
        c
    } else {
        let mut c = imap::Client::new(t);
        c.read_greeting()?;
        c.secure(&s.host, &tls)?
    };
    c.login(&a.login, pw)
        .map_err(|(e, _)| anyhow::anyhow!("Авторизация IMAP: {e}"))
}
pub fn smtp(a: &Account, pw: &str) -> Result<SmtpTransport> {
    let s = &a.outgoing;
    let b = if s.tls == "tls" {
        SmtpTransport::relay(&s.host)?
    } else {
        SmtpTransport::starttls_relay(&s.host)?
    };
    Ok(b.port(s.port)
        .timeout(Some(Duration::from_secs(45)))
        .credentials(Credentials::new(a.smtp_login.clone(), pw.into()))
        .build())
}
pub fn test(a: &Account, side: &str, pw: &str) -> Result<()> {
    if side == "out" {
        ensure!(
            smtp(a, pw)?.test_connection()?,
            "SMTP не подтвердил соединение"
        );
    } else if a.incoming.proto == "imap" {
        imap_login(a, pw)?.logout()?;
    } else {
        pop_cmd(&mut pop_login(a, pw)?, "QUIT")?;
    }
    Ok(())
}
pub fn sync(c: &mut Connection, a: &Account, pw: &str) -> Result<usize> {
    if a.incoming.proto == "pop3" {
        let mut p = pop_login(a, pw)?;
        return pop_sync(c, a, &mut p);
    }
    let mut session = imap_login(a, pw)?;
    let count = imap_sync(c, a, &mut session)?;
    session.logout()?;
    Ok(count)
}
pub fn imap_sync<T: Read + Write>(
    c: &mut Connection,
    a: &Account,
    session: &mut imap::Session<T>,
) -> Result<usize> {
    let mut count = 0;
    let folders = session.list(Some(""), Some("*"))?;
    for f in folders.iter().filter(|f| {
        !f.attributes()
            .contains(&imap::types::NameAttribute::NoSelect)
    }) {
        let folder = f.name();
        let valid = session
            .select(folder)?
            .uid_validity
            .context("Сервер не сообщил UIDVALIDITY")?;
        let t = c.transaction()?;
        let old = t
            .query_row(
                "SELECT validity FROM folders WHERE account=? AND name=?",
                params![a.id, folder],
                |r| r.get::<_, u32>(0),
            )
            .optional()?;
        if old.is_some_and(|v| v != valid) {
            t.execute(
                "DELETE FROM hidden WHERE account=? AND folder=?",
                params![a.id, folder],
            )?;
            t.execute(
                "DELETE FROM messages WHERE account=? AND folder=?",
                params![a.id, folder],
            )?;
        }
        t.execute(
            "INSERT OR REPLACE INTO folders VALUES(?,?,?)",
            params![a.id, folder, valid],
        )?;
        t.commit()?;
        let dirty = c
            .prepare(
                "SELECT uid,seen FROM messages WHERE account=? AND folder=? AND pending_seen=1",
            )?
            .query_map(params![a.id, folder], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (uid, seen) in dirty {
            let flags = if seen {
                "+FLAGS.SILENT (\\Seen)"
            } else {
                "-FLAGS.SILENT (\\Seen)"
            };
            session.uid_store(&uid, flags)?;
            c.execute(
                "UPDATE messages SET pending_seen=0 WHERE account=? AND folder=? AND uid=? AND seen=?",
                params![a.id, folder, uid,seen],
            )?;
        }
        let flags = session.uid_fetch("1:*", "(UID FLAGS)")?;
        let mut remote = std::collections::HashSet::new();
        for record in flags.iter() {
            let Some(uid) = record.uid else { continue };
            let uid = uid.to_string();
            remote.insert(uid.clone());
            let seen = record.flags().contains(&imap::types::Flag::Seen);
            let updated=c.execute("UPDATE messages SET seen=CASE WHEN pending_seen=1 THEN seen ELSE ? END WHERE account=? AND folder=? AND uid=?",params![seen,a.id,folder,uid])?;
            if updated > 0 {
                continue;
            }
            let hidden: bool = c.query_row(
                "SELECT EXISTS(SELECT 1 FROM hidden WHERE account=? AND folder=? AND uid=?)",
                params![a.id, folder, uid],
                |r| r.get(0),
            )?;
            if hidden {
                continue;
            }
            let fetched = session.uid_fetch(&uid, "(UID BODY.PEEK[])")?;
            if let Some(raw) = fetched
                .iter()
                .find(|v| v.uid == record.uid)
                .and_then(|v| v.body())
            {
                use mailparse::MailHeaderMap;
                let sender = mailparse::parse_mail(raw)?
                    .headers
                    .get_first_value("From")
                    .unwrap_or_default();
                let outgoing = crate::delivery::addresses(&sender)
                    .iter()
                    .any(|v| v.eq_ignore_ascii_case(&a.address));
                let t = c.transaction()?;
                mime::save(&t, &a.id, folder, &uid, raw, outgoing, seen)?;
                t.commit()?;
                if old == Some(valid) && folder.eq_ignore_ascii_case("INBOX") && !outgoing && !seen
                {
                    count += 1;
                }
            }
        }
        let saved = c
            .prepare("SELECT uid FROM messages WHERE account=? AND folder=?")?
            .query_map(params![a.id, folder], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let t = c.transaction()?;
        for uid in saved {
            if !remote.contains(&uid) {
                t.execute(
                    "DELETE FROM messages WHERE account=? AND folder=? AND uid=?",
                    params![a.id, folder, uid],
                )?;
            }
        }
        t.commit()?;
    }
    Ok(count)
}
fn ok<R: BufRead>(p: &mut R) -> Result<()> {
    let mut s = String::new();
    p.read_line(&mut s)?;
    ensure!(s.starts_with("+OK"), "Ответ POP3: {}", s.trim());
    Ok(())
}
fn pop_cmd<T: Read + Write>(p: &mut BufReader<T>, cmd: &str) -> Result<()> {
    ensure!(!cmd.contains(['\r', '\n']), "Недопустимый перевод строки");
    write!(p.get_mut(), "{cmd}\r\n")?;
    p.get_mut().flush()?;
    ok(p)
}
fn pop_login(a: &Account, pw: &str) -> Result<BufReader<TlsStream<TcpStream>>> {
    let s = &a.incoming;
    let tcp = socket(s)?;
    let tls = TlsConnector::new()?;
    let mut p = if s.tls == "tls" {
        let mut p = BufReader::new(tls.connect(&s.host, tcp)?);
        ok(&mut p)?;
        p
    } else {
        let mut p = BufReader::new(tcp);
        ok(&mut p)?;
        pop_cmd(&mut p, "STLS")?;
        BufReader::new(tls.connect(&s.host, p.into_inner())?)
    };
    pop_cmd(&mut p, &format!("USER {}", a.login)).context("Авторизация POP3")?;
    pop_cmd(&mut p, &format!("PASS {pw}")).context("Авторизация POP3")?;
    Ok(p)
}
fn multiline<T: Read>(p: &mut BufReader<T>) -> Result<Vec<u8>> {
    let mut data = vec![];
    loop {
        let mut line = vec![];
        ensure!(p.read_until(b'\n', &mut line)? > 0, "Обрыв ответа POP3");
        if line == b".\r\n" {
            break;
        }
        data.extend_from_slice(if line.starts_with(b"..") {
            &line[1..]
        } else {
            &line
        });
    }
    Ok(data)
}
pub fn pop_sync<T: Read + Write>(
    c: &mut Connection,
    a: &Account,
    p: &mut BufReader<T>,
) -> Result<usize> {
    let mut count = 0;
    let initialized: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM folders WHERE account=? AND name='INBOX')",
        [&a.id],
        |r| r.get(0),
    )?;
    pop_cmd(p, "UIDL").context(
        "Сервер не поддерживает UIDL или недоступен. Загрузка остановлена для защиты от дублей",
    )?;
    let list = String::from_utf8(multiline(p)?)?;
    c.execute("INSERT OR IGNORE INTO folders VALUES(?,'INBOX',0)", [&a.id])?;
    for row in list.lines() {
        let mut cols = row.split_whitespace();
        let n = cols.next().context("UIDL")?.parse::<u32>()?;
        let uid = cols.next().context("Нет UIDL")?;
        let saved: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM pop_history WHERE account=? AND uid=?)",
            params![a.id, uid],
            |r| r.get(0),
        )?;
        if !saved {
            pop_cmd(p, &format!("RETR {n}"))?;
            let raw = multiline(p)?;
            let t = c.transaction()?;
            mime::save(&t, &a.id, "INBOX", uid, &raw, false, false)?;
            t.execute("INSERT INTO pop_history VALUES(?,?)", params![a.id, uid])?;
            t.commit()?;
            if initialized {
                count += 1;
            }
        }
        if !a.keep {
            pop_cmd(p, &format!("DELE {n}"))?;
        }
    }
    pop_cmd(p, "QUIT")?;
    Ok(count)
}

// Explicit, opt-in verification against an operator-specified mailbox. Ordinary CI
// uses loopback fixtures and never reads credentials or sends an external message.
#[cfg(test)]
#[test]
#[ignore = "requires explicit SMTP recipient, TLS servers and interactive credentials"]
fn live_pop_smtp() -> Result<()> {
    use crate::{delivery, store};
    use lettre::Transport;
    let setting = std::env::var;
    let address = setting("SLOP_LIVE_ADDRESS")?;
    let host = setting("SLOP_LIVE_HOST")?;
    let recipient = setting("SLOP_LIVE_RECIPIENT")?;
    let password = setting("SLOP_LIVE_PASSWORD")?;
    let mut a = Account {
        id: String::new(),
        name: "Slop Mail".into(),
        address: address.clone(),
        login: address.clone(),
        smtp_login: address,
        signature: String::new(),
        incoming: Server {
            proto: "pop3".into(),
            host: host.clone(),
            port: 995,
            tls: "tls".into(),
        },
        outgoing: Server {
            proto: "smtp".into(),
            host,
            port: 465,
            tls: "tls".into(),
        },
        keep: true,
    };
    let mut c = Connection::open_in_memory()?;
    c.execute_batch(store::SCHEMA)?;
    store::save_account(&mut c, &mut a)?;
    let incoming = (|| -> Result<()> {
        let mut p = pop_login(&a, &password)?;
        pop_cmd(&mut p, "UIDL")?;
        let list = String::from_utf8(multiline(&mut p)?)?;
        if let Some(last) = list.lines().last() {
            let mut fields = last.split_whitespace();
            let number = fields.next().context("Нет номера")?.parse::<u32>()?;
            let uid = fields.next().context("Нет UIDL")?;
            pop_cmd(&mut p, &format!("RETR {number}"))?;
            let raw = multiline(&mut p)?;
            let t = c.transaction()?;
            let local = mime::save(&t, &a.id, "INBOX", uid, &raw, false, false)?;
            t.commit()?;
            let opened:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM messages JOIN search ON search.rowid=messages.id WHERE messages.id=?)",[local],|r|r.get(0))?;
            ensure!(opened, "Письмо отсутствует в индексе");
            println!("POP3 TLS: UIDL, RETR, MIME and local search index verified");
        } else {
            println!("POP3 TLS: authenticated; mailbox is empty");
        }
        pop_cmd(&mut p, "QUIT")?;
        Ok(())
    })();
    if let Err(e) = &incoming {
        println!("POP3 check failed: {e:#}");
    }
    let transport = smtp(&a, &password)?;
    ensure!(
        transport
            .test_connection()
            .context("SMTP TLS connection test")?,
        "SMTP NOOP"
    );
    println!("SMTP TLS: authenticated");
    delivery::enqueue(
        &mut c,
        delivery::Compose {
            cc: String::new(),
            bcc: String::new(),
            account: a.id.clone(),
            to: recipient,
            personal: false,
            parent: None,
            signature: String::new(),
            files: vec![],
            slot: "live-test".into(),
            subject: "Проверка Slop Mail".into(),
            body: "Тестовое письмо для проверки отправки через настольный клиент Slop Mail.".into(),
        },
    )?;
    let id: String = c.query_row("SELECT id FROM queue", [], |r| r.get(0))?;
    delivery::send_one(&mut c, &id, |_, env, raw| {
        transport
            .send_raw(env, raw)
            .map(|response| {
                println!("SMTP accepted: {}", response.code());
            })
            .map_err(delivery::smtp_failure)
    })?;
    let (state, error): (String, String) =
        c.query_row("SELECT state,error FROM queue WHERE id=?", [id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
    ensure!(state == "sent", "SMTP state={state}: {error}");
    incoming
}
