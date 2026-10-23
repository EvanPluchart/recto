// Recto landing page: the signature try-out, the stage slider, the download note, entrances.
// The page reads and works without it.
(function () {
  "use strict";

  // Signature: the typed name in Recto's four signature styles.
  var input = document.getElementById("try-name");
  var signatures = document.querySelectorAll(".try-sig");
  function sign() {
    var name = input.value.trim() || "Votre nom";
    signatures.forEach(function (el) { el.textContent = name; });
  }
  if (input) {
    input.addEventListener("input", sign);
    document.querySelectorAll("[data-fill]").forEach(function (button) {
      button.addEventListener("click", function () {
        input.value = button.getAttribute("data-fill");
        sign();
      });
    });
  }

  // Stages: one slider (or the stage names) drives the clause, its detail and the screenshot.
  var stages = [
    { name: "Lire", clause: "D'abord, le PDF s'ouvre en lecture.", detail: "Comme une visionneuse : on lit, on cherche, on copie, on imprime. Rien ne bouge par erreur." },
    { name: "Modifier", clause: "Ensuite, Modifier sort les outils.", detail: "Une barre d'outils et les miniatures des pages. Chaque outil a sa lettre au clavier : T pour le texte, S pour signer." },
    { name: "Signer", clause: "Puis vous annotez et signez.", detail: "Mention, surlignage, formes, tampons, signature. Tout reste déplaçable et modifiable après coup." },
    { name: "Caviarder", clause: "Enfin, vous caviardez avant d'envoyer.", detail: "Le nom sous le rectangle noir est retiré du fichier enregistré, pas seulement caché." }
  ];
  var range = document.getElementById("stage-range");
  var clause = document.querySelector(".stage-clause");
  var detail = document.querySelector(".stage-detail");
  var images = document.querySelectorAll(".stage-img");
  var names = document.querySelectorAll("[data-stage]");
  var reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  var current = 0;
  function show(index) {
    if (index === current) return;
    current = index;
    var stage = stages[index];
    range.value = index;
    range.setAttribute("aria-valuetext", stage.name);
    images.forEach(function (img, i) { img.classList.toggle("is-current", i === index); });
    names.forEach(function (button, i) {
      if (i === index) button.setAttribute("aria-current", "step");
      else button.removeAttribute("aria-current");
    });
    function swap() {
      clause.textContent = stage.clause;
      detail.textContent = stage.detail;
      clause.style.opacity = detail.style.opacity = "";
    }
    if (reduce) { swap(); return; }
    clause.style.opacity = detail.style.opacity = "0";
    setTimeout(swap, 200);
  }
  if (range) {
    range.addEventListener("input", function () { show(Number(range.value)); });
    names.forEach(function (button) {
      button.addEventListener("click", function () { show(Number(button.getAttribute("data-stage"))); });
    });
  }

  // Download: what the chosen file is.
  var kind = document.getElementById("cta-kind");
  var note = document.getElementById("cta-note");
  var notes = {
    setup: "L'installateur ajoute Recto au menu Démarrer et à « Ouvrir avec » pour les PDF, sans remplacer votre lecteur par défaut. Fichier <em>Recto-…-windows-x64-setup.exe</em>, 21 Mo.",
    zip: "La version portable se décompresse où vous voulez, une clé USB par exemple, et ne touche pas au registre. Fichier <em>Recto-…-windows-x64.zip</em>."
  };
  if (kind && note) {
    kind.addEventListener("change", function () { note.innerHTML = notes[kind.value]; });
  }

  // Entrances: each section's parts rise once, staggered; the screenshot opens with a shutter.
  var parts = document.querySelectorAll(".rise, .shutter, .hero-slot");
  if (!("IntersectionObserver" in window)) {
    parts.forEach(function (el) { el.classList.add("is-in"); });
    return;
  }
  var seen = new IntersectionObserver(function (entries) {
    entries.forEach(function (entry) {
      if (!entry.isIntersecting) return;
      entry.target.classList.add("is-in");
      seen.unobserve(entry.target);
    });
  }, { rootMargin: "0px 0px -8% 0px" });
  document.querySelectorAll("section, figure").forEach(function (section) {
    section.querySelectorAll(".rise").forEach(function (el, i) { el.style.setProperty("--delay", i * 80 + "ms"); });
  });
  parts.forEach(function (el) { seen.observe(el); });
})();
