use crate::{
    attachments, delivery, network,
    store::{self, Account},
};
use anyhow::{bail, ensure, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::sync::Mutex;
static SYNC: Mutex<()> = Mutex::new(());
static DELIVERY: Mutex<()> = Mutex::new(());
#[tauri::command]
pub async fn request(
    app: tauri::AppHandle,
    op: String,
    profile: String,
    args: Value,
) -> Result<Value, String> {
    let arm = matches!(op.as_str(), "enqueue" | "bulk" | "sync" | "resume_work");
    let result = execute(op, profile.clone(), args).await?;
    if arm {
        crate::scheduler::resume(app, profile).await?;
    }
    Ok(result)
}
pub async fn execute(op: String, profile: String, args: Value) -> Result<Value, String> {
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
        "about" => return Ok(json!({"version":env!("CARGO_PKG_VERSION"),"windows":cfg!(windows)})),
        "discover" => return crate::discovery::discover(s("address")),
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
        "resume_work"=>Ok(Value::Null),
        "deadlines"=>store::rows(&c,"SELECT DISTINCT due,'deliver' AS op FROM send_holds UNION SELECT DISTINCT due,'apply_moves' FROM moves UNION SELECT DISTINCT until,'wake_due' FROM snoozes",&[]),
        "wake_due"=>{c.execute("DELETE FROM snoozes WHERE until<=unixepoch()",[])?;Ok(Value::Null)}
        "accounts" => Ok(json!(store::accounts(&c)?)),
        "account" | "test" => {
            let mut acc: Account = serde_json::from_value(a["account"].clone())?;
            if op == "test" {
                let side = s("side");
                let mut pw = s(side).to_string();
                if pw.is_empty() {
                    pw = network::secret(pid, &acc, side)?;
                }
                return Ok(json!({"idle":network::test(&acc, side, &pw)?}));
            } else {
                if acc.id.is_empty(){acc.id=store::id();}
                for side in ["in", "out"] {
                    network::save_secret(pid,&acc,side,s(side),a["remember"]!=false)?;
                }
                store::save_account(&mut c,&mut acc)?;
            }
            Ok(json!(acc.id))
        }
        "sync" => {
            let _guard = SYNC.lock().map_err(|_| anyhow::anyhow!("Ошибка синхронизации"))?;
            let acc = store::account(&c, s("account"))?;
            let pw = network::secret(pid, &acc, "in")?;
            Ok(json!(network::sync(&mut c, &acc, &pw)?))
        }
        "folders" => store::rows(&c, "WITH names AS (SELECT account,name FROM folders UNION SELECT account,shown_folder FROM mail_items), counts AS (SELECT account,shown_folder,count(*) AS unread FROM mail_items WHERE seen=0 GROUP BY account,shown_folder) SELECT n.account,n.name,coalesce(c.unread,0) AS unread FROM names n LEFT JOIN counts c ON n.account=c.account AND n.name=c.shown_folder ORDER BY 1,2",&[]),
        "bulk"=>crate::organize::bulk(&mut c,&a),
        "apply_moves"=>{
            let _guard=SYNC.lock().map_err(|_|anyhow::anyhow!("Ошибка синхронизации"))?;
            crate::organize::apply_moves(&mut c,pid)
        }
        "undo_move"=>{ensure!(c.execute("DELETE FROM moves WHERE batch=? AND due>unixepoch()",[s("batch")])?>0,"Время отмены истекло");Ok(Value::Null)}
        "saved_searches"=>store::rows(&c,"SELECT query FROM saved_searches ORDER BY query",&[]),
        "save_search"=>{ensure!(!s("q").trim().is_empty(),"Введите запрос");c.execute("INSERT OR IGNORE INTO saved_searches VALUES(?)",[s("q").trim()])?;Ok(Value::Null)}
        "drop_search"=>{c.execute("DELETE FROM saved_searches WHERE query=?",[s("q")])?;Ok(Value::Null)}
        "rules"=>store::rows(&c,"SELECT * FROM rules ORDER BY id",&[]),
        "save_rule"=>{let pattern=s("pattern").trim();ensure!(!pattern.is_empty() && (!s("label").trim().is_empty()||!s("folder").is_empty()||a["quiet"]==true),"Укажите отправителя и действие");if !s("folder").is_empty(){crate::organize::valid_folder(s("folder"))?;}
            c.execute("INSERT INTO rules(pattern,label,destination,quiet) VALUES(?,?,?,?)",params![pattern,s("label").trim(),s("folder").trim(),a["quiet"]==true])?;Ok(Value::Null)}
        "drop_rule"=>{c.execute("DELETE FROM rules WHERE id=?",[n("id")])?;Ok(Value::Null)}
        "restore_links"=>{crate::network::restore_links(&c,pid,n("id"))?;Ok(Value::Null)}
        "open_url"=>{let url=reqwest::Url::parse(s("url"))?;ensure!(matches!(url.scheme(),"http"|"https") && url.host().is_some(),"Разрешены только веб-ссылки");open::that(url.as_str())?;Ok(Value::Null)}

        "list" => list(&c, &a),
        "mark_seen" => {
            let seen=a["seen"].as_bool().ok_or_else(||anyhow::anyhow!("Не указано состояние прочтения"))?;
            let ids:Vec<i64>=serde_json::from_value(a["ids"].clone())?;
            c.execute("UPDATE messages SET seen=?1,pending_seen=1 WHERE thread=?2 AND seen<>?1 AND id IN (SELECT value FROM json_each(?3))",params![seen,s("thread"),serde_json::to_string(&ids)?])?;
            Ok(Value::Null)
        }
        "thread" => {
            let t = c.transaction()?;
            t.execute("UPDATE messages SET seen=1,pending_seen=1 WHERE thread=? AND id=? AND seen=0", params![s("thread"),a["hit"].as_i64()])?;
            let mut data = store::rows(&t, "SELECT id,account,folder,thread,stamp,subject,sender,recipients,reply_to,body,html,outgoing,EXISTS(SELECT 1 FROM stars WHERE message=messages.id) AS starred FROM messages WHERE thread=? ORDER BY stamp,id", &[&s("thread")])?;
            t.commit()?;
            for m in data.as_array_mut().unwrap(){m["authors"]=crate::book::participants(m["sender"].as_str().unwrap_or(""));m["addressees"]=crate::book::participants(m["recipients"].as_str().unwrap_or(""));}
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
            c.execute("INSERT OR REPLACE INTO drafts(slot,data,modified) VALUES(?,?,unixepoch())", params![s("slot"), a["data"].to_string()])?;
            Ok(Value::Null)
        }
        "preview" => Ok(json!(delivery::recipients(s("to"))?.iter().map(ToString::to_string).collect::<Vec<_>>())),
        "recipient_plan"=>Ok(json!(delivery::fields(s("to"),s("cc"),s("bcc"))?.into_iter().map(|(field,addr)|json!({"field":field,"email":addr.to_string()})).collect::<Vec<_>>())),
        "enqueue" => {let batch=store::id(); delivery::enqueue_batch(&mut c,serde_json::from_value(a.clone())?,&batch,Some(&a))?;Ok(json!(batch))}
        "cancel_send"=>delivery::cancel(&mut c,s("batch")),
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
        "contacts" => crate::book::list(&c),
        "contact"=>Ok(json!(crate::book::save(&c,&a)?)),
        "contact_history"=>crate::book::history(&c,s("id")),
        "import_contacts"=>Ok(json!(crate::book::import(&mut c,&a["items"])?)),
        "export_book"=>{let ext=if s("format")=="csv"{"csv"}else{"vcf"};if let Some(path)=rfd::FileDialog::new().set_file_name(format!("contacts.{ext}")).save_file(){std::fs::write(path,s("text"))?;}Ok(Value::Null)},
        "groups"=>crate::book::groups(&c),
        "save_group"=>crate::book::save_group(&mut c,&a),
        "remove_group"=>{c.execute("DELETE FROM contact_groups WHERE id=?",[s("id")])?;Ok(Value::Null)},
        "star"=>{if a["on"]==true{c.execute("INSERT OR IGNORE INTO stars VALUES(?)",[n("id")])?;}else{c.execute("DELETE FROM stars WHERE message=?",[n("id")])?;}Ok(Value::Null)},
        "drafts"=>store::rows(&c,"SELECT slot,data,modified FROM drafts ORDER BY modified DESC,slot",&[]),
        "remove_draft"=>{c.execute("DELETE FROM drafts WHERE slot=?",[s("slot")])?;Ok(Value::Null)},
        "snippets"=>store::rows(&c,"SELECT id,name,body FROM snippets ORDER BY name",&[]),
        "save_snippet"=>{ensure!(!s("name").trim().is_empty()&&!s("body").trim().is_empty(),"Введите название и текст");let id=if s("id").is_empty(){store::id()}else{s("id").into()};c.execute("INSERT OR REPLACE INTO snippets VALUES(?,?,?)",params![id,s("name"),s("body")])?;Ok(Value::Null)},
        "remove_snippet"=>{c.execute("DELETE FROM snippets WHERE id=?",[s("id")])?;Ok(Value::Null)},
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
        "file" => { attachments::handle(&c,n("id"),s("action"))?;Ok(Value::Null) }
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
    let mut words = Vec::new();
    let mut fields = Vec::new();
    for word in s("q").split_whitespace() {
        let (key, value) = word.split_once(':').unwrap_or(("", ""));
        match key.to_lowercase().as_str() {
            "от" | "from" if !value.is_empty() => fields.push(("sender", value)),
            "файл" | "file" if !value.is_empty() => fields.push(("file", value)),
            "метка" if !value.is_empty() => fields.push(("label", value)),
            _ => words.push(format!("\"{}\"*", word.replace('"', "\"\""))),
        }
    }
    let filter = words.join(" AND ");
    let mut sql = String::from("WITH hits AS (SELECT id,thread,account,shown_folder AS folder,labels,subject,sender,recipients,stamp,substr(body,1,150) AS preview,seen,count(*) OVER(PARTITION BY thread) AS total,sum(1-seen) OVER(PARTITION BY thread) AS unread,first_value(subject) OVER(PARTITION BY thread ORDER BY stamp,id) AS topic,row_number() OVER(PARTITION BY thread ORDER BY stamp DESC,id DESC) AS rank FROM mail_items m WHERE 1");
    let mut vals = Vec::<String>::new();
    for (field, value) in fields {
        match field {
            "sender" => {
                sql += " AND id IN (SELECT rowid FROM search WHERE search MATCH ?)";
                vals.push(format!("sender:\"{}\"*", value.replace('"', "\"\"")));
            }
            "file" => {
                sql+=" AND EXISTS(SELECT 1 FROM attachments WHERE message=m.id AND instr(lower(name),lower(?))>0)";
                vals.push(value.into());
            }
            _ => {
                sql += " AND EXISTS(SELECT 1 FROM labels l WHERE l.message=m.id AND l.label=?)";
                vals.push(value.into());
            }
        }
    }
    let snoozed = "EXISTS(SELECT 1 FROM snoozes z WHERE z.thread=m.thread AND z.until>unixepoch())";
    if a["view"] == "snoozed" {
        sql += " AND ";
        sql += snoozed;
    } else if s("q").is_empty() {
        sql += " AND NOT ";
        sql += snoozed;
    }
    for (cond, val) in [
        (
            " AND m.id IN (SELECT rowid FROM search WHERE search MATCH ?)",
            filter.as_str(),
        ),
        (" AND account=?", s("account")),
        (" AND thread=?", s("branch")),
        (" AND shown_folder=?", s("folder")),
        (" AND stamp>=strftime('%s',?,'utc')+0", s("from")),
        (" AND stamp<strftime('%s',?,'+1 day','utc')+0", s("until")),
    ] {
        if !val.is_empty() {
            sql += cond;
            vals.push(val.into())
        }
    }
    if a["view"] == "unread" {
        sql += " AND seen=0";
    }
    if a["view"] == "starred" {
        sql += " AND EXISTS(SELECT 1 FROM stars WHERE message=m.id)";
    }
    if a["files"] == true || a["view"] == "files" {
        sql += " AND EXISTS(SELECT 1 FROM attachments WHERE message=m.id)"
    }
    sql += ") SELECT id,thread,account,folder,subject,sender,recipients,stamp,preview,labels,seen,total,unread,topic FROM hits";
    if s("branch").is_empty() && (filter.is_empty() || a["grouped"] == true) {
        sql += " WHERE rank=1";
    }
    sql += if !s("branch").is_empty() || a["sort"] == "oldest" {
        " ORDER BY stamp ASC,id ASC"
    } else {
        " ORDER BY stamp DESC,id DESC"
    };
    let refs = vals
        .iter()
        .map(|v| v as &dyn rusqlite::ToSql)
        .collect::<Vec<_>>();
    store::rows(c, &sql, &refs)
}
