import ICAL from "ical.js";
import Papa from "papaparse";
const emailList = (s) =>
  String(s || "")
    .split(/[,;]/)
    .map((x) => x.trim())
    .filter(Boolean);
export function readContacts(text, csv) {
  text = text.replace(/^\uFEFF/, "");
  if (!csv) {
    const cards =
      text.match(/^BEGIN:VCARD\s*$[\s\S]*?^END:VCARD[ \t]*\r?$/gim) || [];
    if (!cards.length) throw Error("В файле нет карточек vCard");
    return cards.map((raw) => {
      const c = new ICAL.Component(ICAL.parse(raw.trim())),
        props = c.getAllProperties("email");
      const primary = (p) =>
        Number(p.getParameter("pref")) ||
        ([p.getParameter("type")]
          .flat()
          .some((t) => String(t).toLowerCase() === "pref")
          ? 1
          : 999);
      props.sort((a, b) => primary(a) - primary(b));
      const labels = {};
      const emails = props.map((p) => {
        const email = String(p.getFirstValue()).replace(/^mailto:/i, "");
        labels[email] = String(
          p.getParameter("x-slop-label") ||
            [p.getParameter("type")]
              .flat()
              .filter((t) => t && String(t).toLowerCase() !== "pref")
              .join(", "),
        );
        return email;
      });
      return {
        name: String(c.getFirstPropertyValue("fn") || emails[0] || ""),
        emails: emails.join(", "),
        labels,
      };
    });
  }
  const result = Papa.parse(text, {
    header: true,
    skipEmptyLines: "greedy",
    transformHeader: (s) => s.trim().toLowerCase(),
  });
  if (result.errors.some((e) => e.code !== "UndetectableDelimiter"))
    throw Error("Ошибка CSV: " + result.errors[0].message);
  const emailColumns = result.meta.fields.filter(
    (k) =>
      /email|e-mail|электронн.*почт|почтовый адрес/i.test(k) &&
      !/(type|метка|label|display|тип)/i.test(k),
  );
  if (!emailColumns.length) throw Error("В CSV нужна колонка Email");
  return result.data.map((r) => {
    const emails = emailColumns.flatMap((k) => emailList(r[k]));
    let labels = {};
    if (r.labels) {
      try {
        labels = JSON.parse(r.labels);
      } catch {
        throw Error("Неверная колонка Labels");
      }
    }
    return {
      name:
        r.name ||
        r["display name"] ||
        r["отображаемое имя"] ||
        r["имя"] ||
        [r["first name"], r["last name"]].filter(Boolean).join(" ") ||
        emails[0] ||
        "",
      emails: emails.join(", "),
      labels,
    };
  });
}
export function importPreview(rows, people) {
  const seen = new Set(
    people.flatMap((p) => emailList(p.emails).map((s) => s.toLowerCase())),
  );
  const items = [],
    duplicates = [],
    invalid = [];
  for (const p of rows) {
    const fresh = [];
    for (const email of emailList(p.emails)) {
      if (!/^[^\s@]+@[^\s@]+$/.test(email)) {
        invalid.push(email);
        continue;
      }
      const key = email.toLowerCase();
      if (seen.has(key)) {
        duplicates.push(email);
        continue;
      }
      seen.add(key);
      fresh.push(email);
    }
    if (fresh.length) items.push({ ...p, emails: fresh.join(", ") });
  }
  return { items, duplicates, invalid };
}
export function writeContacts(people, csv) {
  if (csv)
    return (
      "\uFEFF" +
      Papa.unparse(
        {
          fields: ["Name", "Email", "Labels"],
          data: people.map((p) => [
            p.name,
            p.emails,
            JSON.stringify(p.labels || {}),
          ]),
        },
        { escapeFormulae: true },
      )
    );
  return (
    people
      .map((p) => {
        const c = new ICAL.Component("vcard");
        c.addPropertyWithValue("version", "4.0");
        c.addPropertyWithValue("fn", p.name || emailList(p.emails)[0]);
        emailList(p.emails).forEach((e, i) => {
          const prop = c.addPropertyWithValue("email", e);
          if (i === 0) prop.setParameter("pref", "1");
          if (p.labels?.[e]) prop.setParameter("x-slop-label", p.labels[e]);
        });
        return c.toString();
      })
      .join("\r\n") + "\r\n"
  );
}
