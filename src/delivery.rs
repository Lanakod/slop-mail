use crate::{
    mime, network,
    store::{self, Account},
};
use anyhow::{ensure, Context, Result};
use lettre::{
    address::Envelope,
    message::{header::ContentType, Attachment, Mailbox, MultiPart, SinglePart},
    Message, Transport,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use std::{collections::HashSet, path::PathBuf};
pub fn addresses(input: &str) -> Vec<String> {
    let Ok(parsed) = mailparse::addrparse(input) else {
        return vec![];
    };
    let mut out = vec![];
    for entry in parsed.iter() {
        match entry {
            mailparse::MailAddr::Single(s) => out.push(s.addr.clone()),
            mailparse::MailAddr::Group(g) => out.extend(g.addrs.iter().map(|s| s.addr.clone())),
        }
    }
    out
}
#[derive(Deserialize)]
pub struct Compose {
    pub account: String,
    pub to: String,
    #[serde(default)]
    pub cc: String,
    #[serde(default)]
    pub bcc: String,
    pub subject: String,
    pub body: String,
    #[serde(default)]
    pub signature: String,
    pub personal: bool,
    pub parent: Option<i64>,
    #[serde(default)]
    pub files: Vec<PathBuf>,
    pub slot: String,
}
pub fn recipients(s: &str) -> Result<Vec<lettre::Address>> {
    let mut seen = HashSet::new();
    let mut to = vec![];
    for a in s.parse::<lettre::message::Mailboxes>()? {
        let addr = a.email;
        if seen.insert(addr.to_string().to_lowercase()) {
            to.push(addr)
        }
    }
    ensure!(!to.is_empty(), "Укажите получателей");
    Ok(to)
}
pub fn fields(to: &str, cc: &str, bcc: &str) -> Result<Vec<(&'static str, lettre::Address)>> {
    let mut seen = HashSet::new();
    let mut result = vec![];
    for (field, text) in [("to", to), ("cc", cc), ("bcc", bcc)] {
        if !text.trim().is_empty() {
            for addr in recipients(text)? {
                if seen.insert(addr.to_string().to_lowercase()) {
                    result.push((field, addr))
                }
            }
        }
    }
    ensure!(!result.is_empty(), "Укажите получателей");
    Ok(result)
}
pub fn enqueue(c: &mut Connection, d: Compose) -> Result<usize> {
    let a = store::account(c, &d.account)?;
    let to = fields(&d.to, &d.cc, &d.bcc)?;
    let groups = if d.personal {
        to.into_iter().map(|(_, addr)| vec![("to", addr)]).collect()
    } else {
        vec![to]
    };
    let parent = if let Some(parent) = d.parent {
        Some(
            c.query_row("SELECT mid,refs FROM messages WHERE id=?", [parent], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?,
        )
    } else {
        None
    };
    let mut files = vec![];
    for path in d.files {
        files.push((
            mime::safe_name(
                &path
                    .file_name()
                    .context("Нет имени файла")?
                    .to_string_lossy(),
            ),
            std::fs::read(path)?,
        ));
    }
    let t = c.transaction()?;
    let count = groups.len();
    let batch = store::id();
    for group in groups {
        let mut builder = Message::builder()
            .message_id(Some(format!(
                "<{}@{}>",
                store::id(),
                a.address
                    .split('@')
                    .next_back()
                    .context("Адрес отправителя")?
            )))
            .from(Mailbox::new(Some(a.name.clone()), a.address.parse()?))
            .subject(&d.subject);
        for (field, addr) in &group {
            let m = Mailbox::new(None, addr.clone());
            builder = match *field {
                "cc" => builder.cc(m),
                "bcc" => builder.bcc(m),
                _ => builder.to(m),
            };
        }
        if let Some((mid, refs)) = &parent {
            builder = builder.in_reply_to(mid.clone()).references(refs.clone())
        }
        let text = if d.signature.is_empty() {
            d.body.clone()
        } else {
            format!("{}\r\n\r\n-- \r\n{}", d.body, d.signature)
        };
        let message = if files.is_empty() {
            builder.singlepart(SinglePart::plain(text))?
        } else {
            let mut mp = MultiPart::mixed().singlepart(SinglePart::plain(text));
            for (n, b) in &files {
                mp = mp.singlepart(
                    Attachment::new(n.clone())
                        .body(b.clone(), ContentType::parse("application/octet-stream")?),
                );
            }
            builder.multipart(mp)?
        };
        let recipients = group
            .iter()
            .map(|(_, addr)| addr.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        t.execute(
            "INSERT INTO queue VALUES(?,?,?,?,'pending','',?,?)",
            params![
                store::id(),
                a.id,
                recipients,
                message.formatted(),
                store::now(),
                batch
            ],
        )?;
    }
    t.execute("DELETE FROM drafts WHERE slot=?", [d.slot])?;
    t.commit()?;
    Ok(count)
}
pub fn dispatch(c: &mut Connection, profile: &str) -> Result<()> {
    // The OS releases this lock even if the process exits during SMTP DATA.
    let path =
        std::path::Path::new(c.path().context("Нет пути профиля")?).with_extension("send-lock");
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    match lock.try_lock() {
        Ok(()) => (),
        Err(std::fs::TryLockError::WouldBlock) => return Ok(()),
        Err(std::fs::TryLockError::Error(e)) => return Err(e.into()),
    }
    c.execute("UPDATE queue SET state='unknown',error='Приложение закрылось до сохранения подтверждения' WHERE state='sending'",[])?;
    let jobs = c
        .prepare("SELECT id FROM queue WHERE state='pending' ORDER BY rowid")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for id in jobs {
        send_one(c, &id, |a, env, raw| {
            let pw =
                network::secret(profile, a, "out").map_err(|e| ("failed", format!("{e:#}")))?;
            let transport = network::smtp(a, &pw).map_err(|e| ("failed", e.to_string()))?;
            transport
                .send_raw(env, raw)
                .map(|_| ())
                .map_err(smtp_failure)
        })?;
    }
    Ok(())
}
pub fn smtp_failure(e: lettre::transport::smtp::Error) -> (&'static str, String) {
    let state = if e.is_transient() || e.is_permanent() || e.is_tls() {
        "failed"
    } else {
        "unknown"
    };
    (state, e.to_string())
}
pub fn send_one<F>(c: &mut Connection, id: &str, send: F) -> Result<()>
where
    F: FnOnce(&Account, &Envelope, &[u8]) -> std::result::Result<(), (&'static str, String)>,
{
    let job = c
        .query_row(
            "SELECT account,recipients,raw FROM queue WHERE id=? AND state='pending'",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((account, to, raw)) = job else {
        return Ok(());
    };
    let a = store::account(c, &account)?;
    let env = Envelope::new(Some(a.address.parse()?), recipients(&to)?)?;
    if c.execute(
        "UPDATE queue SET state='sending' WHERE id=? AND state='pending'",
        [id],
    )? == 0
    {
        return Ok(());
    }
    let result = send(&a, &env, &raw);
    let t = c.transaction()?;
    let (state, error) = match result {
        Ok(()) => {
            mime::save(&t, &a.id, "Отправленные (локально)", id, &raw, true, true)?;
            ("sent", String::new())
        }
        Err(e) => e,
    };
    t.execute(
        "UPDATE queue SET state=?,error=? WHERE id=?",
        params![state, error, id],
    )?;
    t.commit()?;
    Ok(())
}
