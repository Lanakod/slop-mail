use crate::{mime, store};
use anyhow::{ensure, Context, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
fn materialize(root: &Path, name: &str, data: &[u8]) -> Result<PathBuf> {
    std::fs::create_dir_all(root)?;
    let directory = root.join(store::id());
    std::fs::create_dir(&directory)?;
    let path = directory.join(mime::safe_name(name));
    std::fs::write(&path, data)?;
    Ok(path)
}
pub fn handle(c: &Connection, id: i64, action: &str) -> Result<()> {
    let (name, data): (String, Vec<u8>) =
        c.query_row("SELECT name,data FROM attachments WHERE id=?", [id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
    if action == "save" {
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name(mime::safe_name(&name))
            .save_file()
        {
            std::fs::write(path, data)?;
        }
        return Ok(());
    }
    let root = Path::new(c.path().context("Нет пути профиля")?)
        .parent()
        .context("Нет каталога профиля")?
        .join("Opened");
    let path = materialize(&root, &name, &data)?;
    match action {
        "open" => open::that(path)?,
        "choose" => choose_application(&path)?,
        _ => anyhow::bail!("Неизвестное действие с вложением"),
    }
    Ok(())
}
#[cfg(target_os = "macos")]
fn open_selected(path: &Path, app: &Path) -> Result<()> {
    ensure!(
        std::process::Command::new("/usr/bin/open")
            .arg("-a")
            .arg(app)
            .arg(path)
            .status()?
            .success(),
        "Не удалось открыть вложение в выбранном приложении"
    );
    Ok(())
}
#[cfg(target_os = "macos")]
fn choose_application(path: &Path) -> Result<()> {
    if let Some(app) = rfd::FileDialog::new()
        .set_directory("/Applications")
        .set_title("Выберите приложение")
        .add_filter("Приложения", &["app"])
        .pick_file()
    {
        open_selected(path, &app)?;
    }
    Ok(())
}
#[cfg(target_os = "windows")]
fn choose_application(path: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
        UI::Shell::{SHOpenWithDialog, OAIF_EXEC, OAIF_HIDE_REGISTRATION, OPENASINFO},
    };
    let filename: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let request = OPENASINFO {
        pcszFile: filename.as_ptr(),
        pcszClass: std::ptr::null(),
        oaifInFlags: OAIF_EXEC | OAIF_HIDE_REGISTRATION,
    };
    // This worker owns the COM initialization and filename through the modal call.
    let initialized = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
    ensure!(initialized >= 0, "COM: {initialized:#x}");
    let result = unsafe { SHOpenWithDialog(std::ptr::null_mut(), &request) };
    unsafe { CoUninitialize() };
    ensure!(
        result >= 0 || result as u32 == 0x800704c7,
        "Open With: {result:#x}"
    );
    Ok(())
}
/// Only consider regular files directly inside Slop Mail's UUID directories.
pub fn cache(c: &Connection, clean: bool) -> Result<serde_json::Value> {
    let mut bytes = 0u64;
    let mut removed = 0u64;
    let mut oldest = 0u64;
    if let Some(parent) = c.path().and_then(|p| Path::new(p).parent()) {
        let root = parent.join("Opened");
        if root.symlink_metadata().is_ok_and(|m| m.is_dir()) {
            for entry in root.read_dir()? {
                let dir = entry?;
                if !dir.file_type()?.is_dir()
                    || uuid::Uuid::parse_str(&dir.file_name().to_string_lossy()).is_err()
                {
                    continue;
                }
                for child in dir.path().read_dir()? {
                    let file = child?;
                    if !file.file_type()?.is_file() {
                        continue;
                    }
                    let meta = file.metadata()?;
                    let age = meta.modified()?.elapsed().unwrap_or_default().as_secs() / 86400;
                    oldest = oldest.max(age);
                    bytes += meta.len();
                    if clean && age >= 7 {
                        // Keep locally edited copies and files whose source was deleted.
                        let data = std::fs::read(file.path())?;
                        let stored:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM attachments WHERE length(data)=? AND data=?)",rusqlite::params![data.len(),data],|r|r.get(0))?;
                        if stored && std::fs::remove_file(file.path()).is_ok() {
                            removed += meta.len();
                            bytes -= meta.len();
                        }
                    }
                }
                if clean {
                    let _ = std::fs::remove_dir(dir.path());
                }
            }
        }
    }
    Ok(serde_json::json!({"bytes":bytes,"removed":removed,"oldest":oldest}))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distinct_paths_for_duplicate_names() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let a = materialize(tmp.path(), "../../письмо", b"a")?;
        let b = materialize(tmp.path(), "C:\\folder\\письмо", b"b")?;
        assert_ne!(a, b);
        assert!(a.starts_with(tmp.path()) && b.starts_with(tmp.path()));
        assert_eq!(std::fs::read(a)?, b"a");
        Ok(())
    }
    #[test]
    fn cache_cleanup_preserves_changed_and_recent_files() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let c = Connection::open(dir.path().join("mail.sqlite"))?;
        c.execute_batch(crate::store::SCHEMA)?;
        c.execute("INSERT INTO attachments(data) VALUES(?)", [b"abc"])?;
        let root = dir.path().join("Opened");
        let old = materialize(&root, "old", b"abc")?;
        let edited = materialize(&root, "edit", b"xyz")?;
        let recent = materialize(&root, "new", b"abc")?;
        for path in [&old, &edited] {
            std::fs::OpenOptions::new()
                .write(true)
                .open(path)?
                .set_modified(
                    std::time::SystemTime::now() - std::time::Duration::from_secs(86400 * 8),
                )?;
        }
        assert_eq!(cache(&c, false)?["bytes"], 9);
        assert_eq!(cache(&c, true)?["removed"], 3);
        assert!(!old.exists() && edited.exists() && recent.exists());
        Ok(())
    }
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "Opens local text documents in desktop applications"]
    fn macos_open_default_and_selected() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let p = materialize(
            tmp.path(),
            "Slop Mail — проверка.txt",
            "Тест вложения".as_bytes(),
        )?;
        open::that(&p)?;
        open_selected(&p, Path::new("/System/Applications/TextEdit.app"))?;
        Ok(())
    }
}
