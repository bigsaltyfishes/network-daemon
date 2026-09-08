//! network-manager-tui: an nmtui-shaped terminal UI for network-daemon.

mod app;
mod client;

use std::{
    io::{self, Stdout},
    path::PathBuf,
    time::Duration,
};

use app::App;
use client::{ClientError, DaemonClient};
use crossterm::{
    ExecutableCommand,
    event::{Event, KeyEventKind, poll, read},
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};
use ratatui::Terminal;

fn main() {
    if let Err(error) = run() {
        eprintln!("network-manager-tui: {error}");
        std::process::exit(1);
    }
}

/// Run the nmtui-style activity picker and its child screens.
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let socket =
        PathBuf::from(std::env::var("NETWORK_DAEMON_SOCK").unwrap_or_else(
            |_| "/var/run/network-daemon/network-daemon.sock".to_string(),
        ));

    let mut client = match DaemonClient::connect(&socket) {
        Ok(client) => client,
        Err(error) => {
            let hint = match &error {
                ClientError::Connect(io_error)
                    if io_error.kind()
                        == std::io::ErrorKind::PermissionDenied =>
                {
                    " Hint: add your user to the 'network' group, then log in again."
                }
                _ => " Hint: is network-daemon running?",
            };
            return Err(format!(
                "failed to connect to daemon at {}: {error}.{hint}",
                socket.display()
            )
            .into());
        }
    };
    client.set_read_timeout(Duration::from_secs(5));

    let mut app = App::default();
    app.refresh_interfaces(&mut client);

    let mut stdout: Stdout = io::stdout();
    enable_raw_mode()?;
    stdout.execute(EnterAlternateScreen)?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = event_loop(&mut terminal, &mut app, &mut client);

    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    result
}

fn event_loop(
    terminal: &mut Terminal<ratatui::backend::CrosstermBackend<Stdout>>,
    app: &mut App,
    client: &mut DaemonClient,
) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        app.tick(client);
        terminal.draw(|frame| app.draw(frame))?;

        if poll(Duration::from_millis(100))?
            && let Event::Key(key) = read()?
            && key.kind == KeyEventKind::Press
            && app.handle_key(key.code, client)
        {
            break;
        }
    }
    Ok(())
}
