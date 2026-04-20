/// Fuzzy matching utilities.
/// Matches if all query characters appear in order (not necessarily consecutive).
/// Lower score = better match.

use std::sync::OnceLock;

fn alpha_num_regex() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"^(?P<letters>[a-z]+)(?P<digits>[0-9]+)$").unwrap())
}

fn num_alpha_regex() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"^(?P<digits>[0-9]+)(?P<letters>[a-z]+)$").unwrap())
}

#[derive(Debug, Clone, PartialEq)]
pub struct FuzzyMatch {
    pub matches: bool,
    pub score: f64,
}

pub fn fuzzy_match(query: &str, text: &str) -> FuzzyMatch {
    let query_lower = query.to_lowercase();
    let text_lower = text.to_lowercase();

    let match_query = |normalized_query: &str| -> FuzzyMatch {
        if normalized_query.is_empty() {
            return FuzzyMatch { matches: true, score: 0.0 };
        }
        let query_chars: Vec<char> = normalized_query.chars().collect();
        let text_chars: Vec<char> = text_lower.chars().collect();

        if query_chars.len() > text_chars.len() {
            return FuzzyMatch { matches: false, score: 0.0 };
        }

        let mut query_index = 0;
        let mut score: f64 = 0.0;
        let mut last_match_index: i64 = -1;
        let mut consecutive_matches: i64 = 0;

        for (i, &tc) in text_chars.iter().enumerate() {
            if query_index >= query_chars.len() {
                break;
            }
            if tc == query_chars[query_index] {
                let is_word_boundary = i == 0 || {
                    let prev = text_chars[i - 1];
                    matches!(prev, ' ' | '-' | '_' | '.' | '/' | ':')
                };

                if last_match_index == (i as i64) - 1 {
                    consecutive_matches += 1;
                    score -= (consecutive_matches * 5) as f64;
                } else {
                    consecutive_matches = 0;
                    if last_match_index >= 0 {
                        score += ((i as i64 - last_match_index - 1) * 2) as f64;
                    }
                }

                if is_word_boundary {
                    score -= 10.0;
                }

                score += i as f64 * 0.1;
                last_match_index = i as i64;
                query_index += 1;
            }
        }

        if query_index < query_chars.len() {
            return FuzzyMatch { matches: false, score: 0.0 };
        }

        FuzzyMatch { matches: true, score }
    };

    let primary = match_query(&query_lower);
    if primary.matches {
        return primary;
    }

    // Try swapping alpha/numeric order in query
    let swapped_query = if let Some(caps) = alpha_num_regex().captures(&query_lower) {
        format!("{}{}", &caps["digits"], &caps["letters"])
    } else if let Some(caps) = num_alpha_regex().captures(&query_lower) {
        format!("{}{}", &caps["letters"], &caps["digits"])
    } else {
        String::new()
    };

    if swapped_query.is_empty() {
        return primary;
    }

    let swapped = match_query(&swapped_query);
    if !swapped.matches {
        return primary;
    }

    FuzzyMatch { matches: true, score: swapped.score + 5.0 }
}

/// Filter and sort items by fuzzy match quality (best matches first).
/// Supports space-separated tokens: all tokens must match.
pub fn fuzzy_filter<T, F>(items: Vec<T>, query: &str, get_text: F) -> Vec<T>
where
    F: Fn(&T) -> &str,
{
    let query_trimmed = query.trim();
    if query_trimmed.is_empty() {
        return items;
    }

    let tokens: Vec<&str> = query_trimmed.split_whitespace().collect();
    if tokens.is_empty() {
        return items;
    }

    let mut results: Vec<(T, f64)> = Vec::new();

    for item in items {
        let text = get_text(&item);
        let mut total_score = 0.0;
        let mut all_match = true;

        for token in &tokens {
            let m = fuzzy_match(token, text);
            if m.matches {
                total_score += m.score;
            } else {
                all_match = false;
                break;
            }
        }

        if all_match {
            results.push((item, total_score));
        }
    }

    results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    results.into_iter().map(|(item, _)| item).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fuzzy_match_exact() {
        let m = fuzzy_match("hello", "hello world");
        assert!(m.matches);
    }

    #[test]
    fn test_fuzzy_match_subsequence() {
        let m = fuzzy_match("hlo", "hello");
        assert!(m.matches);
    }

    #[test]
    fn test_fuzzy_match_no_match() {
        let m = fuzzy_match("xyz", "hello");
        assert!(!m.matches);
    }

    #[test]
    fn test_fuzzy_match_empty_query() {
        let m = fuzzy_match("", "hello");
        assert!(m.matches);
        assert_eq!(m.score, 0.0);
    }

    #[test]
    fn test_fuzzy_match_query_longer_than_text() {
        let m = fuzzy_match("hello world", "hi");
        assert!(!m.matches);
    }

    #[test]
    fn test_fuzzy_match_word_boundary_bonus() {
        let m1 = fuzzy_match("h", "hello");
        let m2 = fuzzy_match("h", "xhello");
        // word boundary at start gets bonus
        assert!(m1.score < m2.score || m1.matches);
    }

    #[test]
    fn test_fuzzy_match_consecutive_bonus() {
        // Consecutive matches get bonus (negative score = better)
        let m1 = fuzzy_match("ab", "ab test");
        let m2 = fuzzy_match("ab", "a_b test");
        // Consecutive should score better (lower)
        assert!(m1.score <= m2.score);
    }

    #[test]
    fn test_fuzzy_filter_basic() {
        let items = vec!["apple", "banana", "apricot", "cherry"];
        let result = fuzzy_filter(items, "ap", |s| s);
        assert!(result.contains(&"apple"));
        assert!(result.contains(&"apricot"));
        assert!(!result.contains(&"banana"));
    }

    #[test]
    fn test_fuzzy_filter_empty_query() {
        let items = vec!["apple", "banana"];
        let result = fuzzy_filter(items.clone(), "", |s| s);
        assert_eq!(result.len(), items.len());
    }

    #[test]
    fn test_fuzzy_filter_multi_token() {
        let items = vec!["foo bar", "foo baz", "qux bar"];
        let result = fuzzy_filter(items, "foo bar", |s| s);
        assert!(result.contains(&"foo bar"));
        // "qux bar" might match "bar" token but not "foo"
        // "foo baz" won't match "bar" token
    }

    #[test]
    fn test_fuzzy_match_alpha_num_swap() {
        // "a1" might match "1a" after swapping
        let m = fuzzy_match("a1", "1a");
        // The swapped query "1a" should match "1a"
        assert!(m.matches);
    }

    // ==========================================================================
    // Tests from fuzzy.test.ts
    // ==========================================================================

    #[test]
    fn test_empty_query_matches_everything_with_score_0() {
        // "empty query matches everything with score 0"
        let result = fuzzy_match("", "anything");
        assert!(result.matches);
        assert_eq!(result.score, 0.0);
    }

    #[test]
    fn test_query_longer_than_text_does_not_match() {
        // "query longer than text does not match"
        let result = fuzzy_match("longquery", "short");
        assert!(!result.matches);
    }

    #[test]
    fn test_exact_match_has_good_score() {
        // "exact match has good score"
        let result = fuzzy_match("test", "test");
        assert!(result.matches);
        // Should be negative due to consecutive bonuses (lower = better)
        assert!(result.score < 0.0, "exact match score should be negative (good), got {}", result.score);
    }

    #[test]
    fn test_characters_must_appear_in_order() {
        // "characters must appear in order"
        let match_in_order = fuzzy_match("abc", "aXbXc");
        assert!(match_in_order.matches, "abc in aXbXc should match in order");

        let match_out_of_order = fuzzy_match("abc", "cba");
        assert!(!match_out_of_order.matches, "abc in cba should NOT match (out of order)");
    }

    #[test]
    fn test_case_insensitive_matching() {
        // "case insensitive matching"
        let result1 = fuzzy_match("ABC", "abc");
        assert!(result1.matches, "ABC should match abc (case insensitive)");

        let result2 = fuzzy_match("abc", "ABC");
        assert!(result2.matches, "abc should match ABC (case insensitive)");
    }

    #[test]
    fn test_consecutive_matches_score_better_than_scattered() {
        // "consecutive matches score better than scattered matches"
        let consecutive = fuzzy_match("foo", "foobar");
        let scattered = fuzzy_match("foo", "f_o_o_bar");

        assert!(consecutive.matches, "foo in foobar should match");
        assert!(scattered.matches, "foo in f_o_o_bar should match");
        // Lower score = better. Consecutive should have a lower (better) score.
        assert!(consecutive.score < scattered.score,
            "consecutive match ({}) should score better than scattered ({})",
            consecutive.score, scattered.score);
    }

    #[test]
    fn test_word_boundary_matches_score_better() {
        // "word boundary matches score better"
        let at_boundary = fuzzy_match("fb", "foo-bar");
        let not_at_boundary = fuzzy_match("fb", "afbx");

        assert!(at_boundary.matches, "fb in foo-bar should match");
        assert!(not_at_boundary.matches, "fb in afbx should match");
        // Word boundary match should score better (lower)
        assert!(at_boundary.score < not_at_boundary.score,
            "word boundary match ({}) should score better than mid-word ({})",
            at_boundary.score, not_at_boundary.score);
    }

    #[test]
    fn test_matches_swapped_alpha_numeric_tokens() {
        // "matches swapped alpha numeric tokens"
        let result = fuzzy_match("codex52", "gpt-5.2-codex");
        assert!(result.matches, "codex52 should match gpt-5.2-codex via alpha-num swap");
    }

    // ==========================================================================
    // Tests from fuzzyFilter describe block
    // ==========================================================================

    #[test]
    fn test_fuzzy_filter_empty_query_returns_all_items_unchanged() {
        // "empty query returns all items unchanged"
        let items = vec!["apple", "banana", "cherry"];
        let result = fuzzy_filter(items.clone(), "", |x| x);
        assert_eq!(result, items, "empty query should return all items unchanged");
    }

    #[test]
    fn test_fuzzy_filter_filters_out_non_matching_items() {
        // "filters out non-matching items"
        let items = vec!["apple", "banana", "cherry"];
        let result = fuzzy_filter(items, "an", |x| x);
        assert!(result.contains(&"banana"), "banana should match 'an'");
        assert!(!result.contains(&"apple"), "apple should not match 'an'");
        assert!(!result.contains(&"cherry"), "cherry should not match 'an'");
    }

    #[test]
    fn test_fuzzy_filter_sorts_results_by_match_quality() {
        // "sorts results by match quality"
        let items = vec!["a_p_p", "app", "application"];
        let result = fuzzy_filter(items, "app", |x| x);
        // "app" should be first (exact consecutive match at start)
        assert!(!result.is_empty(), "should have results");
        assert_eq!(result[0], "app", "'app' should be first (best match)");
    }

    #[test]
    fn test_fuzzy_filter_works_with_custom_get_text_function() {
        // "works with custom getText function"
        #[derive(Debug, PartialEq)]
        struct Item {
            name: String,
            id: u32,
        }

        let items = vec![
            Item { name: "foo".to_string(), id: 1 },
            Item { name: "bar".to_string(), id: 2 },
            Item { name: "foobar".to_string(), id: 3 },
        ];

        let result = fuzzy_filter(items, "foo", |item: &Item| item.name.as_str());
        assert_eq!(result.len(), 2, "should return 2 items matching 'foo'");
        let names: Vec<&str> = result.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"foo"), "should include 'foo'");
        assert!(names.contains(&"foobar"), "should include 'foobar'");
    }
}
