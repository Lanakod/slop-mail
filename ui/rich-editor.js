const commands = [
  ["bold", "Жирный", "Ж"],
  ["italic", "Курсив", "К"],
  ["insertUnorderedList", "Маркированный список", "• Список"],
  ["insertOrderedList", "Нумерованный список", "1. Список"],
];
function safeMarkup(html) {
  const template = document.createElement("template");
  template.innerHTML = html;
  function copy(node, host) {
    if (node.nodeType === 3) {
      host.append(document.createTextNode(node.textContent));
      return;
    }
    const tag = node.nodeName.toLowerCase();
    if (["script", "style", "iframe", "object"].includes(tag)) return;
    let next = host;
    if (
      [
        "b",
        "strong",
        "i",
        "em",
        "p",
        "div",
        "br",
        "ul",
        "ol",
        "li",
        "a",
      ].includes(tag)
    ) {
      next = document.createElement(tag);
      if (
        tag === "a" &&
        /^(https?:\/\/|mailto:)/i.test(node.getAttribute("href") || "")
      )
        next.setAttribute("href", node.getAttribute("href"));
      host.append(next);
    }
    if (tag === "span") {
      const tags = [];
      if (/^(bold|[6-9]00)$/.test(node.style.fontWeight)) tags.push("strong");
      if (node.style.fontStyle === "italic") tags.push("em");
      for (const name of tags) {
        const wrapper = document.createElement(name);
        next.append(wrapper);
        next = wrapper;
      }
    }
    for (const child of node.childNodes) copy(child, next);
  }
  const clean = document.createElement("div");
  for (const child of template.content.childNodes) copy(child, clean);
  return clean.innerHTML;
}
export function richEditor(host, draft, onChange) {
  const body = host.querySelector('[data-field="body"]'),
    toolbar = host.querySelector(".format-tools");
  if (draft.html) body.innerHTML = safeMarkup(draft.html);
  else
    (draft.body || "").split(/\r?\n/).forEach((line, i) => {
      if (i) body.append(document.createElement("br"));
      body.append(document.createTextNode(line));
    });
  const execute = (cmd, value = null) => {
    body.focus();
    document.execCommand(cmd, false, value);
    onChange();
  };
  function button(title, label, callback) {
    const b = document.createElement("button");
    b.type = "button";
    b.title = title;
    b.setAttribute("aria-label", title);
    b.textContent = label;
    b.onmousedown = (e) => e.preventDefault();
    b.onclick = callback;
    toolbar.append(b);
    return b;
  }
  for (const [cmd, title, label] of commands)
    button(title, label, () => execute(cmd));
  const linkForm = host.querySelector(".link-form"),
    url = linkForm.querySelector("input");
  let range = null;
  function selection() {
    const s = window.getSelection();
    return s.rangeCount && body.contains(s.anchorNode)
      ? s.getRangeAt(0).cloneRange()
      : null;
  }
  function restore(r) {
    body.focus();
    const sel = window.getSelection();
    if (!r || !body.contains(r.commonAncestorContainer)) {
      r = document.createRange();
      r.selectNodeContents(body);
      r.collapse(false);
    }
    sel.removeAllRanges();
    sel.addRange(r);
  }
  button("Вставить ссылку", "Ссылка", () => {
    range = selection();
    url.value = "";
    linkForm.hidden = false;
    url.focus();
  });
  linkForm.onsubmit = (e) => {
    e.preventDefault();
    let target;
    try {
      target = new URL(url.value);
    } catch {
      target = null;
    }
    if (!target || !["https:", "http:", "mailto:"].includes(target.protocol)) {
      url.setCustomValidity("Введите адрес https://, http:// или mailto:");
      url.reportValidity();
      return;
    }
    restore(range);
    if (window.getSelection().isCollapsed) {
      const a = document.createElement("a");
      a.href = target.href;
      a.textContent = url.value;
      execute("insertHTML", a.outerHTML);
    } else execute("createLink", target.href);
    linkForm.hidden = true;
  };
  url.oninput = () => url.setCustomValidity("");
  linkForm.querySelector('[type="button"]').onclick = () => {
    linkForm.hidden = true;
    restore(range);
  };
  body.onpaste = (e) => {
    e.preventDefault();
    execute("insertText", e.clipboardData.getData("text/plain"));
  };
  return {
    value: () => ({
      body: body.innerText ?? body.textContent,
      html: safeMarkup(body.innerHTML),
    }),
    insertSnippet() {
      const r = selection();
      return (text) => {
        restore(r);
        execute("insertText", text);
      };
    },
  };
}
