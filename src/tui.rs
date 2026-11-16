//! Read-only terminal workspace. The save API is intentionally absent here.
use crate::{
    Error,
    api::Client,
    config::Config,
    output::{clean, duration},
    plan::{Plan, Project, Timesheet, days_in_month, validate_month},
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
const BG: Color = Color::Rgb(15, 22, 30);
const INK: Color = Color::Rgb(230, 233, 226);
const MUTED: Color = Color::Rgb(150, 166, 179);
const TEAL: Color = Color::Rgb(111, 220, 194);
const AMBER: Color = Color::Rgb(245, 196, 112);
const SELECT: Color = Color::Rgb(29, 55, 62);
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
    pub plan: Option<Plan>,
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
            message: "tui does not support JSON; use timesheet, balance or plan".into(),
        });
    }
    if options.demo && options.plan.is_some() {
        return Err(Error::Usage {
            message: "--demo cannot be combined with a real booking plan".into(),
        });
    }
    if let Some(p) = &options.plan {
        p.validate()?;
        if p.month != options.month {
            return Err(Error::InvalidInput {
                message: "TUI month must match the preview plan".into(),
            });
        }
    }
    if !options.snapshot && (!std::io::stdin().is_terminal() || !std::io::stdout().is_terminal()) {
        return Err(Error::TtyRequired {
            message: "tui requires terminal stdin and stdout".into(),
        });
    }
    Ok(())
}
struct Workspace {
    month: String,
    sheet: Timesheet,
    plan: Option<Plan>,
    activity: usize,
    day: usize,
    query: String,
    searching: bool,
    help: bool,
    details: bool,
    detail_scroll: u16,
    error_detail: bool,
    loading: bool,
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
            plan: options.plan.clone(),
            activity: 0,
            day: 0,
            query: String::new(),
            searching: false,
            help: false,
            details: false,
            detail_scroll: 0,
            error_detail: false,
            loading: !options.demo,
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
    fn planned(&self, date: &str) -> Option<&crate::plan::PlannedDay> {
        self.plan
            .as_ref()
            .filter(|p| self.project().is_some_and(|a| a.id == p.project_id))
            .and_then(|p| p.days.iter().find(|d| d.date == date))
    }
    fn accept(&mut self, sheet: Timesheet) {
        if let Some(p) = &self.plan {
            if p.employee_id != sheet.employee_id || p.base_url != sheet.base_url {
                self.error = Some(
                    "Plan belongs to another account. Reopen with the correct credentials.".into(),
                );
                self.loading = false;
                self.plan = None;
                return;
            }
            let Some(index) = sheet.projects.iter().position(|a| a.id == p.project_id) else {
                self.error = Some("Plan activity no longer exists. Generate a new plan.".into());
                self.loading = false;
                self.plan = None;
                self.sheet = sheet;
                return;
            };
            self.activity = index;
        }
        self.sheet = sheet;
        self.activity = self
            .activity
            .min(self.sheet.projects.len().saturating_sub(1));
        self.loading = false;
        self.error = None;
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
        Constraint::Length(if app.plan.is_some() { 4 } else { 3 }),
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
        "Attention · e shows details"
    } else {
        "Connected · read only"
    };
    let title = Line::from(vec![
        Span::styled(
            "YOOBI",
            Style::default().fg(TEAL).add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "   {}   /   {} booked",
            app.month,
            duration(i64::from(app.sheet.total_minutes))
        )),
    ]);
    let mut header = vec![title, Line::styled(mode, Style::default().fg(MUTED))];
    if let Some(plan) = &app.plan
        && let Ok(b) = plan.balance()
    {
        header.push(Line::styled(
            format!(
                "Plan balance  {} {}",
                crate::output::magnitude(b.balance_minutes),
                if b.balance_minutes < 0 {
                    "debt"
                } else {
                    "surplus"
                }
            ),
            Style::default().fg(TEAL),
        ));
    }
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
                    Style::default().bg(SELECT).fg(TEAL)
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
            let plan = app.planned(date);
            let changed = plan.is_some_and(|p| u32::from(p.hours) * 60 != booked);
            let weekday = NaiveDate::parse_from_str(date, "%Y-%m-%d")
                .map(|d| d.format("%a").to_string())
                .unwrap_or_default();
            let date_label = if area.width < 75 {
                format!("{} {weekday}", &date[8..])
            } else {
                format!("{} {weekday}", &date[5..])
            };
            let planned = plan
                .map(|p| duration(i64::from(p.hours) * 60))
                .unwrap_or_else(|| "—".into());
            let state = if locked {
                "locked"
            } else if changed {
                "change"
            } else if plan.is_some() {
                "same"
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
                Cell::from(planned),
                Cell::from(state),
            ])
            .style(Style::default().fg(if changed {
                AMBER
            } else if booked == 0 {
                MUTED
            } else {
                INK
            }))
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
                Constraint::Length(7),
                Constraint::Length(7),
                Constraint::Min(7),
            ],
        )
        .header(
            Row::new(["DATE", "BOOKED", "PLAN", "STATE"])
                .style(Style::default().fg(MUTED))
                .bottom_margin(1),
        )
        .block(frame_block(&name))
        .row_highlight_style(
            Style::default()
                .bg(SELECT)
                .fg(TEAL)
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
            Paragraph::new(format!("{}\n\ne / Esc back · r retry", clean(error)))
                .style(Style::default().bg(BG).fg(AMBER))
                .block(frame_block("Review needed"))
                .wrap(Wrap { trim: true }),
            parts[1],
        );
    }
    let status = if app.error.is_some() {
        Line::from(Span::styled(
            "Attention · e details · r retry",
            Style::default().fg(AMBER),
        ))
    } else if app.searching {
        Line::from(format!("Find date: {}_   Esc cancel", app.query))
    } else if !app.query.is_empty() {
        Line::from(format!("Date filter: {} · Esc clears", app.query))
    } else {
        Line::from(Span::styled(
            if app.details && app.plan.is_some() {
                "Plan dates only · Enter returns"
            } else if app.details {
                "Enter returns to ledger · no writes"
            } else if app.plan.is_some() {
                "Plan dates only · no writes"
            } else {
                "Read only · bookings are never changed here"
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
                Style::default().fg(TEAL),
            )),
        ]),
        parts[2],
    );
    if app.help {
        let popup = Rect::new(
            area.x + 1,
            area.y + 1,
            area.width.saturating_sub(2).min(80),
            area.height.saturating_sub(2).min(15),
        );
        frame.render_widget(Clear, popup);
        frame.render_widget(Paragraph::new("? / Esc     Close help\n↑↓ / j k    Select a day\nPgUp/PgDn   Move ten days\nHome / End  First / last day\nEnter       Detail / ledger\nTab / S-Tab Next / prev activity\n[ / ]       Previous / next month\n/           Find date; Enter keeps\nEsc         Clear date filter\nr           Refresh timesheet\nq / Ctrl-C  Quit\nRead only. Apply bookings with CLI.").block(Block::bordered().title(" Keyboard guide ")).style(Style::default().fg(INK).bg(BG)),popup);
    }
}
fn detail(frame: &mut Frame, app: &Workspace, date: Option<&str>, area: Rect, scroll: u16) -> u16 {
    let mut lines = vec![];
    if let Some(date) = date {
        lines.push(Line::from(Span::styled(
            date,
            Style::default().fg(TEAL).add_modifier(Modifier::BOLD),
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
        if let Some(p) = app.planned(date) {
            lines.push(Line::raw(format!(
                "Actual       {}",
                duration(i64::from(p.actual_minutes))
            )));
            lines.push(Line::styled(
                format!("Planned      {}", duration(i64::from(p.hours) * 60)),
                Style::default().fg(AMBER),
            ));
            if p.before_minutes != booked && u32::from(p.hours) * 60 != booked {
                lines.push(Line::styled(
                    "STALE · regenerate plan",
                    Style::default().fg(AMBER),
                ));
            }
        }
        lines.push(Line::raw(
            if app.project().is_some_and(|p| p.locked_days.contains(date)) {
                "State        locked"
            } else {
                "State        open"
            },
        ));
    }
    if let Some(plan) = &app.plan {
        lines.push(Line::raw(""));
        lines.push(Line::styled("Projected balance", Style::default().fg(TEAL)));
        if let Ok(b) = plan.balance() {
            lines.push(Line::raw(format!(
                "{} {}",
                crate::output::magnitude(b.balance_minutes),
                if b.balance_minutes < 0 {
                    "debt"
                } else {
                    "surplus"
                }
            )));
            lines.push(Line::raw(format!(
                "Actual       {}",
                duration(b.actual_minutes)
            )));
            lines.push(Line::raw(format!(
                "Planned      {}",
                duration(b.booked_minutes)
            )));
            lines.push(Line::raw(format!(
                "Opening      {}",
                duration(b.opening_balance_minutes)
            )));
        }
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            "Includes plan dates only.",
            Style::default().fg(MUTED),
        ));
    } else {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            "Compare with actual work",
            Style::default().fg(TEAL),
        ));
        lines.push(Line::raw(
            "Open with --plan FILE to see actual hours, proposed bookings and balance.",
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
            if d == 11 { 240 } else { 480 }
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
fn demo_plan(sheet: &Timesheet) -> Plan {
    Plan {
        version: 1,
        base_url: sheet.base_url.clone(),
        employee_id: sheet.employee_id.clone(),
        month: sheet.month.clone(),
        project_id: "demo-delivery".into(),
        opening_balance_minutes: 450,
        days: [(11, 345), (12, 480)]
            .into_iter()
            .map(|(d, actual)| {
                let date = format!("{}-{d:02}", sheet.month);
                let before = sheet.projects[0].days[&date];
                crate::plan::PlannedDay {
                    date,
                    actual_minutes: actual,
                    before_minutes: before,
                    hours: 8,
                }
            })
            .collect(),
    }
}
pub fn snapshot(options: &Options) -> Result<String, Error> {
    let mut app = Workspace::new(options);
    app.accept(demo(&options.month));
    app.plan = Some(demo_plan(&app.sheet));
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
    let (request, requests) = mpsc::channel::<String>();
    let (results, responses) = mpsc::channel();
    if let Some(config) = config {
        std::thread::spawn(move || {
            while let Ok(mut month) = requests.recv() {
                while let Ok(new) = requests.try_recv() {
                    month = new;
                }
                let result = Client::new(&config).and_then(|c| c.timesheet(&month));
                if results.send((month, result)).is_err() {
                    break;
                }
            }
        });
        request.send(app.month.clone()).ok();
    } else {
        app.accept(demo(&app.month));
        app.plan = Some(demo_plan(&app.sheet));
        app.day = 10;
    }
    let _restore = Restore;
    let mut terminal = ratatui::try_init().map_err(io_error)?;
    loop {
        while let Ok((month, result)) = responses.try_recv() {
            if month == app.month {
                match result {
                    Ok(sheet) => app.accept(sheet),
                    Err(e) => {
                        app.loading = false;
                        app.error = Some(e.to_string());
                    }
                }
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
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            break;
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
            KeyCode::Enter => {
                app.details = !app.details;
                app.detail_scroll = 0;
            }
            KeyCode::Char('e') if app.error.is_some() => app.error_detail = !app.error_detail,
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
                    if app.plan.is_some() && !app.demo {
                        app.error = Some(
                            "Month is fixed to the preview plan. Reopen without --plan to browse."
                                .into(),
                        );
                        continue;
                    }
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
                if app.demo {
                    app.accept(demo(&app.month));
                    app.plan = Some(demo_plan(&app.sheet));
                } else {
                    app.loading = true;
                    request.send(app.month.clone()).ok();
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
            plan: None,
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
                app.plan = Some(demo_plan(&app.sheet));
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
        let mut term = Terminal::new(TestBackend::new(40, 16)).unwrap();
        term.draw(|f| draw(f, app)).unwrap();
        let b = term.backend().buffer();
        if let Ok(dir) = std::env::var("YOOBI_VISUAL_QA") {
            let cells:Vec<_>=(0..16).flat_map(|y|(0..40).map(move |x|(x,y))).map(|(x,y)| {let c=&b[(x,y)];serde_json::json!({"x":x,"y":y,"text":c.symbol(),"fg":format!("{:?}",c.fg),"bg":format!("{:?}",c.bg)})}).collect();
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                format!("{dir}/{name}.json"),
                serde_json::to_string(&cells).unwrap(),
            )
            .unwrap();
        }
        (0..16)
            .map(|y| (0..40).map(|x| b[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }
    #[test]
    fn smallest_help_filter_detail_and_error_states_are_complete() {
        let mut app = Workspace::new(&opts());
        app.accept(demo("2026-11"));
        app.plan = Some(demo_plan(&app.sheet));
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
        assert!(s.contains("Actual"));
        assert!(s.contains("5h45"));
        assert!(s.contains("Plan dates only"));
        app.detail_scroll = u16::MAX;
        let s = state_screen(&mut app, "detail-bottom-40x16");
        assert!(s.contains("Opening"));
        assert!(s.contains("Includes plan dates only"));
        assert!(app.detail_scroll < 20);
        app.details = false;
        app.error =
            Some("Plan belongs to another account. Reopen with the correct credentials.".into());
        app.error_detail = true;
        let s = state_screen(&mut app, "error-40x16");
        assert!(s.contains("correct credentials"));
        assert!(s.contains("r retry"));
        assert!(!s.contains("Connected"));
    }
    #[test]
    fn missing_plan_activity_is_reported_without_a_misleading_balance() {
        let sheet = demo("2026-11");
        let mut plan = demo_plan(&sheet);
        plan.project_id = "removed".into();
        let mut app = Workspace::new(&opts());
        app.plan = Some(plan);
        app.accept(sheet);
        assert!(app.error.is_some());
        assert!(app.plan.is_none());
    }
    #[test]
    fn account_mismatch_never_exposes_plan() {
        let mut a = Workspace::new(&opts());
        a.plan = Some(Plan {
            version: 1,
            base_url: "https://other.invalid".into(),
            employee_id: "other".into(),
            month: "2026-11".into(),
            project_id: "demo-delivery".into(),
            opening_balance_minutes: 0,
            days: vec![],
        });
        a.accept(demo("2026-11"));
        assert!(a.error.is_some());
        assert!(a.sheet.projects.is_empty());
    }
}
