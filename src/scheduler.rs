use crate::{api, desktop, store::Account};
use serde_json::{json, Value};
use std::{sync::Mutex, time::Duration};
use tauri::{async_runtime::JoinHandle, AppHandle, Emitter, State};

#[derive(Default)]
pub struct Scheduler(Mutex<Option<JoinHandle<()>>>);

#[tauri::command]
pub fn set_schedule(
    app: AppHandle,
    scheduler: State<'_, Scheduler>,
    profile: String,
    minutes: u32,
    notify: bool,
) -> Result<(), String> {
    let mut task = scheduler.0.lock().map_err(|e| e.to_string())?;
    if let Some(old) = task.take() {
        old.abort();
    }
    if !profile.is_empty() && minutes > 0 {
        *task = Some(tauri::async_runtime::spawn(async move {
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
        let result =
            api::execute("sync".into(), profile.into(), json!({"account":account.id})).await;
        match result.map(|count| count.as_u64().unwrap_or_default() as usize) {
            Ok(count) if notify && count > 0 => {
                if let Err(error) =
                    desktop::notify_mail(app.clone(), profile.into(), account.id.clone(), count)
                {
                    errors.push(error);
                }
            }
            Err(error) => errors.push(format!("{} · {error}", account.address)),
            _ => {}
        }
    }
    resume(app.clone(), profile.to_owned()).await?;
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join(" / "))
    }
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
