use crate::store::Server;
use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};
use std::{io::Read, time::Duration};

fn parse(xml: &str, address: &str) -> Result<Value> {
    let doc = roxmltree::Document::parse(xml)?;
    let (local, domain) = address.rsplit_once('@').context("Нужен email")?;
    let provider = doc
        .descendants()
        .find(|n| n.has_tag_name("emailProvider"))
        .context("Нет emailProvider")?;
    let endpoint = |tag: &str, protocols: &[&str]| -> Result<(Server, String)> {
        for node in provider.children().filter(|n| n.has_tag_name(tag)) {
            let field = |name: &str| {
                node.children()
                    .find(|n| n.has_tag_name(name))
                    .and_then(|n| n.text())
                    .unwrap_or("")
                    .trim()
            };
            let proto = node.attribute("type").unwrap_or("");
            let tls = match field("socketType") {
                "SSL" => "tls",
                "STARTTLS" => "starttls",
                _ => continue,
            };
            if !protocols.contains(&proto) {
                continue;
            }
            let auth: Vec<_> = node
                .children()
                .filter(|n| n.has_tag_name("authentication"))
                .filter_map(|n| n.text())
                .collect();
            if !auth.is_empty()
                && !auth
                    .iter()
                    .any(|a| matches!(a.trim(), "password-cleartext" | "plain" | "password"))
            {
                continue;
            }
            let host = field("hostname").replace("%EMAILDOMAIN%", domain);
            let Ok(port) = field("port").parse::<u16>() else {
                continue;
            };
            if port == 0 || !dns_name(&host) {
                continue;
            }
            let login = field("username")
                .replace("%EMAILADDRESS%", address)
                .replace("%EMAILLOCALPART%", local)
                .replace("%EMAILDOMAIN%", domain);
            return Ok((
                Server {
                    proto: proto.into(),
                    host,
                    port,
                    tls: tls.into(),
                },
                if login.is_empty() {
                    address.into()
                } else {
                    login
                },
            ));
        }
        bail!("Нет совместимого TLS-сервера {tag} с паролем")
    };
    let (incoming, login) = endpoint("incomingServer", &["imap", "pop3"])?;
    let (outgoing, smtp_login) = endpoint("outgoingServer", &["smtp"])?;
    Ok(json!({ "incoming":incoming,"outgoing":outgoing,"login":login,"smtp_login":smtp_login }))
}
fn dns_name(host: &str) -> bool {
    host.len() <= 253
        && host.contains('.')
        && host.split('.').all(|s| {
            !s.is_empty()
                && s.len() <= 63
                && !s.starts_with('-')
                && !s.ends_with('-')
                && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}
pub fn discover(address: &str) -> Result<Value> {
    let addr = address.parse::<lettre::Address>()?;
    let domain = addr.domain().to_ascii_lowercase();
    ensure!(dns_name(&domain), "Проверьте домен адреса");
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .timeout(Duration::from_secs(5))
        .build()?;
    for url in [
        format!("https://autoconfig.{domain}/mail/config-v1.1.xml"),
        format!("https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml"),
        format!("https://autoconfig.thunderbird.net/v1.1/{domain}"),
    ] {
        let Ok(res) = client.get(&url).send().and_then(|r| r.error_for_status()) else {
            continue;
        };
        let mut xml = String::new();
        if res.take(524289).read_to_string(&mut xml).is_ok() && xml.len() <= 524288 {
            if let Ok(mut config) = parse(&xml, address) {
                config["source"] = json!(url);
                return Ok(config);
            }
        }
    }
    bail!("Для {domain} не найдены совместимые настройки. Выберите шаблон или заполните поля вручную.")
}
#[test]
fn reads_secure_endpoints_and_login_placeholders() {
    let xml = r#"<clientConfig><emailProvider><incomingServer type="imap"><socketType>plain</socketType></incomingServer><incomingServer type="pop3"><hostname>mx.%EMAILDOMAIN%</hostname><port>995</port><socketType>SSL</socketType><username>%EMAILLOCALPART%</username></incomingServer><outgoingServer type="smtp"><hostname>relay.example.org</hostname><port>587</port><socketType>STARTTLS</socketType></outgoingServer></emailProvider></clientConfig>"#;
    let data = parse(xml, "user@example.org").unwrap();
    assert_eq!(data["login"], "user");
    assert_eq!(data["incoming"]["host"], "mx.example.org");
    assert_eq!(data["outgoing"]["tls"], "starttls");
}
