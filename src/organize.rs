use crate::store;
use anyhow::{ensure, Result};
use rusqlite::{params, Connection, TransactionBehavior};
use serde_json::{json, Value};

pub fn migrate(c: &Connection) -> Result<()> {
    if c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? >= 2 {
        return Ok(());
    }
    let t = rusqlite::Transaction::new_unchecked(c, TransactionBehavior::Immediate)?;
    t.execute_batch("
    CREATE TABLE IF NOT EXISTS snoozes(thread TEXT PRIMARY KEY,until INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS moves(message INTEGER PRIMARY KEY REFERENCES messages ON DELETE CASCADE,destination TEXT NOT NULL,batch TEXT NOT NULL,due INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS rules(id INTEGER PRIMARY KEY,pattern TEXT NOT NULL,label TEXT NOT NULL,destination TEXT NOT NULL,quiet INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS labels(message INTEGER REFERENCES messages ON DELETE CASCADE,label TEXT,PRIMARY KEY(message,label));
    CREATE TABLE IF NOT EXISTS saved_searches(query TEXT PRIMARY KEY);
    CREATE TABLE IF NOT EXISTS send_holds(batch TEXT PRIMARY KEY,due INTEGER NOT NULL,draft TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS arrivals(account TEXT PRIMARY KEY,message INTEGER REFERENCES messages ON DELETE CASCADE);
    CREATE VIEW IF NOT EXISTS mail_items AS SELECT m.*,coalesce(v.destination,m.folder) AS shown_folder,
    (SELECT group_concat(label,', ') FROM labels WHERE message=m.id) AS labels FROM messages m LEFT JOIN moves v ON v.message=m.id;
    PRAGMA user_version=2;")?;
    t.commit()?;
    Ok(())
}

pub fn arrived(c: &Connection, id: i64, account: &str) -> Result<bool> {
    let (sender, thread, seen): (String, String, bool) = c.query_row(
        "SELECT sender,thread,seen FROM messages WHERE id=?",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    c.execute("DELETE FROM snoozes WHERE thread=?", [thread])?;
    let mut quiet = false;
    let addresses = crate::delivery::addresses(&sender);
    let mut rules = c.prepare("SELECT pattern,label,destination,quiet FROM rules ORDER BY id")?;
    let mut rows = rules.query([])?;
    while let Some(r) = rows.next()? {
        let pattern: String = r.get(0)?;
        if !addresses.iter().any(|addr| {
            addr.eq_ignore_ascii_case(&pattern)
                || addr
                    .rsplit_once('@')
                    .is_some_and(|(_, d)| d.eq_ignore_ascii_case(pattern.trim_start_matches('@')))
        }) {
            continue;
        }
        let label: String = r.get(1)?;
        let folder: String = r.get(2)?;
        if !label.is_empty() {
            c.execute(
                "INSERT OR IGNORE INTO labels VALUES(?,?)",
                params![id, label],
            )?;
        }
        if !folder.is_empty() {
            c.execute(
                "INSERT OR IGNORE INTO moves VALUES(?,?,'rule',0)",
                params![id, folder],
            )?;
        }
        quiet |= r.get::<_, bool>(3)?;
    }
    if !quiet && !seen {
        c.execute(
            "INSERT OR REPLACE INTO arrivals VALUES(?,?)",
            params![account, id],
        )?;
    }
    Ok(!quiet)
}
pub fn valid_folder(folder: &str) -> Result<()> {
    ensure!(
        !folder.trim().is_empty() && !folder.chars().any(char::is_control),
        "Введите имя папки"
    );
    Ok(())
}
pub fn bulk(c: &mut Connection, a: &Value) -> Result<Value> {
    let ids: Vec<i64> = serde_json::from_value(a["ids"].clone())?;
    ensure!(!ids.is_empty(), "Выберите письма");
    let ids = json!(ids).to_string();
    let t = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut result = Value::Null;
    match a["action"].as_str().unwrap_or("") {
        "read" | "unread" => {
            t.execute("UPDATE messages SET seen=?1,pending_seen=1 WHERE id IN (SELECT value FROM json_each(?2)) AND seen<>?1",params![a["action"]=="read",ids])?;
        }
        "star" => {
            t.execute("INSERT OR IGNORE INTO stars SELECT id FROM messages WHERE id IN (SELECT value FROM json_each(?))",[ids])?;
        }
        "move" => {
            let dest = a["folder"].as_str().unwrap_or("").trim();
            valid_folder(dest)?;
            ensure!(!t.query_row("SELECT EXISTS(SELECT 1 FROM moves WHERE message IN (SELECT value FROM json_each(?)))",[&ids],|r|r.get::<_,bool>(0))?,"Предыдущее перемещение ещё выполняется");
            let batch = store::id();
            t.execute("INSERT OR REPLACE INTO moves SELECT id,?1,?2,unixepoch()+10 FROM messages WHERE id IN (SELECT value FROM json_each(?3)) AND folder<>?1",params![dest,batch,ids])?;
            result = json!(batch);
        }
        "snooze" => {
            let until = a["until"].as_i64().unwrap_or(0);
            ensure!(until > store::now(), "Выберите время в будущем");
            t.execute("INSERT OR REPLACE INTO snoozes SELECT DISTINCT thread,? FROM messages WHERE id IN (SELECT value FROM json_each(?))",params![until,ids])?;
        }
        "wake" => {
            t.execute("DELETE FROM snoozes WHERE thread IN (SELECT thread FROM messages WHERE id IN (SELECT value FROM json_each(?)))",[ids])?;
        }
        _ => anyhow::bail!("Неизвестное действие"),
    }
    t.commit()?;
    Ok(result)
}

pub fn apply_moves(c: &mut Connection, pid: &str) -> Result<Value> {
    let boxes=store::rows(c,"SELECT DISTINCT account FROM moves JOIN messages ON messages.id=moves.message WHERE due<=unixepoch()",&[])?;
    for item in boxes.as_array().unwrap() {
        let acc = store::account(c, item["account"].as_str().unwrap())?;
        let t = c.transaction()?;
        t.execute("UPDATE messages SET folder=(SELECT destination FROM moves WHERE message=messages.id),uid=CASE WHEN ?2='pop3' THEN uid ELSE 'local:'||id END WHERE account=?1 AND id IN (SELECT message FROM moves WHERE due<=unixepoch()) AND (?2='pop3' OR uid GLOB '*[^0-9]*' AND uid NOT LIKE 'moving:%')",params![acc.id,acc.incoming.proto])?;
        t.execute("DELETE FROM moves WHERE message IN (SELECT id FROM messages WHERE account=? AND (?2='pop3' OR uid LIKE 'local:%') AND folder=moves.destination)",params![acc.id,acc.incoming.proto])?;
        t.commit()?;
        if acc.incoming.proto == "imap" {
            let pw = crate::network::secret(pid, &acc, "in")?;
            crate::network::flush_moves(c, &acc, &pw)?;
        }
    }
    Ok(Value::Null)
}
