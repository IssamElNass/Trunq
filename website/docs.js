// Docs navigation, section contents, reading progress and FAQ. Needs site.js.

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
  const currentLabel = $("[data-docs-current]");
  const progress = $("[data-reading-progress]");
  const narrow = matchMedia("(max-width: 960px)");

  // Collapse the menu on small screens, and close it after picking a page.
  const syncMenu = () => (menu.open = !narrow.matches);
  syncMenu();
  // Collapsing the menu moves the content, so land on the linked heading again.
  // The FAQ is built after the browser's own jump, and fonts can shift the page, so land again once settled.
  if (location.hash) {
    const land = () => { if (!userScrolled) document.getElementById(location.hash.slice(1))?.scrollIntoView({ behavior: "instant" }); };
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
  let subs = [];
  let subLinks = [];
  const heading = (section) => $("h2[id], h1", section);
  const headingText = (h) => [...h.childNodes].filter((n) => n.nodeType === Node.TEXT_NODE).map((n) => n.textContent).join("").trim();

  function setSection(section) {
    if (section === current) return;
    current = section;
    const id = $("h2[id]", section)?.id || section.id;
    sideLinks.forEach((a) => {
      if (a.getAttribute("href") === "#" + id) a.setAttribute("aria-current", "location");
      else a.removeAttribute("aria-current");
    });
    currentLabel.textContent = headingText(heading(section));
    // Scroll only the sidebar. scrollIntoView on a nav link also interrupts
    // the document's smooth anchor navigation and can pull it off its target.
    const active = sideLinks.find((a) => a.hasAttribute("aria-current"));
    if (!narrow.matches && active) {
      const side = $(".docs__side");
      const box = side.getBoundingClientRect(), link = active.getBoundingClientRect();
      if (link.top < box.top + 12) side.scrollTop += link.top - box.top - 12;
      else if (link.bottom > box.bottom - 12) side.scrollTop += link.bottom - box.bottom + 12;
    }

    subs = $$("h3[id]", section);
    toc.innerHTML = subs.length
      ? subs.map((h) => `<a href="#${h.id}">${esc(headingText(h))}</a>`).join("")
      : `<a href="#${id}" aria-current="location">${esc(headingText(heading(section)))}</a>`;
    subLinks = $$("a", toc);
  }

  let queued = false;
  function updatePosition() {
    queued = false;
    const line = $("[data-bar]").getBoundingClientRect().bottom + (narrow.matches ? 96 : 72);
    const section = sections.findLast((s) => heading(s).getBoundingClientRect().top <= line) || sections[0];
    setSection(section);
    const sub = subs.findLast((h) => h.getBoundingClientRect().top <= line);
    if (subs.length) subLinks.forEach((a) => {
      if (sub && a.getAttribute("href") === "#" + sub.id) a.setAttribute("aria-current", "location");
      else a.removeAttribute("aria-current");
    });
    const doc = $("#doc");
    const distance = Math.max(1, doc.offsetHeight - innerHeight + line);
    progress.value = Math.min(100, Math.max(0, (scrollY - doc.offsetTop) / distance * 100));
    progress.textContent = Math.round(progress.value) + "%";
  }
  const schedule = () => { if (!queued) { queued = true; requestAnimationFrame(updatePosition); } };
  addEventListener("scroll", schedule, { passive: true });
  addEventListener("resize", schedule);
  document.fonts?.ready.then(schedule);
  updatePosition();
})();
