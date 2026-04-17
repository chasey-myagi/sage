// TUI — multi-agent panel using ratatui + crossterm.
//
// Layout: horizontal split — 22-char agent list on the left, chat on the right.
// The right panel is vertically split: chat history on top, 3-line input at bottom.
//
// Each live agent gets a background `connect_task` that holds the Unix socket open.
// Keyboard shortcuts: Tab / Up / Down — switch agents; Enter — send; Ctrl+C — quit.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use crossterm::{
    event::{Event, EventStream, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt as _;
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader},
    net::unix::OwnedWriteHalf,
    sync::{Mutex, mpsc},
};

// ── Protocol (mirrors daemon.rs — intentional copy to avoid coupling) ──

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMsg {
    Send { text: String },
    Reset,
    Ping,
    Shutdown,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMsg {
    TextDelta { text: String },
    ToolStart { name: String, id: String },
    ToolEnd { id: String, is_error: bool },
    CompactionStart { reason: String, message_count: usize },
    CompactionEnd { tokens_before: u64, messages_compacted: usize },
    RunEnd,
    RunError { error: String },
    Pong,
    ResetOk,
    ShutdownOk,
}

// ── Internal event bus ────────────────────────────────────────────────

/// Commands sent from the UI to a per-agent background task.
#[derive(Debug)]
enum DaemonCmd {
    Send(String),
    Reset,
}

/// Events produced by a per-agent background task back to the UI.
#[derive(Debug)]
enum AgentEvent {
    Connected,
    Disconnected,
    TextDelta(String),
    ToolStart(String),
    ToolEnd { is_error: bool },
    CompactionStart(String),
    RunEnd,
    RunError(String),
    ResetOk,
}

// ── Agent pane state ─────────────────────────────────────────────────

struct AgentPane {
    name: String,
    connected: bool,
    lines: Vec<String>,       // displayed chat lines (completed)
    current_line: String,     // in-progress streamed text
    cmd_tx: mpsc::Sender<DaemonCmd>,
}

impl AgentPane {
    fn push_done(&mut self) {
        if !self.current_line.is_empty() {
            self.lines.push(self.current_line.clone());
            self.current_line.clear();
        }
    }

    fn apply(&mut self, ev: AgentEvent) {
        match ev {
            AgentEvent::Connected => {
                self.connected = true;
                self.lines.push("─── connected ───".into());
            }
            AgentEvent::Disconnected => {
                self.connected = false;
                self.push_done();
                self.lines.push("─── disconnected ───".into());
            }
            AgentEvent::TextDelta(t) => {
                // Accumulate until RunEnd
                for ch in t.chars() {
                    if ch == '\n' {
                        self.lines.push(self.current_line.clone());
                        self.current_line.clear();
                    } else {
                        self.current_line.push(ch);
                    }
                }
            }
            AgentEvent::ToolStart(name) => {
                self.push_done();
                self.lines.push(format!("  [tool: {name}]"));
            }
            AgentEvent::ToolEnd { is_error } => {
                if is_error {
                    self.lines.push("  [tool ERROR]".into());
                }
            }
            AgentEvent::CompactionStart(reason) => {
                self.push_done();
                self.lines.push(format!("  [compacting: {reason}]"));
            }
            AgentEvent::RunEnd => {
                self.push_done();
            }
            AgentEvent::RunError(e) => {
                self.push_done();
                self.lines.push(format!("Error: {e}"));
            }
            AgentEvent::ResetOk => {
                self.push_done();
                self.lines.push("─── session reset ───".into());
            }
        }
    }
}

// ── Application state ─────────────────────────────────────────────────

struct App {
    panes: Vec<AgentPane>,
    selected: usize,
    list_state: ListState,
    input: String,
    /// Receiving end for events from background tasks
    event_rx: mpsc::UnboundedReceiver<(usize, AgentEvent)>,
}

impl App {
    fn selected_pane(&self) -> &AgentPane {
        &self.panes[self.selected]
    }

    fn select(&mut self, idx: usize) {
        self.selected = idx;
        self.list_state.select(Some(idx));
    }

    fn prev(&mut self) {
        if self.panes.is_empty() {
            return;
        }
        let i = if self.selected == 0 {
            self.panes.len() - 1
        } else {
            self.selected - 1
        };
        self.select(i);
    }

    fn next(&mut self) {
        if self.panes.is_empty() {
            return;
        }
        let i = (self.selected + 1) % self.panes.len();
        self.select(i);
    }

    fn send_input(&mut self) {
        let text = self.input.trim().to_string();
        if text.is_empty() {
            return;
        }
        self.input.clear();

        if self.panes.is_empty() {
            return;
        }

        let pane = &mut self.panes[self.selected];

        // Handle built-in TUI commands
        if text.eq_ignore_ascii_case("/reset") {
            let _ = pane.cmd_tx.try_send(DaemonCmd::Reset);
            return;
        }

        pane.lines.push(format!("You: {text}"));
        let _ = pane.cmd_tx.try_send(DaemonCmd::Send(text));
    }

    fn drain_events(&mut self) {
        while let Ok((idx, ev)) = self.event_rx.try_recv() {
            if idx < self.panes.len() {
                self.panes[idx].apply(ev);
            }
        }
    }
}

// ── Background socket task ────────────────────────────────────────────

fn socket_path(name: &str) -> PathBuf {
    PathBuf::from(format!("/tmp/sage-{name}.sock"))
}

async fn connect_task(
    idx: usize,
    name: String,
    mut cmd_rx: mpsc::Receiver<DaemonCmd>,
    event_tx: mpsc::UnboundedSender<(usize, AgentEvent)>,
) {
    let sock = socket_path(&name);

    let stream = match tokio::net::UnixStream::connect(&sock).await {
        Ok(s) => s,
        Err(_) => {
            let _ = event_tx.send((idx, AgentEvent::Disconnected));
            return;
        }
    };

    let _ = event_tx.send((idx, AgentEvent::Connected));

    let (read_half, write_half) = stream.into_split();
    let writer: Arc<Mutex<OwnedWriteHalf>> = Arc::new(Mutex::new(write_half));
    let mut reader = BufReader::new(read_half);
    let mut srv_line = String::new();

    loop {
        tokio::select! {
            // Command from UI
            cmd = cmd_rx.recv() => {
                match cmd {
                    None => break,   // sender dropped
                    Some(DaemonCmd::Send(text)) => {
                        let msg = ClientMsg::Send { text };
                        if send_msg(&writer, &msg).await.is_err() { break; }
                    }
                    Some(DaemonCmd::Reset) => {
                        if send_msg(&writer, &ClientMsg::Reset).await.is_err() { break; }
                    }
                }
            }
            // Response from daemon
            result = reader.read_line(&mut srv_line) => {
                match result {
                    Ok(0) | Err(_) => break,  // EOF or error
                    Ok(_) => {
                        let trimmed = srv_line.trim();
                        if !trimmed.is_empty() {
                            if let Ok(msg) = serde_json::from_str::<ServerMsg>(trimmed) {
                                let ev = server_msg_to_event(msg);
                                if let Some(ev) = ev {
                                    let _ = event_tx.send((idx, ev));
                                }
                            }
                        }
                        srv_line.clear();
                    }
                }
            }
        }
    }

    let _ = event_tx.send((idx, AgentEvent::Disconnected));
}

fn server_msg_to_event(msg: ServerMsg) -> Option<AgentEvent> {
    match msg {
        ServerMsg::TextDelta { text } => Some(AgentEvent::TextDelta(text)),
        ServerMsg::ToolStart { name, .. } => Some(AgentEvent::ToolStart(name)),
        ServerMsg::ToolEnd { is_error, .. } => Some(AgentEvent::ToolEnd { is_error }),
        ServerMsg::CompactionStart { reason, .. } => Some(AgentEvent::CompactionStart(reason)),
        ServerMsg::RunEnd => Some(AgentEvent::RunEnd),
        ServerMsg::RunError { error } => Some(AgentEvent::RunError(error)),
        ServerMsg::ResetOk => Some(AgentEvent::ResetOk),
        _ => None,
    }
}

async fn send_msg(writer: &Arc<Mutex<OwnedWriteHalf>>, msg: &ClientMsg) -> Result<()> {
    let line = serde_json::to_string(msg)? + "\n";
    let mut w = writer.lock().await;
    w.write_all(line.as_bytes()).await?;
    Ok(())
}

// ── Agent discovery ───────────────────────────────────────────────────

/// Return names of agents whose daemon socket is reachable.
async fn discover_agents(filter: &Option<Vec<String>>) -> Vec<String> {
    let home = match sage_runner::home_dir() {
        Some(h) => h,
        None => return vec![],
    };

    let agents_dir = home.join(".sage").join("agents");
    let mut names = Vec::new();

    if let Ok(mut rd) = tokio::fs::read_dir(&agents_dir).await {
        while let Ok(Some(entry)) = rd.next_entry().await {
            if !entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();

            // Filter by requested names if provided
            if let Some(allowed) = filter {
                if !allowed.contains(&name) {
                    continue;
                }
            }

            // Only include agents whose socket is reachable
            if tokio::net::UnixStream::connect(socket_path(&name)).await.is_ok() {
                names.push(name);
            }
        }
    }

    names.sort();
    names
}

// ── Rendering ─────────────────────────────────────────────────────────

fn render(f: &mut ratatui::Frame, app: &App) {
    let outer = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(22), Constraint::Min(0)])
        .split(f.area());

    // ── Left: agent list ────────────────────────────────────────────
    let items: Vec<ListItem> = app
        .panes
        .iter()
        .map(|p| {
            let dot = if p.connected { "● " } else { "○ " };
            let dot_style = if p.connected {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            ListItem::new(Line::from(vec![
                Span::styled(dot, dot_style),
                Span::raw(&p.name),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Agents"))
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        );

    let mut ls = app.list_state.clone();
    f.render_stateful_widget(list, outer[0], &mut ls);

    // ── Right: chat + input ─────────────────────────────────────────
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(outer[1]);

    render_chat(f, app, right[0]);
    render_input(f, app, right[1]);
}

fn render_chat(f: &mut ratatui::Frame, app: &App, area: Rect) {
    if app.panes.is_empty() {
        let p = Paragraph::new("No agents connected.\nRun `sage start --agent <name>` first.")
            .block(Block::default().borders(Borders::ALL).title("Chat"))
            .wrap(Wrap { trim: false });
        f.render_widget(p, area);
        return;
    }

    let pane = app.selected_pane();

    // Collect all lines (history + in-progress)
    let mut all: Vec<String> = pane.lines.clone();
    if !pane.current_line.is_empty() {
        all.push(pane.current_line.clone());
    }

    // Show only the last N lines that fit in the area
    let max_lines = area.height.saturating_sub(2) as usize; // -2 for borders
    let start = if all.len() > max_lines {
        all.len() - max_lines
    } else {
        0
    };

    let lines: Vec<Line> = all[start..]
        .iter()
        .map(|l| Line::from(l.as_str()))
        .collect();

    let title = format!(" {} ", pane.name);
    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(title))
        .wrap(Wrap { trim: false });

    f.render_widget(p, area);
}

fn render_input(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let input_text = app.input.as_str();
    let p = Paragraph::new(input_text)
        .block(Block::default().borders(Borders::ALL).title("Input (Enter=send, Tab=switch, Ctrl+C=quit)"))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

// ── Main event loop ───────────────────────────────────────────────────

async fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    let mut event_stream = EventStream::new();
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(50));

    loop {
        tokio::select! {
            _ = tick.tick() => {
                app.drain_events();
                terminal.draw(|f| render(f, app))?;
            }
            maybe_event = event_stream.next() => {
                let Some(Ok(event)) = maybe_event else { break; };
                match event {
                    Event::Key(key) => {
                        match (key.modifiers, key.code) {
                            (KeyModifiers::CONTROL, KeyCode::Char('c')) => break,
                            (_, KeyCode::Tab) | (_, KeyCode::Down) => app.next(),
                            (_, KeyCode::Up) => app.prev(),
                            (_, KeyCode::Enter) => app.send_input(),
                            (_, KeyCode::Backspace) => { app.input.pop(); }
                            (_, KeyCode::Char(c)) => app.input.push(c),
                            _ => {}
                        }
                    }
                    Event::Resize(_, _) => {
                        terminal.draw(|f| render(f, app))?;
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

// ── Public entry point ────────────────────────────────────────────────

/// Run the multi-agent TUI.
///
/// If `filter` is `Some`, only agents in that list are shown.
/// If `filter` is `None`, all agents with a live socket are discovered.
pub async fn run_tui(filter: Option<Vec<String>>) -> Result<()> {
    let agent_names = discover_agents(&filter).await;

    if agent_names.is_empty() {
        println!("No running agent daemons found.");
        println!("Start one with: sage start --agent <name>");
        return Ok(());
    }

    // Create event channel
    let (event_tx, event_rx) = mpsc::unbounded_channel::<(usize, AgentEvent)>();

    // Create panes + spawn background tasks
    let mut panes = Vec::new();
    for (idx, name) in agent_names.iter().enumerate() {
        let (cmd_tx, cmd_rx) = mpsc::channel::<DaemonCmd>(32);
        panes.push(AgentPane {
            name: name.clone(),
            connected: false,
            lines: Vec::new(),
            current_line: String::new(),
            cmd_tx,
        });

        let name_clone = name.clone();
        let tx_clone = event_tx.clone();
        tokio::spawn(async move {
            connect_task(idx, name_clone, cmd_rx, tx_clone).await;
        });
    }

    let mut list_state = ListState::default();
    list_state.select(Some(0));

    let mut app = App {
        panes,
        selected: 0,
        list_state,
        input: String::new(),
        event_rx,
    };

    // Setup terminal
    enable_raw_mode().context("cannot enable raw mode")?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen).context("cannot enter alternate screen")?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("cannot create terminal")?;

    let result = run_event_loop(&mut terminal, &mut app).await;

    // Restore terminal
    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();

    result
}
