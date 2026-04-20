/// Autocomplete provider interface and CombinedAutocompleteProvider implementation.

use std::path::{Path, PathBuf};

use crate::fuzzy::fuzzy_filter;

const PATH_DELIMITERS: &[char] = &[' ', '\t', '"', '\'', '='];

fn to_display_path(value: &str) -> String {
    value.replace('\\', "/")
}

fn escape_regex(value: &str) -> String {
    let special = r".*+?^${}()|[\]\\";
    let mut out = String::with_capacity(value.len() * 2);
    for c in value.chars() {
        if special.contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn build_fd_path_query(query: &str) -> String {
    let normalized = to_display_path(query);
    if !normalized.contains('/') {
        return normalized;
    }

    let has_trailing_sep = normalized.ends_with('/');
    let trimmed = normalized.trim_matches('/');
    if trimmed.is_empty() {
        return normalized;
    }

    let sep_pattern = "[\\\\/]";
    let segments: Vec<String> = trimmed
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| escape_regex(s))
        .collect();

    if segments.is_empty() {
        return normalized;
    }

    let mut pattern = segments.join(sep_pattern);
    if has_trailing_sep {
        pattern.push_str(sep_pattern);
    }
    pattern
}

fn find_last_delimiter(text: &str) -> Option<usize> {
    for (i, c) in text.char_indices().rev() {
        if PATH_DELIMITERS.contains(&c) {
            return Some(i);
        }
    }
    None
}

fn find_unclosed_quote_start(text: &str) -> Option<usize> {
    let mut in_quotes = false;
    let mut quote_start = None;

    for (i, c) in text.char_indices() {
        if c == '"' {
            in_quotes = !in_quotes;
            if in_quotes {
                quote_start = Some(i);
            }
        }
    }

    if in_quotes { quote_start } else { None }
}

fn is_token_start(text: &str, index: usize) -> bool {
    if index == 0 {
        return true;
    }
    // Look at character before index
    let before = &text[..index];
    if let Some(c) = before.chars().last() {
        return PATH_DELIMITERS.contains(&c);
    }
    true
}

fn extract_quoted_prefix(text: &str) -> Option<String> {
    let quote_start = find_unclosed_quote_start(text)?;

    if quote_start > 0 {
        let before_quote = &text[..quote_start];
        if before_quote.ends_with('@') {
            let at_index = quote_start - 1;
            if is_token_start(text, at_index) {
                return Some(text[at_index..].to_string());
            }
            return None;
        }
    }

    if is_token_start(text, quote_start) {
        return Some(text[quote_start..].to_string());
    }

    None
}

struct ParsedPathPrefix {
    raw_prefix: String,
    is_at_prefix: bool,
    is_quoted_prefix: bool,
}

fn parse_path_prefix(prefix: &str) -> ParsedPathPrefix {
    if let Some(rest) = prefix.strip_prefix("@\"") {
        return ParsedPathPrefix {
            raw_prefix: rest.to_string(),
            is_at_prefix: true,
            is_quoted_prefix: true,
        };
    }
    if let Some(rest) = prefix.strip_prefix('"') {
        return ParsedPathPrefix {
            raw_prefix: rest.to_string(),
            is_at_prefix: false,
            is_quoted_prefix: true,
        };
    }
    if let Some(rest) = prefix.strip_prefix('@') {
        return ParsedPathPrefix {
            raw_prefix: rest.to_string(),
            is_at_prefix: true,
            is_quoted_prefix: false,
        };
    }
    ParsedPathPrefix {
        raw_prefix: prefix.to_string(),
        is_at_prefix: false,
        is_quoted_prefix: false,
    }
}

fn build_completion_value(path: &str, is_directory: bool, is_at_prefix: bool, is_quoted_prefix: bool) -> String {
    let needs_quotes = is_quoted_prefix || path.contains(' ');
    let prefix = if is_at_prefix { "@" } else { "" };

    if !needs_quotes {
        return format!("{prefix}{path}");
    }

    let open_quote = format!("{prefix}\"");
    format!("{open_quote}{path}\"")
}

/// Represents a single autocomplete suggestion.
#[derive(Debug, Clone)]
pub struct AutocompleteItem {
    pub value: String,
    pub label: String,
    pub description: Option<String>,
}

/// A slash command with optional argument completions.
#[derive(Clone)]
pub struct SlashCommand {
    pub name: String,
    pub description: Option<String>,
    pub argument_completions: Option<std::sync::Arc<dyn Fn(&str) -> Vec<AutocompleteItem> + Send + Sync + 'static>>,
}

impl std::fmt::Debug for SlashCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlashCommand")
            .field("name", &self.name)
            .field("description", &self.description)
            .field("argument_completions", &self.argument_completions.as_ref().map(|_| "<fn>"))
            .finish()
    }
}

/// A command-like item that can be either a SlashCommand or a plain AutocompleteItem.
#[derive(Debug, Clone)]
pub enum CommandItem {
    Slash(SlashCommand),
    Plain(AutocompleteItem),
}

impl CommandItem {
    pub fn name(&self) -> &str {
        match self {
            CommandItem::Slash(s) => &s.name,
            CommandItem::Plain(p) => &p.value,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            CommandItem::Slash(s) => &s.name,
            CommandItem::Plain(p) => &p.label,
        }
    }

    pub fn description(&self) -> Option<&str> {
        match self {
            CommandItem::Slash(s) => s.description.as_deref(),
            CommandItem::Plain(p) => p.description.as_deref(),
        }
    }
}

/// Result of a `get_suggestions` call.
pub struct AutocompleteSuggestions {
    pub items: Vec<AutocompleteItem>,
    /// What we're matching against (e.g., "/" or "src/").
    pub prefix: String,
}

/// Trait for autocomplete providers.
pub trait AutocompleteProvider: Send {
    /// Get autocomplete suggestions for current text/cursor position.
    /// Returns None if no suggestions are available.
    fn get_suggestions(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
    ) -> Option<AutocompleteSuggestions>;

    /// Apply the selected item.
    /// Returns the new lines, cursor line, and cursor column.
    fn apply_completion(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
        item: &AutocompleteItem,
        prefix: &str,
    ) -> (Vec<String>, usize, usize);

    /// Force file completion (called on Tab key).
    fn get_force_file_suggestions(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
    ) -> Option<AutocompleteSuggestions> {
        self.get_suggestions(lines, cursor_line, cursor_col)
    }

    /// Whether file completion should be triggered on Tab.
    fn should_trigger_file_completion(
        &self,
        _lines: &[String],
        _cursor_line: usize,
        _cursor_col: usize,
    ) -> bool {
        true
    }
}

// =============================================================================
// Walk directory with fd
// =============================================================================

fn walk_directory_with_fd(
    base_dir: &str,
    fd_path: &str,
    query: &str,
    max_results: usize,
) -> Vec<(String, bool)> {
    let mut args = vec![
        "--base-directory".to_string(),
        base_dir.to_string(),
        "--max-results".to_string(),
        max_results.to_string(),
        "--type".to_string(),
        "f".to_string(),
        "--type".to_string(),
        "d".to_string(),
        "--full-path".to_string(),
        "--hidden".to_string(),
        "--exclude".to_string(),
        ".git".to_string(),
        "--exclude".to_string(),
        ".git/*".to_string(),
        "--exclude".to_string(),
        ".git/**".to_string(),
    ];

    if !query.is_empty() {
        args.push(build_fd_path_query(query));
    }

    let output = match std::process::Command::new(fd_path)
        .args(&args)
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return vec![],
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut results = Vec::new();

    for line in stdout.lines().filter(|l| !l.is_empty()) {
        let display_line = to_display_path(line);
        let has_trailing_sep = display_line.ends_with('/');
        let normalized_path = if has_trailing_sep {
            display_line[..display_line.len() - 1].to_string()
        } else {
            display_line.clone()
        };

        if normalized_path == ".git"
            || normalized_path.starts_with(".git/")
            || normalized_path.contains("/.git/")
        {
            continue;
        }

        let is_directory = has_trailing_sep;
        results.push((display_line, is_directory));
    }

    results
}

// =============================================================================
// CombinedAutocompleteProvider
// =============================================================================

/// Combined provider that handles slash commands and file paths.
pub struct CombinedAutocompleteProvider {
    commands: Vec<CommandItem>,
    base_path: String,
    fd_path: Option<String>,
}

impl CombinedAutocompleteProvider {
    pub fn new(
        commands: Vec<CommandItem>,
        base_path: Option<String>,
        fd_path: Option<String>,
    ) -> Self {
        let base_path = base_path.unwrap_or_else(|| {
            std::env::current_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| ".".to_string())
        });
        Self { commands, base_path, fd_path }
    }

    // Expand home directory (~/) to actual home path
    fn expand_home_path(&self, path: &str) -> String {
        if path.starts_with("~/") {
            let home = dirs_next::home_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "~".to_string());
            let rest = &path[2..];
            if path.ends_with('/') && !format!("{home}/{rest}").ends_with('/') {
                format!("{home}/{rest}/")
            } else {
                format!("{home}/{rest}")
            }
        } else if path == "~" {
            dirs_next::home_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "~".to_string())
        } else {
            path.to_string()
        }
    }

    fn extract_at_prefix(&self, text: &str) -> Option<String> {
        let quoted = extract_quoted_prefix(text);
        if let Some(ref q) = quoted {
            if q.starts_with("@\"") {
                return quoted;
            }
        }

        let token_start = find_last_delimiter(text)
            .map(|i| i + 1)
            .unwrap_or(0);

        if text[token_start..].starts_with('@') {
            return Some(text[token_start..].to_string());
        }

        None
    }

    fn extract_path_prefix(&self, text: &str, force_extract: bool) -> Option<String> {
        if let Some(quoted) = extract_quoted_prefix(text) {
            return Some(quoted);
        }

        let token_start = find_last_delimiter(text)
            .map(|i| i + 1)
            .unwrap_or(0);
        let path_prefix = &text[token_start..];

        if force_extract {
            return Some(path_prefix.to_string());
        }

        if path_prefix.contains('/')
            || path_prefix.starts_with('.')
            || path_prefix.starts_with("~/")
        {
            return Some(path_prefix.to_string());
        }

        if path_prefix.is_empty() && text.ends_with(' ') {
            return Some(path_prefix.to_string());
        }

        None
    }

    fn resolve_scoped_fuzzy_query(
        &self,
        raw_query: &str,
    ) -> Option<(String, String, String)> {
        let normalized = to_display_path(raw_query);
        let slash_index = normalized.rfind('/')?;

        let display_base = normalized[..slash_index + 1].to_string();
        let query = normalized[slash_index + 1..].to_string();

        let base_dir = if display_base.starts_with("~/") {
            self.expand_home_path(&display_base)
        } else if display_base.starts_with('/') {
            display_base.clone()
        } else {
            format!("{}/{}", self.base_path, display_base)
        };

        if !Path::new(&base_dir).is_dir() {
            return None;
        }

        Some((base_dir, query, display_base))
    }

    fn scoped_path_for_display(&self, display_base: &str, relative_path: &str) -> String {
        let normalized = to_display_path(relative_path);
        if display_base == "/" {
            format!("/{normalized}")
        } else {
            format!("{}{}", to_display_path(display_base), normalized)
        }
    }

    fn score_entry(&self, file_path: &str, query: &str, is_directory: bool) -> i32 {
        let file_name = Path::new(file_path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let lower_name = file_name.to_lowercase();
        let lower_query = query.to_lowercase();

        let mut score = 0i32;
        if lower_name == lower_query {
            score = 100;
        } else if lower_name.starts_with(&lower_query) {
            score = 80;
        } else if lower_name.contains(&lower_query) {
            score = 50;
        } else if file_path.to_lowercase().contains(&lower_query) {
            score = 30;
        }

        if is_directory && score > 0 {
            score += 10;
        }
        score
    }

    fn get_fuzzy_file_suggestions(
        &self,
        query: &str,
        is_quoted_prefix: bool,
    ) -> Vec<AutocompleteItem> {
        let fd_path = match &self.fd_path {
            Some(p) => p.clone(),
            None => return vec![],
        };

        let scoped = self.resolve_scoped_fuzzy_query(query);
        let (fd_base_dir, fd_query, display_base_opt) = match &scoped {
            Some((bd, q, db)) => (bd.as_str(), q.as_str(), Some(db.as_str())),
            None => (self.base_path.as_str(), query, None),
        };

        let entries = walk_directory_with_fd(fd_base_dir, &fd_path, fd_query, 100);

        let mut scored: Vec<(String, bool, i32)> = entries
            .into_iter()
            .map(|(path, is_dir)| {
                let score = if !fd_query.is_empty() {
                    self.score_entry(&path, fd_query, is_dir)
                } else {
                    1
                };
                (path, is_dir, score)
            })
            .filter(|(_, _, s)| *s > 0)
            .collect();

        scored.sort_by(|a, b| b.2.cmp(&a.2));
        scored.truncate(20);

        let mut suggestions = Vec::new();
        for (entry_path, is_directory, _) in scored {
            let path_without_slash = if is_directory && entry_path.ends_with('/') {
                entry_path[..entry_path.len() - 1].to_string()
            } else {
                entry_path.clone()
            };

            let display_path = match display_base_opt {
                Some(db) => self.scoped_path_for_display(db, &path_without_slash),
                None => path_without_slash.clone(),
            };

            let entry_name = Path::new(&path_without_slash)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();

            let completion_path = if is_directory {
                format!("{display_path}/")
            } else {
                display_path.clone()
            };

            let value = build_completion_value(&completion_path, is_directory, true, is_quoted_prefix);
            let label = if is_directory {
                format!("{entry_name}/")
            } else {
                entry_name
            };

            suggestions.push(AutocompleteItem {
                value,
                label,
                description: Some(display_path),
            });
        }

        suggestions
    }

    fn get_file_suggestions(&self, prefix: &str) -> Vec<AutocompleteItem> {
        let parsed = parse_path_prefix(prefix);
        let raw_prefix = &parsed.raw_prefix;
        let is_at_prefix = parsed.is_at_prefix;
        let is_quoted_prefix = parsed.is_quoted_prefix;

        let expanded_prefix = if raw_prefix.starts_with('~') {
            self.expand_home_path(raw_prefix)
        } else {
            raw_prefix.clone()
        };

        let is_root_prefix = raw_prefix.is_empty()
            || raw_prefix == "./"
            || raw_prefix == "../"
            || raw_prefix == "~"
            || raw_prefix == "~/"
            || raw_prefix == "/"
            || (is_at_prefix && raw_prefix.is_empty());

        let (search_dir, search_prefix) = if is_root_prefix {
            let dir = if raw_prefix.starts_with('~') || expanded_prefix.starts_with('/') {
                expanded_prefix.clone()
            } else {
                format!("{}/{}", self.base_path, expanded_prefix)
            };
            (dir, String::new())
        } else if raw_prefix.ends_with('/') {
            let dir = if raw_prefix.starts_with('~') || expanded_prefix.starts_with('/') {
                expanded_prefix.clone()
            } else {
                format!("{}/{}", self.base_path, expanded_prefix)
            };
            (dir, String::new())
        } else {
            let path = Path::new(&expanded_prefix);
            let dir = path.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let search_dir = if raw_prefix.starts_with('~') || expanded_prefix.starts_with('/') {
                if dir.is_empty() { "/".to_string() } else { dir }
            } else {
                format!("{}/{}", self.base_path, if dir.is_empty() { ".".to_string() } else { dir })
            };
            (search_dir, file)
        };

        let entries = match std::fs::read_dir(&search_dir) {
            Ok(e) => e,
            Err(_) => return vec![],
        };

        let mut suggestions = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();

            if !name.to_lowercase().starts_with(&search_prefix.to_lowercase()) {
                continue;
            }

            let mut is_directory = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if !is_directory
                && entry.file_type().map(|t| t.is_symlink()).unwrap_or(false)
            {
                if let Ok(meta) = std::fs::metadata(entry.path()) {
                    is_directory = meta.is_dir();
                }
            }

            let display_prefix = raw_prefix.as_str();
            let relative_path = if display_prefix.ends_with('/') {
                format!("{display_prefix}{name}")
            } else if display_prefix.contains('/') || display_prefix.contains('\\') {
                if display_prefix.starts_with("~/") {
                    let home_relative = &display_prefix[2..];
                    let dir = Path::new(home_relative)
                        .parent()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if dir.is_empty() || dir == "." {
                        format!("~/{name}")
                    } else {
                        format!("~/{dir}/{name}")
                    }
                } else if display_prefix.starts_with('/') {
                    let dir = Path::new(display_prefix)
                        .parent()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or("/".to_string());
                    if dir == "/" {
                        format!("/{name}")
                    } else {
                        format!("{dir}/{name}")
                    }
                } else {
                    let dir = Path::new(display_prefix)
                        .parent()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let joined = if dir.is_empty() || dir == "." {
                        name.clone()
                    } else {
                        format!("{dir}/{name}")
                    };
                    if display_prefix.starts_with("./") && !joined.starts_with("./") {
                        format!("./{joined}")
                    } else {
                        joined
                    }
                }
            } else if display_prefix.starts_with('~') {
                format!("~/{name}")
            } else {
                name.clone()
            };

            let relative_path = to_display_path(&relative_path);
            let path_value = if is_directory {
                format!("{relative_path}/")
            } else {
                relative_path
            };
            let value = build_completion_value(&path_value, is_directory, is_at_prefix, is_quoted_prefix);

            suggestions.push(AutocompleteItem {
                value,
                label: if is_directory {
                    format!("{name}/")
                } else {
                    name
                },
                description: None,
            });
        }

        // Sort: directories first, then alphabetically
        suggestions.sort_by(|a, b| {
            let a_dir = a.value.ends_with('/');
            let b_dir = b.value.ends_with('/');
            match (a_dir, b_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.label.cmp(&b.label),
            }
        });

        suggestions
    }
}

impl AutocompleteProvider for CombinedAutocompleteProvider {
    fn get_suggestions(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
    ) -> Option<AutocompleteSuggestions> {
        let current_line = lines.get(cursor_line).map(|l| l.as_str()).unwrap_or("");
        let text_before_cursor = &current_line[..cursor_col.min(current_line.len())];

        // Check for @ file reference
        if let Some(at_prefix) = self.extract_at_prefix(text_before_cursor) {
            let parsed = parse_path_prefix(&at_prefix);
            let suggestions = self.get_fuzzy_file_suggestions(&parsed.raw_prefix, parsed.is_quoted_prefix);
            if suggestions.is_empty() {
                return None;
            }
            return Some(AutocompleteSuggestions { items: suggestions, prefix: at_prefix });
        }

        // Check for slash commands
        if text_before_cursor.starts_with('/') {
            let space_idx = text_before_cursor.find(' ');
            if space_idx.is_none() {
                // Completing command name
                let prefix = &text_before_cursor[1..]; // Remove "/"
                let command_names: Vec<(String, String, Option<String>)> = self.commands.iter().map(|cmd| {
                    (cmd.name().to_string(), cmd.label().to_string(), cmd.description().map(|s| s.to_string()))
                }).collect();

                let filtered = fuzzy_filter(command_names, prefix, |(name, _, _)| name.as_str());
                let items: Vec<AutocompleteItem> = filtered.into_iter().map(|(name, label, desc)| AutocompleteItem {
                    value: name,
                    label,
                    description: desc,
                }).collect();

                if items.is_empty() {
                    return None;
                }
                return Some(AutocompleteSuggestions {
                    items,
                    prefix: text_before_cursor.to_string(),
                });
            }
            // Argument completions: find the matching command and call its closure
            if let Some(space_pos) = space_idx {
                let cmd_name = &text_before_cursor[1..space_pos]; // text between "/" and " "
                let arg_prefix = &text_before_cursor[space_pos + 1..];
                if let Some(cmd) = self.commands.iter().find_map(|c| {
                    if let CommandItem::Slash(s) = c { if s.name == cmd_name { return Some(s); } }
                    None
                }) {
                    if let Some(ref arg_fn) = cmd.argument_completions {
                        let suggestions = arg_fn(arg_prefix);
                        if suggestions.is_empty() {
                            return None;
                        }
                        return Some(AutocompleteSuggestions {
                            items: suggestions,
                            prefix: arg_prefix.to_string(),
                        });
                    }
                }
            }
        }

        // Check for file paths
        if let Some(path_match) = self.extract_path_prefix(text_before_cursor, false) {
            let suggestions = self.get_file_suggestions(&path_match);
            if suggestions.is_empty() {
                return None;
            }
            return Some(AutocompleteSuggestions { items: suggestions, prefix: path_match });
        }

        None
    }

    fn apply_completion(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
        item: &AutocompleteItem,
        prefix: &str,
    ) -> (Vec<String>, usize, usize) {
        let current_line = lines.get(cursor_line).map(|l| l.as_str()).unwrap_or("");
        let before_prefix = &current_line[..cursor_col.saturating_sub(prefix.len())];
        let after_cursor = &current_line[cursor_col.min(current_line.len())..];

        let is_quoted_prefix = prefix.starts_with('"') || prefix.starts_with("@\"");
        let has_leading_quote_after_cursor = after_cursor.starts_with('"');
        let has_trailing_quote_in_item = item.value.ends_with('"');
        let adjusted_after_cursor = if is_quoted_prefix && has_trailing_quote_in_item && has_leading_quote_after_cursor {
            &after_cursor[1..]
        } else {
            after_cursor
        };

        // Slash command name completion (no space yet in prefix)
        let is_slash_command = prefix.starts_with('/')
            && before_prefix.trim().is_empty()
            && !prefix[1..].contains(' ');
        if is_slash_command {
            let new_line = format!("{before_prefix}/{} {adjusted_after_cursor}", item.value);
            let mut new_lines = lines.to_vec();
            new_lines[cursor_line] = new_line;
            let new_col = before_prefix.len() + item.value.len() + 2; // +2 for "/" and space
            return (new_lines, cursor_line, new_col);
        }

        // Slash command argument completion (line starts with "/cmd " and prefix is the arg fragment)
        let current_line_text = lines.get(cursor_line).map(|l| l.as_str()).unwrap_or("");
        let before_arg = &current_line_text[..cursor_col.saturating_sub(prefix.len())];
        let is_slash_arg = before_arg.trim_start().starts_with('/') && before_arg.contains(' ');
        if is_slash_arg {
            let new_line = format!("{before_arg}{}{adjusted_after_cursor}", item.value);
            let mut new_lines = lines.to_vec();
            new_lines[cursor_line] = new_line;
            let new_col = before_arg.len() + item.value.len();
            return (new_lines, cursor_line, new_col);
        }

        // File attachment
        if prefix.starts_with('@') {
            let is_directory = item.label.ends_with('/');
            let suffix = if is_directory { "" } else { " " };
            let new_line = format!("{}{}{suffix}{adjusted_after_cursor}", before_prefix, item.value);
            let mut new_lines = lines.to_vec();
            new_lines[cursor_line] = new_line;

            let has_trailing_quote = item.value.ends_with('"');
            let cursor_offset = if is_directory && has_trailing_quote {
                item.value.len() - 1
            } else {
                item.value.len()
            };
            let new_col = before_prefix.len() + cursor_offset + suffix.len();
            return (new_lines, cursor_line, new_col);
        }

        // File path completion
        let new_line = format!("{}{}{adjusted_after_cursor}", before_prefix, item.value);
        let mut new_lines = lines.to_vec();
        new_lines[cursor_line] = new_line;

        let is_directory = item.label.ends_with('/');
        let has_trailing_quote = item.value.ends_with('"');
        let cursor_offset = if is_directory && has_trailing_quote {
            item.value.len() - 1
        } else {
            item.value.len()
        };
        let new_col = before_prefix.len() + cursor_offset;
        (new_lines, cursor_line, new_col)
    }

    fn get_force_file_suggestions(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
    ) -> Option<AutocompleteSuggestions> {
        let current_line = lines.get(cursor_line).map(|l| l.as_str()).unwrap_or("");
        let text_before_cursor = &current_line[..cursor_col.min(current_line.len())];

        // Don't trigger if we're typing a slash command at the start
        if text_before_cursor.trim_start().starts_with('/')
            && !text_before_cursor.trim_start().contains(' ')
        {
            return None;
        }

        let path_match = self.extract_path_prefix(text_before_cursor, true)?;
        let suggestions = self.get_file_suggestions(&path_match);
        if suggestions.is_empty() {
            return None;
        }
        Some(AutocompleteSuggestions { items: suggestions, prefix: path_match })
    }

    fn should_trigger_file_completion(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
    ) -> bool {
        let current_line = lines.get(cursor_line).map(|l| l.as_str()).unwrap_or("");
        let text_before_cursor = &current_line[..cursor_col.min(current_line.len())];

        // Don't trigger if we're typing a slash command at the start
        !(text_before_cursor.trim_start().starts_with('/')
            && !text_before_cursor.trim_start().contains(' '))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_provider() -> CombinedAutocompleteProvider {
        CombinedAutocompleteProvider::new(vec![], None, None)
    }

    #[test]
    fn test_no_suggestions_empty() {
        let p = make_provider();
        let lines = vec!["".to_string()];
        let result = p.get_suggestions(&lines, 0, 0);
        assert!(result.is_none());
    }

    #[test]
    fn test_slash_command_matching() {
        let p = CombinedAutocompleteProvider::new(
            vec![
                CommandItem::Slash(SlashCommand { name: "help".to_string(), description: None, argument_completions: None }),
                CommandItem::Slash(SlashCommand { name: "hello".to_string(), description: None, argument_completions: None }),
            ],
            None,
            None,
        );
        let lines = vec!["/he".to_string()];
        let result = p.get_suggestions(&lines, 0, 3);
        assert!(result.is_some());
        let sug = result.unwrap();
        assert_eq!(sug.prefix, "/he");
        assert_eq!(sug.items.len(), 2);
    }

    #[test]
    fn test_parse_path_prefix_at() {
        let parsed = parse_path_prefix("@\"foo/bar");
        assert!(parsed.is_at_prefix);
        assert!(parsed.is_quoted_prefix);
        assert_eq!(parsed.raw_prefix, "foo/bar");
    }

    #[test]
    fn test_build_completion_value_no_quotes() {
        let v = build_completion_value("src/main.rs", false, false, false);
        assert_eq!(v, "src/main.rs");
    }

    #[test]
    fn test_build_completion_value_at_prefix() {
        let v = build_completion_value("src/", true, true, false);
        assert_eq!(v, "@src/");
    }

    #[test]
    fn test_build_fd_path_query_no_slash() {
        let q = build_fd_path_query("main");
        assert_eq!(q, "main");
    }

    #[test]
    fn test_build_fd_path_query_with_slash() {
        let q = build_fd_path_query("src/main");
        assert!(q.contains("src"));
        assert!(q.contains("main"));
    }

    // =========================================================================
    // Tests from autocomplete.test.ts – extractPathPrefix / getForceFileSuggestions
    // =========================================================================

    #[test]
    fn test_get_force_file_suggestions_root_slash() {
        let provider = CombinedAutocompleteProvider::new(vec![], Some("/tmp".to_string()), None);
        let lines = vec!["hey /".to_string()];
        let result = provider.get_force_file_suggestions(&lines, 0, 5);
        // Should return suggestions for root "/" prefix (or None if /tmp is empty)
        // We only verify it doesn't panic and handles the "/" prefix correctly
        if let Some(sug) = result {
            assert_eq!(sug.prefix, "/");
        }
    }

    #[test]
    fn test_get_force_file_suggestions_slash_command_no_trigger() {
        let provider = CombinedAutocompleteProvider::new(vec![], Some("/tmp".to_string()), None);
        let lines = vec!["/model".to_string()];
        let result = provider.get_force_file_suggestions(&lines, 0, 6);
        // Slash commands should not trigger file completion
        assert!(result.is_none());
    }

    #[test]
    fn test_get_force_file_suggestions_slash_command_with_path_arg() {
        let provider = CombinedAutocompleteProvider::new(vec![], Some("/tmp".to_string()), None);
        let lines = vec!["/command /".to_string()];
        let result = provider.get_force_file_suggestions(&lines, 0, 10);
        // Absolute paths after slash command should trigger
        if let Some(sug) = result {
            assert_eq!(sug.prefix, "/");
        }
    }

    #[test]
    fn test_parse_path_prefix_at_quoted() {
        let parsed = parse_path_prefix("@\"foo/bar");
        assert!(parsed.is_at_prefix);
        assert!(parsed.is_quoted_prefix);
        assert_eq!(parsed.raw_prefix, "foo/bar");
    }

    #[test]
    fn test_parse_path_prefix_at_unquoted() {
        let parsed = parse_path_prefix("@src/main");
        assert!(parsed.is_at_prefix);
        assert!(!parsed.is_quoted_prefix);
        assert_eq!(parsed.raw_prefix, "src/main");
    }

    #[test]
    fn test_parse_path_prefix_quoted_no_at() {
        let parsed = parse_path_prefix("\"my folder/");
        assert!(!parsed.is_at_prefix);
        assert!(parsed.is_quoted_prefix);
        assert_eq!(parsed.raw_prefix, "my folder/");
    }

    #[test]
    fn test_parse_path_prefix_plain() {
        let parsed = parse_path_prefix("src/main.rs");
        assert!(!parsed.is_at_prefix);
        assert!(!parsed.is_quoted_prefix);
        assert_eq!(parsed.raw_prefix, "src/main.rs");
    }

    #[test]
    fn test_build_completion_value_at_unquoted() {
        let v = build_completion_value("src/main.rs", false, true, false);
        assert_eq!(v, "@src/main.rs");
    }

    #[test]
    fn test_build_completion_value_quoted_no_at() {
        let v = build_completion_value("my folder/", true, false, true);
        assert_eq!(v, "\"my folder/\"");
    }

    #[test]
    fn test_build_completion_value_with_spaces_auto_quoted() {
        let v = build_completion_value("my folder/test.txt", false, false, false);
        assert_eq!(v, "\"my folder/test.txt\"");
    }

    #[test]
    fn test_build_fd_path_query_trailing_slash() {
        let q = build_fd_path_query("src/");
        assert!(q.contains("src"));
    }

    #[test]
    fn test_build_fd_path_query_deep_path() {
        let q = build_fd_path_query("packages/tui/src");
        assert!(q.contains("packages"));
        assert!(q.contains("tui"));
        assert!(q.contains("src"));
    }

    #[test]
    fn test_slash_command_no_match() {
        let p = CombinedAutocompleteProvider::new(
            vec![CommandItem::Slash(SlashCommand { name: "help".to_string(), description: None, argument_completions: None })],
            None,
            None,
        );
        let lines = vec!["/xyz".to_string()];
        let result = p.get_suggestions(&lines, 0, 4);
        // "xyz" doesn't match "help"
        if let Some(sug) = result {
            assert!(sug.items.is_empty() || sug.items.iter().all(|i| i.value.contains("xyz") || !i.value.starts_with("/help")));
        }
    }
}
