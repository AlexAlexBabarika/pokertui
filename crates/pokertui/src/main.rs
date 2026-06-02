mod adapter;
mod app;
mod menu;
mod net;
mod net_adapter;
mod settings;
mod state;
mod table;
mod ui;

use std::io::{self, Stdout};
use std::time::Duration;

use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::app::App;
use crate::net::NetClient;
use crate::settings::Settings;
use crate::table::Table;

const TICK: Duration = Duration::from_millis(50);

/// Terminal Texas Hold'em. With no arguments, plays a local table against bots.
/// With `--join`, connects to a `poker-server` room.
#[derive(Parser, Debug)]
#[command(name = "pokertui")]
struct Args {
    /// Connect to a server at this address, e.g. 127.0.0.1:4000.
    #[arg(long)]
    join: Option<String>,
    /// Display name to join with.
    #[arg(long, default_value = "you")]
    name: String,
}

fn main() -> io::Result<()> {
    let args = Args::parse();
    install_panic_hook();

    let mut terminal = setup_terminal()?;
    let result = match &args.join {
        // Networked play: the render loop is mode-agnostic, driving a
        // `Box<dyn Table>` and never opening the local settings menu.
        Some(addr) => {
            let mut client: Box<dyn Table> = Box::new(NetClient::connect(addr, &args.name));
            run_net(&mut terminal, client.as_mut())
        }
        // Local play vs bots: keeps the settings-menu wiring intact.
        None => run_local(&mut terminal),
    };
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

/// The local table loop, with the settings menu overlay.
fn run_local(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
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

/// The mode-agnostic render loop used for networked play.
fn run_net(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    table: &mut dyn Table,
) -> io::Result<()> {
    loop {
        let view = table.view();
        terminal.draw(|frame| ui::render(frame, &view))?;

        if event::poll(TICK)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                code => {
                    table.handle_key(code);
                }
            }
        }

        table.step();
    }
}
