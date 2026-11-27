// The user guide's search: Fuse.js (fuzzy, so a typo still finds the
// section) over search.json, which scripts/monocard-guide.py writes. Both
// load on the first focus of the field, not with the page.
(() => {
  const input = document.getElementById("guide-q");
  const list = document.getElementById("guide-hits");
  const note = document.getElementById("guide-q-note");
  if (!input || !list || !note) return;

  let fuse = null;
  let loading = null;

  const load = () =>
    (loading ??= Promise.all([
      new Promise((ok, fail) => {
        const s = document.createElement("script");
        s.src = input.dataset.fuse;
        s.onload = ok;
        s.onerror = fail;
        document.head.append(s);
      }),
      fetch("search.json").then((r) => {
        if (!r.ok) throw new Error(`search.json: ${r.status}`);
        return r.json();
      }),
    ])
      .then(([, sections]) => {
        fuse = new Fuse(sections, {
          keys: [
            { name: "h", weight: 3 },
            { name: "t", weight: 1 },
            { name: "c", weight: 0.5 },
          ],
          includeMatches: true,
          ignoreLocation: true,
          threshold: 0.3,
          minMatchCharLength: 2,
        });
      })
      .catch(() => {
        loading = null; // the next keystroke tries again
        note.textContent = "The search could not load.";
      }));

  // The text with its longest match marked, as nodes: never as HTML.
  const marked = (text, indices, around) => {
    const frag = document.createDocumentFragment();
    if (!indices?.length) {
      frag.append(around ? text.slice(0, around * 2) + (text.length > around * 2 ? "…" : "") : text);
      return frag;
    }
    const [s, e] = indices.reduce((a, b) => (b[1] - b[0] > a[1] - a[0] ? b : a));
    const from = around ? Math.max(0, s - around) : 0;
    const to = around ? Math.min(text.length, e + 1 + around * 1.5) : text.length;
    const mark = document.createElement("mark");
    mark.textContent = text.slice(s, e + 1);
    frag.append(from > 0 ? "…" : "", text.slice(from, s), mark, text.slice(e + 1, to), to < text.length ? "…" : "");
    return frag;
  };

  const show = (query) => {
    const q = query.trim();
    list.replaceChildren();
    if (q.length < 2 || !fuse) {
      list.hidden = true;
      note.textContent = "";
      return;
    }
    const hits = fuse.search(q, { limit: 8 });
    for (const { item, matches } of hits) {
      const at = (key) => matches.find((m) => m.key === key)?.indices;
      const a = document.createElement("a");
      a.href = item.p + (item.a ? `#${item.a}` : "");
      const h = document.createElement("span");
      h.className = "hit-h";
      h.append(marked(item.h, at("h"), 0));
      const c = document.createElement("span");
      c.className = "hit-c";
      c.textContent = item.c;
      const t = document.createElement("span");
      t.className = "hit-t";
      t.append(marked(item.t, at("t"), 40));
      a.append(h, c, t);
      const li = document.createElement("li");
      li.append(a);
      list.append(li);
    }
    if (!hits.length) {
      const li = document.createElement("li");
      li.className = "hit-none";
      li.textContent = "No section matches.";
      list.append(li);
    }
    list.hidden = false;
    note.textContent = hits.length ? `${hits.length} ${hits.length === 1 ? "section" : "sections"} found.` : "No section matches.";
  };

  input.addEventListener("focus", load, { once: true });
  input.addEventListener("input", async () => {
    await load();
    show(input.value);
  });

  const links = () => [...list.querySelectorAll("a")];
  input.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      // An IME's Enter confirms a character, not the search.
      if (event.isComposing || event.keyCode === 229) return;
      links()[0]?.click();
    } else if (event.key === "ArrowDown") {
      event.preventDefault();
      links()[0]?.focus();
    } else if (event.key === "Escape" && input.value) {
      input.value = "";
      show("");
    }
  });
  list.addEventListener("keydown", (event) => {
    const all = links();
    const i = all.indexOf(document.activeElement);
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const next = i + (event.key === "ArrowDown" ? 1 : -1);
      (next < 0 ? input : all[Math.min(next, all.length - 1)]).focus();
    } else if (event.key === "Escape") {
      input.focus();
    }
  });
  // A result on this page scrolls to it: the list steps aside.
  list.addEventListener("click", (event) => {
    if (event.target.closest("a")) list.hidden = true;
  });

  // "/" goes to the field from anywhere but another field.
  document.addEventListener("keydown", (event) => {
    if (event.key !== "/" || event.metaKey || event.ctrlKey || event.altKey) return;
    if (event.target.closest?.("input, textarea, select, [contenteditable]")) return;
    event.preventDefault();
    input.focus();
  });
})();
