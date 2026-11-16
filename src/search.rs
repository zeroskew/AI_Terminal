//! Search in session transcripts: the user's and the agent's messages in Claude, Codex and
//! Antigravity conversations (tool calls, tool output and thinking are left out). Runs on a thread; the
//! search window polls the job.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use crate::agents::{self, Kind};

/// A conversation to search.
#[derive(Clone)]
pub struct Target {
    pub agent_id: String,
    pub kind: Kind,
    pub project: PathBuf,
}

/// One matching message.
#[derive(Clone)]
pub struct Hit {
    /// Unix seconds (None when the line had no readable timestamp).
    pub time: Option<u64>,
    /// Written by the user (else the agent).
    pub user: bool,
    /// The text around the match, on one line, and the match's byte range in it.
    pub snippet: String,
    pub range: (usize, usize),
}

/// The matches in one conversation.
#[derive(Clone)]
pub struct Found {
    pub agent_id: String,
    /// Matching messages (first [`MAX_HITS`] kept) and how many there were.
    pub hits: Vec<Hit>,
    pub count: usize,
    /// Newest match (for ordering).
    pub newest: u64,
}

/// Snippets kept per conversation.
pub const MAX_HITS: usize = 50;

pub struct Job {
    pub query: String,
    pub found: Mutex<Vec<Found>>,
    pub scanned: AtomicUsize,
    pub total: usize,
    pub done: AtomicBool,
    pub cancel: AtomicBool,
}

impl Job {
    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::Relaxed)
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Starts a search of `targets` for `query` (case-insensitive), in messages from the last
/// `days` days (0 = all).
pub fn start(targets: Vec<Target>, query: String, days: u32, ctx: egui::Context) -> Arc<Job> {
    let job = Arc::new(Job { query: query.clone(), found: Mutex::new(Vec::new()), scanned: AtomicUsize::new(0), total: targets.len(), done: AtomicBool::new(false), cancel: AtomicBool::new(false) });
    let since = (days > 0).then(|| now_secs().saturating_sub(days as u64 * 86_400));
    // The thread holds a weak reference: closing the window (dropping the job) stops it.
    let weak = Arc::downgrade(&job);
    std::thread::spawn(move || {
        let needle = query.to_lowercase();
        for t in targets {
            let Some(job) = weak.upgrade() else { return };
            if job.cancel.load(Ordering::Relaxed) {
                return;
            }
            if let Some(f) = search_one(&t, &needle, since, &job.cancel) {
                job.found.lock().unwrap().push(f);
            }
            job.scanned.fetch_add(1, Ordering::Relaxed);
            ctx.request_repaint();
        }
        if let Some(job) = weak.upgrade() {
            job.done.store(true, Ordering::Relaxed);
        }
        ctx.request_repaint();
    });
    job
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn transcript(t: &Target) -> Option<PathBuf> {
    match t.kind {
        Kind::Claude => Some(agents::claude_dir(&t.project)?.join(format!("{}.jsonl", t.agent_id))),
        Kind::Codex => agents::codex_rollout(&t.agent_id),
        _ => None,
    }
}

/// Adds a message's match (if it has one) to `found`.
fn add_match(found: &mut Found, user: bool, time: Option<u64>, texts: &[String], needle: &str) {
    for text in texts {
        if let Some((snippet, range)) = snippet(text, needle) {
            found.count += 1;
            found.newest = found.newest.max(time.unwrap_or(0));
            if found.hits.len() < MAX_HITS {
                found.hits.push(Hit { time, user, snippet, range });
            }
            return;
        }
    }
}

fn search_one(t: &Target, needle: &str, since: Option<u64>, cancel: &AtomicBool) -> Option<Found> {
    let mut found = Found { agent_id: t.agent_id.clone(), hits: Vec::new(), count: 0, newest: 0 };
    if t.kind == Kind::Antigravity {
        for m in agents::agy_messages(&t.agent_id) {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            if since.is_some_and(|s| m.time.is_some_and(|tm| tm < s)) {
                continue;
            }
            add_match(&mut found, m.user, m.time, std::slice::from_ref(&m.text), needle);
        }
        return (found.count > 0).then_some(found);
    }
    let path = transcript(t)?;
    // Nothing written since the cut-off: no message in it can be recent enough.
    if let Some(s) = since {
        let m = std::fs::metadata(&path).ok()?.modified().ok()?;
        if m.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) < s {
            return None;
        }
    }
    let f = std::fs::File::open(&path).ok()?;
    // A raw-line check skips most lines without parsing them. JSON escapes quotes and
    // backslashes (and the case of escapes like \n differs), so only use it for plain queries.
    let prefilter = !needle.contains(['"', '\\']) && !needle.chars().any(char::is_control);
    for (n, line) in BufReader::new(f).lines().enumerate() {
        if n % 256 == 0 && cancel.load(Ordering::Relaxed) {
            return None;
        }
        let Ok(line) = line else { continue };
        if prefilter && !contains_ci(&line, needle) {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        let Some((user, time, texts)) = message_texts(t.kind, &v) else { continue };
        if let (Some(s), Some(tm)) = (since, time) {
            if tm < s {
                continue;
            }
        }
        add_match(&mut found, user, time, &texts, needle);
    }
    (found.count > 0).then_some(found)
}

fn contains_ci(hay: &str, needle_lower: &str) -> bool {
    if hay.is_ascii() && needle_lower.is_ascii() {
        let (h, n) = (hay.as_bytes(), needle_lower.as_bytes());
        return n.is_empty() || h.windows(n.len()).any(|w| w.eq_ignore_ascii_case(n));
    }
    hay.to_lowercase().contains(needle_lower)
}

/// A transcript line's message: (from the user, time, its text parts). None for anything that
/// isn't a user / agent message with text.
fn message_texts(kind: Kind, v: &serde_json::Value) -> Option<(bool, Option<u64>, Vec<String>)> {
    let time = v["timestamp"].as_str().and_then(agents::parse_utc);
    let text_parts = |c: &serde_json::Value, types: &[&str]| -> Vec<String> {
        match c {
            serde_json::Value::String(s) => vec![s.clone()],
            serde_json::Value::Array(a) => a.iter().filter(|p| p["type"].as_str().is_some_and(|t| types.contains(&t))).filter_map(|p| p["text"].as_str().map(str::to_string)).collect(),
            _ => Vec::new(),
        }
    };
    let (user, texts) = match kind {
        Kind::Claude => {
            let user = match v["type"].as_str()? {
                "user" => true,
                "assistant" => false,
                _ => return None,
            };
            if v["isMeta"].as_bool() == Some(true) {
                return None;
            }
            (user, text_parts(&v["message"]["content"], &["text"]))
        }
        Kind::Codex => {
            let p = &v["payload"];
            if v["type"] != "response_item" || p["type"] != "message" {
                return None;
            }
            let user = match p["role"].as_str()? {
                "user" => true,
                "assistant" => false,
                _ => return None,
            };
            (user, text_parts(&p["content"], &["input_text", "output_text", "text"]))
        }
        _ => return None,
    };
    let texts: Vec<String> = texts.into_iter().filter(|t| !t.trim_start().starts_with('<')).collect();
    (!texts.is_empty()).then_some((user, time, texts))
}

/// The text around the first match of `needle_lower`, whitespace folded to single spaces, with
/// the match's byte range in the snippet.
fn snippet(text: &str, needle_lower: &str) -> Option<(String, (usize, usize))> {
    // Fold whitespace first so the snippet is one line; keep a char-wise lowercase copy aligned
    // with it (same char count) to find the match.
    let folded: Vec<char> = {
        let mut out = Vec::with_capacity(text.len());
        let mut space = false;
        for c in text.chars() {
            if c.is_whitespace() {
                if !space && !out.is_empty() {
                    out.push(' ');
                }
                space = true;
            } else {
                out.push(c);
                space = false;
            }
        }
        out
    };
    let lower: Vec<char> = folded.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();
    let n: Vec<char> = needle_lower.chars().collect();
    if n.is_empty() || n.len() > lower.len() {
        return None;
    }
    let at = (0..=lower.len() - n.len()).find(|&i| lower[i..i + n.len()] == n[..])?;
    const BEFORE: usize = 60;
    const AFTER: usize = 140;
    let from = at.saturating_sub(BEFORE);
    let to = (at + n.len() + AFTER).min(folded.len());
    let mut s = String::new();
    if from > 0 {
        s.push('…');
    }
    s.extend(&folded[from..at]);
    let start = s.len();
    s.extend(&folded[at..at + n.len()]);
    let end = s.len();
    s.extend(&folded[at + n.len()..to]);
    if to < folded.len() {
        s.push('…');
    }
    Some((s, (start, end)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_folds_and_marks() {
        let (s, (a, b)) = snippet("first line\n\n  Hello   World again", "world").unwrap();
        assert_eq!(s, "first line Hello World again");
        assert_eq!(&s[a..b], "World");
        assert!(snippet("nothing here", "xyz").is_none());
    }

    #[test]
    fn claude_and_codex_messages() {
        let c: serde_json::Value = serde_json::from_str(r#"{"type":"assistant","timestamp":"2026-11-15T10:00:00.000Z","message":{"role":"assistant","content":[{"type":"thinking","thinking":"x"},{"type":"text","text":"Found it"}]}}"#).unwrap();
        let (user, time, texts) = message_texts(Kind::Claude, &c).unwrap();
        assert!(!user && time.is_some());
        assert_eq!(texts, vec!["Found it".to_string()]);
        let tool: serde_json::Value = serde_json::from_str(r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"x"}]}}"#).unwrap();
        assert!(message_texts(Kind::Claude, &tool).is_none());
        let x: serde_json::Value = serde_json::from_str(r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"fix the bug"}]}}"#).unwrap();
        assert!(message_texts(Kind::Codex, &x).unwrap().0);
        assert!(contains_ci("Hello WORLD", "world"));
    }
}
