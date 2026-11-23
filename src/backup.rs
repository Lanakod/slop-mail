use crate::{book, store};
use anyhow::{ensure, Context, Result};
use rusqlite::{backup::Backup, Connection, OpenFlags};
use std::{path::Path, time::Duration};

fn snapshot(source: &Connection, dest: &mut Connection) -> Result<()> {
    Backup::new(source, dest)?.run_to_completion(128, Duration::from_millis(5), None)?;
    let check: String = dest.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    ensure!(check == "ok", "Повреждена база профиля");
    dest.query_row("SELECT name FROM profile LIMIT 1", [], |r| {
        r.get::<_, String>(0)
    })?;
    Ok(())
}
pub fn save(source: &Connection, path: &Path) -> Result<()> {
    let temp = tempfile::NamedTempFile::new_in(path.parent().context("Каталог копии")?)?;
    {
        let mut dest = Connection::open(temp.path())?;
        snapshot(source, &mut dest)?;
        dest.execute_batch("PRAGMA journal_mode=DELETE;")?;
    }
    temp.persist_noclobber(path)
        .context("Выберите новое имя резервной копии")?;
    Ok(())
}
pub fn restore(path: &Path) -> Result<store::Profile> {
    let source = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let version: i64 = source.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    ensure!(
        (3..=4).contains(&version),
        "Поддерживаются копии 0.5.2 и 0.6.0"
    );
    let root = store::app_dir()?.join("Profiles");
    std::fs::create_dir_all(&root)?;
    let tmp = tempfile::Builder::new()
        .prefix("restored-")
        .tempdir_in(root)?;
    {
        let mut dest = Connection::open(tmp.path().join("mail.sqlite"))?;
        snapshot(&source, &mut dest)?;
        book::migrate(&dest)?;
        let tx = dest.transaction()?;
        tx.execute(
            "UPDATE profile SET id=?,name=name || ' (копия)'",
            [store::id()],
        )?;
        tx.execute_batch("UPDATE queue SET state='unknown',error='Восстановлено из копии. Проверьте доставку перед повтором.' WHERE state IN ('held','pending','sending');
            DELETE FROM send_holds; DELETE FROM moves; DELETE FROM sent_copies WHERE folder IS NULL;
            UPDATE messages SET pending_seen=0; UPDATE mailbox_display SET last_attempt=NULL,last_ok=NULL,problem='';")?;
        tx.commit()?;
    }
    let profile = store::register(tmp.path(), None)?;
    let _ = tmp.keep();
    Ok(profile)
}
