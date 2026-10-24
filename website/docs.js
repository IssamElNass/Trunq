// Docs page: sidebar highlighting, "On this page" for the current section, heading anchors. Needs site.js.

(() => {
  "use strict";

  const { $, $$, esc, faq, addCommands, goto } = window.Trunq;

  // FAQ: built from faq.js before anything below reads the headings.
  const faqBox = $("[data-faq-docs]");
  faqBox.innerHTML = faq.topics.map(([id, label]) => `
    <h3 id="faq-${id}">${esc(label)}</h3>
    ${faq.items.filter((i) => i.topic === id).map((i) =>
      `<details id="faq-${i.id}"><summary>${esc(i.q)}</summary><p>${faq.answer(i)}</p></details>`).join("")}`).join("");
  addCommands(faq.items.map((i) => ({
    group: "FAQ", label: i.q, icon: "book", extra: "faq question", body: `${i.a} ${i.k || ""}`.toLowerCase(), searchOnly: true,
    action: () => { openFaq("faq-" + i.id); goto("#faq-" + i.id)(); },
  })));
  // A link straight to a question opens it.
  function openFaq(id) {
    const d = document.getElementById(id);
    if (d && d.tagName === "DETAILS") d.open = true;
  }
  openFaq(location.hash.slice(1));
  addEventListener("hashchange", () => openFaq(location.hash.slice(1)));
  const sideLinks = $$(".sidenav a");
  const toc = $("[data-toc]");
  const menu = $("[data-docs-menu]");
  const narrow = matchMedia("(max-width: 960px)");

  // Collapse the menu on small screens, and close it after picking a page.
  const syncMenu = () => (menu.open = !narrow.matches);
  syncMenu();
  // Collapsing the menu moves the content, so land on the linked heading again.
  // The FAQ is built after the browser's own jump, and fonts can shift the page, so land again once settled.
  if (location.hash) {
    const land = () => { if (!userScrolled) document.getElementById(location.hash.slice(1))?.scrollIntoView(); };
    let userScrolled = false;
    addEventListener("wheel", () => (userScrolled = true), { once: true, passive: true });
    addEventListener("touchmove", () => (userScrolled = true), { once: true, passive: true });
    land();
    document.fonts?.ready.then(land);
    addEventListener("load", land, { once: true });
  }
  narrow.addEventListener("change", syncMenu);
  sideLinks.forEach((a) => a.addEventListener("click", () => { if (narrow.matches) menu.open = false; }));

  // Hover anchors so people can link to any heading.
  $$(".doc h2[id], .doc h3[id]").forEach((h) => {
    const a = document.createElement("a");
    a.className = "anchor";
    a.href = "#" + h.id;
    a.setAttribute("aria-label", `Link to "${h.textContent}"`);
    a.textContent = "#";
    h.append(a);
  });

  // Each section is an h2 plus everything until the next one.
  const sections = $$(".doc section").filter((s) => $("h2[id]", s) || s.id);
  let current = null;
  let tocObserver = null;

  function setSection(section) {
    if (section === current) return;
    current = section;
    const id = $("h2[id]", section)?.id || section.id;
    sideLinks.forEach((a) => a.toggleAttribute("aria-current", a.getAttribute("href") === "#" + id));
    $(`.sidenav a[href="#${id}"]`)?.scrollIntoView({ block: "nearest" });

    const subs = $$("h3[id]", section);
    toc.innerHTML = subs.length
      ? subs.map((h) => `<a href="#${h.id}">${esc(h.firstChild.textContent)}</a>`).join("")
      : `<a href="#${id}" aria-current="true">${esc(($("h2", section) || $("h1", section)).firstChild.textContent)}</a>`;
    tocObserver?.disconnect();
    if (!subs.length) return;
    const links = $$("a", toc);
    tocObserver = new IntersectionObserver((entries) => {
      for (const e of entries) {
        if (!e.isIntersecting) continue;
        links.forEach((a) => a.toggleAttribute("aria-current", a.getAttribute("href") === "#" + e.target.id));
      }
    }, { rootMargin: "-20% 0px -70% 0px" });
    subs.forEach((h) => tocObserver.observe(h));
  }

  const sectionObserver = new IntersectionObserver((entries) => {
    for (const e of entries) if (e.isIntersecting) setSection(e.target);
  }, { rootMargin: "-25% 0px -70% 0px" });
  sections.forEach((s) => sectionObserver.observe(s));
  setSection(sections[0]);
})();
