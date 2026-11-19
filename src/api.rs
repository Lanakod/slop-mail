use crate::{
    delivery, mime, network,
    store::{self, Account},
};
use anyhow::{bail, ensure, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Mutex};
static SYNC: Mutex<()> = Mutex::new(());
static DELIVERY: Mutex<()> = Mutex::new(());
#[tauri::command]
pub async fn request(op: String, profile: String, args: Value) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || run(&op, &profile, args))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))
}
pub(crate) fn run(op: &str, pid: &str, a: Value) -> Result<Value> {
    let s = |k: &str| a[k].as_str().unwrap_or("");
    let n = |k: &str| a[k].as_i64().unwrap_or(0);
    match op {
        "confirm" => {
            return Ok(json!(
                rfd::MessageDialog::new()
                    .set_title("Slop Mail")
                    .set_description(s("text"))
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .show()
                    == rfd::MessageDialogResult::Yes
            ))
        }
        "profiles" => return Ok(json!(store::profiles()?)),
        "profile" => {
            let new = a["new"] == true;
            let path = if new && a["default"] == true {
                store::app_dir()?.join("Profiles").join(store::id())
            } else {
                let Some(p) = rfd::FileDialog::new().pick_folder() else {
                    return Ok(Value::Null);
                };
                p
            };
            return Ok(json!(store::register(
                &path,
                if new { Some(s("name")) } else { None }
            )?));
        }
        "pick" => {
            return Ok(json!(rfd::FileDialog::new()
                .pick_files()
                .unwrap_or_default()))
        }
        _ => (),
    }
    let mut c = store::db(pid)?;
    match op {
        "accounts" => Ok(json!(store::accounts(&c)?)),
        "account" | "test" => {
            let mut acc: Account = serde_json::from_value(a["account"].clone())?;
            if op == "test" {
                let side = s("side");
                let mut pw = s(side).to_string();
                if pw.is_empty() {
                    pw = network::secret(pid, &acc, side, None)?;
                }
                network::test(&acc, side, &pw)?;
            } else {
                if acc.id.is_empty(){acc.id=store::id();}
                for side in ["in", "out"] {
                    if !s(side).is_empty() {
                        network::secret(pid, &acc, side, Some(s(side)))?;
                    }
                }
                store::save_account(&mut c,&mut acc)?;
            }
            Ok(json!(acc.id))
        }
        "sync" => {
            let _guard = SYNC.lock().map_err(|_| anyhow::anyhow!("Ошибка синхронизации"))?;
            let acc = store::account(&c, s("account"))?;
            let pw = network::secret(pid, &acc, "in", None)?;
            network::sync(&mut c, &acc, &pw)?;
            Ok(Value::Null)
        }
        "folders" => store::rows(&c, "SELECT account,name FROM folders UNION SELECT DISTINCT account,folder FROM messages ORDER BY 1,2", &[]),
        "list" => list(&c, &a),
        "thread" => {
            let t = c.transaction()?;
            t.execute("UPDATE messages SET seen=1,pending_seen=1 WHERE thread=? AND seen=0", [s("thread")])?;
            let data = store::rows(&t, "SELECT id,account,folder,thread,stamp,subject,sender,recipients,reply_to,body,html,outgoing FROM messages WHERE thread=? ORDER BY stamp,id", &[&s("thread")])?;
            t.commit()?;
            Ok(data)
        }
        "reply" => {
            let (sender, to, reply, outgoing): (String, String, String, bool) = c.query_row("SELECT sender,recipients,reply_to,outgoing FROM messages WHERE id=?", [n("id")], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            let own = store::accounts(&c)?.into_iter().map(|a| a.address.to_lowercase()).collect::<Vec<_>>();
            let mut addrs = delivery::addresses(if outgoing {
                &to
            } else if reply.is_empty() {
                &sender
            } else {
                &reply
            });
            if a["all"] == true {
                addrs.extend(delivery::addresses(&to).into_iter().filter(|x| !own.contains(&x.to_lowercase())))
            }
            let mut seen = std::collections::HashSet::new();
            addrs.retain(|x| seen.insert(x.to_lowercase()));
            Ok(json!(addrs.join(", ")))
        }
        "draft" => Ok(c.query_row("SELECT data FROM drafts WHERE slot=?", [s("slot")], |r| r.get::<_, String>(0)).optional()?.map(|x| serde_json::from_str(&x)).transpose()?.unwrap_or(Value::Null)),
        "save_draft" => {
            c.execute("INSERT OR REPLACE INTO drafts VALUES(?,?)", params![s("slot"), a["data"].to_string()])?;
            Ok(Value::Null)
        }
        "preview" => Ok(json!(delivery::recipients(s("to"))?.iter().map(ToString::to_string).collect::<Vec<_>>())),
        "enqueue" => Ok(json!(delivery::enqueue(&mut c, serde_json::from_value(a)?)?)),
        "deliver" => {
            let _guard = DELIVERY.lock().map_err(|_| anyhow::anyhow!("Ошибка очереди"))?;
            delivery::dispatch(&mut c, pid)?;
            Ok(Value::Null)
        }
        "queue" => store::rows(&c, "SELECT id,account,recipients,state,error,created,batch FROM queue ORDER BY rowid DESC", &[]),
        "retry" => {
            c.execute("UPDATE queue SET state='pending',error='' WHERE id=? AND state IN ('failed','unknown')", [s("id")])?;
            Ok(Value::Null)
        }
        "contacts" => store::rows(&c, "SELECT id,name,emails FROM contacts ORDER BY name", &[]),
        "contact" => {
            let emails = delivery::recipients(s("emails"))?.into_iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
            let id = if s("id").is_empty() { store::id() } else { s("id").into() };
            c.execute("INSERT OR REPLACE INTO contacts VALUES(?,?,?)", params![id, s("name"), emails])?;
            Ok(Value::Null)
        }
        "remove_contact" => {
            c.execute("DELETE FROM contacts WHERE id=?", [s("id")])?;
            Ok(Value::Null)
        }
        "attachments" => store::rows(&c, "SELECT id,name,mime,cid,length(data) AS size FROM attachments WHERE message=?", &[&n("id")]),
        "inline" => {
            use base64::Engine;
            let (mime, data): (String, Vec<u8>) = c.query_row("SELECT mime,data FROM attachments WHERE id=? AND cid<>''", [n("id")], |r| Ok((r.get(0)?, r.get(1)?)))?;
            ensure!(matches!(mime.as_str(), "image/png" | "image/jpeg" | "image/gif" | "image/webp"), "Формат изображения");
            Ok(json!(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(data))))
        }
        "file" => {
            let (name, data): (String, Vec<u8>) = c.query_row("SELECT name,data FROM attachments WHERE id=?", [n("id")], |r| Ok((r.get(0)?, r.get(1)?)))?;
            let name = mime::safe_name(&name);
            if s("action") == "save" {
                if let Some(p) = rfd::FileDialog::new().set_file_name(name).save_file() {
                    std::fs::write(p, data)?;
                }
            } else {
                let dir = std::env::temp_dir().join(store::id());
                std::fs::create_dir(&dir)?;
                let path = dir.join(name);
                std::fs::write(&path, data)?;
                if s("action") == "open" {
                    open::that_detached(path)?
                } else {
                    open_with(path)?
                }
            }
            Ok(Value::Null)
        }
        "delete" => {
            let id = n("id");
            let t = c.transaction()?;
            t.execute("INSERT OR IGNORE INTO hidden SELECT account,folder,uid FROM messages WHERE id=?", [id])?;
            t.execute("DELETE FROM messages WHERE id=?", [id])?;
            t.commit()?;
            Ok(Value::Null)
        }
        _ => bail!("Неизвестная команда"),
    }
}

pub(crate) fn list(c: &Connection, a: &Value) -> Result<Value> {
    let s = |k: &str| a[k].as_str().unwrap_or("");
    let filter = s("q")
        .split_whitespace()
        .map(|w| format!("\"{}\"*", w.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ");
    let mut sql = String::from("WITH hits AS (SELECT m.*,row_number() OVER(PARTITION BY thread ORDER BY stamp DESC,id DESC) AS rank FROM messages m WHERE 1");
    let mut vals = Vec::<String>::new();
    for (cond, val) in [
        (
            " AND m.id IN (SELECT rowid FROM search WHERE search MATCH ?)",
            filter.as_str(),
        ),
        (" AND account=?", s("account")),
        (" AND folder=?", s("folder")),
        (" AND stamp>=strftime('%s',?,'utc')+0", s("from")),
        (" AND stamp<strftime('%s',?,'+1 day','utc')+0", s("until")),
    ] {
        if !val.is_empty() {
            sql += cond;
            vals.push(val.into())
        }
    }
    if a["files"] == true {
        sql += " AND EXISTS(SELECT 1 FROM attachments WHERE message=m.id)"
    }
    sql += ") SELECT id,thread,account,folder,subject,sender,recipients,stamp,substr(body,1,150) AS preview,seen FROM hits WHERE rank=1 ORDER BY stamp DESC,id DESC";
    let refs = vals
        .iter()
        .map(|v| v as &dyn rusqlite::ToSql)
        .collect::<Vec<_>>();
    store::rows(c, &sql, &refs)
}
fn open_with(path: PathBuf) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        if let Some(app) = rfd::FileDialog::new()
            .set_directory("/Applications")
            .set_can_create_directories(false)
            .add_filter("Приложения", &["app"])
            .pick_file()
        {
            ensure!(
                std::process::Command::new("open")
                    .arg("-a")
                    .arg(app)
                    .arg(path)
                    .status()?
                    .success(),
                "Не удалось открыть файл"
            );
        }
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("rundll32.exe")
            .arg("shell32.dll,OpenAs_RunDLL")
            .arg(path)
            .spawn()?;
    }
    Ok(())
}
