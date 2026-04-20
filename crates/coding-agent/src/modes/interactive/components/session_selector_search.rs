//! Session selector search utilities.
//!
//! Translated from `components/session-selector-search.ts`.
//!
//! Pure logic for parsing search queries and filtering / sorting session lists.

use std::time::SystemTime;

// ============================================================================
// SortMode / NameFilter
// ============================================================================

/// How to sort sessions in the selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortMode {
    Threaded,
    Recent,
    Relevance,
}

/// Whether to show all sessions or only named ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameFilter {
    All,
    Named,
}

// ============================================================================
// SessionInfo (subset used for search)
// ============================================================================

/// Minimal session information required for search / sort.
#[derive(Debug, Clone)]
pub struct SessionSearchInfo {
    pub id: String,
    pub name: Option<String>,
    /// Combined text of all messages (for full-text search).
    pub all_messages_text: String,
    pub cwd: String,
    pub modified: SystemTime,
}

impl SessionSearchInfo {
    fn search_text(&self) -> String {
        format!(
            "{} {} {} {}",
            self.id,
            self.name.as_deref().unwrap_or(""),
            self.all_messages_text,
            self.cwd
        )
    }
}

/// Returns `true` if the session has a non-empty name.
pub fn has_session_name(session: &SessionSearchInfo) -> bool {
    session
        .name
        .as_ref()
        .map(|n| !n.trim().is_empty())
        .unwrap_or(false)
}

// ============================================================================
// ParsedSearchQuery
// ============================================================================

/// A single token in a search query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// Fuzzy-matched token.
    Fuzzy(String),
    /// Exact phrase (from quoted string).
    Phrase(String),
}

/// Parsed representation of a search query.
#[derive(Debug, Clone)]
pub enum ParsedSearchQuery {
    /// Empty query — matches everything.
    Empty,
    /// Token-based query (fuzzy + phrase tokens).
    Tokens(Vec<Token>),
    /// Regex query (`re:<pattern>`).
    Regex(regex::Regex),
    /// Parse error (e.g., invalid regex). Matches nothing.
    Error(String),
}

/// Match result from applying a query to a session.
#[derive(Debug, Clone)]
pub struct MatchResult {
    pub matches: bool,
    /// Lower is better; only meaningful when `matches` is `true`.
    pub score: f64,
}

// ============================================================================
// Parse
// ============================================================================

/// Parse a raw search string into a `ParsedSearchQuery`.
///
/// Mirrors `parseSearchQuery()` from TypeScript.
pub fn parse_search_query(query: &str) -> ParsedSearchQuery {
    let trimmed = query.trim();

    if trimmed.is_empty() {
        return ParsedSearchQuery::Empty;
    }

    // Regex mode: "re:<pattern>"
    if let Some(pattern) = trimmed.strip_prefix("re:") {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            return ParsedSearchQuery::Error("Empty regex".into());
        }
        match regex::Regex::new(&format!("(?i){pattern}")) {
            Ok(re) => return ParsedSearchQuery::Regex(re),
            Err(e) => return ParsedSearchQuery::Error(e.to_string()),
        }
    }

    // Token mode with quote support: foo "bar baz" qux
    let mut tokens: Vec<Token> = Vec::new();
    let mut buf = String::new();
    let mut in_quote = false;
    let mut had_unclosed_quote = false;
    let chars: Vec<char> = trimmed.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];
        if ch == '"' {
            if in_quote {
                let v = buf.trim().to_string();
                buf.clear();
                if !v.is_empty() {
                    tokens.push(Token::Phrase(v));
                }
                in_quote = false;
            } else {
                // Flush pending fuzzy token
                let v = buf.trim().to_string();
                buf.clear();
                if !v.is_empty() {
                    tokens.push(Token::Fuzzy(v));
                }
                in_quote = true;
            }
        } else if !in_quote && ch.is_whitespace() {
            let v = buf.trim().to_string();
            buf.clear();
            if !v.is_empty() {
                tokens.push(Token::Fuzzy(v));
            }
        } else {
            buf.push(ch);
        }
        i += 1;
    }

    if in_quote {
        had_unclosed_quote = true;
    }

    // Unbalanced quotes: fall back to simple whitespace tokenization
    if had_unclosed_quote {
        let fallback_tokens = trimmed
            .split_whitespace()
            .map(|t| Token::Fuzzy(t.to_string()))
            .collect();
        return ParsedSearchQuery::Tokens(fallback_tokens);
    }

    // Flush final buffer
    let v = buf.trim().to_string();
    if !v.is_empty() {
        if in_quote {
            tokens.push(Token::Phrase(v));
        } else {
            tokens.push(Token::Fuzzy(v));
        }
    }

    ParsedSearchQuery::Tokens(tokens)
}

// ============================================================================
// Match
// ============================================================================

/// Simple fuzzy match: returns Some(score) where lower is better.
/// Score = position of first matching char (very rough approximation).
fn fuzzy_match(pattern: &str, text: &str) -> Option<f64> {
    if pattern.is_empty() {
        return Some(0.0);
    }
    let pat_lower = pattern.to_lowercase();
    let text_lower = text.to_lowercase();

    // Try substring first (score = position)
    if let Some(pos) = text_lower.find(&pat_lower) {
        return Some(pos as f64 * 0.1);
    }

    // Simple sequential character match
    let mut score = 0.0;
    let mut pat_chars = pat_lower.chars().peekable();
    for (i, ch) in text_lower.char_indices() {
        if pat_chars.peek() == Some(&ch) {
            score += i as f64 * 0.01;
            pat_chars.next();
            if pat_chars.peek().is_none() {
                return Some(score);
            }
        }
    }

    None // No match
}

fn normalize_whitespace_lower(s: &str) -> String {
    s.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Apply a parsed query to a single session.
pub fn match_session(session: &SessionSearchInfo, parsed: &ParsedSearchQuery) -> MatchResult {
    let text = session.search_text();

    match parsed {
        ParsedSearchQuery::Empty => MatchResult {
            matches: true,
            score: 0.0,
        },

        ParsedSearchQuery::Error(_) => MatchResult {
            matches: false,
            score: 0.0,
        },

        ParsedSearchQuery::Regex(re) => {
            if let Some(m) = re.find(&text) {
                MatchResult {
                    matches: true,
                    score: m.start() as f64 * 0.1,
                }
            } else {
                MatchResult {
                    matches: false,
                    score: 0.0,
                }
            }
        }

        ParsedSearchQuery::Tokens(tokens) => {
            if tokens.is_empty() {
                return MatchResult {
                    matches: true,
                    score: 0.0,
                };
            }

            let mut total_score = 0.0;
            let normalized_text = normalize_whitespace_lower(&text);

            for token in tokens {
                match token {
                    Token::Phrase(phrase) => {
                        let norm_phrase = normalize_whitespace_lower(phrase);
                        if norm_phrase.is_empty() {
                            continue;
                        }
                        if let Some(pos) = normalized_text.find(&norm_phrase) {
                            total_score += pos as f64 * 0.1;
                        } else {
                            return MatchResult {
                                matches: false,
                                score: 0.0,
                            };
                        }
                    }
                    Token::Fuzzy(value) => match fuzzy_match(value, &text) {
                        Some(score) => total_score += score,
                        None => {
                            return MatchResult {
                                matches: false,
                                score: 0.0,
                            };
                        }
                    },
                }
            }

            MatchResult {
                matches: true,
                score: total_score,
            }
        }
    }
}

// ============================================================================
// Filter and sort
// ============================================================================

/// Filter and sort a list of sessions according to query, sort mode, and name filter.
///
/// Mirrors `filterAndSortSessions()` from TypeScript.
pub fn filter_and_sort_sessions(
    sessions: &[SessionSearchInfo],
    query: &str,
    sort_mode: SortMode,
    name_filter: NameFilter,
) -> Vec<SessionSearchInfo> {
    // Apply name filter first
    let name_filtered: Vec<&SessionSearchInfo> = sessions
        .iter()
        .filter(|s| name_filter == NameFilter::All || has_session_name(s))
        .collect();

    let trimmed = query.trim();
    if trimmed.is_empty() {
        return name_filtered.iter().map(|s| (*s).clone()).collect();
    }

    let parsed = parse_search_query(query);

    if matches!(parsed, ParsedSearchQuery::Error(_)) {
        return vec![];
    }

    // Recent mode: filter only, preserve incoming order
    if sort_mode == SortMode::Recent {
        return name_filtered
            .iter()
            .filter(|s| match_session(s, &parsed).matches)
            .map(|s| (*s).clone())
            .collect();
    }

    // Relevance mode: sort by score, then by modified desc
    let mut scored: Vec<(&SessionSearchInfo, f64)> = name_filtered
        .iter()
        .filter_map(|s| {
            let r = match_session(s, &parsed);
            if r.matches { Some((*s, r.score)) } else { None }
        })
        .collect();

    scored.sort_by(|(sa, score_a), (sb, score_b)| {
        score_a
            .partial_cmp(score_b)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                let ta = sb
                    .modified
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default();
                let tb = sa
                    .modified
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default();
                ta.cmp(&tb)
            })
    });

    scored.into_iter().map(|(s, _)| s.clone()).collect()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn make_session(id: &str, name: Option<&str>, text: &str) -> SessionSearchInfo {
        SessionSearchInfo {
            id: id.to_string(),
            name: name.map(str::to_string),
            all_messages_text: text.to_string(),
            cwd: "/tmp".to_string(),
            modified: UNIX_EPOCH + Duration::from_secs(1000),
        }
    }

    // --- parseSearchQuery ---

    #[test]
    fn empty_query_is_empty() {
        assert!(matches!(parse_search_query(""), ParsedSearchQuery::Empty));
        assert!(matches!(parse_search_query("  "), ParsedSearchQuery::Empty));
    }

    #[test]
    fn regex_mode() {
        let q = parse_search_query("re:foo.*bar");
        assert!(matches!(q, ParsedSearchQuery::Regex(_)));
    }

    #[test]
    fn regex_empty_pattern_is_error() {
        let q = parse_search_query("re:");
        assert!(matches!(q, ParsedSearchQuery::Error(_)));
    }

    #[test]
    fn token_mode_basic() {
        let q = parse_search_query("foo bar");
        if let ParsedSearchQuery::Tokens(tokens) = q {
            assert_eq!(tokens.len(), 2);
            assert_eq!(tokens[0], Token::Fuzzy("foo".into()));
            assert_eq!(tokens[1], Token::Fuzzy("bar".into()));
        } else {
            panic!("expected Tokens");
        }
    }

    #[test]
    fn quoted_phrase_token() {
        let q = parse_search_query(r#"foo "bar baz" qux"#);
        if let ParsedSearchQuery::Tokens(tokens) = q {
            assert_eq!(tokens.len(), 3);
            assert_eq!(tokens[0], Token::Fuzzy("foo".into()));
            assert_eq!(tokens[1], Token::Phrase("bar baz".into()));
            assert_eq!(tokens[2], Token::Fuzzy("qux".into()));
        } else {
            panic!("expected Tokens");
        }
    }

    #[test]
    fn unclosed_quote_fallback_to_whitespace() {
        let q = parse_search_query(r#"foo "bar"#);
        if let ParsedSearchQuery::Tokens(tokens) = q {
            // Falls back to simple whitespace split
            assert!(tokens.len() >= 1);
        } else {
            panic!("expected Tokens");
        }
    }

    // --- matchSession ---

    #[test]
    fn empty_query_always_matches() {
        let s = make_session("s1", None, "anything");
        let r = match_session(&s, &ParsedSearchQuery::Empty);
        assert!(r.matches);
    }

    #[test]
    fn simple_token_matches_id() {
        let s = make_session("session-abc", None, "");
        let parsed = parse_search_query("abc");
        let r = match_session(&s, &parsed);
        assert!(r.matches);
    }

    #[test]
    fn simple_token_no_match() {
        let s = make_session("session-xyz", None, "hello world");
        let parsed = parse_search_query("nope");
        let r = match_session(&s, &parsed);
        assert!(!r.matches);
    }

    #[test]
    fn phrase_token_exact() {
        let s = make_session("s1", None, "fix the bug in the compiler");
        let parsed = parse_search_query(r#""fix the bug""#);
        let r = match_session(&s, &parsed);
        assert!(r.matches);
    }

    #[test]
    fn regex_match() {
        let s = make_session("s1", Some("rust project"), "");
        let parsed = parse_search_query("re:rust");
        let r = match_session(&s, &parsed);
        assert!(r.matches);
    }

    // --- hasSessionName ---

    #[test]
    fn has_session_name_true() {
        let s = make_session("s1", Some("My Session"), "");
        assert!(has_session_name(&s));
    }

    #[test]
    fn has_session_name_false_none() {
        let s = make_session("s1", None, "");
        assert!(!has_session_name(&s));
    }

    #[test]
    fn has_session_name_false_whitespace() {
        let s = make_session("s1", Some("   "), "");
        assert!(!has_session_name(&s));
    }

    // --- filterAndSortSessions ---

    #[test]
    fn empty_query_returns_all() {
        let sessions = vec![make_session("s1", None, "a"), make_session("s2", None, "b")];
        let result = filter_and_sort_sessions(&sessions, "", SortMode::Recent, NameFilter::All);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn name_filter_named_only() {
        let sessions = vec![
            make_session("s1", Some("Named"), ""),
            make_session("s2", None, ""),
        ];
        let result = filter_and_sort_sessions(&sessions, "", SortMode::Recent, NameFilter::Named);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "s1");
    }

    #[test]
    fn invalid_regex_returns_empty() {
        let sessions = vec![make_session("s1", None, "anything")];
        let result =
            filter_and_sort_sessions(&sessions, "re:[invalid", SortMode::Recent, NameFilter::All);
        assert!(result.is_empty());
    }

    #[test]
    fn recent_mode_preserves_order() {
        let sessions = vec![
            make_session("a", None, "target match"),
            make_session("b", None, "no"),
            make_session("c", None, "another target match"),
        ];
        let result =
            filter_and_sort_sessions(&sessions, "target", SortMode::Recent, NameFilter::All);
        assert_eq!(result[0].id, "a");
        assert_eq!(result[1].id, "c");
    }
}
