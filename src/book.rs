use crate::{delivery, store};
use anyhow::{ensure, Result};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::HashSet;

pub fn migrate(c: &Connection) -> Result<()> {
    if c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? >= 1 {
        return Ok(());
    }
    let t = rusqlite::Transaction::new_unchecked(c, rusqlite::TransactionBehavior::Immediate)?;
    if t.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? == 0 {
        t.execute_batch("ALTER TABLE contacts ADD COLUMN labels TEXT NOT NULL DEFAULT '{}';
 ALTER TABLE contacts ADD COLUMN favorite INTEGER NOT NULL DEFAULT 0;
 ALTER TABLE drafts ADD COLUMN modified INTEGER NOT NULL DEFAULT 0;
 CREATE TABLE contact_groups(id TEXT PRIMARY KEY,name TEXT NOT NULL);
 CREATE TABLE group_members(group_id TEXT REFERENCES contact_groups ON DELETE CASCADE, contact TEXT REFERENCES contacts ON DELETE CASCADE,PRIMARY KEY(group_id,contact));
 CREATE TABLE stars(message INTEGER PRIMARY KEY REFERENCES messages ON DELETE CASCADE);
 CREATE TABLE snippets(id TEXT PRIMARY KEY,name TEXT NOT NULL,body TEXT NOT NULL);
 PRAGMA user_version=1;")?;
    }
    t.commit()?;
    Ok(())
}
pub fn list(c: &Connection) -> Result<Value> {
    let mut data=store::rows(c,"SELECT id,name,emails,labels,favorite FROM contacts ORDER BY favorite DESC,name COLLATE NOCASE",&[])?;
    for p in data.as_array_mut().unwrap() {
        p["labels"] = serde_json::from_str(p["labels"].as_str().unwrap())?;
    }
    Ok(data)
}
pub fn save(c: &Connection, p: &Value) -> Result<String> {
    let s = |k: &str| p[k].as_str().unwrap_or("");
    let emails = delivery::recipients(s("emails"))?
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let id = if s("id").is_empty() {
        store::id()
    } else {
        s("id").to_owned()
    };
    let labels = p["labels"].as_object().cloned().unwrap_or_default();
    c.execute("INSERT INTO contacts(id,name,emails,labels,favorite) VALUES(?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,emails=excluded.emails,labels=excluded.labels,favorite=excluded.favorite",params![id,s("name"),emails,json!(labels).to_string(),p["favorite"]==true||p["favorite"]==1])?;
    Ok(id)
}
pub fn import(c: &mut Connection, items: &Value) -> Result<usize> {
    let t = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let mut seen = HashSet::new();
    for row in t
        .prepare("SELECT emails FROM contacts")?
        .query_map([], |r| r.get::<_, String>(0))?
    {
        seen.extend(
            delivery::addresses(&row?)
                .into_iter()
                .map(|a| a.to_lowercase()),
        );
    }
    let mut count = 0;
    for item in items
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Нет контактов"))?
    {
        let mut item = item.clone();
        let emails = delivery::recipients(item["emails"].as_str().unwrap_or(""))?
            .into_iter()
            .map(|a| a.to_string())
            .filter(|a| seen.insert(a.to_lowercase()))
            .collect::<Vec<_>>();
        if emails.is_empty() {
            continue;
        }
        item["emails"] = json!(emails.join(", "));
        item["id"] = Value::Null;
        save(&t, &item)?;
        count += 1;
    }
    t.commit()?;
    Ok(count)
}
pub fn history(c: &Connection, id: &str) -> Result<Value> {
    let emails: String =
        c.query_row("SELECT emails FROM contacts WHERE id=?", [id], |r| r.get(0))?;
    let keys: HashSet<_> = delivery::addresses(&emails)
        .into_iter()
        .map(|s| s.to_lowercase())
        .collect();
    let mut statement=c.prepare("SELECT id,thread,stamp,subject,sender,recipients FROM messages ORDER BY stamp DESC,id DESC")?;
    let mut cursor = statement.query([])?;
    let mut threads = HashSet::new();
    let mut hits = vec![];
    while let Some(row) = cursor.next()? {
        let thread: String = row.get(1)?;
        if threads.contains(&thread) {
            continue;
        }
        let from: String = row.get(4)?;
        let to: String = row.get(5)?;
        if [from, to]
            .iter()
            .flat_map(|s| delivery::addresses(s))
            .any(|s| keys.contains(&s.to_lowercase()))
        {
            threads.insert(thread.clone());
            hits.push(json!({"id":row.get::<_,i64>(0)?,"thread":thread,"stamp":row.get::<_,i64>(2)?,"subject":row.get::<_,String>(3)?}));
        }
    }
    Ok(json!(hits))
}
pub fn groups(c: &Connection) -> Result<Value> {
    let mut rows = store::rows(c, "SELECT id,name FROM contact_groups ORDER BY name", &[])?;
    for g in rows.as_array_mut().unwrap() {
        let members = c
            .prepare("SELECT contact FROM group_members WHERE group_id=?")?
            .query_map([g["id"].as_str().unwrap()], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        g["members"] = json!(members);
    }
    Ok(rows)
}
pub fn save_group(c: &mut Connection, a: &Value) -> Result<Value> {
    let name = a["name"].as_str().unwrap_or("").trim();
    ensure!(!name.is_empty(), "Введите название группы");
    let id = a["id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(store::id);
    let t = c.transaction()?;
    t.execute(
        "INSERT INTO contact_groups VALUES(?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name",
        params![id, name],
    )?;
    t.execute("DELETE FROM group_members WHERE group_id=?", [&id])?;
    for member in a["members"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Нет списка участников"))?
    {
        t.execute(
            "INSERT OR IGNORE INTO group_members VALUES(?,?)",
            params![id, member.as_str()],
        )?;
    }
    t.commit()?;
    Ok(json!(id))
}
