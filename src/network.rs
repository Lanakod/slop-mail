use crate::{
    mime,
    store::{self, Account, Server},
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
    imap_login_on(a, pw, socket(&a.incoming)?)
}
fn imap_login_on(a: &Account, pw: &str, t: TcpStream) -> Result<Imap> {
    let s = &a.incoming;
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
// Shut down the blocking IDLE read when the active profile or schedule changes.
#[derive(Default)]
pub struct IdleSocket(Mutex<(bool, Option<TcpStream>)>);
impl IdleSocket {
    pub fn stop(&self) {
        let mut state = self.0.lock().unwrap();
        state.0 = true;
        if let Some(stream) = state.1.take() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }
    pub fn listen(
        &self,
        profile: &str,
        account: &Account,
        reconnect: bool,
        changed: impl FnMut(),
    ) -> Result<()> {
        let pw = secret(profile, account, "in")?;
        let stream = socket(&account.incoming)?;
        {
            let mut state = self.0.lock().unwrap();
            ensure!(!state.0, "Ожидание остановлено");
            state.1 = Some(stream.try_clone()?);
        }
        let result = imap_login_on(account, &pw, stream)
            .and_then(|mut session| watch_inbox(&mut session, reconnect, changed));
        self.0.lock().unwrap().1.take();
        result
    }
}

pub(crate) fn watch_inbox<T: Read + Write + imap::extensions::idle::SetReadTimeout>(
    session: &mut imap::Session<T>,
    reconnect: bool,
    mut changed: impl FnMut(),
) -> Result<()> {
    if !session.capabilities()?.has_str("IDLE") {
        session.logout()?;
        return Ok(());
    }
    let snapshot = |m: imap::types::Mailbox| (m.uid_validity, m.uid_next, m.exists);
    let mut previous = snapshot(session.examine("INBOX")?);
    if reconnect {
        changed();
    }
    loop {
        session
            .idle()?
            .wait_with_timeout(Duration::from_secs(25 * 60))?;
        let current = snapshot(session.examine("INBOX")?);
        if current != previous {
            changed();
            previous = current;
        }
        while session.unsolicited_responses.try_recv().is_ok() {}
    }
}
pub fn restore_links(c: &Connection, pid: &str, id: i64) -> Result<()> {
    let (account, folder, uid, mid): (String, String, String, String) = c.query_row(
        "SELECT account,folder,uid,mid FROM messages WHERE id=?",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    let a = store::account(c, &account)?;
    let pw = secret(pid, &a, "in")?;
    let raw = if a.incoming.proto == "imap" {
        let mut s = imap_login(&a, &pw)?;
        let valid: u32 = c.query_row(
            "SELECT validity FROM folders WHERE account=? AND name=?",
            params![account, folder],
            |r| r.get(0),
        )?;
        ensure!(
            s.select(&folder)?.uid_validity == Some(valid),
            "Папка изменилась. Сначала получите почту"
        );
        let uid = uid.parse::<u32>()?;
        let data = s
            .uid_fetch(uid.to_string(), "(UID BODY.PEEK[])")?
            .iter()
            .find(|m| m.uid == Some(uid))
            .and_then(|m| m.body())
            .context("Письмо отсутствует на сервере")?
            .to_vec();
        s.logout()?;
        data
    } else {
        let mut p = pop_login(&a, &pw)?;
        pop_cmd(&mut p, "UIDL")?;
        let list = String::from_utf8(multiline(&mut p)?)?;
        let n = list
            .lines()
            .find_map(|line| {
                let mut s = line.split_whitespace();
                let n = s.next()?.parse::<u32>().ok()?;
                (s.next() == Some(uid.as_str())).then_some(n)
            })
            .context("Сервер больше не хранит эту копию")?;
        pop_cmd(&mut p, &format!("RETR {n}"))?;
        let data = multiline(&mut p)?;
        pop_cmd(&mut p, "QUIT")?;
        data
    };
    let html = mime::original_html(&raw, &mid)?;
    c.execute(
        "UPDATE messages SET html=? WHERE id=? AND mid=?",
        params![html, id, mid],
    )?;
    Ok(())
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
pub fn test(a: &Account, side: &str, pw: &str) -> Result<Option<bool>> {
    if side == "out" {
        ensure!(
            smtp(a, pw)?.test_connection()?,
            "SMTP не подтвердил соединение"
        );
    } else if a.incoming.proto == "imap" {
        let mut session = imap_login(a, pw)?;
        let idle = session.capabilities()?.has_str("IDLE");
        session.logout()?;
        return Ok(Some(idle));
    } else {
        pop_cmd(&mut pop_login(a, pw)?, "QUIT")?;
    }
    Ok(None)
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
pub fn flush_moves(c: &mut Connection, a: &Account, pw: &str) -> Result<()> {
    let pending=store::rows(c,"SELECT DISTINCT m.folder,f.validity FROM moves v JOIN messages m ON m.id=v.message JOIN folders f ON f.account=m.account AND f.name=m.folder WHERE m.account=? AND v.due<=unixepoch() AND m.uid NOT GLOB '*[^0-9]*'", &[&a.id])?;
    if pending.as_array().unwrap().is_empty() {
        return Ok(());
    }
    let mut session = imap_login(a, pw)?;
    let folders = session.list(Some(""), Some("*"))?;
    for row in pending.as_array().unwrap() {
        let folder = row["folder"].as_str().unwrap();
        ensure!(
            session.select(folder)?.uid_validity.map(u64::from) == row["validity"].as_u64(),
            "Папка изменилась. Сначала получите почту"
        );
        move_selected(c, a, &mut session, folder, &folders)?;
    }
    session.logout()?;
    Ok(())
}
// IMAP4rev1 mailbox arguments use modified UTF-7 for non-ASCII folder names.
fn mailbox_wire_name(name: &str) -> String {
    use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine};
    let mut result = String::new();
    let mut bytes = Vec::new();
    for ch in name.chars().chain(std::iter::once('\0')) {
        if ch.is_ascii() {
            if !bytes.is_empty() {
                result.push('&');
                result.push_str(&STANDARD_NO_PAD.encode(&bytes).replace('/', ","));
                result.push('-');
                bytes.clear();
            }
            if ch == '&' {
                result.push_str("&-");
            } else if ch != '\0' {
                result.push(ch);
            }
        } else {
            for unit in ch.encode_utf16(&mut [0; 2]) {
                bytes.extend(unit.to_be_bytes());
            }
        }
    }
    result
}
fn move_selected<T: Read + Write>(
    c: &mut Connection,
    a: &Account,
    session: &mut imap::Session<T>,
    folder: &str,
    folders: &[imap::types::Name],
) -> Result<()> {
    let pending = store::rows(c, "SELECT m.id,m.uid,v.destination FROM moves v JOIN messages m ON m.id=v.message WHERE m.account=? AND m.folder=? AND m.uid NOT GLOB '*[^0-9]*' AND v.due<=unixepoch()", &[&a.id,&folder])?;
    for job in pending.as_array().unwrap() {
        let id = job["id"].as_i64().unwrap();
        let requested = job["destination"].as_str().unwrap();
        let uid = job["uid"].as_str().unwrap().parse::<u32>()?;
        let special = format!("\\{requested}");
        let encoded = mailbox_wire_name(requested);
        let special_folder=folders.iter().find(|f|matches!(requested,"Trash"|"Archive") &&
            f.attributes().iter().any(|attr|matches!(attr, imap::types::NameAttribute::Custom(name) if name.eq_ignore_ascii_case(&special))));
        let dest = special_folder
            .or_else(|| folders.iter().find(|f| f.name() == requested))
            .map(|f| f.name())
            .unwrap_or(&encoded);
        if dest != folder {
            let moved = (|| -> imap::error::Result<()> {
                if !folders.iter().any(|f| f.name() == dest)
                    && session
                        .list(Some(""), Some(dest))?
                        .iter()
                        .all(|f| f.name() != dest)
                {
                    session.create(dest)?;
                }
                session.uid_mv(uid.to_string(), dest)
            })();
            if let Err(error) = moved {
                if matches!(
                    error,
                    imap::error::Error::No(_)
                        | imap::error::Error::Bad(_)
                        | imap::error::Error::Validate(_)
                ) {
                    c.execute("DELETE FROM moves WHERE message=?", [id])?;
                }
                return Err(error).context("Сервер не подтвердил перемещение IMAP");
            }
        }
        let t = c.transaction()?;
        if dest != folder {
            t.execute(
                "UPDATE messages SET folder=?,uid=? WHERE id=?",
                params![dest, format!("moving:{id}"), id],
            )?;
        }
        t.execute("DELETE FROM moves WHERE message=?", [id])?;
        t.commit()?;
    }
    Ok(())
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
                "SELECT uid,seen FROM messages WHERE account=? AND folder=? AND pending_seen=1 AND uid NOT GLOB '*[^0-9]*'",
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
        move_selected(c, a, session, folder, &folders)?;
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
                let mid = mailparse::parse_mail(raw)?
                    .headers
                    .get_first_value("Message-ID")
                    .unwrap_or_default();
                let reused=t.execute("UPDATE messages SET uid=? WHERE id=(SELECT id FROM messages WHERE account=? AND folder=? AND uid LIKE 'moving:%' AND mid=? LIMIT 1)",params![uid,a.id,folder,mid])?;
                let id = mime::save(&t, &a.id, folder, &uid, raw, outgoing, seen)?;
                let arrival = old == Some(valid)
                    && folder.eq_ignore_ascii_case("INBOX")
                    && !outgoing
                    && reused == 0;
                let notify = arrival && crate::organize::arrived(&t, id, &a.id)?;
                t.commit()?;
                if notify && !seen {
                    count += 1;
                }
            }
        }
        let saved = c
            .prepare(
                "SELECT uid FROM messages WHERE account=? AND folder=? AND uid NOT LIKE 'local:%'",
            )?
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
            let id = mime::save(&t, &a.id, "INBOX", uid, &raw, false, false)?;
            let notify = initialized && crate::organize::arrived(&t, id, &a.id)?;
            t.execute("INSERT INTO pop_history VALUES(?,?)", params![a.id, uid])?;
            t.commit()?;
            if notify {
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
