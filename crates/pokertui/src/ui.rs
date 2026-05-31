use poker_core::{Card, Rank, Suit};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Gauge, Padding, Paragraph, Widget, Wrap};

use crate::state::{ChatLine, GameState, LogEntry, LogTone, Seat, SeatStatus};

// ---------------------------------------------------------------- palette ----

mod pal {
    use ratatui::style::Color;
    pub const LIME: Color = Color::Rgb(0x82, 0xcc, 0x16);
    pub const RED: Color = Color::Rgb(0xe7, 0x00, 0x0b);
    pub const BLUE: Color = Color::Rgb(0x2f, 0x7b, 0xff);
    pub const GREEN: Color = Color::Rgb(0x5e, 0xa5, 0x00);
    pub const AMBER: Color = Color::Rgb(0xf5, 0x9e, 0x0b);
    pub const VIOLET: Color = Color::Rgb(0xa7, 0x8b, 0xfa);
    pub const MUTED: Color = Color::Rgb(0xa1, 0xa1, 0xaa);
    pub const DIM: Color = Color::Rgb(0x57, 0x57, 0x66);
    pub const BORDER: Color = Color::Rgb(0x3a, 0x3a, 0x44);
    pub const BACK: Color = Color::Rgb(0x4a, 0x6b, 0xa0);
    pub const SPADE: Color = Color::Reset;
}

// ---------------------------------------------------------------- entry ------

pub fn render(frame: &mut Frame, state: &GameState) {
    let area = frame.area();

    let [title_area, body] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);

    render_title_bar(frame, title_area, state);

    if body.width < 96 || body.height < 30 {
        render_too_small(frame, body);
        return;
    }

    let rail_w: u16 = 40;
    let [table_area, rail_area] =
        Layout::horizontal([Constraint::Min(40), Constraint::Length(rail_w)]).areas(body);

    render_table(frame, table_area, state);
    render_rail(frame, rail_area, state);
}

fn render_too_small(frame: &mut Frame, area: Rect) {
    let msg = Paragraph::new(vec![
        Line::from("terminal too small").style(Style::default().fg(pal::AMBER)),
        Line::from("resize to at least 96×30 (target 120×40)")
            .style(Style::default().fg(pal::MUTED)),
    ])
    .alignment(Alignment::Center);
    let centered = centered_rect(area, 60, 4);
    frame.render_widget(msg, centered);
}

// ---------------------------------------------------------------- title bar --

fn render_title_bar(frame: &mut Frame, area: Rect, state: &GameState) {
    let left = Line::from(vec![Span::raw(" ♠ Poker In Terminal ").bold()]);
    let right = Line::from(vec![
        Span::styled(
            format!("blinds {}  ·  ", state.blinds),
            Style::default().fg(pal::MUTED),
        ),
        Span::styled(
            state.phase.label,
            Style::default().fg(pal::LIME).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ]);
    frame.render_widget(Paragraph::new(left).alignment(Alignment::Left), area);
    frame.render_widget(Paragraph::new(right).alignment(Alignment::Right), area);
}

// ---------------------------------------------------------------- table ------

fn render_table(frame: &mut Frame, area: Rect, state: &GameState) {
    // Vertical regions inside the table column:
    //   row 0   : (in body coords) spacer / divider header
    //   rows 1..top_h : opponent strip
    //   middle  : pot + board, centered
    //   bottom  : hero pod + action bar + key hints
    //
    // Bottom block height: 6 (hero) + 1 (gap) + 4 (action bar) + 1 (gap) + 1 (hints) = 13
    let bottom_h: u16 = 13;
    let top_h: u16 = 8;
    let [top_strip, middle, bottom] = Layout::vertical([
        Constraint::Length(top_h),
        Constraint::Min(5),
        Constraint::Length(bottom_h),
    ])
    .areas(area);

    render_opponents(frame, top_strip, state);
    render_center(frame, middle, state);
    render_bottom(frame, bottom, state);
}

fn render_opponents(frame: &mut Frame, area: Rect, state: &GameState) {
    // Two active pods on the left, folded-roster on the right.
    let [active_col, _, folded_col] = Layout::horizontal([
        Constraint::Length(48),
        Constraint::Length(2),
        Constraint::Min(0),
    ])
    .areas(area);

    let [rook_slot, gizmo_slot] =
        Layout::horizontal([Constraint::Length(22), Constraint::Min(22)]).areas(active_col);

    if let Some(rook) = state.by_name("rook") {
        render_pod(frame, inset(rook_slot, 2, 1), rook, false);
    }
    if let Some(gizmo) = state.by_name("gizmo") {
        render_pod(frame, inset(gizmo_slot, 2, 1), gizmo, false);
    }

    // Folded names, stacked.
    let folded: Vec<&Seat> = state
        .players
        .iter()
        .filter(|p| p.status == SeatStatus::Folded)
        .collect();
    let folded_inner = inset(folded_col, 0, 1);
    for (i, p) in folded.iter().enumerate() {
        if (i as u16) >= folded_inner.height {
            break;
        }
        let line = Line::from(vec![
            Span::styled(p.name, Style::default().fg(pal::DIM)),
            Span::styled(" · ", Style::default().fg(pal::DIM)),
            Span::styled(p.pos, Style::default().fg(pal::DIM)),
            Span::styled("  folded", Style::default().fg(pal::DIM)),
        ]);
        put_line(frame, folded_inner.x, folded_inner.y + i as u16, line);
    }
}

fn render_center(frame: &mut Frame, area: Rect, state: &GameState) {
    // Board (4 tall) sits below pot pill (1 tall) with a 1-row gap.
    let needed_h: u16 = 1 + 1 + CARD_H;
    let block_h = needed_h.min(area.height);
    let block = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(block_h) / 2,
        width: area.width,
        height: block_h,
    };
    let [pot_row, _, board_row] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(CARD_H),
    ])
    .areas(block);

    render_pot_pill(frame, pot_row, state);
    render_board(frame, board_row, state);
}

fn render_bottom(frame: &mut Frame, area: Rect, state: &GameState) {
    let [hero_row, _gap, action_row, _gap2, hints_row] = Layout::vertical([
        Constraint::Length(6),
        Constraint::Length(1),
        Constraint::Length(4),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);

    if let Some(hero) = state.hero() {
        // Hero pod is 10 wide × 6 tall, centered.
        let pod_area = centered_rect(hero_row, POD_W, 6);
        render_pod(frame, pod_area, hero, true);
    }

    let action_area = inset(action_row, 2, 0);
    let action_area = Rect {
        width: action_area.width.saturating_sub(2),
        ..action_area
    };
    render_action_bar(frame, action_area, state);

    let hints_area = inset(hints_row, 2, 0);
    render_key_hints(frame, hints_area);
}

// ---------------------------------------------------------------- rail -------

fn render_rail(frame: &mut Frame, area: Rect, state: &GameState) {
    // Rail has a left divider — one of ratatui's stock borders, not ASCII.
    let frame_block = Block::default()
        .borders(Borders::LEFT)
        .border_style(Style::default().fg(pal::BORDER));
    let inner = frame_block.inner(area);
    frame.render_widget(frame_block, area);

    let inner = inset(inner, 1, 1);
    // Stack: HAND (5) · gap (1) · EQUITY (5) · gap (1) · LOG (flex) · gap (1) · CHAT (6)
    let hand_h: u16 = 5;
    let eq_h: u16 = 5;
    let chat_h: u16 = 6;
    let gap: u16 = 1;
    let log_h = inner
        .height
        .saturating_sub(hand_h + eq_h + chat_h + gap * 3)
        .max(3);
    let [hand_a, _, eq_a, _, log_a, _, chat_a] = Layout::vertical([
        Constraint::Length(hand_h),
        Constraint::Length(gap),
        Constraint::Length(eq_h),
        Constraint::Length(gap),
        Constraint::Length(log_h),
        Constraint::Length(gap),
        Constraint::Length(chat_h),
    ])
    .areas(inner);

    render_panel_hand(frame, hand_a, state);
    render_panel_equity(frame, eq_a, state);
    render_panel_log(frame, log_a, state);
    render_panel_chat(frame, chat_a, state);
}

// ---------------------------------------------------------------- pod --------

const CARD_W: u16 = 5;
const CARD_H: u16 = 4;
const POD_W: u16 = CARD_W * 2; // 10

fn render_pod(frame: &mut Frame, area: Rect, seat: &Seat, hero_is_actor: bool) {
    let hero = seat.is_hero();
    // Pod is 10 wide × 6 tall — clip what we got.
    let pod = Rect {
        width: POD_W.min(area.width),
        height: 6.min(area.height),
        ..area
    };

    // Cards row (rows 0..=3).
    let [cards_row, name_row, info_row] = Layout::vertical([
        Constraint::Length(CARD_H),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(pod);
    let [left_card, right_card] =
        Layout::horizontal([Constraint::Length(CARD_W), Constraint::Length(CARD_W)])
            .areas(cards_row);

    let face_up = hero;
    match (face_up, seat.hole_cards) {
        (true, Some([c0, c1])) => {
            render_card(frame, left_card, c0, hero);
            render_card(frame, right_card, c1, hero);
        }
        _ => {
            render_card_back(frame, left_card);
            render_card_back(frame, right_card);
        }
    }

    // Name · pos line.
    let mut name_spans = vec![
        Span::styled(
            seat.name,
            Style::default()
                .fg(if hero { pal::LIME } else { Color::Reset })
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" · ", Style::default().fg(pal::MUTED)),
        Span::styled(
            seat.pos,
            Style::default().fg(if hero { pal::LIME } else { pal::MUTED }),
        ),
    ];
    if hero {
        name_spans.push(Span::raw(" "));
        name_spans.push(Span::styled(
            "ⓑ",
            Style::default().fg(pal::AMBER).add_modifier(Modifier::BOLD),
        ));
    }
    put_line(frame, name_row.x, name_row.y, Line::from(name_spans));

    // Info line: ◉ stack   <tag>
    let chip_seg = format!("◉ {}", fmt_int(seat.stack));
    let tag = if hero {
        if hero_is_actor { "▸ TO ACT" } else { "—" }
    } else {
        seat.last_action
    };
    let tag_color = match (hero, seat.status) {
        (true, _) => pal::LIME,
        (_, SeatStatus::Folded) => pal::DIM,
        _ => verb_color(seat.last_action),
    };
    let info_line = Line::from(vec![
        Span::styled(
            chip_seg,
            Style::default().fg(pal::AMBER).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            tag,
            Style::default().fg(tag_color).add_modifier(Modifier::BOLD),
        ),
    ]);
    put_line(frame, info_row.x, info_row.y, info_line);
}

// ---------------------------------------------------------------- board ------

fn render_board(frame: &mut Frame, area: Rect, state: &GameState) {
    let slots = 5u16;
    let gap = 1u16;
    let total_w = slots * CARD_W + (slots - 1) * gap;
    let strip = centered_rect(area, total_w, CARD_H);

    let mut x = strip.x;
    for i in 0..slots as usize {
        let slot = Rect {
            x,
            y: strip.y,
            width: CARD_W,
            height: CARD_H,
        };
        match state.phase.board.get(i) {
            Some(&card) if i < state.phase.dealt => {
                let fresh = matches!(state.phase.label, "TURN") && i == 3
                    || matches!(state.phase.label, "RIVER" | "SHOWDOWN") && i == 4;
                render_card(frame, slot, card, fresh);
            }
            _ => render_card_slot(frame, slot),
        }
        x += CARD_W + gap;
    }
}

fn render_pot_pill(frame: &mut Frame, area: Rect, state: &GameState) {
    let amount = fmt_int(state.phase.pot);
    let line = Line::from(vec![
        Span::styled(
            "POT  ",
            Style::default().fg(pal::MUTED).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("◉ {amount}"),
            Style::default().fg(pal::AMBER).add_modifier(Modifier::BOLD),
        ),
    ]);
    frame.render_widget(Paragraph::new(line).alignment(Alignment::Center), area);
}

// ---------------------------------------------------------------- action -----

fn render_action_bar(frame: &mut Frame, area: Rect, state: &GameState) {
    let to_call = state.phase.to_call;
    let stack = state.hero().map(|h| h.stack).unwrap_or(0);
    let raise_to = state.phase.pot.max(1_800);

    let buttons = [
        ('F', "FOLD", String::new(), pal::RED),
        (
            'C',
            if to_call > 0 { "CALL" } else { "CHECK" },
            if to_call > 0 {
                fmt_int(to_call)
            } else {
                "—".into()
            },
            pal::LIME,
        ),
        ('R', "RAISE", fmt_int(raise_to), pal::AMBER),
        ('A', "ALL-IN", fmt_int(stack), Color::Reset),
    ];

    let n = buttons.len() as u16;
    // 4 buttons, 1-col gaps between → (area.width - (n-1)) / n.
    let btn_w = area.width.saturating_sub(n - 1) / n;
    let constraints: Vec<Constraint> = (0..n)
        .flat_map(|i| {
            if i == 0 {
                vec![Constraint::Length(btn_w)]
            } else {
                vec![Constraint::Length(1), Constraint::Length(btn_w)]
            }
        })
        .collect();
    let cols = Layout::horizontal(constraints).split(area);

    for (i, (key, label, amount, color)) in buttons.into_iter().enumerate() {
        let col = cols[i * 2];
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(pal::BORDER))
            .padding(Padding::horizontal(1));
        let inner = block.inner(col);
        frame.render_widget(block, col);

        let header = Line::from(vec![
            Span::styled(
                key.to_string(),
                Style::default().fg(pal::LIME).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                label,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
        ]);
        put_line(frame, inner.x, inner.y, header);
        if !amount.is_empty() && inner.height >= 2 {
            put_line(
                frame,
                inner.x,
                inner.y + 1,
                Line::from(Span::styled(amount, Style::default().fg(pal::MUTED))),
            );
        }
    }
}

fn render_key_hints(frame: &mut Frame, area: Rect) {
    let hints = [
        ("F", "fold"),
        ("C", "check/call"),
        ("R", "raise"),
        ("A", "all-in"),
        ("↑↓", "bet size"),
        ("⏎", "confirm"),
        ("Tab", "next seat"),
        ("Q", "quit"),
    ];
    let mut spans: Vec<Span> = Vec::new();
    for (i, (k, label)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("   "));
        }
        spans.push(Span::styled(
            format!(" {k} "),
            Style::default().fg(pal::LIME).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(*label, Style::default().fg(pal::DIM)));
    }
    // Bound by the caller's rect so the strip doesn't bleed into the rail.
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ---------------------------------------------------------------- panels -----

fn render_panel_hand(frame: &mut Frame, area: Rect, state: &GameState) {
    let block = panel_block("HAND");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(hero) = state.hero() else { return };
    let Some([c0, c1]) = hero.hole_cards else {
        return;
    };

    let hole = Line::from(vec![
        suit_text_span(c0),
        Span::raw("  "),
        suit_text_span(c1),
    ]);
    let rank = Line::from(vec![
        Span::styled("▸ ", Style::default().fg(pal::LIME)),
        Span::styled(state.phase.rank, Style::default().fg(Color::Reset)),
    ]);
    let hint = Line::from(Span::styled(
        state.phase.hint,
        Style::default().fg(pal::MUTED),
    ));
    frame.render_widget(
        Paragraph::new(vec![hole, rank, hint]).wrap(Wrap { trim: true }),
        inner,
    );
}

fn render_panel_equity(frame: &mut Frame, area: Rect, state: &GameState) {
    let block = panel_block("EQUITY · POT ODDS");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [eq_row, odds_row, verdict_row] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    // "win 38%" label on the left, ratatui Gauge filling the rest of the row.
    let eq = state.phase.equity;
    let label = Line::from(vec![
        Span::styled("win ", Style::default().fg(pal::MUTED)),
        Span::styled(
            format!("{eq}%"),
            Style::default().fg(pal::LIME).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ]);
    let label_w = label.width() as u16;
    let [label_area, gauge_area] =
        Layout::horizontal([Constraint::Length(label_w), Constraint::Min(0)]).areas(eq_row);
    frame.render_widget(Paragraph::new(label), label_area);
    let gauge = Gauge::default()
        .ratio((eq as f64 / 100.0).clamp(0.0, 1.0))
        .gauge_style(Style::default().fg(pal::LIME).bg(pal::BORDER))
        .label("");
    frame.render_widget(gauge, gauge_area);

    if state.phase.to_call > 0 {
        let odds = format!("{:.1}%", state.phase.odds_pct);
        let to_call = format!("to call {}", fmt_int(state.phase.to_call));
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("odds ", Style::default().fg(pal::MUTED)),
                Span::styled(
                    odds,
                    Style::default()
                        .fg(Color::Reset)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(to_call, Style::default().fg(pal::MUTED)),
            ])),
            odds_row,
        );
        let ev_good = state.phase.equity as f32 > state.phase.odds_pct;
        let verdict = if ev_good {
            "▸ +EV — call profitable"
        } else {
            "▸ marginal"
        };
        let color = if ev_good { pal::LIME } else { pal::AMBER };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                verdict,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ))),
            verdict_row,
        );
    } else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "no bet to face",
                Style::default().fg(pal::MUTED),
            ))),
            odds_row,
        );
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "▸ pot-control",
                Style::default().fg(pal::LIME).add_modifier(Modifier::BOLD),
            ))),
            verdict_row,
        );
    }
}

fn render_panel_log(frame: &mut Frame, area: Rect, state: &GameState) {
    let block = panel_block("ACTION LOG");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible = inner.height as usize;
    let start = state.log.len().saturating_sub(visible);
    let lines: Vec<Line> = state.log[start..].iter().map(log_entry_line).collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_panel_chat(frame: &mut Frame, area: Rect, state: &GameState) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal::BORDER))
        .title(Span::styled(
            " CHAT ",
            Style::default()
                .fg(pal::VIOLET)
                .add_modifier(Modifier::BOLD),
        ))
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible = inner.height as usize;
    let start = state.chat.len().saturating_sub(visible);
    let lines: Vec<Line> = state.chat[start..].iter().map(chat_line).collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

fn log_entry_line(e: &LogEntry) -> Line<'static> {
    let who_color = if e.who == "·" { pal::DIM } else { pal::MUTED };
    let who_span = Span::styled(format!("{:<6}", e.who), Style::default().fg(who_color));
    let action_color = match e.tone {
        LogTone::Dim => pal::DIM,
        LogTone::Muted => pal::MUTED,
        LogTone::Fg => Color::Reset,
        LogTone::Amber => pal::AMBER,
    };
    Line::from(vec![
        who_span,
        Span::raw(" "),
        Span::styled(e.what.to_string(), Style::default().fg(action_color)),
    ])
}

fn chat_line(c: &ChatLine) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{}: ", c.who),
            Style::default()
                .fg(pal::VIOLET)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(c.msg.to_string(), Style::default().fg(pal::MUTED)),
    ])
}

fn panel_block(title: &'static str) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal::BORDER))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(pal::LIME).add_modifier(Modifier::BOLD),
        ))
        .padding(Padding::horizontal(1))
}

// ---------------------------------------------------------------- cards ------

pub fn render_card(frame: &mut Frame, area: Rect, card: Card, highlight: bool) {
    frame.render_widget(CardFace { card, highlight }, area);
}

pub fn render_card_back(frame: &mut Frame, area: Rect) {
    frame.render_widget(CardBack, area);
}

pub fn render_card_slot(frame: &mut Frame, area: Rect) {
    frame.render_widget(CardSlot, area);
}

struct CardFace {
    card: Card,
    highlight: bool,
}

impl Widget for CardFace {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < CARD_W || area.height < CARD_H {
            return;
        }
        let border = if self.highlight {
            Style::default().fg(pal::LIME).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(pal::BORDER)
        };
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(border);
        let inner = block.inner(area);
        block.render(area, buf);

        let color = suit_color(self.card.suit());
        let rank = rank_label(self.card.rank());
        // Inner is 3×2: rank top-left, suit at (1,1) of inner.
        write_str(
            buf,
            inner.x,
            inner.y,
            rank,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        );
        let suit_x = inner.x + 1;
        let suit_y = inner.y + 1;
        let suit = suit_glyph(self.card.suit());
        write_str(
            buf,
            suit_x,
            suit_y,
            &suit.to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        );
    }
}

struct CardBack;

impl Widget for CardBack {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < CARD_W || area.height < CARD_H {
            return;
        }
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(pal::BACK));
        let inner = block.inner(area);
        block.render(area, buf);
        let fill = Style::default().fg(pal::BACK);
        let pat = "░".repeat(inner.width as usize);
        for dy in 0..inner.height {
            write_str(buf, inner.x, inner.y + dy, &pat, fill);
        }
    }
}

struct CardSlot;

impl Widget for CardSlot {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < CARD_W || area.height < CARD_H {
            return;
        }
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(pal::DIM));
        let inner = block.inner(area);
        block.render(area, buf);
        let cx = inner.x + inner.width / 2;
        let cy = inner.y + inner.height.saturating_sub(1) / 2;
        if let Some(cell) = buf.cell_mut((cx, cy)) {
            cell.set_char('·').set_style(Style::default().fg(pal::DIM));
        }
    }
}

// ---------------------------------------------------------------- helpers ----

fn put_line(frame: &mut Frame, x: u16, y: u16, line: Line<'_>) {
    let buf_area = frame.area();
    if y >= buf_area.y + buf_area.height || x >= buf_area.x + buf_area.width {
        return;
    }
    let max_w = buf_area.x + buf_area.width - x;
    let width = (line.width() as u16).min(max_w);
    if width == 0 {
        return;
    }
    let area = Rect {
        x,
        y,
        width,
        height: 1,
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn write_str(buf: &mut Buffer, x: u16, y: u16, s: &str, style: Style) {
    let mut cx = x;
    for ch in s.chars() {
        if let Some(cell) = buf.cell_mut((cx, y)) {
            cell.set_char(ch).set_style(style);
        }
        cx = cx.saturating_add(1);
    }
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    }
}

fn inset(area: Rect, dx: u16, dy: u16) -> Rect {
    Rect {
        x: area.x + dx.min(area.width),
        y: area.y + dy.min(area.height),
        width: area.width.saturating_sub(dx),
        height: area.height.saturating_sub(dy),
    }
}

fn fmt_int(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, &b) in bytes.iter().enumerate() {
        let from_end = bytes.len() - i;
        if i > 0 && from_end.is_multiple_of(3) {
            out.push(',');
        }
        out.push(b as char);
    }
    out
}

fn rank_label(rank: Rank) -> &'static str {
    match rank {
        Rank::Two => "2",
        Rank::Three => "3",
        Rank::Four => "4",
        Rank::Five => "5",
        Rank::Six => "6",
        Rank::Seven => "7",
        Rank::Eight => "8",
        Rank::Nine => "9",
        Rank::Ten => "10",
        Rank::Jack => "J",
        Rank::Queen => "Q",
        Rank::King => "K",
        Rank::Ace => "A",
    }
}

fn suit_glyph(suit: Suit) -> char {
    match suit {
        Suit::Clubs => '♣',
        Suit::Diamonds => '♦',
        Suit::Hearts => '♥',
        Suit::Spades => '♠',
    }
}

fn suit_color(suit: Suit) -> Color {
    match suit {
        Suit::Hearts => pal::RED,
        Suit::Diamonds => pal::BLUE,
        Suit::Clubs => pal::GREEN,
        Suit::Spades => pal::SPADE,
    }
}

/// Render a card's text form (e.g. "A♠") with the suit-appropriate color.
fn suit_text_span(card: Card) -> Span<'static> {
    let color = suit_color(card.suit());
    let s = format!("{}{}", rank_label(card.rank()), suit_glyph(card.suit()));
    Span::styled(s, Style::default().fg(color).add_modifier(Modifier::BOLD))
}

fn verb_color(action: &str) -> Color {
    let a = action.to_ascii_lowercase();
    if a.starts_with("fold") {
        pal::DIM
    } else if a.starts_with("bet") || a.starts_with("raise") || a.starts_with("all") {
        pal::AMBER
    } else if a.starts_with("check") {
        pal::MUTED
    } else {
        Color::Reset
    }
}

// ---------------------------------------------------------------- tests ------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::GameState;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn dump(width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let state = GameState::demo();
        terminal.draw(|frame| render(frame, &state)).expect("draw");
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

    #[test]
    fn workbench_rail_roomy_renders() {
        let frame = dump(120, 40);
        println!("\n{}", frame);

        assert!(frame.contains("Poker In Terminal"), "title missing");
        assert!(frame.contains("blinds 50 / 100"), "blinds missing");
        assert!(frame.contains("TURN"), "phase label missing");
        assert!(frame.contains("POT"), "pot label missing");
        assert!(frame.contains("1,850"), "pot amount missing");
        assert!(frame.contains("HAND"), "hand panel missing");
        assert!(frame.contains("EQUITY"), "equity panel missing");
        assert!(frame.contains("ACTION LOG"), "log panel missing");
        assert!(frame.contains("CHAT"), "chat panel missing");
        assert!(frame.contains("FOLD"), "fold button missing");
        assert!(frame.contains("CALL"), "call button missing");
        assert!(frame.contains("RAISE"), "raise button missing");
        assert!(frame.contains("ALL-IN"), "all-in button missing");
        assert!(frame.contains("rook"), "active opponent missing");
        assert!(frame.contains("gizmo"), "active opponent missing");
        assert!(frame.contains("nova"), "folded opponent missing");
        assert!(frame.contains("nice spot"), "chat content missing");
    }

    #[test]
    fn too_small_terminal_shows_notice() {
        let frame = dump(60, 20);
        assert!(frame.contains("terminal too small"), "no notice rendered");
    }
}
