import { escapeHtml as h, splitQuote } from "./helpers.js";
function webLink(value) {
  try {
    const url = new URL(value);
    return ["https:", "http:", "mailto:"].includes(url.protocol)
      ? url.href
      : "";
  } catch {
    return "";
  }
}
function anchor(label, url) {
  return '<a href="' + h(url) + '" title="' + h(url) + '">' + h(label) + "</a>";
}
function linkText(text) {
  return text
    .split(/(https?:\/\/[^\s<>"\u0000]+)/gi)
    .map((part, i) => {
      if (!(i % 2)) return h(part);
      const url = part
        .replace(/[.,;:!?»]+$/u, "")
        .replace(/\)$/, (m) => (part.includes("(") ? m : ""));
      return webLink(url)
        ? anchor(url, url) + h(part.slice(url.length))
        : h(part);
    })
    .join("");
}
// Older HTML-only messages may contain html2text reference links and wrapped footnotes.
function restoreFootnotes(text, links) {
  const lines = text.replace(/\r\n/g, "\n").split("\n");
  const notes = new Map();
  for (let i = 0; i < lines.length; i++) {
    const match = lines[i].match(/^\[(\d+)\]:\s*(mailto:\S+)\s*$/i);
    if (!match) continue;
    let url = match[2],
      end = i;
    while (
      end + 1 < lines.length &&
      /^(?=[^\s]*%)[\w%?&=+@.,;:/~-]+$/.test(lines[end + 1])
    )
      url += lines[++end];
    if (webLink(url)) notes.set(match[1], { url, start: i, end });
    i = end;
  }
  const used = new Set();
  const result = lines.map((line) =>
    line.replace(/\[([^\]\n]+)\]\[(\d+)\]/g, (original, label, id) => {
      const note = notes.get(id);
      if (!note) return original;
      used.add(id);
      links.push(anchor(label, note.url));
      return "\x00" + (links.length - 1) + "\x00";
    }),
  );
  for (const id of used) {
    const note = notes.get(id);
    for (let i = note.start; i <= note.end; i++) result[i] = "";
  }
  return result.join("\n").trimEnd();
}
export function chatBody(message) {
  let source = message.body;
  const links = [];
  if (message.html) {
    const tpl = document.createElement("template");
    tpl.innerHTML = message.html;
    const text = (node) => {
      if (node.nodeType === 3) return node.textContent.replace(/\x00/g, "");
      const label = Array.from(node.childNodes, text).join("");
      if (node.nodeName === "A") {
        const url = webLink(node.getAttribute("href"));
        if (url) {
          links.push(anchor(node.textContent || url, url));
          return "\x00" + (links.length - 1) + "\x00";
        }
      }
      if (node.nodeName === "BLOCKQUOTE")
        return (
          "\n" +
          label
            .trim()
            .split("\n")
            .map((s) => "> " + s)
            .join("\n") +
          "\n"
        );
      return /^(P|DIV|BR|LI|TR|H[1-6])$/.test(node.nodeName)
        ? label + "\n"
        : label;
    };
    const body = text(tpl.content);
    if (links.length) source = body.trim();
  }
  if (!links.length)
    source = restoreFootnotes(source.replace(/\x00/g, ""), links);
  return splitQuote(source)
    .map(({ quoted, text }) => {
      const html = links.length
        ? text
            .split(/\x00(\d+)\x00/g)
            .map((s, i) => (i % 2 ? links[Number(s)] || "" : linkText(s)))
            .join("")
        : linkText(text);
      const pre = "<pre>" + html + "</pre>";
      return quoted
        ? "<details><summary>Цитата</summary>" + pre + "</details>"
        : pre;
    })
    .join("");
}

export function mailtoDraft(href) {
  const url = new URL(href);
  if (url.protocol !== "mailto:") throw Error("Ожидается почтовая ссылка");
  const params = new URLSearchParams(url.search.slice(1).replace(/\+/g, "%2B"));
  const to = decodeURIComponent(url.pathname),
    subject = params.get("subject") || "",
    body = params.get("body") || "";
  if (/[\r\n]/.test(to + subject))
    throw Error("Некорректный адрес или тема письма");
  return { to, subject, body };
}
