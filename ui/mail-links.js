import { escapeHtml as h, splitQuote } from "./helpers.js";
function webLink(value) {
  try {
    const url = new URL(value);
    return ["https:", "http:"].includes(url.protocol) ? url.href : "";
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
