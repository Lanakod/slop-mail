use crate::{
    mime,
    store::{Account, Server},
};
use anyhow::{bail, ensure, Context, Result};
use lettre::{transport::smtp::authentication::Credentials, SmtpTransport};
use native_tls::{TlsConnector, TlsStream};
use rusqlite::{params, Connection, OptionalExtension};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    time::Duration,
};
pub fn secret(profile: &str, a: &Account, side: &str, value: Option<&str>) -> Result<String> {
    let (server, login) = if side == "in" {
        (&a.incoming, &a.login)
    } else {
        (&a.outgoing, &a.smtp_login)
    };
    let key = serde_json::to_string(&(
        profile,
        &a.id,
        side,
        &server.proto,
        &server.host,
        server.port,
        &server.tls,
        login,
    ))?;
    let e = keyring::Entry::new("org.slopmail.desktop", &key)?;
    if let Some(v) = value.filter(|v| !v.is_empty()) {
        e.set_password(v)?;
        Ok(v.into())
    } else {
        e.get_password()
            .context("Нужен пароль. Откройте настройки ящика")
    }
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
pub fn sync(c: &mut Connection, a: &Account, pw: &str) -> Result<()> {
    if a.incoming.proto == "pop3" {
        let mut p = pop_login(a, pw)?;
        return pop_sync(c, a, &mut p);
    }
    let mut session = imap_login(a, pw)?;
    imap_sync(c, a, &mut session)?;
    session.logout()?;
    Ok(())
}
pub fn imap_sync<T: Read + Write>(
    c: &mut Connection,
    a: &Account,
    session: &mut imap::Session<T>,
) -> Result<()> {
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
            .prepare("SELECT uid FROM messages WHERE account=? AND folder=? AND pending_seen=1")?
            .query_map(params![a.id, folder], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for uid in dirty {
            session.uid_store(&uid, "+FLAGS.SILENT (\\Seen)")?;
            c.execute(
                "UPDATE messages SET pending_seen=0 WHERE account=? AND folder=? AND uid=?",
                params![a.id, folder, uid],
            )?;
        }
        let flags = session.uid_fetch("1:*", "(UID FLAGS)")?;
        let mut remote = std::collections::HashSet::new();
        for record in flags.iter() {
            let Some(uid) = record.uid else { continue };
            let uid = uid.to_string();
            remote.insert(uid.clone());
            let seen = record.flags().contains(&imap::types::Flag::Seen);
            let updated=c.execute("UPDATE messages SET seen=CASE WHEN pending_seen=1 THEN 1 ELSE ? END WHERE account=? AND folder=? AND uid=?",params![seen,a.id,folder,uid])?;
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
    Ok(())
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
) -> Result<()> {
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
        }
        if !a.keep {
            pop_cmd(p, &format!("DELE {n}"))?;
        }
    }
    pop_cmd(p, "QUIT")
}
