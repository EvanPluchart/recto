//! UI strings. Source strings are English; `t` returns the translation for the current language.
//! M0 detects French from `RECTO_LANG`, `LC_ALL` or `LANG`; system locale detection comes later.

use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lang {
    En,
    Fr,
}

const FR: &[(&str, &str)] = &[
    ("Recto", "Recto"),
    ("Cannot open the document", "Impossible d’ouvrir le document"),
    ("Cannot start the renderer", "Impossible de démarrer le moteur de rendu"),
    ("Cannot render the page", "Impossible d’afficher la page"),
    ("Loading…", "Chargement…"),
    ("Fit width", "Pleine largeur"),
    ("Fit page", "Page entière"),
    ("−", "−"),
    ("+", "+"),
    ("Page", "Page"),
    ("Sidebar", "Panneau"),
    ("Thumbnails", "Vignettes"),
    ("Outline", "Signets"),
    ("This document has no outline.", "Ce document n’a pas de signets."),
    ("Find in document", "Rechercher dans le document"),
    ("Searching…", "Recherche…"),
    ("No results", "Aucun résultat"),
    ("Previous", "Précédent"),
    ("Next", "Suivant"),
    ("Close", "Fermer"),
    ("Open…", "Ouvrir…"),
    ("Recent", "Récents"),
    ("Open a PDF…", "Ouvrir un PDF…"),
    ("Or drop a file onto this window.", "Ou déposez un fichier sur cette fenêtre."),
    ("Recent files", "Fichiers récents"),
    ("No recent files", "Aucun fichier récent"),
    ("Password", "Mot de passe"),
    ("This document is protected by a password.", "Ce document est protégé par un mot de passe."),
    ("Wrong password", "Mot de passe incorrect"),
    ("Unlock", "Déverrouiller"),
];

fn lang() -> Lang {
    static LANG: OnceLock<Lang> = OnceLock::new();
    *LANG.get_or_init(|| {
        let value = ["RECTO_LANG", "LC_ALL", "LANG"].iter().find_map(|key| std::env::var(key).ok().filter(|v| !v.is_empty()));
        match value {
            Some(v) if v.to_ascii_lowercase().starts_with("fr") => Lang::Fr,
            _ => Lang::En,
        }
    })
}

pub fn t(source: &'static str) -> &'static str {
    match lang() {
        Lang::En => source,
        Lang::Fr => FR.iter().find(|(en, _)| *en == source).map_or(source, |(_, fr)| fr),
    }
}
