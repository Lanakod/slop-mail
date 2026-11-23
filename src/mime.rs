use crate::store::{id, now};
use anyhow::Result;
use mailparse::{MailHeaderMap, ParsedMail};
use rusqlite::{params, OptionalExtension, Transaction};

#[derive(Default)]
struct Parts {
    text: String,
    html: String,
    files: Vec<(String, String, String, Vec<u8>)>,
}
fn parts(m: &ParsedMail<'_>, p: &mut Parts) -> Result<()> {
    let disp = m.get_content_disposition();
    let name = disp
        .params
        .get("filename")
        .or_else(|| m.ctype.params.get("name"));
    let kind = &m.ctype.mimetype;
    if name.is_some()
        || disp.disposition == mailparse::DispositionType::Attachment
        || kind.starts_with("image/")
    {
        p.files.push((
            safe_name(name.map_or("attachment", String::as_str)),
            kind.clone(),
            m.headers
                .get_first_value("Content-ID")
                .unwrap_or_default()
                .trim_matches(&['<', '>'][..])
                .into(),
            m.get_body_raw()?,
        ));
    } else if !m.subparts.is_empty() {
        for part in &m.subparts {
            parts(part, p)?;
        }
    } else if kind == "text/plain" {
        p.text += &m.get_body()?;
    } else if kind == "text/html" {
        p.html += &m.get_body()?;
    }
    Ok(())
}
pub fn safe_name(s: &str) -> String {
    let leaf = s.rsplit(['/', '\\']).next().unwrap_or("");
    let name: String = leaf
        .chars()
        .filter(|c| !c.is_control() && !"<>:\"|?*".contains(*c))
        .take(120)
        .collect();
    let n = name.trim_matches([' ', '.']);
    if n.is_empty()
        || matches!(
            n.split('.').next().unwrap_or("").to_uppercase().as_str(),
            "CON"
                | "PRN"
                | "AUX"
                | "NUL"
                | "COM1"
                | "COM2"
                | "COM3"
                | "COM4"
                | "COM5"
                | "COM6"
                | "COM7"
                | "COM8"
                | "COM9"
                | "LPT1"
                | "LPT2"
                | "LPT3"
                | "LPT4"
                | "LPT5"
                | "LPT6"
                | "LPT7"
                | "LPT8"
                | "LPT9"
        )
    {
        format!("attachment_{n}")
    } else {
        n.into()
    }
}
fn ids(s: &str) -> Vec<String> {
    s.split('<')
        .skip(1)
        .filter_map(|x| x.split_once('>'))
        .filter(|(x, _)| !x.is_empty() && !x.chars().any(char::is_whitespace))
        .map(|(x, _)| format!("<{x}>"))
        .collect()
}
pub fn save(
    t: &Transaction,
    account: &str,
    folder: &str,
    uid: &str,
    raw: &[u8],
    outgoing: bool,
    seen: bool,
) -> Result<i64> {
    if let Some(existing) = t
        .query_row(
            "SELECT id FROM messages WHERE account=? AND folder=? AND uid=?",
            params![account, folder, uid],
            |r| r.get(0),
        )
        .optional()?
    {
        return Ok(existing);
    }
    let m = mailparse::parse_mail(raw)?;
    let header = |key| m.headers.get_first_value(key).unwrap_or_default();
    let mid = ids(&header("Message-ID"))
        .into_iter()
        .next()
        .unwrap_or_else(|| format!("<{}@local.invalid>", id()));
    let mut refs = ids(&header("References"));
    refs.extend(ids(&header("In-Reply-To")));
    refs.push(mid.clone());
    let mut distinct = std::collections::HashSet::new();
    refs.retain(|v| distinct.insert(v.clone()));
    let mut thread = None;
    for v in &refs {
        if let Some(existing) = t
            .query_row("SELECT thread FROM refs WHERE ref=?", [v], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
        {
            if let Some(root) = &thread {
                if root != &existing {
                    t.execute(
                        "UPDATE messages SET thread=? WHERE thread=?",
                        params![root, existing],
                    )?;
                    t.execute(
                        "UPDATE refs SET thread=? WHERE thread=?",
                        params![root, existing],
                    )?;
                }
            } else {
                thread = Some(existing);
            }
        }
    }
    let thread = thread.unwrap_or_else(|| refs[0].clone());
    for v in &refs {
        t.execute("INSERT OR IGNORE INTO refs VALUES(?,?)", params![v, thread])?;
    }
    let mut p = Parts::default();
    parts(&m, &mut p)?;
    let html = clean_html(&p.html);
    if p.text.is_empty() {
        p.text = html2text::from_read(html.as_bytes(), 100).unwrap_or_default();
    }
    let recipients = [header("To"), header("Cc")]
        .into_iter()
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    t.execute("INSERT INTO messages(account,folder,uid,mid,thread,stamp,subject,sender,recipients,reply_to,refs,body,html,outgoing,seen) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",params![account,folder,uid,mid,thread,mailparse::dateparse(&header("Date")).unwrap_or_else(|_|now()),header("Subject"),header("From"),recipients,header("Reply-To"),refs.join(" "),p.text,html,outgoing,seen])?;
    let row = t.last_insert_rowid();
    for (n, m, c, b) in p.files {
        t.execute(
            "INSERT INTO attachments(message,name,mime,cid,data) VALUES(?,?,?,?,?)",
            params![row, n, m, c, b],
        )?;
    }
    Ok(row)
}

fn clean_html(html: &str) -> String {
    ammonia::Builder::default()
        .add_url_schemes(&["cid"])
        .add_generic_attributes(&["style", "align"])
        .filter_style_properties(
            [
                "color",
                "background-color",
                "font-family",
                "font-size",
                "font-weight",
                "font-style",
                "line-height",
                "text-align",
                "text-decoration",
                "vertical-align",
                "width",
                "max-width",
                "padding",
                "padding-left",
                "padding-right",
                "padding-top",
                "padding-bottom",
                "margin",
                "border-radius",
                "border-collapse",
            ]
            .into(),
        )
        .clean(html)
        .to_string()
}
pub fn original_html(raw: &[u8], mid: &str) -> Result<String> {
    let msg = mailparse::parse_mail(raw)?;
    if let Some(original) = msg.headers.get_first_value("Message-ID") {
        anyhow::ensure!(original.trim() == mid, "Идентификатор письма изменился");
    }
    let mut p = Parts::default();
    parts(&msg, &mut p)?;
    Ok(clean_html(&p.html))
}

#[cfg(test)]
mod layout_tests {
    use super::clean_html;

    #[test]
    fn preserves_newsletter_layout_without_loading_stylesheets() {
        let clean = clean_html(
            r#"<table><tr><td align="center" style="padding:16px;background-color:gold;background-image:url(https://example.org/pixel)">Text</td></tr></table>"#,
        );
        assert!(clean.contains("padding:16px"));
        assert!(clean.contains("background-color:gold"));
        assert!(clean.contains("align=\"center\""));
        assert!(!clean.contains("example.org"));
    }
}
