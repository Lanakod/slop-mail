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
    pub html: String,
    #[serde(default)]
    pub signature: String,
    pub personal: bool,
    pub parent: Option<i64>,
    #[serde(default)]
    pub files: Vec<PathBuf>,
    #[serde(default)]
    pub forwarded: Vec<i64>,
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
#[cfg(test)]
pub fn enqueue(c: &mut Connection, d: Compose) -> Result<usize> {
    enqueue_batch(c, d, &store::id(), None)
}
pub fn enqueue_batch(
    c: &mut Connection,
    d: Compose,
    batch: &str,
    draft: Option<&serde_json::Value>,
) -> Result<usize> {
    let a = store::account(c, &d.account)?;
    let to = fields(&d.to, &d.cc, &d.bcc)?;
    let groups = if d.personal {
        to.into_iter().map(|(_, addr)| vec![("to", addr)]).collect()
    } else {
        vec![to]
    };
    let parent = if let Some(parent) = d.parent {
        c.query_row("SELECT mid,refs FROM messages WHERE id=?", [parent], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .optional()?
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
    for id in d.forwarded {
        let (name, data): (String, Vec<u8>) = c
            .query_row("SELECT name,data FROM attachments WHERE id=?", [id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .context("Исходное вложение больше не доступно")?;
        files.push((mime::safe_name(&name), data));
    }
    let t = c.transaction()?;
    let count = groups.len();
    if let Some(data) = draft {
        t.execute(
            "INSERT INTO send_holds VALUES(?,unixepoch()+10,?)",
            params![batch, data.to_string()],
        )?;
    }
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
        let rich = if d.html.is_empty() {
            None
        } else {
            let signature = ammonia::clean_text(&d.signature);
            let html = ammonia::clean(&d.html);
            let html = if signature.is_empty() {
                html
            } else {
                format!("{html}<pre>-- \n{signature}</pre>")
            };
            Some(
                MultiPart::alternative()
                    .singlepart(SinglePart::plain(text.clone()))
                    .singlepart(SinglePart::html(html)),
            )
        };
        let message = if files.is_empty() {
            match rich {
                Some(part) => builder.multipart(part)?,
                None => builder.singlepart(SinglePart::plain(text))?,
            }
        } else {
            let mut mp = match rich {
                Some(part) => MultiPart::mixed().multipart(part),
                None => MultiPart::mixed().singlepart(SinglePart::plain(text)),
            };
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
            "INSERT INTO queue VALUES(?,?,?,?,?,'',?,?)",
            params![
                store::id(),
                a.id,
                recipients,
                message.formatted(),
                if draft.is_some() { "held" } else { "pending" },
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
    let t = c.transaction()?;
    t.execute("UPDATE queue SET state='pending' WHERE state='held' AND batch IN (SELECT batch FROM send_holds WHERE due<=unixepoch())",[])?;
    t.execute("DELETE FROM send_holds WHERE due<=unixepoch()", [])?;
    t.commit()?;
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
    let pending = store::rows(c, "SELECT job FROM sent_copies WHERE folder IS NULL", &[])?;
    for row in pending.as_array().unwrap() {
        let job = row["job"].as_str().unwrap();
        copy_sent(c, profile, job)?;
    }
    Ok(())
}
pub fn copy_sent(c: &Connection, profile: &str, job: &str) -> Result<()> {
    if !c.query_row(
        "SELECT EXISTS(SELECT 1 FROM sent_copies WHERE job=? AND folder IS NULL)",
        [job],
        |r| r.get::<_, bool>(0),
    )? {
        return Ok(());
    }
    let (account, raw): (String, Vec<u8>) =
        c.query_row("SELECT account,raw FROM queue WHERE id=?", [job], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
    let a = store::account(c, &account)?;
    match network::save_sent(profile, &a, &raw) {
        Ok(folder) => {
            c.execute(
                "UPDATE sent_copies SET folder=?,error='' WHERE job=?",
                params![folder, job],
            )?;
        }
        Err(error) => {
            c.execute(
                "UPDATE sent_copies SET error=? WHERE job=?",
                params![error.to_string(), job],
            )?;
        }
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
            if a.incoming.proto == "imap" {
                t.execute("INSERT INTO sent_copies(job) VALUES(?)", [id])?;
            }
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

pub fn cancel(c: &mut Connection, batch: &str) -> Result<serde_json::Value> {
    use serde_json::{json, Value};
    let t = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let data: String = t
        .query_row(
            "SELECT draft FROM send_holds WHERE batch=? AND due>unixepoch()",
            [batch],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("Время отмены истекло"))?;
    let mut d: Value = serde_json::from_str(&data)?;
    let slot = d["slot"].as_str().unwrap_or("");
    let occupied: bool = t.query_row(
        "SELECT EXISTS(SELECT 1 FROM drafts WHERE slot=?)",
        [slot],
        |r| r.get(0),
    )?;
    if occupied {
        d["slot"] = json!(format!("new:{}", store::id()));
    }
    t.execute(
        "INSERT INTO drafts(slot,data,modified) VALUES(?,?,unixepoch())",
        params![d["slot"].as_str(), d.to_string()],
    )?;
    t.execute("DELETE FROM queue WHERE batch=? AND state='held'", [batch])?;
    t.execute("DELETE FROM send_holds WHERE batch=?", [batch])?;
    t.commit()?;
    Ok(d)
}
