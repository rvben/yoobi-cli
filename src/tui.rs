//! Terminal workspace with explicitly reviewed booking edits.
use crate::{
    Error,
    api::Client,
    booking::{BookingEdit, BookingResult, parse_duration},
    config::Config,
    output::{clean, duration},
    timesheet::{Project, Timesheet, days_in_month, validate_month},
};
use chrono::{Datelike, Local, NaiveDate};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame, Terminal,
    backend::TestBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table, TableState,
        Wrap,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::IsTerminal,
    sync::mpsc,
    time::Duration,
};
const BG: Color = Color::Rgb(23, 23, 24);
const INK: Color = Color::Rgb(244, 241, 239);
const MUTED: Color = Color::Rgb(173, 165, 162);
// Yoobi website brand tokens: --yoobi--red and --yoobi--light-red.
const YOOBI_RED: Color = Color::Rgb(218, 38, 27);
const LIGHT_RED: Color = Color::Rgb(248, 212, 209);
const AMBER: Color = Color::Rgb(245, 196, 112);
const SELECT: Color = Color::Rgb(74, 37, 34);
fn io_error(_: std::io::Error) -> Error {
    Error::Io {
        message: "terminal operation failed; terminal state has been restored".into(),
    }
}
#[derive(Debug, Clone)]
pub struct Options {
    pub month: String,
    pub demo: bool,
    pub snapshot: bool,
    pub width: u16,
    pub height: u16,
}
pub fn current_month() -> String {
    Local::now().format("%Y-%m").to_string()
}
pub fn shift_month(month: &str, delta: i32) -> String {
    let year = month[..4].parse::<i32>().unwrap_or(2026);
    let m = month[5..].parse::<i32>().unwrap_or(1);
    let n = (year * 12 + m - 1 + delta).clamp(12, 9999 * 12 + 11);
    format!("{:04}-{:02}", n / 12, n % 12 + 1)
}
pub fn validate(options: &Options, json: bool) -> Result<(), Error> {
    validate_month(&options.month)?;
    if options.snapshot && !options.demo {
        return Err(Error::Usage {
            message: "--snapshot requires --demo; connected data cannot be captured accidentally"
                .into(),
        });
    }
    if !(40..=240).contains(&options.width) || !(16..=80).contains(&options.height) {
        return Err(Error::Usage {
            message: "snapshot dimensions must be width 40–240 and height 16–80".into(),
        });
    }
    if json {
        return Err(Error::Usage {
            message: "tui does not support JSON; use timesheet or projects".into(),
        });
    }
    if !options.snapshot && (!std::io::stdin().is_terminal() || !std::io::stdout().is_terminal()) {
        return Err(Error::TtyRequired {
            message: "tui requires terminal stdin and stdout".into(),
        });
    }
    Ok(())
}
struct Editor {
    edit: BookingEdit,
    input: String,
    ready: bool,
    error: Option<String>,
}
enum Request {
    Read(String),
    Save(BookingEdit),
}
enum Update {
    Read(String, Result<Timesheet, Error>),
    Save(Result<BookingResult, Error>),
}
struct Workspace {
    month: String,
    sheet: Timesheet,
    activity: usize,
    day: usize,
    query: String,
    searching: bool,
    help: bool,
    details: bool,
    detail_scroll: u16,
    error_detail: bool,
    loading: bool,
    saving: bool,
    editor: Option<Editor>,
    notice: Option<String>,
    error: Option<String>,
    demo: bool,
}
impl Workspace {
    fn new(options: &Options) -> Self {
        Self {
            month: options.month.clone(),
            sheet: Timesheet {
                base_url: String::new(),
                employee_id: String::new(),
                month: options.month.clone(),
                total_minutes: 0,
                projects: vec![],
            },
            activity: 0,
            day: 0,
            query: String::new(),
            searching: false,
            help: false,
            details: false,
            detail_scroll: 0,
            error_detail: false,
            loading: !options.demo,
            saving: false,
            editor: None,
            notice: None,
            error: None,
            demo: options.demo,
        }
    }
    fn project(&self) -> Option<&Project> {
        self.sheet.projects.get(self.activity)
    }
    fn dates(&self) -> Vec<String> {
        (1..=days_in_month(&self.month).unwrap_or(30))
            .map(|d| format!("{}-{d:02}", self.month))
            .filter(|d| self.query.is_empty() || d.contains(&self.query))
            .collect()
    }
    fn accept(&mut self, sheet: Timesheet) {
        self.sheet = sheet;
        self.activity = self
            .activity
            .min(self.sheet.projects.len().saturating_sub(1));
        self.loading = false;
        self.error = None;
    }
    fn begin_edit(&mut self) {
        if self.loading || self.saving || self.error.is_some() {
            return;
        }
        let Some(date) = self.dates().get(self.day).cloned() else {
            return;
        };
        let Some(activity) = self.project() else {
            return;
        };
        let minutes = activity.days.get(&date).copied().unwrap_or(0);
        if activity.locked_days.contains(&date) {
            self.notice = Some("Booking is locked in Yoobi".into());
            return;
        }
        match BookingEdit::from_sheet(&self.sheet, &activity.id, &date, minutes) {
            Ok(edit) => {
                self.editor = Some(Editor {
                    input: duration(i64::from(minutes)),
                    edit,
                    ready: false,
                    error: None,
                })
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }
    /// Returns a write only on the explicit save key in the preview state.
    fn edit_key(&mut self, key: KeyCode) -> Option<BookingEdit> {
        let editor = self.editor.as_mut()?;
        if key == KeyCode::Esc {
            self.editor = None;
            return None;
        }
        if editor.ready {
            match key {
                KeyCode::Enter => {
                    editor.ready = false;
                    editor.error = None;
                }
                KeyCode::Char('s') => {
                    let edit = editor.edit.clone();
                    self.editor = None;
                    return Some(edit);
                }
                _ => {}
            }
        } else {
            match key {
                KeyCode::Enter => match parse_duration(&editor.input) {
                    Ok(minutes) => {
                        editor.edit.minutes = minutes;
                        editor.ready = true;
                        editor.error = None;
                    }
                    Err(_) => editor.error = Some("Use 7h30, 7:30 or 450m; max 24h".into()),
                },
                KeyCode::Backspace => {
                    editor.input.pop();
                    editor.error = None;
                }
                KeyCode::Char(c)
                    if editor.input.len() < 12
                        && (c.is_ascii_digit() || matches!(c, 'h' | 'm' | ':')) =>
                {
                    editor.input.push(c);
                    editor.error = None;
                }
                _ => {}
            }
        }
        None
    }
    fn saved(&mut self, result: Result<BookingResult, Error>) {
        self.saving = false;
        self.loading = false;
        match result {
            Ok(result) => {
                self.accept(result.timesheet);
                self.notice = Some(
                    if self.demo {
                        "DEMO · sample booking updated"
                    } else if result.changed {
                        "Booking saved · verified"
                    } else {
                        "Booking already matches · verified"
                    }
                    .into(),
                );
            }
            Err(e) => {
                self.error = Some(format!("{e}. Refresh with r before editing again."));
                self.notice = None;
            }
        }
    }
    fn move_day(&mut self, delta: isize) {
        self.day = self
            .day
            .saturating_add_signed(delta)
            .min(self.dates().len().saturating_sub(1));
    }
}
fn frame_block(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(MUTED))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(INK).add_modifier(Modifier::BOLD),
        ))
}
fn draw(frame: &mut Frame, app: &mut Workspace) {
    let area = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().bg(BG).fg(INK)),
        area,
    );
    if area.width < 40 || area.height < 16 {
        frame.render_widget(
            Paragraph::new("Terminal too small\nResize to at least 40 × 16.\nq to quit")
                .style(Style::default().fg(INK))
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }
    let parts = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(5),
        Constraint::Length(2),
    ])
    .margin(1)
    .split(area);
    let mode = if app.demo {
        "DEMO · offline sample"
    } else if app.loading {
        "Loading…"
    } else if app.error.is_some() {
        "Attention · ! shows details"
    } else {
        "Connected"
    };
    let title = Line::from(vec![
        Span::styled(
            " YOOBI ",
            Style::default()
                .fg(Color::White)
                .bg(YOOBI_RED)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "   {}   /   {} booked",
            app.month,
            duration(i64::from(app.sheet.total_minutes))
        )),
    ]);
    let header = vec![title, Line::styled(mode, Style::default().fg(MUTED))];
    frame.render_widget(Paragraph::new(header), parts[0]);
    let cols = if area.width >= 105 {
        Layout::horizontal([
            Constraint::Length(27),
            Constraint::Min(32),
            Constraint::Length(29),
        ])
        .spacing(2)
        .split(parts[1])
    } else if area.width >= 75 {
        Layout::horizontal([Constraint::Length(23), Constraint::Min(30)])
            .spacing(2)
            .split(parts[1])
    } else {
        Layout::horizontal([Constraint::Min(1)]).split(parts[1])
    };
    let ledger = if cols.len() > 1 {
        let items: Vec<ListItem> = app
            .sheet
            .projects
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let total: u32 = p.days.values().sum();
                ListItem::new(vec![
                    Line::from(clean(&p.name)),
                    Line::from(Span::styled(
                        format!("  {} booked", duration(i64::from(total))),
                        Style::default().fg(MUTED),
                    )),
                ])
                .style(if i == app.activity {
                    Style::default().bg(SELECT).fg(LIGHT_RED)
                } else {
                    Style::default().fg(INK)
                })
            })
            .collect();
        let widget = List::new(items).block(frame_block("Activities"));
        let mut state = ListState::default().with_selected(Some(app.activity));
        frame.render_stateful_widget(widget, cols[0], &mut state);
        cols[1]
    } else {
        cols[0]
    };
    let dates = app.dates();
    let rows: Vec<Row> = dates
        .iter()
        .map(|date| {
            let booked = app
                .project()
                .and_then(|p| p.days.get(date))
                .copied()
                .unwrap_or(0);
            let locked = app.project().is_some_and(|p| p.locked_days.contains(date));
            let weekday = NaiveDate::parse_from_str(date, "%Y-%m-%d")
                .map(|d| d.format("%a").to_string())
                .unwrap_or_default();
            let date_label = if area.width < 75 {
                format!("{} {weekday}", &date[8..])
            } else {
                format!("{} {weekday}", &date[5..])
            };
            let state = if locked {
                "locked"
            } else if booked == 0 {
                "—"
            } else {
                "open"
            };
            Row::new(vec![
                Cell::from(date_label),
                Cell::from(if booked == 0 {
                    "·".into()
                } else {
                    duration(i64::from(booked))
                }),
                Cell::from(state),
            ])
            .style(Style::default().fg(if booked == 0 { MUTED } else { INK }))
        })
        .collect();
    let name = app
        .project()
        .map(|p| clean(&p.name))
        .unwrap_or_else(|| "Daily bookings".into());
    if app.sheet.projects.is_empty() {
        let msg = if app.loading {
            "Loading timesheet…\nYou can still navigate months or quit."
        } else {
            "No activities in this month.\nUse [ or ] to browse another month."
        };
        frame.render_widget(
            Paragraph::new(msg)
                .block(frame_block("Daily bookings"))
                .wrap(Wrap { trim: true }),
            ledger,
        );
    } else if dates.is_empty() {
        frame.render_widget(
            Paragraph::new(
                "No dates match this filter.\nPress Esc to clear it, or / to search again.",
            )
            .block(frame_block("Daily bookings"))
            .wrap(Wrap { trim: true }),
            ledger,
        );
    } else {
        let table = Table::new(
            rows,
            [
                Constraint::Length(if area.width < 75 { 11 } else { 18 }),
                Constraint::Length(9),
                Constraint::Min(7),
            ],
        )
        .header(
            Row::new(["DATE", "BOOKED", "STATE"])
                .style(Style::default().fg(MUTED))
                .bottom_margin(1),
        )
        .block(frame_block(&name))
        .row_highlight_style(
            Style::default()
                .bg(SELECT)
                .fg(LIGHT_RED)
                .add_modifier(Modifier::BOLD),
        );
        let mut state = TableState::default().with_selected(if dates.is_empty() {
            None
        } else {
            Some(app.day.min(dates.len() - 1))
        });
        frame.render_stateful_widget(table, ledger, &mut state);
    }
    if cols.len() == 3 {
        detail(
            frame,
            app,
            dates.get(app.day).map(String::as_str),
            cols[2],
            0,
        );
    }
    if app.details {
        frame.render_widget(Clear, parts[1]);
        frame.render_widget(
            Block::default().style(Style::default().bg(BG).fg(INK)),
            parts[1],
        );
        let max = detail(
            frame,
            app,
            dates.get(app.day).map(String::as_str),
            parts[1],
            app.detail_scroll,
        );
        app.detail_scroll = app.detail_scroll.min(max);
    }
    if app.error_detail
        && let Some(error) = &app.error
    {
        frame.render_widget(Clear, parts[1]);
        frame.render_widget(
            Paragraph::new(format!("{}\n\n! / Esc back · r refresh", clean(error)))
                .style(Style::default().bg(BG).fg(AMBER))
                .block(frame_block("Review needed"))
                .wrap(Wrap { trim: true }),
            parts[1],
        );
    }
    let status = if app.error.is_some() {
        Line::from(Span::styled(
            "Attention · ! details · r refresh",
            Style::default().fg(AMBER),
        ))
    } else if app.searching {
        Line::from(format!("Find date: {}_   Esc cancel", app.query))
    } else if let Some(notice) = &app.notice {
        Line::from(Span::styled(notice, Style::default().fg(LIGHT_RED)))
    } else if !app.query.is_empty() {
        Line::from(format!("Date filter: {} · Esc clears", app.query))
    } else {
        Line::from(Span::styled(
            if app.details {
                "Enter returns to ledger · e edit booking"
            } else {
                "e edit booking · preview before saving"
            },
            Style::default().fg(MUTED),
        ))
    };
    frame.render_widget(
        Paragraph::new(vec![
            status,
            Line::from(Span::styled(
                if app.details {"↑↓ scroll  Enter back  ? help  q quit"}
                else if area.width < 100 {
                    "↑↓ days  Enter detail  ? help  q quit"
                } else {
                    "↑↓ days  Tab activity  Enter detail  [ ] month  / find  r refresh  ? help  q quit"
                },
                Style::default().fg(LIGHT_RED),
            )),
        ]),
        parts[2],
    );
    if app.help {
        let popup = Rect::new(
            area.x + 1,
            area.y + 1,
            area.width.saturating_sub(2).min(80),
            area.height.saturating_sub(2).min(16),
        );
        frame.render_widget(Clear, popup);
        frame.render_widget(Paragraph::new("? / Esc     Close help\n↑↓ / j k    Select a day\nPgUp/PgDn   Move ten days\nHome / End  First / last day\nEnter       Detail / ledger\nTab / S-Tab Next / prev activity\n[ / ]       Previous / next month\n/           Find date; Enter keeps\nEsc         Clear date filter\nr           Refresh timesheet\ne           Edit selected booking\nq / Ctrl-C  Quit").block(Block::bordered().title(" Keyboard guide ")).style(Style::default().fg(INK).bg(BG)),popup);
    }
    draw_editor(frame, app);
}
fn draw_editor(frame: &mut Frame, app: &Workspace) {
    let area = frame.area();
    if area.width < 40 || area.height < 16 {
        return;
    }
    let Some(editor) = &app.editor else {
        if app.saving {
            let area = Rect::new(
                area.x + 1,
                area.y + 1,
                area.width.saturating_sub(2),
                area.height.saturating_sub(2),
            );
            frame.render_widget(Clear, area);
            frame.render_widget(
                Paragraph::new(
                    "Saving booking…\nReading back to verify.\n\nPlease wait before quitting.",
                )
                .block(Block::bordered().title(" Save booking "))
                .style(Style::default().bg(BG).fg(INK)),
                area,
            );
        }
        return;
    };
    let width = area.width.saturating_sub(2).min(70);
    let height = area.height.saturating_sub(2).min(16);
    let area = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .title(if editor.ready {
            " Review booking "
        } else {
            " Edit booking "
        })
        .style(Style::default().bg(BG).fg(INK));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(3),
    ])
    .margin(1)
    .split(inner);
    let heading = Layout::vertical([Constraint::Length(2), Constraint::Length(1)]).split(rows[0]);
    frame.render_widget(
        Paragraph::new(clean(&editor.edit.activity_name)).wrap(Wrap { trim: true }),
        heading[0],
    );
    frame.render_widget(Paragraph::new(editor.edit.date.as_str()), heading[1]);
    let body = if editor.ready {
        format!(
            "{} → {}\n{}",
            duration(i64::from(editor.edit.before_minutes)),
            duration(i64::from(editor.edit.minutes)),
            if editor.edit.before_minutes == editor.edit.minutes {
                "Already matches · no change needed"
            } else if editor.edit.minutes == 0 {
                "Clear this booking"
            } else {
                "Replace this day's booked total"
            }
        )
    } else {
        format!(
            "Duration: {}_\n7h30 / 7:30 / 450m · 0m clears",
            editor.input
        )
    };
    frame.render_widget(
        Paragraph::new(body)
            .style(Style::default().fg(LIGHT_RED))
            .wrap(Wrap { trim: true }),
        rows[1],
    );
    let footer = if let Some(error) = &editor.error {
        format!("{error}\nEsc cancel")
    } else if editor.ready {
        format!(
            "{}\ns save  Enter edit  Esc cancel",
            if app.demo {
                "DEMO · sample data only"
            } else {
                "Save to Yoobi and verify"
            }
        )
    } else {
        "Enter preview  Esc cancel\nCtrl-U clear input".into()
    };
    frame.render_widget(
        Paragraph::new(footer)
            .wrap(Wrap { trim: true })
            .style(Style::default().fg(if editor.error.is_some() { AMBER } else { MUTED })),
        rows[2],
    );
}

fn detail(frame: &mut Frame, app: &Workspace, date: Option<&str>, area: Rect, scroll: u16) -> u16 {
    let mut lines = vec![];
    if let Some(date) = date {
        lines.push(Line::from(Span::styled(
            date,
            Style::default().fg(LIGHT_RED).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::raw(""));
        let booked = app
            .project()
            .and_then(|p| p.days.get(date))
            .copied()
            .unwrap_or(0);
        lines.push(Line::raw(format!(
            "Booked       {}",
            duration(i64::from(booked))
        )));
        lines.push(Line::raw(
            if app.project().is_some_and(|p| p.locked_days.contains(date)) {
                "State        locked"
            } else {
                "State        open"
            },
        ));
    }
    let line_count = lines
        .iter()
        .map(|line| (line.width() as u16).max(1).div_ceil(area.width.max(1)))
        .sum::<u16>();
    let max_scroll = line_count.saturating_sub(area.height.saturating_sub(1));
    frame.render_widget(
        Paragraph::new(lines)
            .block(frame_block("Day detail"))
            .wrap(Wrap { trim: true })
            .scroll((scroll.min(max_scroll), 0)),
        area,
    );
    max_scroll
}
pub fn demo(month: &str) -> Timesheet {
    let mut days = BTreeMap::new();
    let mut locks = BTreeSet::new();
    for d in 1..=days_in_month(month).unwrap_or(30) {
        let date = format!("{month}-{d:02}");
        let wd = NaiveDate::parse_from_str(&date, "%Y-%m-%d")
            .unwrap()
            .weekday()
            .number_from_monday();
        let minutes = if wd <= 5 && d <= 16 {
            [450, 390, 480][d as usize % 3]
        } else {
            0
        };
        if d <= 6 {
            locks.insert(date.clone());
        }
        days.insert(date, minutes);
    }
    let total_minutes = days.values().sum();
    Timesheet {
        base_url: "https://demo.invalid".into(),
        employee_id: "demo".into(),
        month: month.into(),
        total_minutes,
        projects: vec![
            Project {
                id: "demo-delivery".into(),
                name: "Consultancy / Delivery".into(),
                days,
                locked_days: locks,
            },
            Project {
                id: "demo-internal".into(),
                name: "Internal / Development".into(),
                days: (1..=days_in_month(month).unwrap())
                    .map(|d| (format!("{month}-{d:02}"), 0))
                    .collect(),
                locked_days: BTreeSet::new(),
            },
        ],
    }
}
pub fn snapshot(options: &Options) -> Result<String, Error> {
    let mut app = Workspace::new(options);
    app.accept(demo(&options.month));
    app.day = 10;
    let backend = TestBackend::new(options.width, options.height);
    let mut terminal = Terminal::new(backend).expect("test backend is infallible");
    terminal
        .draw(|f| draw(f, &mut app))
        .expect("test backend is infallible");
    let buffer = terminal.backend().buffer();
    Ok((0..options.height)
        .map(|y| {
            (0..options.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n"))
}
struct Restore;
impl Drop for Restore {
    fn drop(&mut self) {
        ratatui::restore();
    }
}
pub fn run(options: Options) -> Result<(), Error> {
    let config = if options.demo {
        None
    } else {
        Some(Config::load()?)
    };
    let mut app = Workspace::new(&options);
    let (request, requests) = mpsc::channel::<Request>();
    let (results, responses) = mpsc::channel();
    if let Some(config) = config {
        std::thread::spawn(move || {
            let mut pending = None;
            while let Some(job) = pending.take().or_else(|| requests.recv().ok()) {
                let update = match job {
                    Request::Read(mut month) => {
                        while let Ok(next) = requests.try_recv() {
                            match next {
                                Request::Read(next_month) => month = next_month,
                                save @ Request::Save(_) => {
                                    pending = Some(save);
                                    break;
                                }
                            }
                        }
                        let result = Client::new(&config).and_then(|c| c.timesheet(&month));
                        Update::Read(month, result)
                    }
                    Request::Save(edit) => {
                        Update::Save(Client::new(&config).and_then(|c| c.save_booking(&edit)))
                    }
                };
                if results.send(update).is_err() {
                    break;
                }
            }
        });
        request.send(Request::Read(app.month.clone())).ok();
    } else {
        app.accept(demo(&app.month));
        app.day = 10;
    }
    let _restore = Restore;
    let mut terminal = ratatui::try_init().map_err(io_error)?;
    loop {
        loop {
            let update = match responses.try_recv() {
                Ok(update) => update,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if app.saving {
                        app.saved(Err(Error::SaveUncertain {
                            message: "booking worker stopped before verification".into(),
                        }));
                    } else if app.loading {
                        app.loading = false;
                        app.error = Some("timesheet worker stopped; restart the TUI".into());
                    }
                    break;
                }
            };
            match update {
                Update::Save(result) => app.saved(result),
                Update::Read(month, result) if month == app.month => match result {
                    Ok(sheet) => app.accept(sheet),
                    Err(e) => {
                        app.loading = false;
                        app.error = Some(e.to_string());
                    }
                },
                _ => {}
            }
        }
        terminal.draw(|f| draw(f, &mut app)).map_err(io_error)?;
        if !event::poll(Duration::from_millis(100)).map_err(io_error)? {
            continue;
        }
        let Event::Key(key) = event::read().map_err(io_error)? else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        if app.saving {
            continue;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            break;
        }
        if app.editor.is_some() {
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('u') {
                if let Some(editor) = &mut app.editor {
                    editor.input.clear();
                    editor.ready = false;
                    editor.error = None;
                }
            } else if let Some(edit) = app.edit_key(key.code) {
                if app.demo {
                    let mut sheet = app.sheet.clone();
                    if let Some(activity) =
                        sheet.projects.iter_mut().find(|p| p.id == edit.activity_id)
                    {
                        activity.days.insert(edit.date.clone(), edit.minutes);
                        sheet.total_minutes =
                            sheet.projects.iter().flat_map(|p| p.days.values()).sum();
                        app.saved(Ok(BookingResult {
                            verified: true,
                            changed: edit.before_minutes != edit.minutes,
                            booking: edit,
                            timesheet: sheet,
                        }));
                    }
                } else {
                    app.saving = true;
                    app.notice = None;
                    if request.send(Request::Save(edit)).is_err() {
                        app.saved(Err(Error::Io {
                            message: "booking worker is unavailable".into(),
                        }));
                    }
                }
            }
            continue;
        }
        if app.searching {
            match key.code {
                KeyCode::Esc => {
                    app.query.clear();
                    app.searching = false;
                }
                KeyCode::Enter => app.searching = false,
                KeyCode::Backspace => {
                    app.query.pop();
                }
                KeyCode::Char(c) if app.query.len() < 20 && !c.is_control() => app.query.push(c),
                _ => {}
            }
            app.day = 0;
            continue;
        }
        if app.help {
            if key.code == KeyCode::Char('q') {
                break;
            }
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('?')) {
                app.help = false;
            }
            continue;
        }
        match key.code {
            KeyCode::Char('q') => break,
            KeyCode::Char('e') => app.begin_edit(),
            KeyCode::Enter => {
                app.details = !app.details;
                app.detail_scroll = 0;
            }
            KeyCode::Char('!') if app.error.is_some() => app.error_detail = !app.error_detail,
            KeyCode::Down | KeyCode::Char('j') if app.details => {
                app.detail_scroll = app.detail_scroll.saturating_add(1)
            }
            KeyCode::Up | KeyCode::Char('k') if app.details => {
                app.detail_scroll = app.detail_scroll.saturating_sub(1)
            }
            KeyCode::PageDown if app.details => {
                app.detail_scroll = app.detail_scroll.saturating_add(5)
            }
            KeyCode::PageUp if app.details => {
                app.detail_scroll = app.detail_scroll.saturating_sub(5)
            }
            KeyCode::Home if app.details => app.detail_scroll = 0,
            KeyCode::End if app.details => app.detail_scroll = u16::MAX,
            KeyCode::Down | KeyCode::Char('j') => app.move_day(1),
            KeyCode::Up | KeyCode::Char('k') => app.move_day(-1),
            KeyCode::PageDown => app.move_day(10),
            KeyCode::PageUp => app.move_day(-10),
            KeyCode::Home => app.day = 0,
            KeyCode::End => app.day = app.dates().len().saturating_sub(1),
            KeyCode::Tab => {
                if !app.sheet.projects.is_empty() {
                    app.activity = (app.activity + 1) % app.sheet.projects.len();
                }
            }
            KeyCode::BackTab => {
                app.activity = app
                    .activity
                    .checked_sub(1)
                    .unwrap_or(app.sheet.projects.len().saturating_sub(1))
            }
            KeyCode::Char('/') => {
                app.notice = None;
                app.searching = true;
                app.query.clear();
                app.day = 0;
            }
            KeyCode::Esc => {
                app.query.clear();
                app.details = false;
                app.error_detail = false;
                app.day = 0;
            }
            KeyCode::Char('?') => app.help = true,
            KeyCode::Char('[') | KeyCode::Char(']') | KeyCode::Char('r') => {
                if key.code != KeyCode::Char('r') {
                    app.month = shift_month(
                        &app.month,
                        if key.code == KeyCode::Char('[') {
                            -1
                        } else {
                            1
                        },
                    );
                    app.day = 0;
                    app.query.clear();
                    app.sheet.projects.clear();
                    app.sheet.total_minutes = 0;
                }
                app.error = None;
                app.notice = None;
                if app.demo {
                    app.accept(demo(&app.month));
                } else {
                    app.loading = true;
                    request.send(Request::Read(app.month.clone())).ok();
                }
            }
            _ => {}
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn opts() -> Options {
        Options {
            month: "2026-11".into(),
            demo: true,
            snapshot: true,
            width: 120,
            height: 36,
        }
    }
    #[test]
    fn calendar_month_boundaries() {
        assert_eq!(shift_month("2026-12", 1), "2027-01");
        assert_eq!(shift_month("2026-01", -1), "2025-12");
        assert_eq!(shift_month("0001-01", -1), "0001-01");
    }
    #[test]
    fn day_selection_and_filter_are_bounded() {
        let mut a = Workspace::new(&opts());
        a.move_day(1000);
        assert_eq!(a.day, 29);
        a.move_day(-1000);
        assert_eq!(a.day, 0);
        a.query = "-11-11".into();
        assert_eq!(a.dates(), ["2026-11-11"]);
        a.query = "never".into();
        a.move_day(1);
        assert!(a.dates().is_empty());
        assert_eq!(a.day, 0);
    }
    #[test]
    fn snapshots_at_all_supported_layouts() {
        for (width, height) in [(40, 16), (74, 24), (80, 28), (120, 36), (240, 80)] {
            let mut o = opts();
            o.width = width;
            o.height = height;
            let s = snapshot(&o).unwrap();
            assert!(s.contains("YOOBI"));
            assert!(s.contains("q quit"));
            assert_eq!(s.split('\n').count(), usize::from(height));
            assert!(!s.contains('\x1b'));
            if let Ok(dir) = std::env::var("YOOBI_VISUAL_QA") {
                let mut app = Workspace::new(&o);
                app.accept(demo(&o.month));
                app.day = 10;
                let mut term = Terminal::new(TestBackend::new(width, height)).unwrap();
                term.draw(|f| draw(f, &mut app)).unwrap();
                let cells:Vec<_>=(0..height).flat_map(|y|(0..width).map(move |x|(x,y))).map(|(x,y)| {let c=&term.backend().buffer()[(x,y)];serde_json::json!({"x":x,"y":y,"text":c.symbol(),"fg":format!("{:?}",c.fg),"bg":format!("{:?}",c.bg)})}).collect();
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(
                    format!("{dir}/{width}x{height}.json"),
                    serde_json::to_string(&cells).unwrap(),
                )
                .unwrap();
            }
        }
    }
    fn state_screen(app: &mut Workspace, name: &str) -> String {
        state_screen_at(app, name, 40, 16)
    }
    fn state_screen_at(app: &mut Workspace, name: &str, width: u16, height: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(width, height)).unwrap();
        term.draw(|f| draw(f, app)).unwrap();
        let b = term.backend().buffer();
        if let Ok(dir) = std::env::var("YOOBI_VISUAL_QA") {
            let cells:Vec<_>=(0..height).flat_map(|y|(0..width).map(move |x|(x,y))).map(|(x,y)| {let c=&b[(x,y)];serde_json::json!({"x":x,"y":y,"text":c.symbol(),"fg":format!("{:?}",c.fg),"bg":format!("{:?}",c.bg)})}).collect();
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                format!("{dir}/{name}.json"),
                serde_json::to_string(&cells).unwrap(),
            )
            .unwrap();
        }
        (0..height)
            .map(|y| (0..width).map(|x| b[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }
    #[test]
    fn smallest_help_filter_detail_and_error_states_are_complete() {
        let mut app = Workspace::new(&opts());
        app.accept(demo("2026-11"));
        app.day = 10;
        app.help = true;
        let s = state_screen(&mut app, "help-40x16");
        assert!(s.contains("q / Ctrl-C"));
        assert!(s.contains("Refresh timesheet"));
        assert!(s.contains("Close help"));
        app.help = false;
        app.query = "never".into();
        let s = state_screen(&mut app, "filter-40x16");
        assert!(s.contains("No dates match"));
        assert!(s.contains("Date filter: never"));
        app.query.clear();
        app.details = true;
        let s = state_screen(&mut app, "detail-top-40x16");
        assert!(s.contains("Booked"));
        assert!(s.contains("8h00"));
        assert!(!s.contains("Actual"));
        app.detail_scroll = u16::MAX;
        let s = state_screen(&mut app, "detail-bottom-40x16");
        assert!(s.contains("State"));
        assert!(app.detail_scroll < 20);
        app.details = false;
        app.error = Some("Yoobi rejected login. Retry with the correct credentials.".into());
        app.error_detail = true;
        let s = state_screen(&mut app, "error-40x16");
        assert!(s.contains("correct credentials"));
        assert!(s.contains("r refresh"));
        assert!(!s.contains("Connected"));
    }
    #[test]
    fn editor_requires_preview_and_an_explicit_save_key() {
        let mut app = Workspace::new(&opts());
        app.accept(demo("2026-11"));
        app.day = 10;
        app.begin_edit();
        assert!(app.editor.is_some());
        assert!(app.edit_key(KeyCode::Char('s')).is_none());
        app.editor.as_mut().unwrap().input = "7h17".into();
        assert!(app.edit_key(KeyCode::Enter).is_none());
        assert!(app.editor.as_ref().unwrap().ready);
        let screen = state_screen(&mut app, "booking-preview-40x16");
        assert!(screen.contains("7h17"));
        assert!(screen.contains("s save"));
        assert!(screen.contains("DEMO"));
        let screen = state_screen_at(&mut app, "booking-preview-120x36", 120, 36);
        assert!(screen.contains("s save"));
        assert!(screen.contains("2026-11-11"));
        let edit = app.edit_key(KeyCode::Char('s')).unwrap();
        assert_eq!(edit.minutes, 437);
        assert!(app.editor.is_none());
    }
    #[test]
    fn editor_errors_and_cancellation_preserve_the_booking() {
        let mut app = Workspace::new(&opts());
        app.accept(demo("2026-11"));
        app.day = 10;
        let original = app.sheet.clone();
        app.begin_edit();
        app.editor.as_mut().unwrap().input = "25h".into();
        app.edit_key(KeyCode::Enter);
        assert!(!app.editor.as_ref().unwrap().ready);
        let screen = state_screen(&mut app, "booking-error-40x16");
        assert!(screen.contains("max 24h"));
        assert!(screen.contains("Esc cancel"));
        app.edit_key(KeyCode::Esc);
        assert!(app.editor.is_none());
        assert_eq!(app.sheet, original);
        app.day = 1;
        app.begin_edit();
        assert!(app.editor.is_none());
        assert!(app.notice.as_ref().unwrap().contains("locked"));
    }
    #[test]
    fn failed_saves_require_a_fresh_read_before_another_edit() {
        let mut app = Workspace::new(&opts());
        app.accept(demo("2026-11"));
        app.day = 10;
        app.saving = true;
        let screen = state_screen(&mut app, "booking-saving-40x16");
        assert!(screen.contains("Please wait"));
        app.saved(Err(Error::SaveUncertain {
            message: "Could not verify".into(),
        }));
        assert!(!app.saving);
        app.begin_edit();
        assert!(app.editor.is_none());
        app.accept(demo("2026-11"));
        app.begin_edit();
        assert!(app.editor.is_some());
    }
}
