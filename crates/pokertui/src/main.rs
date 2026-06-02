mod adapter;
mod app;
mod menu;
mod settings;
mod state;
mod ui;

use std::io::{self, Stdout};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::app::App;
use crate::settings::Settings;

const TICK: Duration = Duration::from_millis(50);

fn main() -> io::Result<()> {
    install_panic_hook();

    let mut terminal = setup_terminal()?;
    let result = run(&mut terminal);
    teardown_terminal(&mut terminal)?;
    result
}

fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend)
}

fn teardown_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original(info);
    }));
}

fn run(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    let mut app = App::new(Settings::load());
    loop {
        // While the settings menu is open it takes over the screen and pauses
        // the table; otherwise the normal table is drawn.
        if app.is_menu_open() {
            terminal.draw(|frame| menu::render_settings(frame, frame.area(), app.menu()))?;
        } else {
            let view = app.view();
            terminal.draw(|frame| ui::render(frame, &view))?;
        }

        if event::poll(TICK)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            if app.is_menu_open() {
                // The menu owns every key: Esc closes (and commits), the rest
                // navigate. `q` does not quit while the menu is open.
                app.handle_menu_key(key.code);
            } else {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('s') => app.open_menu(),
                    code => {
                        app.handle_key(code);
                    }
                }
            }
        }

        // Drive the bot seats; paced internally so a human can follow along.
        // A no-op while the menu is open.
        app.step();
    }
}
