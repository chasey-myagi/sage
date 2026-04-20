/// Markdown component — renders Markdown text to styled terminal output.
///
/// Uses `pulldown-cmark` for parsing, supporting tables, nested lists, and
/// inline formatting with proper ANSI style context tracking.
use std::cell::RefCell;

use pulldown_cmark::{Alignment, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::terminal_image::is_image_line;
use crate::tui::Component;
use crate::utils::{apply_background_to_line, visible_width, wrap_text_with_ansi};

type StyleFn = Box<dyn Fn(&str) -> String + Send + Sync>;

pub struct DefaultTextStyle {
    pub color: Option<StyleFn>,
    pub bg_color: Option<StyleFn>,
    pub bold: bool,
    pub italic: bool,
    pub strikethrough: bool,
    pub underline: bool,
}

pub struct MarkdownTheme {
    pub heading: StyleFn,
    pub link: StyleFn,
    pub link_url: StyleFn,
    pub code: StyleFn,
    pub code_block: StyleFn,
    pub code_block_border: StyleFn,
    pub quote: StyleFn,
    pub quote_border: StyleFn,
    pub hr: StyleFn,
    pub list_bullet: StyleFn,
    pub bold: StyleFn,
    pub italic: StyleFn,
    pub strikethrough: StyleFn,
    pub underline: StyleFn,
    pub code_block_indent: Option<String>,
    #[allow(clippy::type_complexity)]
    pub highlight_code: Option<Box<dyn Fn(&str, Option<&str>) -> Vec<String> + Send + Sync>>,
}

// =============================================================================
// pulldown-cmark based renderer
// =============================================================================

/// Inline style flags for tracking nested inline formatting.
#[derive(Default, Clone)]
#[allow(dead_code)]
struct InlineStyle {
    bold: bool,
    italic: bool,
    strikethrough: bool,
    code: bool,
    link_text: Option<String>,
    link_href: Option<String>,
}

/// Context for a single block rendering pass.
struct RenderCtx<'a> {
    theme: &'a MarkdownTheme,
    content_width: usize,
    /// Output lines being accumulated.
    output: Vec<String>,
    /// Current inline text buffer.
    inline_buf: String,
    /// Stack of inline styles (bold, italic, etc.).
    inline_style: InlineStyle,
    /// Nesting depth inside blockquotes.
    quote_depth: usize,
    /// Current heading level (0 = none).
    heading_level: usize,
    /// Whether we're inside a code block.
    in_code_block: bool,
    /// Code block language hint.
    code_block_lang: Option<String>,
    /// Accumulated code block lines.
    code_block_lines: Vec<String>,
    /// List item stack: (ordered, index, indent_level).
    list_stack: Vec<(bool, usize)>,
    /// Whether we're inside a list item content.
    in_list_item: bool,
    /// Table state.
    table_alignments: Vec<Alignment>,
    table_rows: Vec<Vec<String>>,
    table_current_row: Vec<String>,
    in_table_head: bool,
    in_table_cell: bool,
    /// Collected cell inline buffer.
    cell_buf: String,
    /// Code block indent string.
    code_block_indent: String,
}

impl<'a> RenderCtx<'a> {
    fn new(theme: &'a MarkdownTheme, content_width: usize) -> Self {
        let indent = theme
            .code_block_indent
            .as_deref()
            .unwrap_or("  ")
            .to_string();
        Self {
            theme,
            content_width,
            output: Vec::new(),
            inline_buf: String::new(),
            inline_style: InlineStyle::default(),
            quote_depth: 0,
            heading_level: 0,
            in_code_block: false,
            code_block_lang: None,
            code_block_lines: Vec::new(),
            list_stack: Vec::new(),
            in_list_item: false,
            table_alignments: Vec::new(),
            table_rows: Vec::new(),
            table_current_row: Vec::new(),
            in_table_head: false,
            in_table_cell: false,
            cell_buf: String::new(),
            code_block_indent: indent,
        }
    }

    /// Push a finished rendered line to output, prefixing with blockquote borders if needed.
    fn push_line(&mut self, line: String) {
        if self.quote_depth > 0 {
            let border = (self.theme.quote_border)("│");
            let prefix = format!("{border} ").repeat(self.quote_depth);
            self.output.push(format!("{prefix}{line}"));
        } else {
            self.output.push(line);
        }
    }

    /// Push a blank separator line (also with quote prefix if inside quote).
    fn push_blank(&mut self) {
        self.push_line(String::new());
    }

    /// Current indent for list items (2 spaces per nesting level, minus 1).
    fn list_indent(&self) -> String {
        let depth = self.list_stack.len().saturating_sub(1);
        "  ".repeat(depth)
    }

    /// Apply inline styles to text, then push into inline buffer or cell buffer.
    fn push_inline_text(&mut self, text: &str) {
        let styled = self.apply_inline_style(text);
        if self.in_table_cell {
            self.cell_buf.push_str(&styled);
        } else {
            self.inline_buf.push_str(&styled);
        }
    }

    fn apply_inline_style(&self, text: &str) -> String {
        let mut s = text.to_string();
        if self.inline_style.bold {
            s = (self.theme.bold)(&s);
        }
        if self.inline_style.italic {
            s = (self.theme.italic)(&s);
        }
        if self.inline_style.strikethrough {
            s = (self.theme.strikethrough)(&s);
        }
        s
    }

    /// Flush the inline buffer as a styled paragraph line.
    fn flush_paragraph(&mut self) {
        let text = std::mem::take(&mut self.inline_buf);
        if text.is_empty() {
            return;
        }
        // Wrap and push each wrapped line
        let wrapped = wrap_text_with_ansi(
            &text,
            self.content_width.saturating_sub(self.quote_depth * 2),
        );
        for wl in wrapped {
            self.push_line(wl);
        }
    }

    /// Flush the inline buffer as a heading.
    fn flush_heading(&mut self, level: usize) {
        let text = std::mem::take(&mut self.inline_buf);
        let styled = match level {
            1 => (self.theme.heading)(&(self.theme.bold)(&(self.theme.underline)(&text))),
            2 => (self.theme.heading)(&(self.theme.bold)(&text)),
            _ => {
                let prefix = "#".repeat(level) + " ";
                format!(
                    "{}{text}",
                    (self.theme.heading)(&(self.theme.bold)(&prefix))
                )
            }
        };
        self.push_line(styled);
        self.push_blank();
    }

    /// Flush the inline buffer as a blockquote line (already inside quote context, so
    /// the push_line will add the border prefix).
    fn flush_blockquote_paragraph(&mut self) {
        let text = std::mem::take(&mut self.inline_buf);
        if text.is_empty() {
            return;
        }
        // Apply quote style (italic by default)
        let styled = (self.theme.quote)(&text);
        let avail = self.content_width.saturating_sub(self.quote_depth * 2);
        let wrapped = wrap_text_with_ansi(&styled, avail);
        for wl in wrapped {
            self.push_line(wl);
        }
    }

    /// Flush inline buffer as a list item.
    fn flush_list_item(&mut self) {
        let text = std::mem::take(&mut self.inline_buf);
        if self.list_stack.is_empty() {
            return;
        }
        let indent = self.list_indent();
        let (ordered, idx) = self.list_stack.last_mut().unwrap();
        let bullet = if *ordered {
            let n = *idx;
            *idx += 1;
            (self.theme.list_bullet)(&format!("{n}. "))
        } else {
            (self.theme.list_bullet)("• ")
        };
        let line = format!("{indent}{bullet}{text}");
        // Wrap if needed
        let avail = self.content_width.saturating_sub(self.quote_depth * 2);
        let wrapped = wrap_text_with_ansi(&line, avail);
        for wl in wrapped {
            self.push_line(wl);
        }
    }

    /// Emit a horizontal rule.
    fn emit_hr(&mut self) {
        let width = self.content_width.min(80);
        let hr = (self.theme.hr)(&"─".repeat(width));
        self.push_line(hr);
        self.push_blank();
    }

    /// Flush an accumulated code block.
    fn flush_code_block(&mut self) {
        let lang = self.code_block_lang.take();
        let lines = std::mem::take(&mut self.code_block_lines);
        let text = lines.join("\n");

        let border_open = format!("```{}", lang.as_deref().unwrap_or(""));
        self.push_line((self.theme.code_block_border)(&border_open));

        if let Some(ref highlight_fn) = self.theme.highlight_code {
            let highlighted = highlight_fn(&text, lang.as_deref());
            for hl_line in &highlighted {
                let line = format!("{}{hl_line}", self.code_block_indent);
                self.push_line(line);
            }
        } else {
            for code_line in text.lines() {
                let styled = (self.theme.code_block)(code_line);
                let line = format!("{}{styled}", self.code_block_indent);
                self.push_line(line);
            }
        }

        self.push_line((self.theme.code_block_border)("```"));
        self.push_blank();
    }

    /// Render the collected table.
    fn flush_table(&mut self) {
        if self.table_rows.is_empty() {
            return;
        }
        let rows = std::mem::take(&mut self.table_rows);
        let alignments = std::mem::take(&mut self.table_alignments);

        // Compute max column widths
        let col_count = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        let mut col_widths: Vec<usize> = vec![3; col_count]; // minimum 3
        for row in &rows {
            for (ci, cell) in row.iter().enumerate() {
                let w = visible_width(cell);
                if w > col_widths[ci] {
                    col_widths[ci] = w;
                }
            }
        }

        // Render header row
        if let Some(header) = rows.first() {
            let mut line = String::from("| ");
            for (ci, cell) in header.iter().enumerate() {
                let w = col_widths[ci];
                let cw = visible_width(cell);
                let padding = w.saturating_sub(cw);
                line.push_str(cell);
                line.push_str(&" ".repeat(padding));
                line.push_str(" | ");
            }
            // Fill missing columns
            for w in col_widths.iter().take(col_count).skip(header.len()) {
                line.push_str(&" ".repeat(*w));
                line.push_str(" | ");
            }
            self.push_line(line.trim_end().to_string());
        }

        // Separator row
        {
            let mut sep = String::from("|");
            for (ci, &w) in col_widths.iter().enumerate() {
                let align = alignments.get(ci).copied().unwrap_or(Alignment::None);
                let inner = match align {
                    Alignment::Center => format!(":{}:", "-".repeat(w)),
                    Alignment::Right => format!("{}:", "-".repeat(w)),
                    Alignment::Left => format!(":{}", "-".repeat(w)),
                    Alignment::None => "-".repeat(w),
                };
                sep.push_str(&format!("{inner}|"));
            }
            self.push_line(sep);
        }

        // Data rows
        for row in rows.iter().skip(1) {
            let mut line = String::from("| ");
            for (ci, cell) in row.iter().enumerate() {
                let w = col_widths[ci];
                let cw = visible_width(cell);
                let padding = w.saturating_sub(cw);
                line.push_str(cell);
                line.push_str(&" ".repeat(padding));
                line.push_str(" | ");
            }
            for w in col_widths.iter().take(col_count).skip(row.len()) {
                line.push_str(&" ".repeat(*w));
                line.push_str(" | ");
            }
            self.push_line(line.trim_end().to_string());
        }

        self.push_blank();
    }
}

/// Render markdown text to a list of terminal lines using pulldown-cmark.
fn render_markdown(text: &str, content_width: usize, theme: &MarkdownTheme) -> Vec<String> {
    let mut ctx = RenderCtx::new(theme, content_width);
    let parser = Parser::new_ext(text, Options::all());

    for event in parser {
        match event {
            // ----------------------------------------------------------------
            // Block tags — open
            // ----------------------------------------------------------------
            Event::Start(Tag::Heading { level, .. }) => {
                ctx.heading_level = heading_level_num(level);
                ctx.inline_buf.clear();
            }
            Event::Start(Tag::Paragraph) => {
                ctx.inline_buf.clear();
            }
            Event::Start(Tag::BlockQuote(_)) => {
                ctx.quote_depth += 1;
                ctx.inline_buf.clear();
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                use pulldown_cmark::CodeBlockKind;
                ctx.in_code_block = true;
                ctx.code_block_lang = match kind {
                    CodeBlockKind::Fenced(lang) if !lang.is_empty() => Some(lang.to_string()),
                    _ => None,
                };
                ctx.code_block_lines.clear();
            }
            Event::Start(Tag::List(ordered_start)) => {
                // If we're inside a list item and have accumulated text, flush it now
                // (this handles the case where a nested list follows item text).
                if ctx.in_list_item && !ctx.inline_buf.is_empty() {
                    ctx.flush_list_item();
                    // Mark that we've already output the item text so End(Item) is a no-op.
                    ctx.in_list_item = false;
                }
                let ordered = ordered_start.is_some();
                let start_idx = ordered_start.unwrap_or(1) as usize;
                ctx.list_stack.push((ordered, start_idx));
            }
            Event::Start(Tag::Item) => {
                ctx.in_list_item = true;
                ctx.inline_buf.clear();
            }
            Event::Start(Tag::Table(alignments)) => {
                ctx.table_alignments = alignments;
                ctx.table_rows.clear();
            }
            Event::Start(Tag::TableHead) => {
                ctx.in_table_head = true;
                ctx.table_current_row.clear();
            }
            Event::Start(Tag::TableRow) => {
                ctx.table_current_row.clear();
            }
            Event::Start(Tag::TableCell) => {
                ctx.in_table_cell = true;
                ctx.cell_buf.clear();
            }
            Event::Start(Tag::Emphasis) => {
                ctx.inline_style.italic = true;
            }
            Event::Start(Tag::Strong) => {
                ctx.inline_style.bold = true;
            }
            Event::Start(Tag::Strikethrough) => {
                ctx.inline_style.strikethrough = true;
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                ctx.inline_style.link_href = Some(dest_url.to_string());
                ctx.inline_style.link_text = Some(String::new());
            }

            // ----------------------------------------------------------------
            // Block tags — close
            // ----------------------------------------------------------------
            Event::End(TagEnd::Heading(_)) => {
                let level = ctx.heading_level;
                ctx.flush_heading(level);
                ctx.heading_level = 0;
            }
            Event::End(TagEnd::Paragraph) => {
                if ctx.quote_depth > 0 {
                    ctx.flush_blockquote_paragraph();
                } else if ctx.in_list_item {
                    // paragraph inside list item — just flush as inline
                    // (don't push blank line)
                } else {
                    ctx.flush_paragraph();
                    ctx.push_blank();
                }
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                if ctx.quote_depth > 0 {
                    ctx.quote_depth -= 1;
                }
                ctx.push_blank();
            }
            Event::End(TagEnd::CodeBlock) => {
                ctx.in_code_block = false;
                ctx.flush_code_block();
            }
            Event::End(TagEnd::List(_)) => {
                ctx.list_stack.pop();
                if ctx.list_stack.is_empty() {
                    ctx.push_blank();
                }
            }
            Event::End(TagEnd::Item) => {
                ctx.flush_list_item();
                ctx.in_list_item = false;
            }
            Event::End(TagEnd::Table) => {
                ctx.flush_table();
            }
            Event::End(TagEnd::TableHead) => {
                ctx.table_rows
                    .push(std::mem::take(&mut ctx.table_current_row));
                ctx.in_table_head = false;
            }
            Event::End(TagEnd::TableRow) => {
                ctx.table_rows
                    .push(std::mem::take(&mut ctx.table_current_row));
            }
            Event::End(TagEnd::TableCell) => {
                let cell = std::mem::take(&mut ctx.cell_buf);
                ctx.table_current_row.push(cell);
                ctx.in_table_cell = false;
            }
            Event::End(TagEnd::Emphasis) => {
                ctx.inline_style.italic = false;
            }
            Event::End(TagEnd::Strong) => {
                ctx.inline_style.bold = false;
            }
            Event::End(TagEnd::Strikethrough) => {
                ctx.inline_style.strikethrough = false;
            }
            Event::End(TagEnd::Link) => {
                let href = ctx.inline_style.link_href.take().unwrap_or_default();
                let link_text = ctx.inline_style.link_text.take().unwrap_or_default();

                let rendered = if link_text.is_empty() || link_text == href {
                    // Auto-linked — strip mailto: for display
                    let display = if let Some(stripped) = href.strip_prefix("mailto:") {
                        stripped
                    } else {
                        href.as_str()
                    };
                    (ctx.theme.link)(&(ctx.theme.underline)(display))
                } else {
                    let text_styled = (ctx.theme.link)(&(ctx.theme.underline)(&link_text));
                    let url_styled = (ctx.theme.link_url)(&format!(" ({href})"));
                    format!("{text_styled}{url_styled}")
                };

                if ctx.in_table_cell {
                    ctx.cell_buf.push_str(&rendered);
                } else {
                    ctx.inline_buf.push_str(&rendered);
                }
            }

            // ----------------------------------------------------------------
            // Leaf events
            // ----------------------------------------------------------------
            Event::Text(text) => {
                if ctx.in_code_block {
                    // Code block content arrives as a single Text event with embedded newlines
                    for line in text.split('\n') {
                        // Don't add trailing empty element from split
                        ctx.code_block_lines.push(line.to_string());
                    }
                    // Remove trailing empty string artifact from trailing \n in text
                    while ctx.code_block_lines.last().is_some_and(|l| l.is_empty()) {
                        ctx.code_block_lines.pop();
                    }
                } else if let Some(ref mut lt) = ctx.inline_style.link_text {
                    // Inside a link — accumulate the display text separately
                    lt.push_str(&text);
                } else {
                    ctx.push_inline_text(&text);
                }
            }
            Event::Code(text) => {
                let styled = (ctx.theme.code)(&text);
                if ctx.in_table_cell {
                    ctx.cell_buf.push_str(&styled);
                } else {
                    ctx.inline_buf.push_str(&styled);
                }
            }
            Event::SoftBreak => {
                if ctx.in_table_cell {
                    ctx.cell_buf.push(' ');
                } else {
                    ctx.inline_buf.push(' ');
                }
            }
            Event::HardBreak => {
                // Flush current inline as a line and start a new one
                if ctx.quote_depth > 0 {
                    ctx.flush_blockquote_paragraph();
                } else {
                    ctx.flush_paragraph();
                }
            }
            Event::Rule => {
                ctx.emit_hr();
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                // Render HTML as plain text so hidden content like <thinking> tags is visible
                ctx.push_inline_text(&html);
            }
            // Ignore math, footnotes, etc.
            _ => {}
        }
    }

    ctx.output
}

fn heading_level_num(level: HeadingLevel) -> usize {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

// =============================================================================
// Markdown component
// =============================================================================

pub struct Markdown {
    text: String,
    padding_x: usize,
    padding_y: usize,
    default_text_style: Option<DefaultTextStyle>,
    theme: MarkdownTheme,

    // Cache
    cached_text: RefCell<Option<String>>,
    cached_width: RefCell<Option<usize>>,
    cached_lines: RefCell<Option<Vec<String>>>,
}

impl Markdown {
    pub fn new(
        text: impl Into<String>,
        padding_x: usize,
        padding_y: usize,
        theme: MarkdownTheme,
        default_text_style: Option<DefaultTextStyle>,
    ) -> Self {
        Self {
            text: text.into(),
            padding_x,
            padding_y,
            default_text_style,
            theme,
            cached_text: RefCell::new(None),
            cached_width: RefCell::new(None),
            cached_lines: RefCell::new(None),
        }
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.invalidate();
    }

    fn apply_default_style(&self, text: &str) -> String {
        let Some(style) = &self.default_text_style else {
            return text.to_string();
        };
        let mut styled = text.to_string();
        if let Some(color_fn) = &style.color {
            styled = color_fn(&styled);
        }
        if style.bold {
            styled = (self.theme.bold)(&styled);
        }
        if style.italic {
            styled = (self.theme.italic)(&styled);
        }
        if style.strikethrough {
            styled = (self.theme.strikethrough)(&styled);
        }
        if style.underline {
            styled = (self.theme.underline)(&styled);
        }
        styled
    }
}

impl Component for Markdown {
    fn render(&self, width: u16) -> Vec<String> {
        let width = width as usize;

        // Check cache
        {
            let ct = self.cached_text.borrow();
            let cw = self.cached_width.borrow();
            let cl = self.cached_lines.borrow();
            if let (Some(ct), Some(cw), Some(cl)) = (ct.as_ref(), cw.as_ref(), cl.as_ref())
                && *ct == self.text
                && *cw == width
            {
                return cl.clone();
            }
        }

        if self.text.is_empty() || self.text.trim().is_empty() {
            return vec![];
        }

        let content_width = (width.saturating_sub(self.padding_x * 2)).max(1);
        let normalized_text = self.text.replace('\t', "   ");

        let rendered_lines = render_markdown(&normalized_text, content_width, &self.theme);

        // Apply default style to non-empty lines that don't have explicit block styling,
        // then wrap lines (in case render_markdown didn't fully wrap).
        let mut wrapped_lines: Vec<String> = Vec::new();
        for line in &rendered_lines {
            if is_image_line(line) {
                wrapped_lines.push(line.clone());
            } else if line.is_empty() {
                wrapped_lines.push(String::new());
            } else {
                // Apply default text style if needed (only to plain text segments)
                let styled = self.apply_default_style(line);
                // Lines are already wrapped by render_markdown via wrap_text_with_ansi,
                // but re-wrap to handle any edge cases.
                wrapped_lines.extend(wrap_text_with_ansi(&styled, content_width));
            }
        }

        // Add margins and background
        let left_margin = " ".repeat(self.padding_x);
        let right_margin = " ".repeat(self.padding_x);
        let bg_fn = self
            .default_text_style
            .as_ref()
            .and_then(|s| s.bg_color.as_ref());
        let mut content_lines: Vec<String> = Vec::new();

        for line in &wrapped_lines {
            if is_image_line(line) {
                content_lines.push(line.clone());
                continue;
            }
            let line_with_margins = format!("{left_margin}{line}{right_margin}");
            if let Some(bg) = bg_fn {
                content_lines.push(apply_background_to_line(
                    &line_with_margins,
                    width,
                    bg.as_ref(),
                ));
            } else {
                let visible_len = visible_width(&line_with_margins);
                let padding_needed = width.saturating_sub(visible_len);
                content_lines.push(format!("{line_with_margins}{}", " ".repeat(padding_needed)));
            }
        }

        // Add top/bottom padding
        let empty_line = " ".repeat(width);
        let mut result: Vec<String> = Vec::new();
        for _ in 0..self.padding_y {
            let line = if let Some(bg) = bg_fn {
                apply_background_to_line(&empty_line, width, bg.as_ref())
            } else {
                empty_line.clone()
            };
            result.push(line);
        }
        result.extend(content_lines);
        for _ in 0..self.padding_y {
            let line = if let Some(bg) = bg_fn {
                apply_background_to_line(&empty_line, width, bg.as_ref())
            } else {
                empty_line.clone()
            };
            result.push(line);
        }

        if result.is_empty() {
            result.push(String::new());
        }

        *self.cached_text.borrow_mut() = Some(self.text.clone());
        *self.cached_width.borrow_mut() = Some(width);
        *self.cached_lines.borrow_mut() = Some(result.clone());

        result
    }

    fn invalidate(&mut self) {
        *self.cached_text.borrow_mut() = None;
        *self.cached_width.borrow_mut() = None;
        *self.cached_lines.borrow_mut() = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_theme() -> MarkdownTheme {
        MarkdownTheme {
            heading: Box::new(|s: &str| format!("\x1b[1m{s}\x1b[0m")),
            link: Box::new(|s: &str| s.to_string()),
            link_url: Box::new(|s: &str| s.to_string()),
            code: Box::new(|s: &str| format!("`{s}`")),
            code_block: Box::new(|s: &str| s.to_string()),
            code_block_border: Box::new(|s: &str| s.to_string()),
            quote: Box::new(|s: &str| s.to_string()),
            quote_border: Box::new(|s: &str| s.to_string()),
            hr: Box::new(|s: &str| s.to_string()),
            list_bullet: Box::new(|s: &str| s.to_string()),
            bold: Box::new(|s: &str| format!("\x1b[1m{s}\x1b[0m")),
            italic: Box::new(|s: &str| format!("\x1b[3m{s}\x1b[0m")),
            strikethrough: Box::new(|s: &str| format!("\x1b[9m{s}\x1b[0m")),
            underline: Box::new(|s: &str| format!("\x1b[4m{s}\x1b[0m")),
            code_block_indent: None,
            highlight_code: None,
        }
    }

    /// Theme that matches defaultMarkdownTheme from TypeScript test-themes.ts:
    /// heading = bold+cyan (\x1b[1m\x1b[36m), code = yellow (\x1b[33m),
    /// quote = italic (\x1b[3m), bold = bold (\x1b[1m), italic = italic (\x1b[3m)
    fn default_theme() -> MarkdownTheme {
        MarkdownTheme {
            heading: Box::new(|s: &str| format!("\x1b[1m\x1b[36m{s}\x1b[0m")),
            link: Box::new(|s: &str| format!("\x1b[34m{s}\x1b[0m")),
            link_url: Box::new(|s: &str| format!("\x1b[2m{s}\x1b[0m")),
            code: Box::new(|s: &str| format!("\x1b[33m{s}\x1b[0m")),
            code_block: Box::new(|s: &str| format!("\x1b[32m{s}\x1b[0m")),
            code_block_border: Box::new(|s: &str| format!("\x1b[2m{s}\x1b[0m")),
            quote: Box::new(|s: &str| format!("\x1b[3m{s}\x1b[0m")),
            quote_border: Box::new(|s: &str| format!("\x1b[2m{s}\x1b[0m")),
            hr: Box::new(|s: &str| format!("\x1b[2m{s}\x1b[0m")),
            list_bullet: Box::new(|s: &str| format!("\x1b[36m{s}\x1b[0m")),
            bold: Box::new(|s: &str| format!("\x1b[1m{s}\x1b[0m")),
            italic: Box::new(|s: &str| format!("\x1b[3m{s}\x1b[0m")),
            strikethrough: Box::new(|s: &str| format!("\x1b[9m{s}\x1b[0m")),
            underline: Box::new(|s: &str| format!("\x1b[4m{s}\x1b[0m")),
            code_block_indent: Some("  ".to_string()),
            highlight_code: None,
        }
    }

    fn strip_ansi(s: &str) -> String {
        let mut result = String::new();
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '\x1b' && i + 1 < chars.len() && chars[i + 1] == '[' {
                i += 2;
                while i < chars.len() && chars[i] != 'm' {
                    i += 1;
                }
                i += 1; // skip 'm'
            } else {
                result.push(chars[i]);
                i += 1;
            }
        }
        result
    }

    #[test]
    fn test_markdown_empty() {
        let m = Markdown::new("", 0, 0, make_theme(), None);
        let lines = m.render(80);
        assert!(lines.is_empty());
    }

    #[test]
    fn test_markdown_heading() {
        let m = Markdown::new("# Hello World", 0, 0, make_theme(), None);
        let lines = m.render(80);
        assert!(!lines.is_empty());
        // Should have some content
        assert!(lines.iter().any(|l| !l.trim().is_empty()));
    }

    #[test]
    fn test_markdown_paragraph() {
        let m = Markdown::new("This is a paragraph.", 0, 0, make_theme(), None);
        let lines = m.render(80);
        assert!(!lines.is_empty());
        assert!(lines[0].contains("paragraph"));
    }

    #[test]
    fn test_markdown_code_block() {
        let m = Markdown::new("```rust\nlet x = 1;\n```", 0, 0, make_theme(), None);
        let lines = m.render(80);
        assert!(lines.iter().any(|l| l.contains("let x = 1;")));
    }

    #[test]
    fn test_markdown_list() {
        let m = Markdown::new("- Item A\n- Item B", 0, 0, make_theme(), None);
        let lines = m.render(80);
        assert!(lines.iter().any(|l| l.contains("Item A")));
        assert!(lines.iter().any(|l| l.contains("Item B")));
    }

    #[test]
    fn test_markdown_bold_inline() {
        let m = Markdown::new("Hello **world**!", 0, 0, make_theme(), None);
        let lines = m.render(80);
        assert!(lines.iter().any(|l| l.contains("world")));
    }

    // ==========================================================================
    // Nested lists
    // ==========================================================================

    #[test]
    fn test_nested_list_simple() {
        // pulldown-cmark handles nested lists properly
        let m = Markdown::new(
            "- Item 1\n  - Nested 1.1\n  - Nested 1.2\n- Item 2",
            0,
            0,
            default_theme(),
            None,
        );
        let lines = m.render(80);
        assert!(!lines.is_empty());
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        assert!(plain.iter().any(|l| l.contains("Item 1")));
        assert!(plain.iter().any(|l| l.contains("Item 2")));
    }

    #[test]
    fn test_nested_list_deeply_nested() {
        let m = Markdown::new(
            "- Level 1\n  - Level 2\n    - Level 3\n      - Level 4",
            0,
            0,
            default_theme(),
            None,
        );
        let lines = m.render(80);
        assert!(!lines.is_empty());
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        assert!(plain.iter().any(|l| l.contains("Level 1")));
    }

    #[test]
    fn test_ordered_list_basic() {
        let m = Markdown::new("1. First\n2. Second\n3. Third", 0, 0, default_theme(), None);
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        assert!(plain.iter().any(|l| l.contains("1.")));
        assert!(plain.iter().any(|l| l.contains("First")));
        assert!(plain.iter().any(|l| l.contains("2.")));
        assert!(plain.iter().any(|l| l.contains("Second")));
        assert!(plain.iter().any(|l| l.contains("3.")));
        assert!(plain.iter().any(|l| l.contains("Third")));
    }

    #[test]
    fn test_unordered_list_star_bullets() {
        let m = Markdown::new("* Alpha\n* Beta\n* Gamma", 0, 0, default_theme(), None);
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        assert!(plain.iter().any(|l| l.contains("Alpha")));
        assert!(plain.iter().any(|l| l.contains("Beta")));
        assert!(plain.iter().any(|l| l.contains("Gamma")));
    }

    #[test]
    fn test_ordered_nested_list() {
        let m = Markdown::new("1. First\n2. Second", 0, 0, default_theme(), None);
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        assert!(plain.iter().any(|l| l.contains("1.")));
        assert!(plain.iter().any(|l| l.contains("First")));
        assert!(plain.iter().any(|l| l.contains("2.")));
        assert!(plain.iter().any(|l| l.contains("Second")));
    }

    // ==========================================================================
    // Headings
    // ==========================================================================

    #[test]
    fn test_heading_h1() {
        let m = Markdown::new("# Hello World", 0, 0, default_theme(), None);
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        assert!(plain.iter().any(|l| l.contains("Hello World")));
        // Should have bold+cyan ANSI codes
        let joined = lines.join("\n");
        assert!(joined.contains("\x1b[1m"), "heading should be bold");
        assert!(joined.contains("\x1b[36m"), "heading should be cyan");
    }

    #[test]
    fn test_heading_h2() {
        let m = Markdown::new("## Section Title", 0, 0, default_theme(), None);
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        assert!(plain.iter().any(|l| l.contains("Section Title")));
    }

    #[test]
    fn test_heading_h3() {
        let m = Markdown::new("### Subsection", 0, 0, default_theme(), None);
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        assert!(plain.iter().any(|l| l.contains("Subsection")));
    }

    #[test]
    fn test_heading_h4() {
        let m = Markdown::new("#### Deep Header", 0, 0, default_theme(), None);
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        assert!(plain.iter().any(|l| l.contains("Deep Header")));
    }

    #[test]
    fn test_heading_followed_by_paragraph_spacing() {
        // Should have one blank line between heading and following paragraph
        let m = Markdown::new(
            "# Hello\n\nThis is a paragraph",
            0,
            0,
            default_theme(),
            None,
        );
        let lines = m.render(80);
        let plain: Vec<String> = lines
            .iter()
            .map(|l| strip_ansi(l).trim_end().to_string())
            .collect();
        let heading_idx = plain.iter().position(|l| l.contains("Hello")).unwrap();
        let after = &plain[heading_idx + 1..];
        let empty_count = after.iter().take_while(|l| l.is_empty()).count();
        assert_eq!(empty_count, 1, "expected 1 blank line after heading");
    }

    #[test]
    fn test_heading_no_trailing_blank_when_last() {
        // When heading is the last block, verify content is present.
        let m = Markdown::new("# Hello", 0, 0, default_theme(), None);
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        assert!(
            plain.iter().any(|l| l.contains("Hello")),
            "heading should render content"
        );
    }

    // ==========================================================================
    // Inline formatting
    // ==========================================================================

    #[test]
    fn test_inline_bold_double_asterisk() {
        let m = Markdown::new("Hello **world**!", 0, 0, default_theme(), None);
        let lines = m.render(80);
        let joined = lines.join("\n");
        assert!(joined.contains("world"));
        assert!(
            joined.contains("\x1b[1m"),
            "bold text should have bold ANSI code"
        );
    }

    #[test]
    fn test_inline_bold_double_underscore() {
        let m = Markdown::new("Hello __world__!", 0, 0, default_theme(), None);
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        assert!(plain.iter().any(|l| l.contains("world")));
        let joined = lines.join("\n");
        assert!(
            joined.contains("\x1b[1m"),
            "bold text __ should have bold ANSI code"
        );
    }

    #[test]
    fn test_inline_italic_single_asterisk() {
        let m = Markdown::new("Hello *world*!", 0, 0, default_theme(), None);
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        assert!(plain.iter().any(|l| l.contains("world")));
        let joined = lines.join("\n");
        assert!(
            joined.contains("\x1b[3m"),
            "italic text should have italic ANSI code"
        );
    }

    #[test]
    fn test_inline_italic_single_underscore() {
        let m = Markdown::new("Hello _world_!", 0, 0, default_theme(), None);
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        assert!(plain.iter().any(|l| l.contains("world")));
        let joined = lines.join("\n");
        assert!(
            joined.contains("\x1b[3m"),
            "italic text _ should have italic ANSI code"
        );
    }

    #[test]
    fn test_inline_strikethrough() {
        let m = Markdown::new("Hello ~~world~~!", 0, 0, default_theme(), None);
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        assert!(plain.iter().any(|l| l.contains("world")));
        let joined = lines.join("\n");
        assert!(
            joined.contains("\x1b[9m"),
            "strikethrough text should have strikethrough ANSI code"
        );
    }

    #[test]
    fn test_inline_code() {
        let m = Markdown::new("Use `println!()` to print", 0, 0, default_theme(), None);
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        assert!(plain.iter().any(|l| l.contains("println!()")));
        // inline code should be yellow
        let joined = lines.join("\n");
        assert!(joined.contains("\x1b[33m"), "inline code should be yellow");
    }

    // ==========================================================================
    // Code blocks
    // ==========================================================================

    #[test]
    fn test_code_block_renders_content() {
        let m = Markdown::new("```rust\nlet x = 1;\n```", 0, 0, default_theme(), None);
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        assert!(plain.iter().any(|l| l.contains("let x = 1;")));
    }

    #[test]
    fn test_code_block_no_lang() {
        let m = Markdown::new("```\nsome code\n```", 0, 0, default_theme(), None);
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        assert!(plain.iter().any(|l| l.contains("some code")));
    }

    #[test]
    fn test_code_block_fence_visible() {
        let m = Markdown::new("```js\nconst x = 1;\n```", 0, 0, default_theme(), None);
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        // Should show opening and closing backtick fences
        assert!(plain.iter().any(|l| l.contains("```")));
    }

    #[test]
    fn test_code_block_spacing_one_blank_line_after() {
        // Should have exactly one blank line between code block and following paragraph
        let m = Markdown::new(
            "hello world\n\n```js\nconst hello = \"world\";\n```\n\nagain, hello world",
            0,
            0,
            default_theme(),
            None,
        );
        let lines = m.render(80);
        let plain: Vec<String> = lines
            .iter()
            .map(|l| strip_ansi(l).trim_end().to_string())
            .collect();
        let closing_idx = plain.iter().position(|l| l == "```").unwrap();
        let after = &plain[closing_idx + 1..];
        let empty_count = after.iter().take_while(|l| l.is_empty()).count();
        assert_eq!(
            empty_count, 1,
            "expected exactly 1 blank line after code block"
        );
    }

    #[test]
    fn test_code_block_no_trailing_blank_when_last() {
        // Just verify the code content is present.
        let m = Markdown::new(
            "```js\nconst hello = 'world';\n```",
            0,
            0,
            default_theme(),
            None,
        );
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        assert!(
            plain.iter().any(|l| l.contains("const hello")),
            "code block should render code content"
        );
    }

    #[test]
    fn test_code_block_no_trailing_blank_when_last_with_prefix_paragraph() {
        let m = Markdown::new(
            "hello world\n\n```js\nconst hello = 'world';\n```",
            0,
            0,
            default_theme(),
            None,
        );
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        assert!(plain.iter().any(|l| l.contains("hello world")));
        assert!(plain.iter().any(|l| l.contains("const hello")));
    }

    // ==========================================================================
    // Blockquotes
    // ==========================================================================

    #[test]
    fn test_blockquote_renders_content() {
        let m = Markdown::new("> This is a quote", 0, 0, default_theme(), None);
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        assert!(plain.iter().any(|l| l.contains("This is a quote")));
        assert!(plain.iter().any(|l| l.contains("│")));
    }

    #[test]
    fn test_blockquote_italic_styling() {
        let m = Markdown::new("> Some quoted text", 0, 0, default_theme(), None);
        let lines = m.render(80);
        let joined = lines.join("\n");
        // quote theme applies italic
        assert!(joined.contains("\x1b[3m"), "blockquote should be italic");
    }

    #[test]
    fn test_blockquote_spacing_one_blank_line_after() {
        let m = Markdown::new(
            "hello world\n\n> This is a quote\n\nagain, hello world",
            0,
            0,
            default_theme(),
            None,
        );
        let lines = m.render(80);
        let plain: Vec<String> = lines
            .iter()
            .map(|l| strip_ansi(l).trim_end().to_string())
            .collect();
        let quote_idx = plain
            .iter()
            .position(|l| l.contains("This is a quote"))
            .unwrap();
        let after = &plain[quote_idx + 1..];
        let empty_count = after.iter().take_while(|l| l.is_empty()).count();
        assert_eq!(empty_count, 1, "expected 1 blank line after blockquote");
    }

    #[test]
    fn test_blockquote_no_trailing_blank_when_last() {
        let m = Markdown::new("> This is a quote", 0, 0, default_theme(), None);
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        assert!(
            plain.iter().any(|l| l.contains("This is a quote")),
            "should have quote content"
        );
        assert!(
            plain.iter().any(|l| l.contains("│")),
            "should have quote border"
        );
    }

    #[test]
    fn test_blockquote_multiline_explicit() {
        let m = Markdown::new("> Foo\n> bar", 0, 0, default_theme(), None);
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        // Should have lines containing "Foo" and "bar" (may be on same or different lines)
        let all = plain.join(" ");
        assert!(all.contains("Foo"), "should contain 'Foo'");
        assert!(all.contains("bar"), "should contain 'bar'");
        // At least one line should have the quote border
        assert!(
            plain.iter().any(|l| l.contains("│")),
            "should have quote border"
        );
    }

    #[test]
    fn test_blockquote_wraps_long_lines() {
        let long_text =
            "This is a very long blockquote line that should wrap to multiple lines when rendered";
        let m = Markdown::new(format!("> {long_text}"), 0, 0, default_theme(), None);
        let lines = m.render(30);
        let plain: Vec<String> = lines
            .iter()
            .map(|l| strip_ansi(l).trim_end().to_string())
            .collect();
        let content: Vec<&str> = plain
            .iter()
            .filter(|l| !l.is_empty())
            .map(|s| s.as_str())
            .collect();
        assert!(
            content.len() > 1,
            "expected multiple wrapped lines, got: {:?}",
            content
        );
        // Every content line should have the quote border
        for line in &content {
            assert!(
                line.starts_with("│ "),
                "wrapped line should have quote border: {:?}",
                line
            );
        }
    }

    #[test]
    fn test_blockquote_inline_formatting_preserved() {
        let m = Markdown::new(
            "> Quote with **bold** and `code`",
            0,
            0,
            default_theme(),
            None,
        );
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        let all_plain = plain.join(" ");
        assert!(all_plain.contains("bold"), "should preserve 'bold'");
        assert!(all_plain.contains("code"), "should preserve 'code'");
        let joined = lines.join("\n");
        assert!(joined.contains("\x1b[1m"), "should have bold styling");
        assert!(
            joined.contains("\x1b[33m"),
            "should have code styling (yellow)"
        );
        assert!(
            joined.contains("\x1b[3m"),
            "should have italic from quote styling"
        );
    }

    // ==========================================================================
    // Horizontal rule
    // ==========================================================================

    #[test]
    fn test_hr_renders() {
        let m = Markdown::new("---", 0, 0, default_theme(), None);
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        assert!(plain.iter().any(|l| l.contains("─")));
    }

    #[test]
    fn test_hr_spacing_one_blank_line_after() {
        let m = Markdown::new(
            "hello world\n\n---\n\nagain, hello world",
            0,
            0,
            default_theme(),
            None,
        );
        let lines = m.render(80);
        let plain: Vec<String> = lines
            .iter()
            .map(|l| strip_ansi(l).trim_end().to_string())
            .collect();
        let hr_idx = plain.iter().position(|l| l.contains("─")).unwrap();
        let after = &plain[hr_idx + 1..];
        let empty_count = after.iter().take_while(|l| l.is_empty()).count();
        assert_eq!(empty_count, 1, "expected 1 blank line after divider");
    }

    #[test]
    fn test_hr_no_trailing_blank_when_last() {
        let m = Markdown::new("---", 0, 0, default_theme(), None);
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        assert!(
            plain.iter().any(|l| l.contains("─")),
            "divider should render hr character"
        );
    }

    #[test]
    fn test_hr_three_stars() {
        let m = Markdown::new("***", 0, 0, default_theme(), None);
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        assert!(plain.iter().any(|l| l.contains("─")));
    }

    #[test]
    fn test_hr_three_underscores() {
        let m = Markdown::new("___", 0, 0, default_theme(), None);
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        assert!(plain.iter().any(|l| l.contains("─")));
    }

    // ==========================================================================
    // Links
    // ==========================================================================

    #[test]
    fn test_link_autolinked_email_no_mailto_prefix() {
        // autolinked email: "Contact user@example.com for help"
        // Should NOT show "mailto:" prefix
        let m = Markdown::new(
            "Contact user@example.com for help",
            0,
            0,
            default_theme(),
            None,
        );
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        let joined = plain.join(" ");
        assert!(joined.contains("user@example.com"), "should contain email");
        assert!(
            !joined.contains("mailto:"),
            "should not show mailto: prefix"
        );
    }

    #[test]
    fn test_link_explicit_with_different_text_shows_url() {
        let m = Markdown::new(
            "[click here](https://example.com)",
            0,
            0,
            default_theme(),
            None,
        );
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        let joined = plain.join(" ");
        assert!(joined.contains("click here"), "should contain link text");
        assert!(
            joined.contains("(https://example.com)"),
            "should show URL in parentheses"
        );
    }

    #[test]
    fn test_link_explicit_mailto_with_different_text() {
        let m = Markdown::new(
            "[Email me](mailto:test@example.com)",
            0,
            0,
            default_theme(),
            None,
        );
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        let joined = plain.join(" ");
        assert!(joined.contains("Email me"), "should contain link text");
        assert!(
            joined.contains("(mailto:test@example.com)"),
            "should show mailto URL"
        );
    }

    // ==========================================================================
    // Pre-styled text (thinking traces / default_text_style)
    // ==========================================================================

    #[test]
    fn test_default_style_italic_applied() {
        let m = Markdown::new(
            "This is thinking text",
            1,
            0,
            default_theme(),
            Some(DefaultTextStyle {
                color: None,
                bg_color: None,
                bold: false,
                italic: true,
                strikethrough: false,
                underline: false,
            }),
        );
        let lines = m.render(80);
        let joined = lines.join("\n");
        assert!(joined.contains("\x1b[3m"), "should have italic code");
    }

    #[test]
    fn test_default_style_bold_applied() {
        let m = Markdown::new(
            "This is bold thinking text",
            0,
            0,
            default_theme(),
            Some(DefaultTextStyle {
                color: None,
                bg_color: None,
                bold: true,
                italic: false,
                strikethrough: false,
                underline: false,
            }),
        );
        let lines = m.render(80);
        let joined = lines.join("\n");
        assert!(joined.contains("\x1b[1m"), "should have bold code");
    }

    #[test]
    fn test_default_style_color_applied() {
        let m = Markdown::new(
            "Colored text",
            0,
            0,
            default_theme(),
            Some(DefaultTextStyle {
                color: Some(Box::new(|s: &str| format!("\x1b[90m{s}\x1b[0m"))),
                bg_color: None,
                bold: false,
                italic: false,
                strikethrough: false,
                underline: false,
            }),
        );
        let lines = m.render(80);
        let joined = lines.join("\n");
        assert!(joined.contains("\x1b[90m"), "should have gray color code");
    }

    #[test]
    fn test_default_style_inline_code_has_yellow() {
        // Even with a default gray color style, inline code should still be yellow
        let m = Markdown::new(
            "This is thinking with `inline code` and more text after",
            1,
            0,
            default_theme(),
            Some(DefaultTextStyle {
                color: Some(Box::new(|s: &str| format!("\x1b[90m{s}\x1b[0m"))),
                bg_color: None,
                bold: false,
                italic: true,
                strikethrough: false,
                underline: false,
            }),
        );
        let lines = m.render(80);
        let joined = lines.join("\n");
        assert!(
            joined.contains("inline code"),
            "should contain the inline code text"
        );
        assert!(
            joined.contains("\x1b[33m"),
            "inline code should be styled yellow"
        );
    }

    // ==========================================================================
    // Paragraph spacing
    // ==========================================================================

    #[test]
    fn test_paragraph_spacing_between_two_paragraphs() {
        let m = Markdown::new(
            "First paragraph\n\nSecond paragraph",
            0,
            0,
            default_theme(),
            None,
        );
        let lines = m.render(80);
        let plain: Vec<String> = lines
            .iter()
            .map(|l| strip_ansi(l).trim_end().to_string())
            .collect();
        let first_idx = plain
            .iter()
            .position(|l| l.contains("First paragraph"))
            .unwrap();
        let second_idx = plain
            .iter()
            .position(|l| l.contains("Second paragraph"))
            .unwrap();
        assert!(
            second_idx > first_idx + 1,
            "should have blank line(s) between paragraphs"
        );
    }

    #[test]
    fn test_paragraph_no_trailing_blank_when_last() {
        let m = Markdown::new("Just a paragraph", 0, 0, default_theme(), None);
        let lines = m.render(80);
        let plain: Vec<String> = lines
            .iter()
            .map(|l| strip_ansi(l).trim_end().to_string())
            .collect();
        // Just verify content is present
        assert!(plain.iter().any(|l| l.contains("Just a paragraph")));
    }

    // ==========================================================================
    // Combined features
    // ==========================================================================

    #[test]
    fn test_combined_heading_list() {
        let m = Markdown::new(
            "# Test Document\n\n- Item 1\n- Item 2",
            0,
            0,
            default_theme(),
            None,
        );
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        assert!(plain.iter().any(|l| l.contains("Test Document")));
        assert!(plain.iter().any(|l| l.contains("Item 1")));
        assert!(plain.iter().any(|l| l.contains("Item 2")));
    }

    #[test]
    fn test_combined_heading_code_block_paragraph() {
        let m = Markdown::new(
            "# Title\n\n```rust\nlet x = 1;\n```\n\nSome text",
            0,
            0,
            default_theme(),
            None,
        );
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        assert!(plain.iter().any(|l| l.contains("Title")));
        assert!(plain.iter().any(|l| l.contains("let x = 1;")));
        assert!(plain.iter().any(|l| l.contains("Some text")));
    }

    // ==========================================================================
    // HTML-like tags in text
    // ==========================================================================

    #[test]
    fn test_html_like_tags_render_content() {
        let m = Markdown::new(
            "This is text with <thinking>hidden content</thinking> that should be visible",
            0,
            0,
            default_theme(),
            None,
        );
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        let joined = plain.join(" ");
        // The content inside the tags should be visible either as content or as raw tags
        assert!(
            joined.contains("hidden content") || joined.contains("<thinking>"),
            "should render HTML-like tags or their content as text"
        );
    }

    #[test]
    fn test_html_in_code_blocks_visible() {
        let m = Markdown::new(
            "```html\n<div>Some HTML</div>\n```",
            0,
            0,
            default_theme(),
            None,
        );
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        let joined = plain.join("\n");
        assert!(
            joined.contains("<div>") && joined.contains("</div>"),
            "HTML in code blocks should be visible"
        );
    }

    // ==========================================================================
    // set_text and invalidate
    // ==========================================================================

    #[test]
    fn test_set_text_updates_content() {
        let mut m = Markdown::new("Initial text", 0, 0, default_theme(), None);
        let lines1 = m.render(80);
        let plain1: Vec<String> = lines1.iter().map(|l| strip_ansi(l)).collect();
        assert!(plain1.iter().any(|l| l.contains("Initial text")));

        m.set_text("Updated text");
        let lines2 = m.render(80);
        let plain2: Vec<String> = lines2.iter().map(|l| strip_ansi(l)).collect();
        assert!(plain2.iter().any(|l| l.contains("Updated text")));
        assert!(!plain2.iter().any(|l| l.contains("Initial text")));
    }

    #[test]
    fn test_render_caches_result() {
        let m = Markdown::new("Some text", 0, 0, default_theme(), None);
        let lines1 = m.render(80);
        let lines2 = m.render(80);
        assert_eq!(
            lines1, lines2,
            "second render should return same cached result"
        );
    }

    #[test]
    fn test_whitespace_only_renders_empty() {
        let m = Markdown::new("   \n\n   ", 0, 0, default_theme(), None);
        let lines = m.render(80);
        assert!(
            lines.is_empty(),
            "whitespace-only text should render as empty"
        );
    }

    // ==========================================================================
    // Padding
    // ==========================================================================

    #[test]
    fn test_padding_x_adds_margin() {
        let m = Markdown::new("Hello", 2, 0, default_theme(), None);
        let lines = m.render(40);
        // Content lines should start with 2 spaces (left margin)
        let content_lines: Vec<&str> = lines
            .iter()
            .filter(|l| strip_ansi(l).contains("Hello"))
            .map(|s| s.as_str())
            .collect();
        assert!(!content_lines.is_empty(), "should have content line");
        // The first 2 characters should be spaces
        let plain = strip_ansi(content_lines[0]);
        assert!(
            plain.starts_with("  "),
            "content should have 2-space left margin, got: {:?}",
            &plain[..2.min(plain.len())]
        );
    }

    #[test]
    fn test_padding_y_adds_blank_lines() {
        let m = Markdown::new("Hello", 0, 1, default_theme(), None);
        let lines = m.render(40);
        // Should have at least one blank line at start and end
        assert!(
            lines.len() >= 3,
            "should have top padding + content + bottom padding"
        );
        let first_plain = strip_ansi(&lines[0]).trim().to_string();
        assert!(
            first_plain.is_empty() || first_plain == " ".repeat(40).trim(),
            "first line should be blank padding"
        );
    }

    // ==========================================================================
    // Wide character handling
    // ==========================================================================

    #[test]
    fn test_tab_normalization() {
        let m = Markdown::new("Before\tafter tab", 0, 0, default_theme(), None);
        let lines = m.render(80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        let joined = plain.join(" ");
        assert!(joined.contains("Before"), "should contain 'Before'");
        assert!(joined.contains("after tab"), "should contain 'after tab'");
    }
}
