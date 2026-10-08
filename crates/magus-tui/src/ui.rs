//! Drawing. Everything here is read-only over `App`.

use std::collections::BTreeMap;

use magus_core::mana::Color;
use magus_core::view::{CardView, PermanentView, PlayOption, Prompt};
use magus_core::{GameView, ObjectId, PlayerId, Target};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color as Tc, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap,
};

use crate::app::{App, Focus, Mode, Screen, attack_targets, graveyard_entries};

pub(crate) const SELECTED: Style = Style::new().add_modifier(Modifier::REVERSED);

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    match &app.screen {
        Screen::Connecting => centered_message(frame, area, "Connecting…"),
        Screen::Waiting { room } => centered_message(
            frame,
            area,
            &format!("Waiting for an opponent to join room \"{room}\"…\n\n(q to quit)"),
        ),
        Screen::PickDeck => draw_deck_picker(frame, area, app),
        Screen::Game => match &app.view {
            Some(view) => draw_game(frame, area, app, view),
            None => centered_message(frame, area, "Starting game…"),
        },
    }
    if app.disconnected {
        let msg = Paragraph::new(" Disconnected from server. Press q to quit. ")
            .on_red()
            .white()
            .bold();
        frame.render_widget(msg, Rect { height: 1, ..area });
    }
}

fn centered_message(frame: &mut Frame, area: Rect, text: &str) {
    let [_, middle, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(5),
        Constraint::Fill(1),
    ])
    .areas(area);
    frame.render_widget(Paragraph::new(text).alignment(Alignment::Center), middle);
}

pub(crate) fn panel(title: impl Into<Line<'static>>, focused: bool) -> Block<'static> {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(title.into());
    if focused {
        block.border_style(Style::new().fg(Tc::Cyan))
    } else {
        block.border_style(Style::new().dark_gray())
    }
}

pub(crate) fn color_of(colors: &[Color]) -> Tc {
    match colors {
        [] => Tc::Gray,
        [color] => mana_color(*color),
        _ => Tc::Yellow,
    }
}

/// The terminal color for a color of mana. Black is shown as magenta, which
/// stays readable on a dark background.
fn mana_color(color: Color) -> Tc {
    match color {
        Color::White => Tc::White,
        Color::Blue => Tc::LightBlue,
        Color::Black => Tc::Magenta,
        Color::Red => Tc::LightRed,
        Color::Green => Tc::LightGreen,
    }
}

/// Splits text into spans, coloring each colored mana symbol such as `{U}`.
/// Generic mana (`{2}`) and everything else keep `base`.
pub(crate) fn mana_text(text: &str, base: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        let Some(len) = rest[start..].find('}') else {
            break;
        };
        let symbol = &rest[start..start + len + 1];
        let mut inner = symbol[1..symbol.len() - 1].chars();
        let color = match (inner.next().and_then(Color::from_symbol), inner.next()) {
            (Some(color), None) => Some(color),
            _ => None,
        };
        if start > 0 {
            spans.push(Span::styled(rest[..start].to_string(), base));
        }
        spans.push(match color {
            Some(color) => Span::styled(symbol.to_string(), base.fg(mana_color(color)).bold()),
            None => Span::styled(symbol.to_string(), base),
        });
        rest = &rest[start + len + 1..];
    }
    if !rest.is_empty() {
        spans.push(Span::styled(rest.to_string(), base));
    }
    spans
}

pub(crate) fn card_name(card: &CardView) -> Span<'static> {
    Span::styled(
        card.name.clone(),
        Style::new().fg(color_of(&card.colors)).bold(),
    )
}

fn draw_deck_picker(frame: &mut Frame, area: Rect, app: &App) {
    let [_, middle, _] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Max(70),
        Constraint::Fill(1),
    ])
    .areas(area);
    let [title, list, status_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Fill(1),
        Constraint::Length(2),
    ])
    .areas(middle);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from("MAGUS").bold().cyan(),
            Line::from("Choose a deck (↑↓, Enter)").dark_gray(),
        ])
        .alignment(Alignment::Center),
        title,
    );
    let items: Vec<ListItem> = app
        .decks
        .iter()
        .map(|d| {
            ListItem::new(vec![
                Line::from(d.name.clone()).bold(),
                Line::from(format!("  {}", d.description)).dark_gray(),
                Line::from(""),
            ])
        })
        .collect();
    let mut state = ListState::default().with_selected(Some(app.deck_sel));
    frame.render_stateful_widget(
        List::new(items)
            .block(panel(" Decks ", true))
            .highlight_style(SELECTED),
        list,
        &mut state,
    );
    if let Some(status) = &app.status {
        frame.render_widget(Paragraph::new(status.clone()).red(), status_area);
    }
}

fn draw_game(frame: &mut Frame, area: Rect, app: &App, view: &GameView) {
    let [main, side] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(46)]).areas(area);
    let stack_height = if view.stack.is_empty() {
        0
    } else {
        (view.stack.len() as u16 + 2).min(8)
    };
    let hand_height = (view.hand.len() as u16 + 2).clamp(3, 12);
    let [header, opponents, stack, mine, hand, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(stack_height),
        Constraint::Fill(1),
        Constraint::Length(hand_height),
        Constraint::Length(3),
    ])
    .areas(main);
    let [detail, log] = Layout::vertical([Constraint::Length(12), Constraint::Fill(1)]).areas(side);

    draw_header(frame, header, view);
    let opponent_ids: Vec<PlayerId> = view.opponents().map(|p| p.id).collect();
    let rows = Layout::vertical(opponent_ids.iter().map(|_| Constraint::Fill(1))).split(opponents);
    for (p, row) in opponent_ids.iter().zip(rows.iter()) {
        draw_player(frame, *row, app, view, *p);
    }
    if !view.stack.is_empty() {
        draw_stack(frame, stack, app, view);
    }
    draw_player(frame, mine, app, view, view.you);
    draw_hand(frame, hand, app, view);
    draw_footer(frame, footer, app, view);
    draw_detail(frame, detail, app, view);
    draw_log(frame, log, view);

    if let Mode::Target {
        targets, sel, card, ..
    } = &app.mode
    {
        draw_target_popup(frame, area, view, *card, targets, *sel);
    }
    if let Mode::CastWay { plays, sel } = &app.mode {
        draw_cast_way_popup(frame, area, view, plays, *sel);
    }
    if let Mode::Graveyard { sel } = &app.mode {
        draw_graveyard_popup(frame, area, app, view, *sel);
    }
    if let Mode::Choose {
        reason,
        options,
        min,
        max,
        sel,
        marked,
    } = &app.mode
    {
        draw_choose_popup(frame, area, reason, options, (*min, *max), *sel, marked);
    }
}

fn draw_header(frame: &mut Frame, area: Rect, view: &GameView) {
    let whose = if view.active == view.you {
        "Your".to_string()
    } else {
        format!("{}'s", view.player_name(view.active))
    };
    let mut spans = vec![
        " MAGUS ".bold().black().on_cyan(),
        Span::raw(format!("  Turn {} · {whose} turn · ", view.turn)),
        Span::styled(view.step.name(), Style::new().bold()),
    ];
    if let Some(p) = view.priority {
        let who = if p == view.you {
            "you".to_string()
        } else {
            view.player_name(p).to_string()
        };
        spans.push(Span::raw(format!(" · priority: {who}")).dark_gray());
    }
    frame.render_widget(Line::from(spans), area);
}

fn draw_player(frame: &mut Frame, area: Rect, app: &App, view: &GameView, p: PlayerId) {
    let player = &view.players[p];
    let you = p == view.you;
    let name = if you {
        format!("{} (you)", player.name)
    } else {
        player.name.clone()
    };
    let mut title = vec![
        Span::raw(" "),
        Span::styled(name, Style::new().bold()),
        Span::raw("  "),
        Span::styled(
            format!("♥ {}", player.life),
            Style::new().fg(Tc::LightRed).bold(),
        ),
        Span::raw(format!(
            "  hand {} · library {} · graveyard {}{} ",
            player.hand_size,
            player.library_size,
            player.graveyard.len(),
            if player.exile.is_empty() {
                String::new()
            } else {
                format!(" · exile {}", player.exile.len())
            }
        ))
        .dark_gray(),
    ];
    if player.lost {
        title.push(" LOST ".on_red());
    }
    if view.active == p {
        title.push(" ◆ ".cyan());
    }
    let focused = matches!(app.mode, Mode::Idle) && app.focus == Focus::Board
        || you && matches!(app.mode, Mode::Attack { .. } | Mode::Block { .. });
    let block = panel(Line::from(title), focused);

    let mut lines = vec![lands_line(view, p)];
    for emblem in &player.emblems {
        lines.push(Line::from(vec![
            "Emblem: ".magenta().bold(),
            Span::raw(emblem.clone()),
        ]));
    }
    let board = app.board_order();
    for perm in view
        .battlefield
        .iter()
        .filter(|perm| perm.controller == p && !perm.card.is_land)
    {
        let id = perm.card.id;
        let (marker, highlight) = match &app.mode {
            Mode::Idle => {
                let selected = app.focus == Focus::Board && board.get(app.board_sel) == Some(&id);
                // ● marks permanents with an ability you could activate now.
                (if app.is_playable(id) { "● " } else { "" }, selected)
            }
            Mode::Attack {
                options,
                chosen,
                sel,
            } => match options.iter().position(|o| o.attacker == id) {
                Some(i) => (
                    if chosen[i].is_some() {
                        "[⚔] "
                    } else {
                        "[ ] "
                    },
                    i == *sel,
                ),
                None => ("", false),
            },
            Mode::Block { options, sel, .. } => {
                match options.iter().position(|o| o.blocker == id) {
                    Some(i) => ("", i == *sel),
                    None => ("", false),
                }
            }
            Mode::Target { targets, sel, .. } => {
                ("", targets.get(*sel) == Some(&Target::Permanent(id)))
            }
            Mode::Discard { .. }
            | Mode::Graveyard { .. }
            | Mode::Choose { .. }
            | Mode::CastWay { .. } => ("", false),
        };
        let mut line = permanent_line(view, perm, marker);
        if let Mode::Attack {
            options, chosen, ..
        } = &app.mode
            && let Some(i) = options.iter().position(|o| o.attacker == id)
            && let Some(d) = chosen[i]
            && let Some(&(defender, planeswalker)) = attack_targets(view, &options[i]).get(d)
        {
            let whom = match planeswalker.and_then(|pw| view.permanent(pw)) {
                Some(pw) => format!("{} ({})", pw.card.name, view.player_name(defender)),
                None => view.player_name(defender).to_string(),
            };
            line.push_span(format!(" → {whom}").yellow());
        }
        if let Mode::Block {
            options, chosen, ..
        } = &app.mode
            && let Some(i) = options.iter().position(|o| o.blocker == id)
        {
            let label = match chosen[i] {
                Some(a) => {
                    let attacker = view
                        .permanent(options[i].attackers[a])
                        .map_or("?", |p| p.card.name.as_str());
                    format!(" ⛨ blocks {attacker}")
                }
                None => " (not blocking)".into(),
            };
            line.push_span(label.yellow());
        }
        lines.push(if highlight {
            line.patch_style(SELECTED)
        } else {
            line
        });
    }
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn lands_line(view: &GameView, p: PlayerId) -> Line<'static> {
    let mut counts: BTreeMap<String, (usize, usize, Tc)> = BTreeMap::new();
    for perm in view
        .battlefield
        .iter()
        .filter(|perm| perm.controller == p && perm.card.is_land)
    {
        // Lands are colorless; color them by the mana they make.
        let color = Color::ALL
            .into_iter()
            .find(|c| perm.card.text.contains(&format!("{{{}}}", c.symbol())))
            .map_or(Tc::Gray, mana_color);
        let entry = counts
            .entry(perm.card.name.clone())
            .or_insert((0, 0, color));
        entry.0 += 1;
        if !perm.tapped {
            entry.1 += 1;
        }
    }
    if counts.is_empty() {
        return Line::from("No lands").dark_gray();
    }
    let mut spans = vec![Span::raw("Lands: ").dark_gray()];
    for (i, (name, (total, untapped, color))) in counts.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(
            format!("{total} {name}"),
            Style::new().fg(color),
        ));
        spans.push(Span::raw(format!(" ({untapped} untapped)")).dark_gray());
    }
    Line::from(spans)
}

fn permanent_line(view: &GameView, perm: &PermanentView, marker: &str) -> Line<'static> {
    let mut spans = vec![Span::raw(marker.to_string()), card_name(&perm.card)];
    if let (Some(power), Some(toughness)) = (perm.power, perm.toughness) {
        let boosted = Some(power) != perm.card.power || Some(toughness) != perm.card.toughness;
        let pt = format!(" {power}/{toughness}");
        spans.push(if boosted {
            pt.yellow().bold()
        } else {
            pt.bold()
        });
    }
    if let Some(loyalty) = perm.loyalty {
        spans.push(format!(" [{loyalty}]").magenta().bold());
    }
    // Current keywords: some may be granted by static abilities or emblems.
    if !perm.keywords.is_empty() {
        let kws: Vec<_> = perm
            .keywords
            .iter()
            .map(|k| k.name().to_lowercase())
            .collect();
        spans.push(format!(" ({})", kws.join(", ")).dark_gray());
    }
    if perm.tapped {
        spans.push(" tapped".dark_gray().italic());
    }
    if perm.summoning_sick {
        spans.push(" sick".dark_gray().italic());
    }
    if perm.damage > 0 {
        spans.push(format!(" {} dmg", perm.damage).red());
    }
    if let Some(defender) = perm.attacking {
        let who = match perm
            .attacking_planeswalker
            .and_then(|pw| view.permanent(pw))
        {
            Some(pw) => pw.card.name.clone(),
            None if defender == view.you => "you".to_string(),
            None => view.player_name(defender).to_string(),
        };
        spans.push(format!(" ⚔ attacking {who}").light_red().bold());
    }
    if let Some(attacker) = perm.blocking {
        let name = view
            .permanent(attacker)
            .map_or("?".to_string(), |p| p.card.name.clone());
        spans.push(format!(" ⛨ blocking {name}").light_blue());
    }
    Line::from(spans)
}

fn draw_stack(frame: &mut Frame, area: Rect, app: &App, view: &GameView) {
    let focused = matches!(app.mode, Mode::Idle) && app.focus == Focus::Stack;
    let items: Vec<ListItem> = view
        .stack
        .iter()
        .rev()
        .map(|item| {
            let mut spans = vec![card_name(&item.card)];
            if !item.is_spell {
                spans.push(" (ability)".dark_gray());
            }
            spans.push(format!(" — {}", view.player_name(item.controller)).dark_gray());
            if let Some(target) = &item.target {
                spans.push(format!(" → {}", view.describe_target(target)).yellow());
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    let selected = match &app.mode {
        Mode::Idle if focused => Some(app.stack_sel),
        Mode::Target { targets, sel, .. } => match targets.get(*sel) {
            Some(Target::Spell(id)) => view.stack.iter().rev().position(|s| s.id == *id),
            _ => None,
        },
        _ => None,
    };
    let mut state = ListState::default().with_selected(selected);
    let list = List::new(items)
        .block(panel(" Stack (top resolves first) ", focused))
        .highlight_style(SELECTED);
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_hand(frame: &mut Frame, area: Rect, app: &App, view: &GameView) {
    let discarding = matches!(app.mode, Mode::Discard { .. });
    let focused = matches!(app.mode, Mode::Idle) && app.focus == Focus::Hand || discarding;
    let items: Vec<ListItem> = view
        .hand
        .iter()
        .map(|card| {
            let playable = app.is_playable(card.id);
            let marker = match &app.mode {
                Mode::Discard { marked, .. } if marked.contains(&card.id) => "✗ ".red(),
                _ if playable => "● ".green(),
                _ => "  ".into(),
            };
            let mut name = card_name(card);
            if !playable && !discarding {
                name = name.patch_style(Style::new().add_modifier(Modifier::DIM));
            }
            let mut spans = vec![marker, name, Span::raw(" ")];
            spans.extend(mana_text(&card.cost, Style::new()));
            spans.push(format!("  {}", card.type_line).dark_gray());
            ListItem::new(Line::from(spans))
        })
        .collect();
    let mut state = ListState::default().with_selected(focused.then_some(app.hand_sel));
    let list = List::new(items)
        .block(panel(format!(" Your hand ({}) ", view.hand.len()), focused))
        .highlight_style(SELECTED);
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App, view: &GameView) {
    let control = if app.full_control {
        "f: full control [on]"
    } else {
        "f: full control [off]"
    };
    let (prompt, keys): (Line, String) = match (&app.mode, &view.prompt) {
        (_, Prompt::GameOver { winner }) => {
            let text = match winner {
                Some(w) if *w == view.you => "You win!".to_string(),
                Some(w) => format!("{} wins.", view.player_name(*w)),
                None => "The game is a draw.".to_string(),
            };
            (Line::from(text).bold().yellow(), "q: quit".into())
        }
        (Mode::Target { .. }, _) => (
            "Choose a target".bold().into(),
            "↑↓: select · Enter: cast · Esc: cancel".into(),
        ),
        (Mode::Attack { .. }, _) => (
            "Declare attackers".bold().into(),
            "↑↓: creature · Space: toggle · a: all · Enter: confirm (none = no attack)".into(),
        ),
        (Mode::Block { .. }, _) => (
            "Declare blockers".bold().into(),
            "↑↓: creature · Space/←→: choose attacker to block · Enter: confirm".into(),
        ),
        (Mode::Choose { min, max, .. }, _) => (
            if *max == 1 {
                "Choose a card".bold().into()
            } else {
                format!("Choose up to {max} cards").bold().into()
            },
            match (*min, *max) {
                (0, 1) => "↑↓: select · Enter: choose · Esc: choose nothing".into(),
                (_, 1) => "↑↓: select · Enter: choose".into(),
                _ => "↑↓: select · Space: mark · Enter: choose marked · Esc: choose nothing".into(),
            },
        ),
        (Mode::CastWay { .. }, _) => (
            "Which one?".bold().into(),
            "↑↓: select · Enter: choose · Esc: cancel".into(),
        ),
        (Mode::Graveyard { .. }, _) => (
            "Graveyards".bold().into(),
            "↑↓: select · Enter: cast with flashback (●) · Esc/g: close".into(),
        ),
        (Mode::Discard { count, .. }, _) => (
            format!("Discard down to hand size: choose {count}")
                .bold()
                .into(),
            "↑↓: card · Space: mark · Enter: confirm".into(),
        ),
        (Mode::Idle, Prompt::Priority { plays }) => {
            let what = if view.stack.is_empty() {
                format!("Your move — {}", view.step.name())
            } else {
                "Respond to the stack, or pass to let it resolve".to_string()
            };
            let hint = if plays.iter().any(|p| view.hand_card(p.card).is_some()) {
                " · Enter: play ●"
            } else {
                ""
            };
            let from_graveyard = plays
                .iter()
                .filter(|p| view.graveyard_card(p.card).is_some())
                .count();
            let graveyards = if from_graveyard > 0 {
                format!("g: graveyards ({from_graveyard} castable)")
            } else {
                "g: graveyards".to_string()
            };
            (
                what.bold().green().into(),
                format!("Space: pass{hint} · Tab: inspect · {graveyards} · {control} · q: quit"),
            )
        }
        (Mode::Idle, Prompt::Waiting { on }) => (
            Line::from(format!("Waiting for {}…", view.player_name(*on))).dark_gray(),
            format!("Tab: inspect · g: graveyards · {control} · q: quit"),
        ),
        (Mode::Idle, _) => (Line::default(), String::new()),
    };
    let status = match &app.status {
        Some(s) => Line::from(s.clone()).red(),
        None => Line::from(keys).dark_gray(),
    };
    frame.render_widget(
        Paragraph::new(vec![prompt, status]).block(Block::default().borders(Borders::TOP)),
        area,
    );
}

fn draw_detail(frame: &mut Frame, area: Rect, app: &App, view: &GameView) {
    let block = panel(" Card ", false);
    let Some(card) = app.selected_card() else {
        frame.render_widget(
            Paragraph::new("Nothing selected.").dark_gray().block(block),
            area,
        );
        return;
    };
    let mut lines = card_lines(&card);
    if let (Some(p), Some(t)) = (card.power, card.toughness) {
        lines.push(Line::from(""));
        let current = view
            .permanent(card.id)
            .and_then(|perm| Some((perm.power?, perm.toughness?, perm.damage)));
        let pt = match current {
            Some((cp, ct, dmg)) if (cp, ct) != (p, t) || dmg > 0 => {
                format!(
                    "{p}/{t}  (now {cp}/{ct}{})",
                    if dmg > 0 {
                        format!(", {dmg} damage")
                    } else {
                        String::new()
                    }
                )
            }
            _ => format!("{p}/{t}"),
        };
        lines.push(Line::from(pt).bold());
    }
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: false })
            .block(block),
        area,
    );
}

/// A card's name and cost, type line and rules text, for detail panes.
pub(crate) fn card_lines(card: &CardView) -> Vec<Line<'static>> {
    let mut title = vec![card_name(card), Span::raw("  ")];
    title.extend(mana_text(&card.cost, Style::new()));
    let mut lines = vec![
        Line::from(title),
        Line::from(card.type_line.clone()).italic(),
        Line::from(""),
    ];
    lines.extend(
        card.text
            .lines()
            .map(|l| Line::from(mana_text(l, Style::new()))),
    );
    if let Some(loyalty) = card.loyalty {
        lines.push(Line::from(""));
        lines.push(Line::from(format!("Loyalty {loyalty}")).bold());
    }
    if let Some(flavor) = card.flavor.as_deref().filter(|f| !f.trim().is_empty()) {
        lines.push(Line::from(""));
        lines.extend(
            flavor
                .lines()
                .map(|l| Line::from(l.to_string()).italic().dark_gray()),
        );
    }
    lines
}

fn draw_log(frame: &mut Frame, area: Rect, view: &GameView) {
    let height = area.height.saturating_sub(2) as usize;
    let lines: Vec<Line> = view.log[view.log.len().saturating_sub(height)..]
        .iter()
        .map(|l| {
            if l.starts_with('─') {
                Line::from(l.clone()).cyan()
            } else {
                Line::from(l.clone())
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).block(panel(" Log ", false)), area);
}

fn draw_graveyard_popup(frame: &mut Frame, area: Rect, app: &App, view: &GameView, sel: usize) {
    let entries = graveyard_entries(view);
    let height = (entries.len() as u16 + 2).min(area.height.saturating_sub(4));
    let width = 60.min(area.width);
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    let items: Vec<ListItem> = entries
        .iter()
        .map(|(owner, card)| {
            let castable = *owner == view.you && app.is_playable(card.id);
            let mut spans = vec![
                if castable {
                    "● ".green()
                } else {
                    "  ".into()
                },
                card_name(card),
                Span::raw(" "),
            ];
            spans.extend(mana_text(&card.cost, Style::new()));
            let whose = if *owner == view.you {
                "yours".to_string()
            } else {
                view.player_name(*owner).to_string()
            };
            spans.push(format!("  {whose}").dark_gray());
            ListItem::new(Line::from(spans))
        })
        .collect();
    let mut state = ListState::default().with_selected(Some(sel));
    frame.render_widget(Clear, popup);
    frame.render_stateful_widget(
        List::new(items)
            .block(panel(" Graveyards (● castable with flashback) ", true))
            .highlight_style(SELECTED),
        popup,
        &mut state,
    );
}

fn draw_choose_popup(
    frame: &mut Frame,
    area: Rect,
    reason: &str,
    options: &[CardView],
    (min, max): (usize, usize),
    sel: usize,
    marked: &[ObjectId],
) {
    let mut items: Vec<ListItem> = options
        .iter()
        .map(|card| {
            let mark = match marked.iter().position(|&id| id == card.id) {
                Some(i) => format!("{}. ", i + 1).green(),
                None if max > 1 => "   ".into(),
                None => "".into(),
            };
            let mut spans = vec![mark, card_name(card), Span::raw(" ")];
            spans.extend(mana_text(&card.cost, Style::new()));
            spans.push(format!("  {}", card.type_line).dark_gray());
            ListItem::new(Line::from(spans))
        })
        .collect();
    if min == 0 {
        items.push(ListItem::new(Line::from("(nothing)").dark_gray()));
    }
    let width = 64.min(area.width);
    // Roughly how many lines the reason wraps to; a spare line is harmless.
    let text_width = width.saturating_sub(2).max(1);
    let reason_height = (reason.chars().count() as u16).div_ceil(text_width).max(1);
    let reason = Paragraph::new(reason.to_string()).wrap(Wrap { trim: true });
    let height = (items.len() as u16 + reason_height + 3).min(area.height.saturating_sub(4));
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    let title = if max == 1 {
        " Choose a card ".to_string()
    } else {
        format!(" Choose up to {max} cards ")
    };
    let block = panel(title, true);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let [top, _, list] = Layout::vertical([
        Constraint::Length(reason_height),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(inner);
    frame.render_widget(reason, top);
    let mut state = ListState::default().with_selected(Some(sel));
    frame.render_stateful_widget(List::new(items).highlight_style(SELECTED), list, &mut state);
}

fn draw_cast_way_popup(
    frame: &mut Frame,
    area: Rect,
    view: &GameView,
    plays: &[PlayOption],
    sel: usize,
) {
    let name = plays
        .first()
        .and_then(|p| {
            view.hand_card(p.card)
                .or_else(|| view.permanent(p.card).map(|perm| &perm.card))
        })
        .map_or("it".to_string(), |c| c.name.clone());
    let items: Vec<ListItem> = plays
        .iter()
        .map(|play| {
            let spans = match (&play.ability, &play.way) {
                (Some(text), _) => mana_text(text, Style::new()),
                (None, way) => {
                    let how = match way {
                        None => "normally".to_string(),
                        Some(way) => format!("bringing {}", way.fetch_name),
                    };
                    let mut spans = vec![Span::raw(format!("Cast {how}  "))];
                    spans.extend(mana_text(&play.cost, Style::new()));
                    spans
                }
            };
            ListItem::new(Line::from(spans))
        })
        .collect();
    let height = (items.len() as u16 + 2).min(area.height.saturating_sub(4));
    let width = 56.min(area.width);
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    let mut state = ListState::default().with_selected(Some(sel));
    let verb = if plays.iter().any(|p| p.ability.is_some()) {
        "Activate"
    } else {
        "Cast"
    };
    frame.render_stateful_widget(
        List::new(items)
            .block(panel(format!(" {verb} {name} "), true))
            .highlight_style(SELECTED),
        popup,
        &mut state,
    );
}

fn draw_target_popup(
    frame: &mut Frame,
    area: Rect,
    view: &GameView,
    card: ObjectId,
    targets: &[Target],
    sel: usize,
) {
    let height = (targets.len() as u16 + 2).min(area.height.saturating_sub(4));
    let width = 50.min(area.width);
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    let name = view
        .hand_card(card)
        .map_or("spell".to_string(), |c| c.name.clone());
    let items: Vec<ListItem> = targets
        .iter()
        .map(|t| ListItem::new(view.describe_target(t)))
        .collect();
    let mut state = ListState::default().with_selected(Some(sel));
    frame.render_widget(Clear, popup);
    frame.render_stateful_widget(
        List::new(items)
            .block(panel(format!(" Target for {name} "), true))
            .highlight_style(SELECTED),
        popup,
        &mut state,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mana_symbols_are_colored() {
        let spans = mana_text("{2}{U}{U}", Style::new());
        let parts: Vec<_> = spans
            .iter()
            .map(|s| (s.content.as_ref(), s.style.fg))
            .collect();
        assert_eq!(
            parts,
            [
                ("{2}", None),
                ("{U}", Some(Tc::LightBlue)),
                ("{U}", Some(Tc::LightBlue)),
            ]
        );
        let spans = mana_text("Tap: Add {R}.", Style::new());
        let parts: Vec<_> = spans
            .iter()
            .map(|s| (s.content.as_ref(), s.style.fg))
            .collect();
        assert_eq!(
            parts,
            [
                ("Tap: Add ", None),
                ("{R}", Some(Tc::LightRed)),
                (".", None)
            ]
        );
    }
}
