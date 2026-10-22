//! Document properties: title, author, subject, keywords, read from and written to the
//! document information dictionary.

use lopdf::Object;

use crate::annot::text_string;
use crate::content::dictionary;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Metadata {
    pub title: String,
    pub author: String,
    pub subject: String,
    pub keywords: String,
}

const KEYS: [&[u8]; 4] = [b"Title", b"Author", b"Subject", b"Keywords"];

impl Metadata {
    fn values(&self) -> [&str; 4] {
        [&self.title, &self.author, &self.subject, &self.keywords]
    }

    /// Reads the information dictionary.
    pub fn read(document: &lopdf::Document) -> Metadata {
        let info = document.trailer.get(b"Info").ok().and_then(|value| dictionary(document, value));
        let value = |key: &[u8]| {
            info.and_then(|info| info.get(key).ok())
                .and_then(|value| lopdf::decode_text_string(crate::content::resolve(document, value)).ok())
                .unwrap_or_default()
        };
        Metadata { title: value(KEYS[0]), author: value(KEYS[1]), subject: value(KEYS[2]), keywords: value(KEYS[3]) }
    }

    /// Writes the fields into the information dictionary (empty ones are removed) and sets the
    /// modification date. The XMP metadata stream, which readers prefer when present, is
    /// removed so it cannot contradict the new values.
    pub fn write(&self, document: &mut lopdf::Document, now: std::time::SystemTime) -> lopdf::Result<()> {
        let mut info = document.trailer.get(b"Info").ok().and_then(|value| dictionary(document, value)).cloned().unwrap_or_default();
        for (key, value) in KEYS.into_iter().zip(self.values()) {
            if value.is_empty() {
                info.remove(key);
            } else {
                info.set(key, text_string(value));
            }
        }
        info.set("ModDate", Object::string_literal(pdf_date(now)));
        let id = document.add_object(info);
        document.trailer.set("Info", Object::Reference(id));
        document.catalog_mut()?.remove(b"Metadata");
        Ok(())
    }
}

/// A PDF date in UTC: `D:YYYYMMDDHHmmSSZ`.
pub fn pdf_date(time: std::time::SystemTime) -> String {
    let seconds = time.duration_since(std::time::UNIX_EPOCH).map(|elapsed| elapsed.as_secs()).unwrap_or(0);
    let (days, rest) = (seconds / 86_400, seconds % 86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("D:{year:04}{month:02}{day:02}{:02}{:02}{:02}Z", rest / 3600, rest % 3600 / 60, rest % 60)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::*;

    #[test]
    fn formats_dates() {
        assert_eq!(pdf_date(UNIX_EPOCH), "D:19700101000000Z");
        // 2026-10-22 13:45:07 UTC.
        assert_eq!(pdf_date(UNIX_EPOCH + Duration::from_secs(1_792_676_707)), "D:20261022134507Z");
        // A leap day.
        assert_eq!(pdf_date(UNIX_EPOCH + Duration::from_secs(951_782_400)), "D:20000229000000Z");
    }

    #[test]
    fn writes_and_reads_back() {
        let mut document = lopdf::Document::with_version("1.7");
        let mut catalog = lopdf::Dictionary::new();
        catalog.set("Type", Object::Name(b"Catalog".to_vec()));
        catalog.set("Metadata", Object::Null);
        let catalog = document.add_object(catalog);
        document.trailer.set("Root", Object::Reference(catalog));
        let metadata = Metadata { title: "Rapport annuel — été".into(), author: "Evan".into(), subject: String::new(), keywords: "a, b".into() };
        metadata.write(&mut document, UNIX_EPOCH).unwrap();
        assert_eq!(Metadata::read(&document), metadata);
        assert!(!document.catalog().unwrap().has(b"Metadata"));
    }
}
