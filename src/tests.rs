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
    book::migrate(&c)?;
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
fn imap_round(c: &mut Connection, a: &store::Account, validity: u32) -> Result<usize> {
    imap_round_with_archive(c, a, validity, false)
}

fn imap_round_with_archive(
    c: &mut Connection,
    a: &store::Account,
    validity: u32,
    archive: bool,
) -> Result<usize> {
    let (port, server) = fixture(move |mut s| {
        let mut moved = false;
        let mut in_archive = false;
        let mut flags = "";
        write!(s.get_mut(), "* OK IMAP ready\r\n")?;
        loop {
            let command = line(&mut s)?;
            let (tag, cmd) = command.split_once(' ').context("IMAP tag")?;
            let reply = if cmd.starts_with("LOGIN") {
                "".into()
            } else if cmd.starts_with("LIST") {
                let mut names = String::from("* LIST () \"/\" \"INBOX\"\r\n");
                if archive {
                    names.push_str("* LIST (\\Archive) \"/\" \"Saved\"\r\n");
                }
                names
            } else if cmd.starts_with("SELECT") {
                in_archive = cmd.contains("Saved");
                format!("* 1 EXISTS\r\n* OK [UIDVALIDITY {validity}] ready\r\n")
            } else if cmd == "UID MOVE 7 \"Saved\"" && archive && !in_archive {
                moved = true;
                String::new()
            } else if cmd.contains("BODY.PEEK") {
                format!(
                    "* 1 FETCH (UID {} BODY[] {{{}}}\r\n{})\r\n",
                    if in_archive { 19 } else { 7 },
                    LETTER.len(),
                    LETTER
                )
            } else if cmd == "UID STORE 7 +FLAGS.SILENT (\\Seen)" {
                flags = "\\Seen";
                String::new()
            } else if cmd == "UID STORE 7 -FLAGS.SILENT (\\Seen)" {
                flags = "";
                String::new()
            } else if cmd.starts_with("UID FETCH") {
                if in_archive == moved {
                    format!(
                        "* 1 FETCH (UID {} FLAGS ({flags}))\r\n",
                        if in_archive { 19 } else { 7 }
                    )
                } else {
                    String::new()
                }
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
    let count = network::imap_sync(c, a, &mut session)?;
    session.logout()?;
    server.join().unwrap()?;
    Ok(count)
}
#[test]
fn imap_sync_applies_both_local_read_states() -> Result<()> {
    let mut c = database()?;
    let a = account(&mut c)?;
    imap_round(&mut c, &a, 1)?;
    for seen in [true, false] {
        c.execute("UPDATE messages SET seen=?,pending_seen=1", [seen])?;
        imap_round(&mut c, &a, 1)?;
        let state: (bool, bool) =
            c.query_row("SELECT seen,pending_seen FROM messages", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
        assert_eq!(state, (seen, false));
    }
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
        html: String::new(),
        cc: String::new(),
        bcc: String::new(),
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
#[test]
fn group_and_personal_deliveries_use_real_envelopes_and_independent_retries() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let profile = store::register(dir.path(), Some("Работа"))?;
    let mut c = store::db(&profile.id)?;
    let a = account(&mut c)?;
    let to = "one@example.org, two@example.org, three@example.org, ONE@example.org";
    assert_eq!(delivery::enqueue(&mut c, compose(&a, to, false, None))?, 1);
    let group = smtp_round(&mut c, &pending(&store::db(&profile.id)?)?[0], false, false)?;
    assert_eq!(group.to.len(), 3);
    let m = mailparse::parse_mail(group.raw.as_bytes())?;
    assert_eq!(
        delivery::addresses(&m.headers.get_first_value("To").unwrap()).len(),
        3
    );
    assert_eq!(delivery::enqueue(&mut c, compose(&a, to, true, None))?, 3);
    let jobs = pending(&c)?;
    let mut ids = std::collections::HashSet::new();
    for (i, j) in jobs.iter().enumerate() {
        let received = smtp_round(&mut c, j, i == 1, i == 2)?;
        assert_eq!(received.to.len(), 1);
        if i != 1 {
            let m = mailparse::parse_mail(received.raw.as_bytes())?;
            let to = m.headers.get_first_value("To").unwrap();
            assert_eq!(delivery::addresses(&to).len(), 1);
            assert_eq!(received.to[0], format!("RCPT TO:<{to}>"));
            for other in ["one", "two", "three"].map(|n| format!("{n}@example.org")) {
                if other != to {
                    assert!(!received.raw.contains(&other))
                }
            }
            assert!(ids.insert(m.headers.get_first_value("Message-ID").unwrap()));
        }
    }
    let states = || -> Result<Vec<String>> {
        Ok(store::db(&profile.id)?
            .prepare("SELECT state FROM queue ORDER BY rowid")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    };
    assert_eq!(states()?, ["sent", "sent", "failed", "unknown"]);
    for j in [&jobs[0], &jobs[1]] {
        api::run("retry", &profile.id, serde_json::json!({"id":j}))?;
    }
    assert_eq!(pending(&c)?, [jobs[1].clone()]);
    let retried = smtp_round(&mut c, &jobs[1], false, false)?;
    assert_eq!(retried.to, ["RCPT TO:<two@example.org>"]);
    let m = mailparse::parse_mail(retried.raw.as_bytes())?;
    assert!(ids.insert(m.headers.get_first_value("Message-ID").unwrap()));
    assert_eq!(states()?, ["sent", "sent", "sent", "unknown"]);
    Ok(())
}

fn pop_round(c: &mut Connection, a: &store::Account, uidl: bool) -> Result<Vec<String>> {
    let (port, server) = fixture(move |mut s| {
        let mut cmds = vec![];
        loop {
            let cmd = line(&mut s)?;
            cmds.push(cmd.clone());
            let response = match cmd.as_str() {
                "UIDL" if !uidl => "-ERR unsupported\r\n".into(),
                "UIDL" => "+OK\r\n1 stable-id\r\n.\r\n".into(),
                "RETR 1" => format!("+OK\r\n{LETTER}.\r\n"),
                "DELE 1" | "QUIT" => "+OK done\r\n".into(),
                _ => anyhow::bail!("Unexpected {cmd}"),
            };
            s.get_mut().write_all(response.as_bytes())?;
            if cmd == "QUIT" || !uidl {
                break;
            }
        }
        Ok(cmds)
    })?;
    let stream = TcpStream::connect(("127.0.0.1", port))?;
    let res = network::pop_sync(c, a, &mut BufReader::new(stream));
    let cmds = server.join().unwrap()?;
    res?;
    Ok(cmds)
}
#[test]
fn pop_persists_uidl_before_deleting_server_copy() -> Result<()> {
    let root = tempfile::tempdir()?;
    let profile = store::register(root.path(), Some("Личное"))?;
    let mut c = store::db(&profile.id)?;
    let mut a = account(&mut c)?;
    assert!(pop_round(&mut c, &a, false)
        .unwrap_err()
        .to_string()
        .contains("UIDL"));
    assert_eq!(pop_round(&mut c, &a, true)?, ["UIDL", "RETR 1", "QUIT"]);
    drop(c);
    let mut c = store::db(&profile.id)?;
    assert_eq!(pop_round(&mut c, &a, true)?, ["UIDL", "QUIT"]);
    a.keep = false;
    assert_eq!(pop_round(&mut c, &a, true)?, ["UIDL", "DELE 1", "QUIT"]);
    Ok(())
}
#[test]
fn portable_profiles_isolate_accounts_threads_contacts_drafts_and_search() -> Result<()> {
    use serde_json::json;
    let root = tempfile::tempdir()?;
    let work = store::register(&root.path().join("Моя почта"), Some("Работа"))?;
    let home = store::register(&root.path().join("Личное"), Some("Личное"))?;
    let mut c = store::db(&work.id)?;
    let mut h = store::db(&home.id)?;
    let a = account(&mut c)?;
    account(&mut h)?;
    let mut other = a.clone();
    other.id = String::new();
    other.address = "support@example.org".into();
    other.login = "support".into();
    other.signature = "Support desk".into();
    store::save_account(&mut c, &mut other)?;
    other.id = String::new();
    other.incoming.host = "other.example.org".into();
    store::save_account(&mut c, &mut other)?;
    assert_eq!(store::accounts(&c)?.len(), 3);
    assert_eq!(store::accounts(&h)?.len(), 1);
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM servers", [], |r| r.get::<_, usize>(0))?,
        3
    );
    let t = c.transaction()?;
    let first = mime::save(&t, &a.id, "INBOX", "7", LETTER.as_bytes(), false, false)?;
    let unrelated = LETTER
        .replace("invoice@example.org", "new@example.org")
        .replace("Invoice", "Other");
    mime::save(
        &t,
        &a.id,
        "Archive",
        "8",
        unrelated.as_bytes(),
        false,
        false,
    )?;
    t.commit()?;
    assert_eq!(
        api::run("list", &work.id, json!({}))?
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        api::run("reply", &work.id, json!({"id":first,"all":true}))?,
        json!("support@example.org")
    );
    for (op, args) in [
        (
            "contact",
            json!({"name":"Алиса","emails":"a@example.org, b@example.org"}),
        ),
        (
            "save_draft",
            json!({"slot":"new","data":{"body":"черновик"}}),
        ),
    ] {
        api::run(op, &work.id, args)?;
    }
    assert_eq!(
        api::run("draft", &home.id, json!({"slot":"new"}))?,
        serde_json::Value::Null
    );
    assert_eq!(api::run("contacts", &home.id, json!({}))?, json!([]));
    assert_eq!(
        api::run("list", &home.id, json!({"q":"привет"}))?,
        json!([])
    );
    api::run("delete", &work.id, json!({"id":first}))?;
    assert_eq!(
        api::run("list", &work.id, json!({"folder":"INBOX","q":"invoice"}))?,
        json!([])
    );
    imap_round(&mut c, &a, 2)?;
    assert_eq!(
        api::run("list", &work.id, json!({"folder":"INBOX"}))?,
        json!([])
    );
    drop(c);
    let moved = root.path().join("Перенесённый профиль");
    std::fs::rename(work.path, &moved)?;
    let opened = store::register(&moved, None)?;
    assert_eq!(work.id, opened.id);
    assert_eq!(store::profiles()?.len(), 2);
    assert_eq!(
        api::run("draft", &opened.id, json!({"slot":"new"}))?,
        json!({"body":"черновик"})
    );
    assert_eq!(
        api::run("list", &opened.id, json!({"q":"привет"}))?
            .as_array()
            .unwrap()
            .len(),
        1
    );
    Ok(())
}
#[test]
fn html_attachment_and_filesystem_names() -> Result<()> {
    let mut c = database()?;
    let t = c.transaction()?;
    let raw=b"From: a@example.org\r\nMessage-ID: <a@example.org>\r\nContent-Type: multipart/related; boundary=demo\r\n\r\n--demo\r\nContent-Type: text/html\r\n\r\n<p>hello</p><script>bad()</script><img src=\"cid:face\"><img src=\"https://example.org/x\">\r\n--demo\r\nContent-Type: image/png\r\nContent-ID: <face>\r\nContent-Disposition: inline; filename=\"../../x.png\"\r\n\r\nabc\r\n--demo--\r\n";
    mime::save(&t, "box", "INBOX", "1", raw, false, false)?;
    t.commit()?;
    let html: String = c.query_row("SELECT html FROM messages", [], |r| r.get(0))?;
    assert!(!html.contains("script"));
    assert!(html.contains("cid:face"));
    let name: String = c.query_row("SELECT name FROM attachments", [], |r| r.get(0))?;
    assert_eq!(name, "x.png");
    assert_eq!(mime::safe_name("..\\каталог/счёт"), "счёт");
    assert_eq!(mime::safe_name("CON.txt"), "attachment_CON.txt");
    Ok(())
}

#[test]
fn unread_notifications_start_after_initial_inbox_and_skip_duplicates() -> Result<()> {
    let mut c = database()?;
    let a = account(&mut c)?;
    assert_eq!(imap_round(&mut c, &a, 1)?, 0);
    // Reuse the fixture's single UID as an arrival in an initialized empty inbox.
    c.execute("DELETE FROM messages", [])?;
    assert_eq!(imap_round(&mut c, &a, 1)?, 1);
    assert_eq!(imap_round(&mut c, &a, 1)?, 0);
    assert_eq!(imap_round(&mut c, &a, 2)?, 0);
    Ok(())
}
#[test]
fn copies_and_blind_copies_have_separate_headers_and_envelope() -> Result<()> {
    let mut c = database()?;
    let a = account(&mut c)?;
    for personal in [false, true] {
        let mut d = compose(&a, "one@example.org", personal, None);
        d.cc = "two@example.org, ONE@example.org".into();
        d.bcc = "three@example.org".into();
        delivery::enqueue(&mut c, d)?;
        for id in pending(&c)? {
            let captured = smtp_round(&mut c, &id, false, false)?;
            let m = mailparse::parse_mail(captured.raw.as_bytes())?;
            assert!(m.headers.get_first_value("Bcc").is_none());
            if personal {
                assert_eq!(captured.to.len(), 1);
                assert!(m.headers.get_first_value("Cc").is_none());
                assert_eq!(
                    captured.to[0],
                    format!("RCPT TO:<{}>", m.headers.get_first_value("To").unwrap())
                );
            } else {
                assert_eq!(captured.to.len(), 3);
                assert_eq!(
                    m.headers.get_first_value("Cc").as_deref(),
                    Some("two@example.org")
                );
                assert!(!captured.raw.contains("three@example.org"));
            }
        }
    }
    Ok(())
}
#[test]
fn book_upgrade_preserves_legacy_rows_and_groups_on_edit() -> Result<()> {
    let mut c = Connection::open_in_memory()?;
    c.execute_batch(store::SCHEMA)?;
    c.execute_batch("PRAGMA foreign_keys=ON;INSERT INTO contacts VALUES('person','Имя','a@example.org, b@example.org');INSERT INTO drafts VALUES('new','{}');")?;
    book::migrate(&c)?;
    book::migrate(&c)?;
    assert_eq!(book::list(&c)?[0]["emails"], "a@example.org, b@example.org");
    book::save_group(
        &mut c,
        &serde_json::json!({"name":"Команда","members":["person"]}),
    )?;
    book::save(
        &c,
        &serde_json::json!({"id":"person","name":"Имя","emails":"b@example.org, a@example.org","labels":{"b@example.org":"Рабочий"},"favorite":true}),
    )?;
    assert_eq!(book::groups(&c)?[0]["members"][0], "person");
    assert_eq!(book::list(&c)?[0]["favorite"], 1);
    assert_eq!(
        book::import(
            &mut c,
            &serde_json::json!([{"name":"Дубликат","emails":"A@example.org"},{"name":"Новый","emails":"other@example.org"}])
        )?,
        1
    );
    Ok(())
}
#[test]
fn saved_views_and_contact_history_match_exact_addresses_across_mailboxes() -> Result<()> {
    let mut c = database()?;
    let person = book::save(
        &c,
        &serde_json::json!({"name":"Alice","emails":"alice@example.org, personal@example.org"}),
    )?;
    let t = c.transaction()?;
    let first = mime::save(&t, "work", "INBOX", "1", LETTER.as_bytes(), false, false)?;
    let second = LETTER
        .replace("alice@example.org", "personal@example.org")
        .replace("<invoice@example.org>", "<other@example.org>");
    mime::save(&t, "home", "INBOX", "1", second.as_bytes(), false, true)?;
    let unrelated = LETTER
        .replace("alice@example.org", "notalice@example.org")
        .replace("<invoice@example.org>", "<third@example.org>");
    mime::save(&t, "home", "INBOX", "3", unrelated.as_bytes(), false, true)?;
    t.commit()?;
    c.execute("INSERT INTO stars VALUES(?)", [first])?;
    assert_eq!(book::history(&c, &person)?.as_array().unwrap().len(), 2);
    for view in ["unread", "starred"] {
        let rows = api::list(&c, &serde_json::json!({"view":view}))?;
        assert_eq!(rows.as_array().unwrap().len(), 1);
        assert_eq!(rows[0]["id"], first);
    }
    assert!(api::list(&c, &serde_json::json!({"view":"files"}))?
        .as_array()
        .unwrap()
        .is_empty());
    Ok(())
}
#[test]
fn expanding_thread_preserves_unread_and_only_opened_message_changes() -> Result<()> {
    use serde_json::json;
    let root = tempfile::tempdir()?;
    let p = store::register(root.path(), Some("Ветки"))?;
    let mut c = store::db(&p.id)?;
    let t = c.transaction()?;
    let first = mime::save(&t, "box", "INBOX", "1", LETTER.as_bytes(), false, false)?;
    let reply = LETTER.replace(
        "Message-ID: <invoice@example.org>",
        "Message-ID: <reply@example.org>\r\nIn-Reply-To: <invoice@example.org>",
    );
    let second = mime::save(&t, "box", "INBOX", "2", reply.as_bytes(), false, false)?;
    t.commit()?;
    let list = api::list(&c, &json!({"grouped":true}))?;
    let key = &list[0]["thread"];
    assert_eq!(list[0]["total"], 2);
    assert_eq!(list[0]["unread"], 2);
    let branch = api::list(&c, &json!({"branch":key}))?;
    assert_eq!(branch[0]["id"], first);
    assert_eq!(branch[1]["id"], second);
    api::run("thread", &p.id, json!({"thread":key,"hit":second}))?;
    assert_eq!(api::list(&c, &json!({}))?[0]["unread"], 1);
    api::run(
        "mark_seen",
        &p.id,
        json!({"thread":key,"ids":[second],"seen":false}),
    )?;
    assert_eq!(api::list(&c, &json!({}))?[0]["unread"], 2);
    Ok(())
}

#[test]
fn organize_messages_and_undo_before_delivery() -> Result<()> {
    use serde_json::json;
    let root = tempfile::tempdir()?;
    let p = store::register(root.path(), Some("Mail"))?;
    let mut c = store::db(&p.id)?;
    let mut a = account(&mut c)?;
    a.incoming.proto = "pop3".into();
    c.execute("UPDATE servers SET proto='pop3' WHERE proto='imap'", [])?;
    let run = |op, args| api::run(op, &p.id, args);
    let t = c.transaction()?;
    let id = mime::save(&t, &a.id, "INBOX", "uid1", LETTER.as_bytes(), false, false)?;
    t.execute(
        "INSERT INTO attachments(message,name,data) VALUES(?,'report.pdf',X'01')",
        [id],
    )?;
    t.commit()?;
    let found = run("list", json!({"q":"от:Alice файл:pdf"}))?;
    assert_eq!(found[0]["id"], id);
    run("save_search", json!({"q":"от:Alice файл:pdf"}))?;
    assert_eq!(
        run("saved_searches", json!({}))?[0]["query"],
        "от:Alice файл:pdf"
    );
    run(
        "bulk",
        json!({"action":"snooze","until":store::now()+3600,"ids":[id]}),
    )?;
    assert_eq!(run("list", json!({}))?, json!([]));
    assert_eq!(run("list", json!({"view":"snoozed"}))?[0]["id"], id);
    run(
        "save_rule",
        json!({"pattern":"example.org","label":"Проект","folder":"Orders","quiet":true}),
    )?;
    assert!(!organize::arrived(&c, id, &a.id)?);
    assert_eq!(
        run("list", json!({"q":"метка:Проект"}))?[0]["folder"],
        "Orders"
    );
    run("apply_moves", json!({}))?;
    assert_eq!(run("folders", json!({}))?[0]["unread"], 1);
    let batch = run("bulk", json!({"action":"move","folder":"Trash","ids":[id]}))?;
    run("undo_move", json!({"batch":batch}))?;
    assert_eq!(run("list", json!({}))?[0]["folder"], "Orders");
    let file = root.path().join("report.txt");
    std::fs::write(&file, "attachment")?;
    let draft = json!({"slot":"new:test","to":"a@example.org,b@example.org","personal":true,"account":a.id,"subject":"Hello","body":"Text","files":[file]});
    let batch = run("enqueue", draft.clone())?;
    run("deliver", json!({}))?;
    let jobs = run("queue", json!({}))?;
    assert_eq!(jobs.as_array().unwrap().len(), 2);
    assert!(jobs
        .as_array()
        .unwrap()
        .iter()
        .all(|j| j["state"] == "held"));
    assert_eq!(run("cancel_send", json!({"batch":batch}))?, draft);
    assert_eq!(run("queue", json!({}))?, json!([]));
    assert_eq!(run("draft", json!({"slot":"new:test"}))?, draft);
    Ok(())
}

#[test]
fn named_html_links_keep_http_target_and_strip_script_attributes() -> Result<()> {
    let raw=b"Message-ID: <x@y>\r\nContent-Type: text/html\r\n\r\n<a href=\"https://example.org/doc\" onclick=\"evil()\">Report</a>";
    let html = mime::original_html(raw, "<x@y>")?;
    assert!(html.contains("https://example.org/doc"));
    assert!(!html.contains("onclick"));
    Ok(())
}

#[test]
fn imap_archive_tracks_new_uid_and_keeps_message_star() -> Result<()> {
    let mut c = database()?;
    let a = account(&mut c)?;
    imap_round(&mut c, &a, 1)?;
    let id: i64 = c.query_row("SELECT id FROM messages", [], |r| r.get(0))?;
    c.execute("INSERT INTO stars VALUES(?)", [id])?;
    c.execute("INSERT INTO moves VALUES(?,'Archive','batch',0)", [id])?;
    imap_round_with_archive(&mut c, &a, 1, true)?;
    let result:(String,String,bool)=c.query_row("SELECT folder,uid,EXISTS(SELECT 1 FROM stars WHERE message=messages.id) FROM messages WHERE id=?",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    assert_eq!(result, ("Saved".into(), "19".into(), true));
    Ok(())
}

#[test]
fn idle_receives_inbox_arrival_and_falls_back_without_capability() -> Result<()> {
    for supported in [true, false] {
        let (port, server) = fixture(move |mut stream| {
            write!(stream.get_mut(), "* OK ready\r\n")?;
            let mut messages = 0;
            let mut idle_tag = String::new();
            loop {
                let command = line(&mut stream)?;
                if command == "DONE" {
                    write!(stream.get_mut(), "{idle_tag} OK idle complete\r\n")?;
                    continue;
                }
                let (tag, cmd) = command.split_once(' ').context("IMAP tag")?;
                if cmd.starts_with("LOGIN") {
                    write!(stream.get_mut(), "{tag} OK logged in\r\n")?;
                } else if cmd == "CAPABILITY" {
                    let idle = if supported { " IDLE" } else { "" };
                    write!(
                        stream.get_mut(),
                        "* CAPABILITY IMAP4rev1{idle}\r\n{tag} OK capabilities\r\n"
                    )?;
                } else if cmd.starts_with("EXAMINE") && supported {
                    write!(
                        stream.get_mut(),
                        "* {messages} EXISTS\r\n{tag} OK selected\r\n"
                    )?;
                } else if cmd == "IDLE" && supported {
                    if messages > 0 {
                        break;
                    }
                    messages = 1;
                    idle_tag = tag.to_string();
                    write!(stream.get_mut(), "+ waiting\r\n* 1 EXISTS\r\n")?;
                } else if cmd == "LOGOUT" {
                    write!(stream.get_mut(), "* BYE logout\r\n{tag} OK done\r\n")?;
                    break;
                } else {
                    anyhow::bail!("Unexpected command {cmd}");
                }
            }
            Ok(())
        })?;
        let mut client = imap::Client::new(TcpStream::connect(("127.0.0.1", port))?);
        client.read_greeting()?;
        let mut session = client.login("test", "fixture").map_err(|e| e.0)?;
        let mut arrivals = 0;
        let result = network::watch_inbox(&mut session, false, || arrivals += 1);
        assert_eq!(arrivals, usize::from(supported));
        assert_eq!(result.is_err(), supported); // The IDLE fixture disconnects after the first arrival.
        server.join().unwrap()?;
    }
    Ok(())
}

#[test]
fn sent_folder_uses_special_use_and_appends_only_once() -> Result<()> {
    let (port, server) = fixture(|mut stream| {
        use std::io::Read;
        let mut appended = false;
        write!(stream.get_mut(), "* OK ready\r\n")?;
        loop {
            let command = line(&mut stream)?;
            let (tag, cmd) = command.split_once(' ').context("IMAP tag")?;
            let reply = if cmd.starts_with("LOGIN") {
                ""
            } else if cmd.starts_with("LIST") {
                "* LIST (\\Sent) \"/\" \"Outgoing\"\r\n"
            } else if cmd == "SELECT \"Outgoing\"" {
                "* 1 EXISTS\r\n"
            } else if cmd == "UID SEARCH HEADER Message-ID \"<invoice@example.org>\"" {
                if appended {
                    "* SEARCH 9\r\n"
                } else {
                    "* SEARCH\r\n"
                }
            } else if cmd.starts_with("APPEND \"Outgoing\"") {
                assert!(!appended && cmd.contains("\\Seen"));
                write!(stream.get_mut(), "+ proceed\r\n")?;
                let mut raw = vec![0; LETTER.len()];
                stream.read_exact(&mut raw)?;
                assert_eq!(raw, LETTER.as_bytes());
                assert!(line(&mut stream)?.is_empty());
                appended = true;
                ""
            } else if cmd == "LOGOUT" {
                write!(stream.get_mut(), "* BYE closing\r\n{tag} OK done\r\n")?;
                break;
            } else {
                anyhow::bail!("Unexpected {cmd}")
            };
            write!(stream.get_mut(), "{reply}{tag} OK done\r\n")?;
        }
        Ok(appended)
    })?;
    let mut client = imap::Client::new(TcpStream::connect(("127.0.0.1", port))?);
    client.read_greeting()?;
    let mut session = client.login("test", "fixture").map_err(|e| e.0)?;
    for _ in 0..2 {
        assert_eq!(
            network::append_sent(&mut session, LETTER.as_bytes())?,
            "Outgoing"
        );
    }
    session.logout()?;
    assert!(server.join().unwrap()?);
    Ok(())
}

#[test]
fn rich_message_preserves_both_bodies_and_attachment() -> Result<()> {
    let mut c = database()?;
    let a = account(&mut c)?;
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("note.txt");
    std::fs::write(&file, "attachment")?;
    let mut draft = compose(&a, "reader@example.org", false, None);
    draft.html =
        "<ol><li><b>Ответ</b> <i>да</i> <a href=\"https://example.org\">ссылка</a></li></ol>"
            .into();
    draft.files.push(file);
    delivery::enqueue(&mut c, draft)?;
    let raw: Vec<u8> = c.query_row("SELECT raw FROM queue", [], |r| r.get(0))?;
    let mail = mailparse::parse_mail(&raw)?;
    assert_eq!(mail.ctype.mimetype, "multipart/mixed");
    assert_eq!(mail.subparts[0].ctype.mimetype, "multipart/alternative");
    let bodies = &mail.subparts[0].subparts;
    assert!(bodies[0].get_body()?.contains("Ответ"));
    let html = bodies[1].get_body()?;
    for fragment in [
        "<ol>",
        "<b>Ответ</b>",
        "<i>да</i>",
        "https://example.org",
        "Работа",
    ] {
        assert!(html.contains(fragment));
    }
    assert_eq!(mail.subparts[1].get_body_raw()?, b"attachment");
    Ok(())
}
