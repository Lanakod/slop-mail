use crate::{
    api, desktop,
    network::IdleSocket,
    store::{self, Account},
};
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{async_runtime::JoinHandle, AppHandle, Emitter, State};

#[derive(Default)]
pub struct Scheduler(Mutex<Vec<JoinHandle<()>>>);

struct IdleGuard(Arc<IdleSocket>);
impl Drop for IdleGuard {
    fn drop(&mut self) {
        self.0.stop();
    }
}

async fn listen(app: AppHandle, profile: String, account: Account, notify: bool) {
    let guard = IdleGuard(Arc::default());
    let mut reconnect = false;
    let mut delay = 5;
    loop {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let socket = guard.0.clone();
        let mailbox = account.clone();
        let pid = profile.clone();
        let worker = tauri::async_runtime::spawn_blocking(move || {
            socket.listen(&pid, &mailbox, reconnect, || {
                let _ = tx.try_send(());
            })
        });
        while rx.recv().await.is_some() {
            delay = 5;
            let result = check_account(&app, &profile, &account, notify).await;
            let work = resume(app.clone(), profile.clone()).await;
            let _ = app.emit(
                "scheduled-mail",
                json!({"profile":profile,"error":result.and(work).err()}),
            );
        }
        if matches!(worker.await, Ok(Ok(()))) {
            return;
        } // No IDLE capability; the scheduled check handles this account.
        tokio::time::sleep(Duration::from_secs(delay)).await;
        delay = (delay * 2).min(300);
        reconnect = true;
    }
}

#[tauri::command]
pub fn set_schedule(
    app: AppHandle,
    scheduler: State<'_, Scheduler>,
    profile: String,
    minutes: u32,
    notify: bool,
    instant: Option<bool>,
) -> Result<(), String> {
    let mut task = scheduler.0.lock().map_err(|e| e.to_string())?;
    for old in task.drain(..) {
        old.abort();
    }
    if !profile.is_empty() && minutes > 0 {
        if instant.unwrap_or(true) {
            let accounts = store::db(&profile)
                .and_then(|c| store::accounts(&c))
                .map_err(|e| e.to_string())?;
            for a in accounts.into_iter().filter(|a| a.incoming.proto == "imap") {
                task.push(tauri::async_runtime::spawn(listen(
                    app.clone(),
                    profile.clone(),
                    a,
                    notify,
                )));
            }
        }
        task.push(tauri::async_runtime::spawn(async move {
            let interval = Duration::from_secs(u64::from(minutes) * 60);
            loop {
                tokio::time::sleep(interval).await;
                let result = check(&app, &profile, notify).await;
                let _ = app.emit(
                    "scheduled-mail",
                    json!({"profile":profile,"error":result.err()}),
                );
            }
        }));
    }
    Ok(())
}

async fn check(app: &AppHandle, profile: &str, notify: bool) -> Result<(), String> {
    api::execute("deliver".into(), profile.into(), Value::Null).await?;
    let rows = api::execute("accounts".into(), profile.into(), Value::Null).await?;
    let accounts: Vec<Account> = serde_json::from_value(rows).map_err(|e| e.to_string())?;
    let mut errors = Vec::new();
    for account in accounts {
        if let Err(error) = check_account(app, profile, &account, notify).await {
            errors.push(format!("{} · {error}", account.address));
        }
    }
    resume(app.clone(), profile.to_owned()).await?;
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join(" / "))
    }
}

async fn check_account(
    app: &AppHandle,
    profile: &str,
    account: &Account,
    notify: bool,
) -> Result<(), String> {
    let count = api::execute("sync".into(), profile.into(), json!({"account":account.id}))
        .await?
        .as_u64()
        .unwrap_or_default() as usize;
    if count > 0 && notify {
        desktop::notify_mail(app.clone(), profile.into(), account.id.clone(), count)?;
    }
    Ok(())
}

pub async fn resume(app: AppHandle, profile: String) -> Result<(), String> {
    use std::{collections::HashSet, sync::LazyLock};
    static ARMED: LazyLock<Mutex<HashSet<(String, String, i64)>>> = LazyLock::new(Default::default);
    let rows = api::execute("deadlines".into(), profile.clone(), Value::Null).await?;
    for row in rows.as_array().unwrap() {
        let op = row["op"].as_str().unwrap().to_owned();
        let due = row["due"].as_i64().unwrap();
        let key = (profile.clone(), op.clone(), due);
        if !ARMED.lock().unwrap().insert(key.clone()) {
            continue;
        }
        let profile = profile.clone();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(
                (due - crate::store::now()).max(0) as u64
            ))
            .await;
            let result = api::execute(op, profile.clone(), Value::Null).await;
            ARMED.lock().unwrap().remove(&key);
            let _ = app.emit(
                "scheduled-mail",
                json!({"profile":profile,"error":result.err()}),
            );
        });
    }
    Ok(())
}
