// Recto landing page: the story that follows the scroll, the signature on the quote, the download
// note, entrances and counters. The page reads and works without it.
(function () {
  "use strict";

  var reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  var observe = "IntersectionObserver" in window;

  // macOS and Linux visitors: say their version is coming instead of offering a Windows download.
  var platform = ((navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || "").toLowerCase();
  var os = /mac/.test(platform) && !/iphone|ipad/.test(navigator.userAgent.toLowerCase()) ? "mac"
    : /linux/.test(platform) && !/android/.test(navigator.userAgent.toLowerCase()) ? "linux" : null;
  if (os) {
    document.querySelectorAll("[data-" + os + "]").forEach(function (el) {
      el.innerHTML = el.getAttribute("data-" + os);
      if (el.hasAttribute("data-href")) el.setAttribute("href", el.getAttribute("data-href"));
    });
  }

  // Header: compact once the page moves, hidden while reading down, back on any scroll up.
  var bar = document.querySelector(".nav-bar");
  var lastY = window.scrollY;
  var ticking = false;
  function onScroll() {
    ticking = false;
    var y = window.scrollY;
    bar.classList.toggle("is-scrolled", y > 24);
    if (y > 240 && y > lastY + 4) bar.classList.add("is-hidden");
    else if (y < lastY - 4 || y <= 240) bar.classList.remove("is-hidden");
    lastY = y;
  }
  if (bar) {
    window.addEventListener("scroll", function () {
      if (!ticking) { ticking = true; requestAnimationFrame(onScroll); }
    }, { passive: true });
    // Keyboard users always find it: focus inside shows it.
    bar.addEventListener("focusin", function () { bar.classList.remove("is-hidden"); });
  }

  // Story: the step in the middle of the screen sets what the quote shows (every earlier step too).
  var doc = document.querySelector(".story-doc");
  var steps = document.querySelectorAll(".step");
  function setStep(index) {
    for (var n = 1; n <= 5; n++) doc.classList.toggle("at-" + n, n <= index);
    steps.forEach(function (step, i) { step.classList.toggle("is-current", i === index); });
  }
  var small = window.matchMedia("(max-width: 900px)").matches;
  if (doc && observe && small && !reduce) {
    // Small screens: no pinning; the quote plays the steps while it is on screen.
    var at = 0;
    var timer = null;
    setStep(0);
    new IntersectionObserver(function (entries) {
      var visible = entries[0].isIntersecting;
      if (visible && !timer) {
        timer = setInterval(function () { at = (at + 1) % 6; setStep(at); }, 2200);
      } else if (!visible && timer) {
        clearInterval(timer);
        timer = null;
      }
    }, { threshold: 0.5 }).observe(doc);
  } else if (doc && observe && !small) {
    setStep(0);
    var middle = new IntersectionObserver(function (entries) {
      entries.forEach(function (entry) {
        if (entry.isIntersecting) setStep(Number(entry.target.getAttribute("data-step")));
      });
    }, { rootMargin: "-45% 0px -45% 0px" });
    steps.forEach(function (step) { middle.observe(step); });
  } else if (doc) {
    setStep(5);
  }

  // Signature: the typed name, style and ink, written on the quote.
  var name = document.getElementById("sign-name");
  var signature = document.querySelector(".ov-sig");
  var write = document.querySelector(".write");
  function pick(group, button) {
    group.querySelectorAll("[role=radio]").forEach(function (b) { b.setAttribute("aria-checked", b === button ? "true" : "false"); });
  }
  if (name && signature) {
    name.addEventListener("input", function () { signature.textContent = name.value.trim() || name.placeholder; });
    document.querySelectorAll(".sign-row").forEach(function (group) {
      group.addEventListener("click", function (event) {
        var button = event.target.closest("[role=radio]");
        if (!button) return;
        pick(group, button);
        if (button.hasAttribute("data-font")) signature.style.fontFamily = "'" + button.getAttribute("data-font") + "', cursive";
        if (button.hasAttribute("data-ink")) signature.style.fill = button.getAttribute("data-ink");
        // Write it again, the way the app does when the style changes.
        if (!reduce && doc.classList.contains("at-3")) {
          write.style.transition = "none";
          write.style.width = "0";
          void write.getBoundingClientRect();
          write.style.transition = "";
          write.style.width = "";
        }
      });
      // Arrow keys move through a radio group.
      group.addEventListener("keydown", function (event) {
        if (["ArrowRight", "ArrowLeft", "ArrowDown", "ArrowUp"].indexOf(event.key) < 0) return;
        var buttons = Array.prototype.slice.call(group.querySelectorAll("[role=radio]"));
        var at = buttons.indexOf(document.activeElement);
        if (at < 0) return;
        event.preventDefault();
        var next = buttons[(at + (event.key === "ArrowRight" || event.key === "ArrowDown" ? 1 : buttons.length - 1)) % buttons.length];
        next.focus();
        next.click();
      });
    });
  }

  // Counters roll up once.
  function count(el) {
    var to = Number(el.getAttribute("data-to"));
    if (reduce || !to) return;
    var start = performance.now();
    (function frame(now) {
      var t = Math.min(1, (now - start) / 1200);
      el.textContent = Math.round(to * (1 - Math.pow(1 - t, 3)));
      if (t < 1) requestAnimationFrame(frame);
    })(start);
  }

  // Entrances: each section's parts rise once, staggered; the window opens with a shutter.
  var parts = document.querySelectorAll(".rise, .shutter, .hero-slot, .hero-dialog, .count");
  if (!observe) {
    parts.forEach(function (el) { el.classList.add("is-in"); });
    return;
  }
  document.querySelectorAll("section, .hero-foot, .board-grid, .figures-list").forEach(function (group) {
    group.querySelectorAll(":scope > .rise, :scope .rise").forEach(function (el, i) { el.style.setProperty("--delay", Math.min(i, 6) * 80 + "ms"); });
  });
  var seen = new IntersectionObserver(function (entries) {
    entries.forEach(function (entry) {
      if (!entry.isIntersecting) return;
      entry.target.classList.add("is-in");
      if (entry.target.classList.contains("count")) count(entry.target);
      seen.unobserve(entry.target);
    });
  }, { rootMargin: "0px 0px -8% 0px" });
  parts.forEach(function (el) { seen.observe(el); });
})();
