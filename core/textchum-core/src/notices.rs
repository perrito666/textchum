//! What the editor has had to say this session.
//!
//! A dialog interrupts; a bar does not. The things the editor says in
//! passing — a language server it could not find, a grammar it could
//! not load, a save that went ahead without its preprocessors — go
//! here, so the last of them can stay on the status bar and the rest
//! can be read back when someone wants them, in the order they came.
//! Both shells keep one of these and draw it their own way.

use std::time::{SystemTime, UNIX_EPOCH};

/// One thing said, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// Milliseconds since the Unix epoch.
    pub at_ms: u64,
    pub text: String,
}

/// The session's notices, oldest first, no more than [`Notices::CAP`]
/// of them.
#[derive(Debug, Default)]
pub struct Notices {
    entries: Vec<Notice>,
}

impl Notices {
    /// How many are kept: enough for a long day, few enough that a
    /// server failing on every keystroke does not grow without bound.
    pub const CAP: usize = 200;

    pub fn new() -> Self {
        Self::default()
    }

    /// Says `text`. Said again straight after itself, it is not
    /// repeated — its time moves instead — since a bar that reads
    /// "failed, failed, failed" says less than one that reads "failed".
    pub fn push(&mut self, text: &str) -> &Notice {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| since.as_millis() as u64)
            .unwrap_or(0);
        if let Some(last) = self.entries.last_mut() {
            if last.text == text {
                last.at_ms = now;
                return self.entries.last().expect("just touched");
            }
        }
        if self.entries.len() >= Self::CAP {
            self.entries.remove(0);
        }
        self.entries.push(Notice {
            at_ms: now,
            text: text.to_string(),
        });
        self.entries.last().expect("just pushed")
    }

    /// The most recent notice, if anything has been said.
    pub fn latest(&self) -> Option<&Notice> {
        self.entries.last()
    }

    /// Everything said, oldest first.
    pub fn all(&self) -> &[Notice] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `[{"at": ms, "text": "…"}, …]`, newest first — the order a list
    /// of them is read in.
    pub fn to_json(&self) -> String {
        let items: Vec<serde_json::Value> = self
            .entries
            .iter()
            .rev()
            .map(|notice| serde_json::json!({ "at": notice.at_ms, "text": notice.text }))
            .collect();
        serde_json::Value::Array(items).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_word_stays_and_the_rest_reads_back_newest_first() {
        let mut notices = Notices::new();
        assert!(notices.latest().is_none());
        notices.push("no server for python");
        notices.push("saved without preprocessing");
        assert_eq!(notices.latest().map(|n| n.text.as_str()), Some("saved without preprocessing"));
        assert_eq!(notices.len(), 2);
        let json: serde_json::Value = serde_json::from_str(&notices.to_json()).unwrap();
        assert_eq!(json[0]["text"], "saved without preprocessing");
        assert_eq!(json[1]["text"], "no server for python");
        assert!(json[0]["at"].as_u64().unwrap() >= json[1]["at"].as_u64().unwrap());
    }

    #[test]
    fn a_repeat_of_the_last_word_is_not_said_twice() {
        let mut notices = Notices::new();
        notices.push("failed");
        notices.push("failed");
        assert_eq!(notices.len(), 1);
        notices.push("other");
        notices.push("failed");
        assert_eq!(notices.len(), 3);
    }

    #[test]
    fn the_oldest_goes_when_the_cap_is_reached() {
        let mut notices = Notices::new();
        for index in 0..(Notices::CAP + 5) {
            notices.push(&format!("notice {index}"));
        }
        assert_eq!(notices.len(), Notices::CAP);
        assert_eq!(notices.all()[0].text, "notice 5");
    }
}
