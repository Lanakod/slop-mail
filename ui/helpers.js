export function escapeHtml(value) {
  return String(value ?? "").replace(
    /[&<>"']/g,
    (c) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
        c
      ],
  );
}
export function splitQuote(body) {
  const lines = body.split(/\r\n|\n|\r/);
  const quoted = lines.map((line) => /^[ \t]*>/.test(line));
  const attribution =
    /^(?:On .+ wrote:|.+ (?:на)?писал(?:а|\(а\))?(?:\s+.*)?:|[-]+\s*Original Message\s*[-]+|Цитата:?)$/iu;
  // Fold a heading only when it actually introduces quoted lines.
  let nextIsQuote = false;
  for (let i = lines.length - 1; i >= 0; i--) {
    const text = lines[i].trim();
    if (!text) {
      quoted[i] = nextIsQuote;
      continue;
    }
    quoted[i] ||= nextIsQuote && attribution.test(text);
    nextIsQuote = quoted[i];
  }
  const blocks = [];
  for (let i = 0; i < lines.length; i++) {
    let last = blocks.at(-1);
    // Blank lines belong to a quotation only between two quoted passages.
    if (!lines[i].trim()) quoted[i] = quoted[i] && !!last?.quoted;
    if (!last || last.quoted !== quoted[i]) {
      last = { quoted: quoted[i], lines: [] };
      blocks.push(last);
    }
    last.lines.push(lines[i]);
  }
  return blocks.map((block) => ({
    quoted: block.quoted,
    text: block.lines.join("\n"),
  }));
}
export function options(items, value) {
  return items
    .map(
      ([id, name]) =>
        `<option value="${escapeHtml(id)}" ${id === value ? "selected" : ""}>${escapeHtml(name)}</option>`,
    )
    .join("");
}
export function basename(path) {
  return path.split(/[\\/]/).pop();
}
