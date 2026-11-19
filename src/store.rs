use anyhow::{anyhow, ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

pub fn id() -> String {
    Uuid::new_v4().to_string()
}
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
}
pub fn app_dir() -> Result<PathBuf> {
    #[cfg(test)]
    {
        TEST_DIR.with(|d| Ok(d.path().to_path_buf()))
    }
    #[cfg(not(test))]
    {
        let p = directories::ProjectDirs::from("org", "SlopMail", "Slop Mail")
            .context("Нет системного каталога")?
            .data_dir()
            .to_owned();
        fs::create_dir_all(&p)?;
        Ok(p)
    }
}
#[cfg(test)]
thread_local! { static TEST_DIR: tempfile::TempDir = tempfile::tempdir().expect("test registry"); }
fn registry() -> Result<Connection> {
    let c = Connection::open(app_dir()?.join("profiles.sqlite"))?;
    c.execute_batch("CREATE TABLE IF NOT EXISTS profiles(id TEXT PRIMARY KEY,name TEXT NOT NULL,path TEXT UNIQUE NOT NULL)")?;
    Ok(c)
}
pub fn profiles() -> Result<Vec<Profile>> {
    let c = registry()?;
    let mut s = c.prepare("SELECT id,name,path FROM profiles ORDER BY name")?;
    let a = s
        .query_map([], |r| {
            Ok(Profile {
                id: r.get(0)?,
                name: r.get(1)?,
                path: PathBuf::from(r.get::<_, String>(2)?),
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(a)
}
pub fn register(path: &Path, name: Option<&str>) -> Result<Profile> {
    let file = path.join("mail.sqlite");
    if let Some(n) = name {
        ensure!(!n.trim().is_empty(), "Введите имя профиля");
        fs::create_dir_all(path)?;
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&file)
            .context("В этой папке уже есть профиль")?;
    } else {
        ensure!(file.is_file(), "В папке нет mail.sqlite");
    }
    let c = Connection::open(&file)?;
    if let Some(n) = name {
        c.execute_batch(SCHEMA)?;
        c.execute("INSERT INTO profile VALUES(?,?)", params![id(), n.trim()])?;
    }
    let (pid, name) = c.query_row("SELECT id,name FROM profile", [], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    let path = path.canonicalize()?;
    registry()?.execute("INSERT INTO profiles VALUES(?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,path=excluded.path",params![pid,name,path.to_string_lossy()])?;
    Ok(Profile {
        id: pid,
        name,
        path,
    })
}
pub fn db(pid: &str) -> Result<Connection> {
    let p: String =
        registry()?.query_row("SELECT path FROM profiles WHERE id=?", [pid], |r| r.get(0))?;
    let c = Connection::open_with_flags(
        Path::new(&p).join("mail.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
    )?;
    c.busy_timeout(Duration::from_secs(15))?;
    c.execute_batch("PRAGMA foreign_keys=ON")?;
    Ok(c)
}

pub const SCHEMA: &str = r#"
PRAGMA journal_mode=WAL;
CREATE TABLE profile(id TEXT PRIMARY KEY,name TEXT);
CREATE TABLE servers(id TEXT PRIMARY KEY,proto TEXT,host TEXT,port INTEGER,tls TEXT,UNIQUE(proto,host,port,tls));
CREATE TABLE accounts(id TEXT PRIMARY KEY,name TEXT,address TEXT,login TEXT,smtp_login TEXT,signature TEXT,incoming TEXT REFERENCES servers,outgoing TEXT REFERENCES servers,keep INTEGER);
CREATE TABLE folders(account TEXT,name TEXT,validity INTEGER,PRIMARY KEY(account,name));
CREATE TABLE messages(id INTEGER PRIMARY KEY,account TEXT NOT NULL,folder TEXT NOT NULL,uid TEXT NOT NULL,mid TEXT,thread TEXT,stamp INTEGER,subject TEXT,sender TEXT,recipients TEXT,reply_to TEXT,refs TEXT,body TEXT,html TEXT,outgoing INTEGER,seen INTEGER,pending_seen INTEGER DEFAULT 0,UNIQUE(account,folder,uid));
CREATE INDEX by_thread ON messages(thread,stamp);
CREATE TABLE refs(ref TEXT PRIMARY KEY,thread TEXT);
CREATE TABLE attachments(id INTEGER PRIMARY KEY,message INTEGER REFERENCES messages ON DELETE CASCADE,name TEXT,mime TEXT,cid TEXT,data BLOB);
CREATE VIRTUAL TABLE search USING fts5(subject,body,sender,recipients,tokenize='unicode61');
CREATE TRIGGER index_message AFTER INSERT ON messages BEGIN INSERT INTO search(rowid,subject,body,sender,recipients) VALUES(new.id,new.subject,new.body,new.sender,new.recipients); END;
CREATE TRIGGER unindex_message AFTER DELETE ON messages BEGIN DELETE FROM search WHERE rowid=old.id; END;
CREATE TABLE contacts(id TEXT PRIMARY KEY,name TEXT,emails TEXT);
CREATE TABLE drafts(slot TEXT PRIMARY KEY,data TEXT);
CREATE TABLE queue(id TEXT PRIMARY KEY,account TEXT,recipients TEXT,raw BLOB,state TEXT,error TEXT,created INTEGER,batch TEXT);
CREATE TABLE hidden(account TEXT,folder TEXT,uid TEXT,PRIMARY KEY(account,folder,uid));
CREATE TABLE pop_history(account TEXT,uid TEXT,PRIMARY KEY(account,uid));
"#;
#[derive(Clone, Serialize, Deserialize)]
pub struct Server {
    pub proto: String,
    pub host: String,
    pub port: u16,
    pub tls: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Account {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub address: String,
    pub login: String,
    pub smtp_login: String,
    pub signature: String,
    pub incoming: Server,
    pub outgoing: Server,
    pub keep: bool,
}
pub fn accounts(c: &Connection) -> Result<Vec<Account>> {
    let mut s=c.prepare("SELECT a.id,name,address,login,smtp_login,signature,i.proto,i.host,i.port,i.tls,o.proto,o.host,o.port,o.tls,keep FROM accounts a JOIN servers i ON i.id=a.incoming JOIN servers o ON o.id=a.outgoing ORDER BY a.rowid")?;
    let a = s
        .query_map([], |r| {
            let server = |n| -> rusqlite::Result<Server> {
                Ok(Server {
                    proto: r.get(n)?,
                    host: r.get(n + 1)?,
                    port: r.get(n + 2)?,
                    tls: r.get(n + 3)?,
                })
            };
            Ok(Account {
                id: r.get(0)?,
                name: r.get(1)?,
                address: r.get(2)?,
                login: r.get(3)?,
                smtp_login: r.get(4)?,
                signature: r.get(5)?,
                incoming: server(6)?,
                outgoing: server(10)?,
                keep: r.get(14)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(a)
}
pub fn account(c: &Connection, id: &str) -> Result<Account> {
    accounts(c)?
        .into_iter()
        .find(|a| a.id == id)
        .context("Выберите ящик")
}
pub fn save_account(c: &mut Connection, a: &mut Account) -> Result<()> {
    a.address.parse::<lettre::Address>()?;
    ensure!(
        matches!(a.incoming.proto.as_str(), "imap" | "pop3") && a.outgoing.proto == "smtp",
        "Неверный протокол"
    );
    if a.id.is_empty() {
        a.id = id()
    }
    let t = c.transaction()?;
    let mut ids = Vec::new();
    for s in [&a.incoming, &a.outgoing] {
        ensure!(
            !s.host.trim().is_empty() && s.port > 0 && matches!(s.tls.as_str(), "tls" | "starttls"),
            "Проверьте настройки сервера"
        );
        t.execute(
            "INSERT OR IGNORE INTO servers VALUES(?,?,?,?,?)",
            params![id(), s.proto, s.host, s.port, s.tls],
        )?;
        ids.push(t.query_row(
            "SELECT id FROM servers WHERE proto=? AND host=? AND port=? AND tls=?",
            params![s.proto, s.host, s.port, s.tls],
            |r| r.get::<_, String>(0),
        )?);
    }
    if let Some((old_login, old_server)) = t
        .query_row(
            "SELECT login,incoming FROM accounts WHERE id=?",
            [&a.id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?
    {
        ensure!(
            old_login == a.login && old_server == ids[0],
            "Создайте отдельный ящик для другого входящего сервера или логина"
        );
    }
    t.execute("INSERT INTO accounts VALUES(?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,address=excluded.address,smtp_login=excluded.smtp_login,signature=excluded.signature,outgoing=excluded.outgoing,keep=excluded.keep",params![a.id,a.name,a.address,a.login,a.smtp_login,a.signature,ids[0],ids[1],a.keep])?;
    t.commit()?;
    Ok(())
}
pub fn rows(c: &Connection, sql: &str, args: &[&dyn rusqlite::ToSql]) -> Result<Value> {
    let mut s = c.prepare(sql)?;
    let names: Vec<String> = s.column_names().iter().map(|x| x.to_string()).collect();
    let mut r = s.query(args)?;
    let mut out = vec![];
    while let Some(row) = r.next()? {
        let mut obj = serde_json::Map::new();
        for (i, n) in names.iter().enumerate() {
            let v = match row.get_ref(i)? {
                rusqlite::types::ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
                rusqlite::types::ValueRef::Integer(v) => json!(v),
                rusqlite::types::ValueRef::Null => Value::Null,
                _ => return Err(anyhow!("Неизвестный тип поля")),
            };
            obj.insert(n.clone(), v);
        }
        out.push(Value::Object(obj));
    }
    Ok(json!(out))
}
