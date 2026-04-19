// enscrive-docs ⌘K search palette
//
// Wires up the overlay defined in templates/_base.html. Trigger via the
// header button, the optional hero CTA, or Cmd/Ctrl+K. Sends queries to
// the endpoint named in the script tag's data-search-endpoint attribute.

(function () {
  "use strict";

  function ready(fn) {
    if (document.readyState !== "loading") fn();
    else document.addEventListener("DOMContentLoaded", fn);
  }

  function debounce(fn, ms) {
    let t;
    return function (...args) {
      clearTimeout(t);
      t = setTimeout(() => fn.apply(this, args), ms);
    };
  }

  function el(tag, attrs, children) {
    const node = document.createElement(tag);
    if (attrs) {
      Object.entries(attrs).forEach(([k, v]) => {
        if (k === "class") node.className = v;
        else if (k === "html") node.innerHTML = v;
        else node.setAttribute(k, v);
      });
    }
    (children || []).forEach((c) => {
      if (typeof c === "string") node.appendChild(document.createTextNode(c));
      else if (c) node.appendChild(c);
    });
    return node;
  }

  ready(function () {
    const script = document.querySelector("script[data-search-endpoint]");
    const endpoint = script ? script.getAttribute("data-search-endpoint") : "/search";
    const overlay = document.getElementById("ed-search-overlay");
    const input = document.getElementById("ed-search-input");
    const results = document.getElementById("ed-search-results");
    const triggers = document.querySelectorAll("[data-search-trigger]");
    if (!overlay || !input || !results) return;

    let activeIndex = -1;
    let lastQuery = "";

    function open() {
      overlay.hidden = false;
      input.value = "";
      results.innerHTML = "";
      lastQuery = "";
      activeIndex = -1;
      setTimeout(() => input.focus(), 0);
    }

    function close() {
      overlay.hidden = true;
    }

    function setActive(idx) {
      const items = results.querySelectorAll("li[data-result]");
      items.forEach((node, i) => node.classList.toggle("active", i === idx));
      activeIndex = idx;
      const node = items[idx];
      if (node && node.scrollIntoView) {
        node.scrollIntoView({ block: "nearest" });
      }
    }

    function navigate(idx) {
      const items = results.querySelectorAll("li[data-result] a");
      const link = items[idx];
      if (link && link.href) window.location.href = link.href;
    }

    function renderResults(items) {
      results.innerHTML = "";
      if (!items || items.length === 0) {
        results.appendChild(el("li", { class: "ed-search-empty" }, ["No matches."]));
        return;
      }
      items.forEach((item) => {
        const url = item.url || "#";
        const title = item.title || item.document_id || "(untitled)";
        const snippet = (item.snippet || item.content || "")
          .replace(/\s+/g, " ")
          .trim()
          .slice(0, 240);
        const score = typeof item.score === "number" ? item.score.toFixed(2) : "";
        results.appendChild(
          el("li", { "data-result": "1" }, [
            el("a", { href: url }, [
              el("span", { class: "ed-search-score" }, [score]),
              el("div", { class: "ed-search-title" }, [title]),
              el("div", { class: "ed-search-snippet" }, [snippet]),
            ]),
          ]),
        );
      });
      setActive(0);
    }

    function renderState(message, cls) {
      results.innerHTML = "";
      results.appendChild(el("li", { class: cls || "ed-search-empty" }, [message]));
    }

    const search = debounce(async function (query) {
      if (!query) {
        results.innerHTML = "";
        return;
      }
      if (query === lastQuery) return;
      lastQuery = query;
      renderState("Searching...", "ed-search-loading");
      try {
        const res = await fetch(endpoint + "?q=" + encodeURIComponent(query));
        if (!res.ok) {
          renderState("Search failed (HTTP " + res.status + ").");
          return;
        }
        const data = await res.json();
        renderResults(data.results || []);
      } catch (e) {
        renderState("Search failed: " + (e && e.message ? e.message : "network error"));
      }
    }, 150);

    triggers.forEach((btn) => btn.addEventListener("click", open));

    input.addEventListener("input", function () {
      search(input.value.trim());
    });

    document.addEventListener("keydown", function (e) {
      const isMeta = e.metaKey || e.ctrlKey;
      if (isMeta && (e.key === "k" || e.key === "K")) {
        e.preventDefault();
        if (overlay.hidden) open();
        else close();
        return;
      }
      if (overlay.hidden) return;
      if (e.key === "Escape") {
        e.preventDefault();
        close();
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        const items = results.querySelectorAll("li[data-result]");
        if (items.length) setActive(Math.min(activeIndex + 1, items.length - 1));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setActive(Math.max(activeIndex - 1, 0));
      } else if (e.key === "Enter") {
        e.preventDefault();
        if (activeIndex >= 0) navigate(activeIndex);
      }
    });

    overlay.addEventListener("click", function (e) {
      if (e.target === overlay) close();
    });
  });
})();
