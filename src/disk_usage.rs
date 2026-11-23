use crate::store;
use anyhow::Result;
use rusqlite::{Connection, MAIN_DB};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, fs, io::ErrorKind};

const MESSAGE_SIZES: &str = "
    WITH files AS (
        SELECT message, sum(length(data)) AS bytes FROM attachments GROUP BY message
    ), sizes AS (
        SELECT m.id,m.account,m.thread,m.folder,m.subject,
               coalesce(octet_length(m.body),0)+coalesce(octet_length(m.html),0) AS text_bytes,
               coalesce(f.bytes,0) AS file_bytes
        FROM messages m LEFT JOIN files f ON f.message=m.id
    )";

fn duplicate_bytes(c: &Connection) -> Result<i64> {
    // Only read payloads whose length occurs more than once. Stream large files from SQLite.
    let mut query = c.prepare(
        "
        SELECT id,length(data) FROM attachments WHERE length(data) IN (
            SELECT length(data) FROM attachments WHERE length(data)>0
            GROUP BY length(data) HAVING count(*)>1
        )",
    )?;
    let mut rows = query.query([])?;
    let mut seen = HashSet::new();
    let mut redundant = 0;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let size: i64 = row.get(1)?;
        let mut blob = c.blob_open(MAIN_DB, "attachments", "data", id, true)?;
        let mut hash = Sha256::new();
        std::io::copy(&mut blob, &mut hash)?;
        if !seen.insert((size, hash.finalize())) {
            redundant += size;
        }
    }
    Ok(redundant)
}

pub fn report(c: &mut Connection) -> Result<Value> {
    let tx = c.transaction()?;
    let accounts = store::rows(&tx, &format!("{MESSAGE_SIZES},
        mail AS (SELECT account,sum(text_bytes) AS text_bytes,sum(file_bytes) AS file_bytes FROM sizes GROUP BY account),
        queued AS (SELECT account,sum(length(raw)) AS bytes FROM queue GROUP BY account)
        SELECT a.name,a.address,coalesce(m.text_bytes,0) AS text_bytes,
               coalesce(m.file_bytes,0) AS file_bytes,coalesce(q.bytes,0) AS queue_bytes
        FROM accounts a LEFT JOIN mail m ON m.account=a.id LEFT JOIN queued q ON q.account=a.id
        ORDER BY (coalesce(m.text_bytes,0)+coalesce(m.file_bytes,0)+coalesce(q.bytes,0)) DESC,a.address,a.id"), &[])?;
    let largest = store::rows(
        &tx,
        &format!(
            "{MESSAGE_SIZES}
        SELECT s.id,s.thread,s.subject,s.folder,a.address,s.text_bytes+s.file_bytes AS bytes
        FROM sizes s JOIN accounts a ON a.id=s.account
        ORDER BY bytes DESC,s.id DESC LIMIT 10"
        ),
        &[],
    )?;
    let duplicates = duplicate_bytes(&tx)?;
    let mut database_bytes = 0;
    if let Some(path) = tx.path() {
        for suffix in ["", "-wal", "-shm"] {
            match fs::metadata(format!("{path}{suffix}")) {
                Ok(metadata) => database_bytes += metadata.len(),
                Err(e) if !suffix.is_empty() && e.kind() == ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
        }
    }
    tx.commit()?;
    Ok(
        json!({"accounts":accounts,"largest":largest,"duplicates":duplicates,"database_bytes":database_bytes}),
    )
}
