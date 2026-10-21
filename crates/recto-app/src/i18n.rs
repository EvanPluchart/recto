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
    ("Undo", "Annuler"),
    ("Redo", "Rétablir"),
    ("Rotate left", "Pivoter à gauche"),
    ("Rotate right", "Pivoter à droite"),
    ("Delete", "Supprimer"),
    ("Blank page", "Page blanche"),
    ("Insert file…", "Insérer un fichier…"),
    ("Extract…", "Extraire…"),
    ("Split…", "Découper…"),
    ("Save", "Enregistrer"),
    ("Save as…", "Enregistrer sous…"),
    ("Dismiss", "Fermer"),
    ("A document needs at least one page.", "Un document doit garder au moins une page."),
    ("Cannot apply the change", "Impossible d’appliquer la modification"),
    ("Cannot insert the file", "Impossible d’insérer le fichier"),
    ("Replace the original file with the edited document?", "Remplacer le fichier d’origine par le document modifié ?"),
    ("Replace", "Remplacer"),
    ("Cancel", "Annuler"),
    ("Saved to", "Enregistré dans"),
    ("Cannot save the document", "Impossible d’enregistrer le document"),
    ("Enter a number of pages.", "Indiquez un nombre de pages."),
    ("That would make a single file; use Save as instead.", "Cela ne ferait qu’un fichier ; utilisez plutôt Enregistrer sous."),
    ("Split here", "Découper ici"),
    ("A file already exists:", "Un fichier existe déjà :"),
    ("Pages extracted.", "Pages extraites."),
    ("files written.", "fichiers écrits."),
    ("Cannot write the files", "Impossible d’écrire les fichiers"),
    ("Discard the unsaved changes?", "Abandonner les modifications non enregistrées ?"),
    ("Discard", "Abandonner"),
    ("Extract pages", "Extraire des pages"),
    ("Pages to copy into a new file, e.g. 1-3, 5.", "Pages à copier dans un nouveau fichier, par ex. 1-3, 5."),
    ("Split", "Découper"),
    ("One file per range, e.g. 1-3, 4-10, 11-.", "Un fichier par plage, par ex. 1-3, 4-10, 11-."),
    ("Number of pages in each file.", "Nombre de pages par fichier."),
    ("One file per top-level bookmark.", "Un fichier par signet de premier niveau."),
    ("Ranges", "Plages"),
    ("Every N pages", "Toutes les N pages"),
    ("Bookmarks", "Signets"),
    ("Continue…", "Continuer…"),
    ("e.g. 1-3, 5", "par ex. 1-3, 5"),
    ("Drop here to put pages at the end", "Déposez ici pour placer les pages à la fin"),
    ("1 page", "1 page"),
    ("pages", "pages"),
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
