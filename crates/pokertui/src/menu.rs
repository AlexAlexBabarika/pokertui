use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph};

use crate::settings::{Cycle, Settings};

/// Number of editable rows in the menu.
pub const ROWS: usize = 4;

/// What the App should do after a key is handled: keep the menu open, or close
/// it (committing the draft).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuOutcome {
    Open,
    Close,
}

pub struct SettingsMenu {
    /// Focused row, `0..ROWS`.
    selected: usize,
    /// Working copy of the settings, committed by the App on close.
    draft: Settings,
}

impl From<Settings> for SettingsMenu {
    fn from(draft: Settings) -> Self {
        Self { selected: 0, draft }
    }
}

impl SettingsMenu {
    /// The working copy, read by the App to commit on close.
    pub fn draft(&self) -> &Settings {
        &self.draft
    }

    /// The focused row index.
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Handle a key press. `Up`/`Down` move the focus (clamped), `Left`/`Right`
    /// cycle the focused row's value, `Esc` closes; anything else is ignored.
    pub fn handle_key(&mut self, key: KeyCode) -> MenuOutcome {
        match key {
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down => self.selected = (self.selected + 1).min(ROWS - 1),
            KeyCode::Left => self.cycle_selected(Cycle::Prev),
            KeyCode::Right => self.cycle_selected(Cycle::Next),
            KeyCode::Esc => return MenuOutcome::Close,
            _ => {}
        }
        MenuOutcome::Open
    }

    /// Cycle the focused row's value in `dir`.
    fn cycle_selected(&mut self, dir: Cycle) {
        match self.selected {
            0 => self.draft.cycle_bot_delay(dir),
            1 => self.draft.cycle_show_win_rate(dir),
            2 => self.draft.cycle_seats(dir),
            3 => self.draft.cycle_blinds(dir),
            _ => {}
        }
    }

    /// `(label, current value)` for each row, in display order.
    fn rows(&self) -> [(&'static str, String); ROWS] {
        [
            ("Bot delay", format!("{} ms", self.draft.bot_delay.as_millis())),
            (
                "Show win rate",
                if self.draft.show_win_rate { "on" } else { "off" }.to_string(),
            ),
            ("Seats", self.draft.seats.to_string()),
            (
                "Blinds",
                format!("{} / {}", self.draft.small_blind, self.draft.big_blind),
            ),
        ]
    }
}

const LIME: Color = Color::Rgb(0x82, 0xcc, 0x16);
const AMBER: Color = Color::Rgb(0xf5, 0x9e, 0x0b);
const MUTED: Color = Color::Rgb(0xa1, 0xa1, 0xaa);
const DIM: Color = Color::Rgb(0x57, 0x57, 0x66);
const BORDER: Color = Color::Rgb(0x3a, 0x3a, 0x44);

/// Draw the full-screen settings overlay: one row per setting, the focused row
/// highlighted, a note that seats/blinds apply next game, and a key-hint footer.
pub fn render_settings(frame: &mut Frame, area: Rect, menu: &SettingsMenu) {
    // Wipe the table underneath so the overlay reads as a separate screen.
    frame.render_widget(Clear, area);

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER))
        .padding(Padding::new(2, 2, 1, 1))
        .title(Span::styled(
            " SETTINGS ",
            Style::default().fg(LIME).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Rows block, then a spacer, the next-game note, and the footer hint.
    let [rows_area, _, note_area, footer_area] = Layout::vertical([
        Constraint::Length(ROWS as u16),
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    let lines: Vec<Line> = menu
        .rows()
        .iter()
        .enumerate()
        .map(|(i, (label, value))| settings_row(i, label, value, i == menu.selected()))
        .collect();
    frame.render_widget(Paragraph::new(lines), rows_area);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "seats and blinds take effect next game",
            Style::default().fg(DIM),
        ))),
        note_area,
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("↑↓", Style::default().fg(LIME).add_modifier(Modifier::BOLD)),
            Span::styled(" select   ", Style::default().fg(MUTED)),
            Span::styled("←→", Style::default().fg(LIME).add_modifier(Modifier::BOLD)),
            Span::styled(" change   ", Style::default().fg(MUTED)),
            Span::styled("Esc", Style::default().fg(LIME).add_modifier(Modifier::BOLD)),
            Span::styled(" done", Style::default().fg(MUTED)),
        ])),
        footer_area,
    );
}

/// One settings row: a focus marker, a left-aligned label, and the current
/// preset value wrapped in cycle arrows when focused.
fn settings_row(_i: usize, label: &str, value: &str, focused: bool) -> Line<'static> {
    let (marker, label_style, value_text, value_style) = if focused {
        (
            "▸ ",
            Style::default().fg(LIME).add_modifier(Modifier::BOLD),
            format!("‹ {value} ›"),
            Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
        )
    } else {
        (
            "  ",
            Style::default().fg(MUTED),
            value.to_string(),
            Style::default().fg(MUTED),
        )
    };
    Line::from(vec![
        Span::styled(marker, Style::default().fg(LIME)),
        Span::styled(format!("{label:<16}"), label_style),
        Span::styled(value_text, value_style),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::time::Duration;

    #[test]
    fn up_and_down_move_selection_within_bounds() {
        let mut menu = SettingsMenu::from(Settings::default());
        assert_eq!(menu.selected(), 0);
        menu.handle_key(KeyCode::Up);
        assert_eq!(menu.selected(), 0, "clamps at the top row");
        menu.handle_key(KeyCode::Down);
        assert_eq!(menu.selected(), 1);
        for _ in 0..10 {
            menu.handle_key(KeyCode::Down);
        }
        assert_eq!(menu.selected(), ROWS - 1, "clamps at the bottom row");
    }

    #[test]
    fn left_and_right_cycle_the_focused_value() {
        let mut menu = SettingsMenu::from(Settings::default());
        // Row 0 is the bot delay, default 500ms.
        menu.handle_key(KeyCode::Right);
        assert_eq!(menu.draft().bot_delay, Duration::from_millis(1000));
        menu.handle_key(KeyCode::Left);
        assert_eq!(menu.draft().bot_delay, Duration::from_millis(500));
        // Move down to the seats row (row 2) and change it.
        menu.handle_key(KeyCode::Down);
        menu.handle_key(KeyCode::Down);
        menu.handle_key(KeyCode::Left);
        assert_eq!(menu.draft().seats, 5);
    }

    #[test]
    fn esc_closes_and_other_keys_keep_it_open() {
        let mut menu = SettingsMenu::from(Settings::default());
        assert_eq!(menu.handle_key(KeyCode::Esc), MenuOutcome::Close);
        assert_eq!(menu.handle_key(KeyCode::Char('x')), MenuOutcome::Open);
        assert_eq!(menu.handle_key(KeyCode::Up), MenuOutcome::Open);
    }

    #[test]
    fn overlay_renders_every_row_label_and_value() {
        let menu = SettingsMenu::from(Settings::default());
        let dump = dump_menu(&menu, 120, 40);
        for needle in [
            "SETTINGS",
            "Bot delay",
            "500 ms",
            "Show win rate",
            "on",
            "Seats",
            "Blinds",
            "50 / 100",
            "next game",
            "Esc",
        ] {
            assert!(dump.contains(needle), "overlay missing {needle:?}\n{dump}");
        }
    }

    fn dump_menu(menu: &SettingsMenu, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| render_settings(frame, frame.area(), menu))
            .expect("draw");
        let buf = terminal.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf.cell((x, y)).expect("cell").symbol());
            }
            out.push('\n');
        }
        out
    }
}
